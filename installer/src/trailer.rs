//! Where the payload sits in a setup exe (shared by the installer and the `pack` build tool):
//!
//!     [exe][zip][u64 zip length, little-endian]["SSPAYLD1"]  [0..7 zero bytes][certificate table]
//!
//! The last part is there when the exe is Authenticode-signed after the payload was appended: signtool puts
//! the certificate table (the PE security directory) at the end of the file, aligned to 8 bytes with zero
//! padding. Authenticode hashes everything else, the payload included, so the signature covers it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const MAGIC: &[u8; 8] = b"SSPAYLD1";

fn read_at<const N: usize>(f: &mut File, at: u64) -> Option<[u8; N]> {
    let mut b = [0u8; N];
    f.seek(SeekFrom::Start(at)).ok()?;
    f.read_exact(&mut b).ok()?;
    Some(b)
}

/// (file offset, size) of the PE security directory (data directory 4), if the file is a PE image.
fn security_dir(f: &mut File) -> Option<(u64, u64)> {
    if &read_at::<2>(f, 0)? != b"MZ" {
        return None;
    }
    let pe = u32::from_le_bytes(read_at::<4>(f, 0x3C)?) as u64;
    if &read_at::<4>(f, pe)? != b"PE\0\0" {
        return None;
    }
    let opt = pe + 24;
    // NumberOfRvaAndSizes and the data directories: PE32+ / PE32
    let (count_at, dirs_at) = match u16::from_le_bytes(read_at::<2>(f, opt)?) {
        0x20B => (opt + 108, opt + 112),
        0x10B => (opt + 92, opt + 96),
        _ => return None,
    };
    if u32::from_le_bytes(read_at::<4>(f, count_at)?) < 5 {
        return None;
    }
    let d = read_at::<8>(f, dirs_at + 4 * 8)?;
    let off = u32::from_le_bytes(d[..4].try_into().ok()?) as u64;
    let len = u32::from_le_bytes(d[4..].try_into().ok()?) as u64;
    Some((off, len))
}

/// Where the content before a signature ends: the start of the certificate table when there is one and it
/// runs to the end of the file, else the file size.
pub fn logical_end(f: &mut File, size: u64) -> u64 {
    match security_dir(f) {
        Some((off, len)) if len > 0 && off.checked_add(len) == Some(size) => off,
        _ => size,
    }
}

/// The file is Authenticode-signed (has a certificate table at its end).
pub fn is_signed(exe: &Path) -> bool {
    let Ok(mut f) = File::open(exe) else { return false };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    logical_end(&mut f, size) < size
}

/// Offset and length of the payload zip in `exe`, if it has one.
pub fn locate(exe: &Path) -> Option<(u64, u64)> {
    let mut f = File::open(exe).ok()?;
    let size = f.metadata().ok()?.len();
    let end = logical_end(&mut f, size);
    // the 16-byte trailer, followed by up to 7 zero bytes when signed
    let back = end.min(16 + 7) as usize;
    if back < 16 {
        return None;
    }
    let mut buf = vec![0u8; back];
    f.seek(SeekFrom::Start(end - back as u64)).ok()?;
    f.read_exact(&mut buf).ok()?;
    for pad in 0..=(back - 16).min(7) {
        let t_end = back - pad;
        if pad > 0 && buf[t_end] != 0 {
            break; // padding is zeros only
        }
        let t = &buf[t_end - 16..t_end];
        if &t[8..] == MAGIC {
            let len = u64::from_le_bytes(t[..8].try_into().ok()?);
            let offset = (end - pad as u64).checked_sub(16 + len)?;
            return Some((offset, len));
        }
    }
    None
}
