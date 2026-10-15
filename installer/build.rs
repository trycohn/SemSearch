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
}
