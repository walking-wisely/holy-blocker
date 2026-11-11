mod status;

use status::DaemonStatus;

#[tauri::command]
fn daemon_status() -> DaemonStatus {
    status::current()
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![daemon_status])
        .run(tauri::generate_context!())
        .expect("error while running the Holy Blocker panel");
}
