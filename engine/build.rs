//! Version resource of the exe (Properties -> Details): product "SemSearch" and the release version from
//! shell/tauri.conf.json, as in SemSearch.exe and the installer; code signing (.github/signpath) checks both.
//! The engine's own version (Cargo.toml, ops::VERSION) is its protocol version and stays apart.

fn main() {
    let conf = "../shell/tauri.conf.json";
    println!("cargo:rerun-if-changed={conf}");
    let text = std::fs::read_to_string(conf).expect("shell/tauri.conf.json");
    let json: serde_json::Value = serde_json::from_str(&text).expect("shell/tauri.conf.json is not JSON");
    let version = json["version"].as_str().expect("shell/tauri.conf.json: no version");
    // "1.2.3", "1.2.3-beta.1", "1.2.3+build": the numbers go into FILEVERSION, the whole string into the texts
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let nums: Vec<u16> = core.split('.').filter_map(|n| n.parse().ok()).collect();
    assert!(
        nums.len() == 3 && version.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)),
        "shell/tauri.conf.json: version {version:?} is not semver"
    );
    let (a, b, c) = (nums[0], nums[1], nums[2]);

    // language neutral (0000, Unicode), like tauri-build writes it for SemSearch.exe
    let rc = format!(
        r#"#pragma code_page(65001)
1 VERSIONINFO
FILEVERSION {a},{b},{c},0
PRODUCTVERSION {a},{b},{c},0
FILEFLAGSMASK 0x3f
FILEFLAGS 0x0
FILEOS 0x40004
FILETYPE 0x1
FILESUBTYPE 0x0
{{
  BLOCK "StringFileInfo"
  {{
    BLOCK "000004b0"
    {{
      VALUE "FileDescription", "SemSearch file-name engine"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "semsearch-engine"
      VALUE "OriginalFilename", "semsearch-engine.exe"
      VALUE "ProductName", "SemSearch"
      VALUE "ProductVersion", "{version}"
    }}
  }}
  BLOCK "VarFileInfo"
  {{
    VALUE "Translation", 0x0, 0x04b0
  }}
}}
"#
    );
    let path = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("semsearch-engine.rc");
    std::fs::write(&path, rc).expect("write semsearch-engine.rc");
    // rc.exe from the Windows SDK (the MSVC build tools have it), linked into the exe
    embed_resource::compile(&path, embed_resource::NONE).manifest_required().expect("version resource");
}
