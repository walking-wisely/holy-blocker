mod backoff;
mod frame;
mod link;
mod message;
mod status;

use std::path::PathBuf;

use link::{Link, SendError};
use message::Outbound;
use status::DaemonStatus;
use tauri::State;

const SOCKET_RELATIVE: &str = "Library/Application Support/holy-blocker/panel.sock";

fn socket_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(SOCKET_RELATIVE))
}

fn send(link: &Link, request: Outbound) -> Result<(), &'static str> {
    link.send(request).map_err(|e| match e {
        SendError::NotConnected => "not-connected",
        SendError::Io => "send-failed",
    })
}

#[tauri::command]
fn daemon_status(link: State<Link>) -> DaemonStatus {
    status::snapshot(link.state())
}

#[tauri::command]
fn request_disarm(link: State<Link>) -> Result<(), &'static str> {
    send(&link, Outbound::RequestDisarm)
}

#[tauri::command]
fn confirm_disarm(link: State<Link>) -> Result<(), &'static str> {
    send(&link, Outbound::ConfirmDisarm)
}

#[tauri::command]
fn cancel_disarm(link: State<Link>) -> Result<(), &'static str> {
    send(&link, Outbound::CancelDisarm)
}

#[tauri::command]
fn raise_protection(link: State<Link>) -> Result<(), &'static str> {
    send(&link, Outbound::RaiseProtection)
}

#[tauri::command]
fn list_apps(link: State<Link>) -> Result<(), &'static str> {
    send(&link, Outbound::ListApps)
}

pub fn run() {
    let path = socket_path().unwrap_or_else(|| PathBuf::from("/nonexistent/panel.sock"));
    tauri::Builder::default()
        .manage(Link::start(path, backoff::Backoff::standard()))
        .invoke_handler(tauri::generate_handler![
            daemon_status,
            request_disarm,
            confirm_disarm,
            cancel_disarm,
            raise_protection,
            list_apps
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Holy Blocker panel");
}
