//! Frame-pacing probe: a windowed D3D11 app (DXGI flip-discard, vsync, waitable swap chain,
//! max frame latency 1) that renders continuously and records present-to-present intervals.
//! Prints one JSON line with frame-time stats; `--iters N` adds a per-pixel ALU loop to mimic
//! a game's GPU cost per frame (0 = desktop/browser-like light frame).
//!
//! Build: cargo build --release (in tools/frameprobe), copy target/release/frameprobe.exe to tools/.
//!   frameprobe.exe [--secs 20] [--warmup 1.5] [--iters 0] [--vsync 1] [--w 1280] [--h 720] [--wait 1] [--buffers 3] [--raw file]
use windows::core::{w, Interface, Result, PCSTR};
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCompile, D3DCOMPILE_OPTIMIZATION_LEVEL3};
use windows::Win32::Graphics::Direct3D::{ID3DBlob, ID3DInclude, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_DRIVER_TYPE_HARDWARE};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED};
use windows::Win32::System::Threading::WaitForSingleObjectEx;
use windows::Win32::UI::WindowsAndMessaging::*;

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_DESTROY {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn arg<T: std::str::FromStr>(name: &str, default: T) -> T {
    let a: Vec<String> = std::env::args().collect();
    for i in 0..a.len() {
        if a[i] == name && i + 1 < a.len() {
            if let Ok(v) = a[i + 1].parse() {
                return v;
            }
        }
    }
    default
}

fn qpc() -> i64 {
    let mut t = 0i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut t);
    }
    t
}

fn compile(src: &str, target: &[u8]) -> Result<Vec<u8>> {
    unsafe {
        let mut code: Option<ID3DBlob> = None;
        let mut err: Option<ID3DBlob> = None;
        let r = D3DCompile(
            src.as_ptr() as _, src.len(), PCSTR::null(), None, None::<&ID3DInclude>,
            PCSTR(b"main\0".as_ptr()), PCSTR(target.as_ptr()),
            D3DCOMPILE_OPTIMIZATION_LEVEL3, 0, &mut code, Some(&mut err),
        );
        if let Err(e) = r {
            if let Some(err) = err {
                let m = std::slice::from_raw_parts(err.GetBufferPointer() as *const u8, err.GetBufferSize());
                eprintln!("{}", String::from_utf8_lossy(m));
            }
            return Err(e);
        }
        let b = code.unwrap();
        Ok(std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()).to_vec())
    }
}

fn timing() -> Option<DWM_TIMING_INFO> {
    let mut ti = DWM_TIMING_INFO { cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
    unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut ti).ok().map(|_| ti) }
}

fn main() -> Result<()> {
    let secs: f64 = arg("--secs", 20.0);
    let warm: f64 = arg("--warmup", 1.5);
    let iters: u32 = arg("--iters", 0);
    let vsync: u32 = arg("--vsync", 1);
    let w: i32 = arg("--w", 1280);
    let h: i32 = arg("--h", 720);
    let raw: String = arg("--raw", String::new());
    let wait: u32 = arg("--wait", 1);
    let buffers: u32 = arg("--buffers", 3);
    unsafe {
        // With the monitor asleep DWM falls back to a ~64 Hz timer and vsync numbers are meaningless
        SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
        let mut freq = 0i64;
        QueryPerformanceFrequency(&mut freq)?;
        let hinst = GetModuleHandleW(None)?;
        let cls = w!("SemSearchFrameProbe");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: cls,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let style = WS_OVERLAPPEDWINDOW;
        let mut rc = RECT { left: 0, top: 0, right: w, bottom: h };
        AdjustWindowRect(&mut rc, style, false)?;
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST, cls, w!("SemSearch frame probe"), style | WS_VISIBLE,
            80, 80, rc.right - rc.left, rc.bottom - rc.top, None, None, Some(hinst.into()), None,
        )?;
        let _ = SetForegroundWindow(hwnd);

        let mut device = None;
        let mut ctx = None;
        D3D11CreateDevice(
            None::<&IDXGIAdapter>, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION,
            Some(&mut device), None, Some(&mut ctx),
        )?;
        let device: ID3D11Device = device.unwrap();
        let ctx: ID3D11DeviceContext = ctx.unwrap();
        let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: w as u32,
            Height: h as u32,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: buffers,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
            Flags: DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT.0 as u32,
            ..Default::default()
        };
        let sc1 = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None::<&IDXGIOutput>)?;
        let sc2: IDXGISwapChain2 = sc1.cast()?;
        sc2.SetMaximumFrameLatency(1)?;
        let waitable = sc2.GetFrameLatencyWaitableObject();
        let tex: ID3D11Texture2D = sc1.GetBuffer(0)?;
        let mut rtv = None;
        device.CreateRenderTargetView(&tex, None, Some(&mut rtv))?;
        let rtv: ID3D11RenderTargetView = rtv.unwrap();

        let vs_src = "float4 main(uint id : SV_VertexID) : SV_Position { float2 uv = float2((id << 1) & 2, id & 2); \
                      return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1); }";
        let ps_src = format!(
            "cbuffer F : register(b0) {{ float t; float3 pad; }} \
             float4 main(float4 pos : SV_Position) : SV_Target {{ float2 p = pos.xy * 0.003; float a = t; \
             [loop] for (int i = 0; i < {iters}; i++) {{ a = sin(a + p.x * (float)i) * cos(a - p.y); }} \
             return float4(frac(a * 0.1 + p.x + t), 0.35, 0.55, 1); }}"
        );
        let vsb = compile(vs_src, b"vs_5_0\0")?;
        let psb = compile(&ps_src, b"ps_5_0\0")?;
        let mut vs = None;
        device.CreateVertexShader(&vsb, None::<&ID3D11ClassLinkage>, Some(&mut vs))?;
        let vs: ID3D11VertexShader = vs.unwrap();
        let mut ps = None;
        device.CreatePixelShader(&psb, None::<&ID3D11ClassLinkage>, Some(&mut ps))?;
        let ps: ID3D11PixelShader = ps.unwrap();
        let cbd = D3D11_BUFFER_DESC {
            ByteWidth: 16,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        };
        let mut cb = None;
        device.CreateBuffer(&cbd, None, Some(&mut cb))?;
        let cb: ID3D11Buffer = cb.unwrap();
        let vp = D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: w as f32, Height: h as f32, MinDepth: 0.0, MaxDepth: 1.0 };

        let refresh_ms = match timing() {
            Some(t) if t.qpcRefreshPeriod > 0 => t.qpcRefreshPeriod as f64 * 1000.0 / freq as f64,
            _ => 0.0,
        };
        let mut times: Vec<i64> = Vec::with_capacity(16384);
        let (mut dwm0, mut dwm1): (Option<DWM_TIMING_INFO>, Option<DWM_TIMING_INFO>) = (None, None);
        let (mut last_pc, mut last_prc, mut have_last) = (0u32, 0u32, false);
        let (mut glitches, mut refreshes) = (0u64, 0u64);
        let mut fg = false;
        let mut stat_err = 0i32;
        let mut msg = MSG::default();
        let mut frame = 0u64;
        let start = qpc();
        'outer: loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'outer;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if vsync > 0 && wait > 0 {
                WaitForSingleObjectEx(waitable, 500, true);
            }
            let t = qpc();
            let el = (t - start) as f64 / freq as f64;
            if el > warm + secs {
                dwm1 = timing();
                break;
            }
            let measuring = el >= warm;
            if measuring {
                if dwm0.is_none() {
                    dwm0 = timing();
                    fg = GetForegroundWindow() == hwnd;
                }
                times.push(t);
            }
            let cbdata = [frame as f32 * 0.004f32, 0.0, 0.0, 0.0];
            ctx.UpdateSubresource(&cb, 0, None, cbdata.as_ptr() as _, 0, 0);
            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None::<&ID3D11DepthStencilView>);
            ctx.RSSetViewports(Some(&[vp]));
            ctx.ClearRenderTargetView(&rtv, &[0.1, 0.1, 0.1, 1.0]);
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&vs, None);
            ctx.PSSetShader(&ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(cb.clone())]));
            ctx.Draw(3, 0);
            let _ = sc1.Present(vsync, DXGI_PRESENT(0));
            frame += 1;
            if measuring {
                let mut fs = DXGI_FRAME_STATISTICS::default();
                let fr = sc1.GetFrameStatistics(&mut fs);
                if let Err(e) = &fr {
                    stat_err = e.code().0;
                }
                if fr.is_ok() && (!have_last || fs.PresentCount != last_pc) {
                    if have_last && fs.PresentCount > last_pc {
                        let dp = (fs.PresentCount - last_pc) as u64;
                        let dr = fs.PresentRefreshCount.wrapping_sub(last_prc) as u64;
                        refreshes += dr;
                        if dr > dp {
                            glitches += dr - dp;
                        }
                    }
                    last_pc = fs.PresentCount;
                    last_prc = fs.PresentRefreshCount;
                    have_last = true;
                }
            }
        }
        let _ = DestroyWindow(hwnd);

        let ints: Vec<f64> = times.windows(2).map(|p| (p[1] - p[0]) as f64 * 1000.0 / freq as f64).collect();
        let n = ints.len().max(1);
        let mut s = ints.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let pct = |q: f64| if s.is_empty() { 0.0 } else { s[((q * (s.len() as f64 - 1.0)).round() as usize).min(s.len() - 1)] };
        let mean = ints.iter().sum::<f64>() / n as f64;
        let refresh = if refresh_ms > 0.0 { refresh_ms } else { pct(0.5) };
        let hitch = ints.iter().filter(|&&x| x > 1.5 * refresh).count();
        let big = ints.iter().filter(|&&x| x > 2.5 * refresh).count();
        let missed: f64 = ints.iter().map(|&x| ((x / refresh).round() - 1.0).max(0.0)).sum();
        let d = |f: fn(&DWM_TIMING_INFO) -> u64| match (&dwm0, &dwm1) {
            (Some(a), Some(b)) => f(b).wrapping_sub(f(a)) as i64,
            _ => -1,
        };
        println!(
            "{{\"frames\": {}, \"fps\": {:.1}, \"refresh_ms\": {:.3}, \"mean_ms\": {:.3}, \"p50_ms\": {:.3}, \"p95_ms\": {:.3}, \
             \"p99_ms\": {:.3}, \"max_ms\": {:.2}, \"hitch_pct\": {:.2}, \"big_hitch_pct\": {:.2}, \"missed_refresh_pct\": {:.2}, \
             \"dxgi_glitches\": {}, \"dxgi_refreshes\": {}, \"dwm_refresh\": {}, \"dwm_frames\": {}, \"dwm_missed\": {}, \
             \"dwm_dropped\": {}, \"dwm_late\": {}, \"iters\": {}, \"foreground\": {}, \"stat_err\": \"{:#x}\"}}",
            ints.len(), 1000.0 / mean, refresh, mean, pct(0.5), pct(0.95), pct(0.99), pct(1.0),
            100.0 * hitch as f64 / n as f64, 100.0 * big as f64 / n as f64,
            100.0 * missed / (missed + n as f64),
            glitches, refreshes,
            d(|t| t.cRefresh), d(|t| t.cFrame), d(|t| t.cFramesMissed), d(|t| t.cFramesDropped), d(|t| t.cFramesLate),
            iters, fg, stat_err
        );
        if !raw.is_empty() {
            let body: Vec<String> = ints.iter().map(|x| format!("{:.3}", x)).collect();
            let _ = std::fs::write(&raw, body.join("\n"));
        }
    }
    Ok(())
}
