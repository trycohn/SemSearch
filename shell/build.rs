fn main() {
    // App commands get ACL permissions "allow-<command>" so that remote (backend-served) pages can be
    // granted exactly these commands in capabilities/*.json.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "win_minimize",
            "win_toggle_maximize",
            "win_hide",
            "show_main",
            "show_settings",
            "tray_hide",
            "quit_app",
            "pick_folder",
            "get_autostart",
            "set_autostart",
            "get_hotkey",
            "set_hotkey",
            "page_ready",
        ]),
    ))
    .expect("tauri-build failed");
}
