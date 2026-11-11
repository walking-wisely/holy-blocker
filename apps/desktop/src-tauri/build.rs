const COMMANDS: &[&str] = &[
    "daemon_status",
    "request_disarm",
    "confirm_disarm",
    "cancel_disarm",
    "raise_protection",
    "list_apps",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
