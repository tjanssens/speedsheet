//! Tauri shell: exposes speedsheet-core to the web UI. All spreadsheet logic lives in the core crate.

use std::sync::Mutex;

use serde::Serialize;
use speedsheet_core::Workbook;
use tauri::State;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use tauri::{Emitter, Manager};

pub mod api;

pub struct AppState {
    wb: Mutex<Workbook>,
    pending_open: Mutex<Option<String>>,
}

type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Every workbook operation goes through this one command; see `api::dispatch`.
/// It is async so slow operations (open, save, sort) never block the UI thread.
#[tauri::command]
async fn call(state: State<'_, AppState>, cmd: String, args: serde_json::Value) -> Res<serde_json::Value> {
    let mut wb = state.wb.lock().map_err(|_| "internal error: state lock poisoned".to_string())?;
    api::dispatch(&mut wb, &cmd, args)
}

/// Opens the system print dialog for the window (which also offers "Save as PDF").
#[tauri::command]
fn print_page(window: tauri::WebviewWindow) -> Res<()> {
    window.print().map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: String,
    os: String,
    os_version: String,
    arch: String,
    feedback_configured: bool,
    /// Release builds get an updater public key injected by CI; dev builds have none.
    updates_configured: bool,
    repo: String,
}

const REPO: &str = "tjanssens/speedsheet";

fn feedback_url() -> Option<&'static str> {
    option_env!("SPEEDSHEET_FEEDBACK_URL").filter(|s| !s.is_empty())
}

fn app_info_value(app: &tauri::AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        os: tauri_plugin_os::platform().to_string(),
        os_version: tauri_plugin_os::version().to_string(),
        arch: tauri_plugin_os::arch().to_string(),
        feedback_configured: feedback_url().is_some(),
        updates_configured: app
            .config()
            .plugins
            .0
            .get("updater")
            .and_then(|u| u.get("pubkey"))
            .and_then(|k| k.as_str())
            .is_some_and(|k| !k.trim().is_empty()),
        repo: REPO.into(),
    }
}

#[tauri::command]
fn app_info(app: tauri::AppHandle) -> AppInfo {
    app_info_value(&app)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackSent {
    issue_url: String,
    number: u64,
}

/// Send feedback to the feedback proxy, which files a GitHub issue. The proxy holds the GitHub
/// token so it never ships inside the app. Fails when no proxy is configured; the UI then falls
/// back to opening a prefilled GitHub issue page.
#[tauri::command]
async fn send_feedback(app: tauri::AppHandle, message: String, contact: Option<String>) -> Res<FeedbackSent> {
    let url = feedback_url().ok_or("feedback service not configured")?;
    let ai = app_info_value(&app);
    let payload = serde_json::json!({
        "message": message,
        "contact": contact.unwrap_or_default(),
        "version": ai.version,
        "os": ai.os,
        "osVersion": ai.os_version,
        "arch": ai.arch,
    });
    tauri::async_runtime::spawn_blocking(move || {
        let resp: serde_json::Value = ureq::post(url)
            .header("User-Agent", "SpeedSheet")
            .send_json(&payload)
            .map_err(|e| format!("could not reach feedback service: {e}"))?
            .body_mut()
            .read_json()
            .map_err(err)?;
        Ok(FeedbackSent {
            issue_url: resp["url"].as_str().unwrap_or_default().to_string(),
            number: resp["number"].as_u64().unwrap_or_default(),
        })
    })
    .await
    .map_err(err)?
}

/// A file passed on the command line or via "Open with" before the UI was ready.
#[tauri::command]
fn take_pending_open(state: State<'_, AppState>) -> Option<String> {
    state.pending_open.lock().ok().and_then(|mut p| p.take())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let first_arg = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState { wb: Mutex::new(Workbook::new()), pending_open: Mutex::new(first_arg) })
        .invoke_handler(tauri::generate_handler![call, print_page, app_info, send_feedback, take_pending_open])
        .build(tauri::generate_context!())
        .expect("error while building SpeedSheet");

    app.run(|handle, event| {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if let tauri::RunEvent::Opened { urls } = &event {
            if let Some(path) = urls.iter().find_map(|u| u.to_file_path().ok()) {
                let p = path.display().to_string();
                if let Some(state) = handle.try_state::<AppState>() {
                    if let Ok(mut pending) = state.pending_open.lock() {
                        *pending = Some(p.clone());
                    }
                }
                let _ = handle.emit("open-file", p);
            }
        }
        let _ = (&handle, &event);
    });
}
