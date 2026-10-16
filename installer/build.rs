//! Tauri build + the program version: SEMSEARCH_VERSION (and the exe's own version) come from
//! shell/tauri.conf.json, so the installer always says the version of what it installs.

fn main() {
    let conf = "../shell/tauri.conf.json";
    println!("cargo:rerun-if-changed={conf}");
    let text = std::fs::read_to_string(conf).expect("shell/tauri.conf.json");
    let json: serde_json::Value = serde_json::from_str(&text).expect("shell/tauri.conf.json is not JSON");
    let version = json["version"].as_str().expect("shell/tauri.conf.json: no version").to_string();
    println!("cargo:rustc-env=SEMSEARCH_VERSION={version}");

    // merged into installer/tauri.conf.json by tauri-build here and by generate_context!() at compile time
    let patch = serde_json::json!({ "version": version }).to_string();
    std::env::set_var("TAURI_CONFIG", &patch);
    println!("cargo:rustc-env=TAURI_CONFIG={patch}");

    // asInvoker: never ask for administrator rights (the name contains "setup"); common controls v6 for the
    // folder dialog
    let manifest = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0"
        processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" />
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}" />
    </application>
  </compatibility>
</assembly>
"#;
    let windows = tauri_build::WindowsAttributes::new().app_manifest(manifest);
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows)).expect("tauri-build failed");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        describe_as_setup();
    }
}

/// Version resource: product name "SemSearch" (productName in tauri.conf.json; code signing wants the project's
/// name on every file), file description "SemSearch Setup" (Task Manager, Properties). tauri-build writes both
/// from productName, so its resource.rc is compiled again, with the description changed, into the same
/// OUT_DIR\resource.lib it has already passed to the linker.
fn describe_as_setup() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let rc = out.join("resource.rc");
    let lib = out.join("resource.lib");
    let text = std::fs::read_to_string(&rc).expect("OUT_DIR\\resource.rc of tauri-build");
    let from = r#"VALUE "FileDescription", "SemSearch""#;
    assert!(
        text.matches(from).count() == 1 && lib.is_file(),
        "tauri-build's resource has changed: no single {from} in {} or no {}",
        rc.display(),
        lib.display()
    );
    std::fs::write(&rc, text.replace(from, r#"VALUE "FileDescription", "SemSearch Setup""#)).expect("resource.rc");
    // the same rc.exe embed-resource uses for tauri-build ($RC, else the Windows SDK's)
    let exe = std::env::var_os("RC")
        .map(std::path::PathBuf::from)
        .or_else(|| embed_resource::find_windows_sdk_tool("rc.exe"))
        .unwrap_or_else(|| "rc.exe".into());
    let ok = std::process::Command::new(&exe)
        .arg("/fo")
        .arg(&lib)
        .arg("/I")
        .arg(&out)
        .arg(&rc)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "{} could not compile {}", exe.display(), rc.display());
}
