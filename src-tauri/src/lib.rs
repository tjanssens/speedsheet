//! Tauri shell: exposes speedsheet-core to the web UI. All spreadsheet logic lives in the core crate.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use speedsheet_core::shared::Hub;
use tauri::{Emitter, Manager, State};

pub mod api;

pub struct AppState {
    /// Which document each window shows; windows on the same file share one document.
    hub: Mutex<Hub>,
    /// A file a window should open once its UI is ready, by window label.
    pending_open: Mutex<HashMap<String, String>>,
    next_window: AtomicUsize,
}

type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Every workbook operation goes through this one command; see `api::dispatch`.
/// It is async so slow operations (open, save, sort) never block the UI thread.
#[tauri::command]
async fn call(
    app: tauri::AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    cmd: String,
    args: serde_json::Value,
) -> Res<serde_json::Value> {
    let reply = api::dispatch_view(&state.hub, window.label(), &cmd, args)?;
    send_events(&app, reply.events);
    Ok(reply.value)
}

/// Deliver messages for other windows (edits, cursors) produced by a command.
fn send_events(app: &tauri::AppHandle, events: Vec<api::Event>) {
    for ev in events {
        let _ = app.emit_to(ev.to.as_str(), ev.name, ev.payload);
    }
}

/// Bring another SpeedSheet window to the front.
#[tauri::command]
fn focus_window(app: tauri::AppHandle, label: String) -> Res<()> {
    let w = app.get_webview_window(&label).ok_or("that window is closed")?;
    let _ = w.unminimize();
    let _ = w.show();
    w.set_focus().map_err(err)
}

/// Open a new, empty window (also used when the app is started again).
#[tauri::command]
fn new_window(app: tauri::AppHandle) -> Res<()> {
    create_window(&app, None)
}

fn create_window(app: &tauri::AppHandle, open: Option<String>) -> Res<()> {
    let state = app.state::<AppState>();
    let label = format!("doc-{}", state.next_window.fetch_add(1, Ordering::Relaxed));
    if let Some(path) = open {
        state.pending_open.lock().map_err(err)?.insert(label.clone(), path);
    }
    tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::App("index.html".into()))
        .title("SpeedSheet")
        .inner_size(1280.0, 800.0)
        .min_inner_size(640.0, 400.0)
        .build()
        .map_err(err)?;
    Ok(())
}

/// Open a file that the system handed to the app (a second launch, "Open with", a Finder
/// double click): in a window that shows nothing yet, otherwise in a new window. When the file
/// is already open, that window's UI asks whether to switch to the existing window.
fn open_from_system(app: &tauri::AppHandle, path: Option<String>) {
    let state = app.state::<AppState>();
    let idle = path.as_ref().and_then(|_| {
        let mut hub = state.hub.lock().ok()?;
        app.webview_windows().into_keys().find(|label| {
            if !hub.has_view(label) {
                return true;
            }
            let doc = hub.doc(label);
            let blank = doc.try_lock().is_ok_and(|wb| wb.path.is_none() && !wb.modified);
            blank && hub.peers_of(label).is_empty()
        })
    });
    match (idle, path) {
        (Some(label), Some(path)) => {
            if let Ok(mut pending) = state.pending_open.lock() {
                pending.insert(label.clone(), path.clone());
            }
            let _ = app.emit_to(label.as_str(), "open-file", path);
            if let Some(w) = app.get_webview_window(&label) {
                let _ = w.set_focus();
            }
        }
        (_, path) => {
            let app = app.clone();
            // Build the window off the event-loop callback.
            tauri::async_runtime::spawn(async move {
                let _ = create_window(&app, path);
            });
        }
    }
}

/// The file argument of a command line, resolved against the directory it was started in.
fn file_arg(args: &[String], cwd: Option<&Path>) -> Option<String> {
    let a = args.iter().skip(1).find(|a| !a.starts_with('-'))?;
    let p = PathBuf::from(a);
    let p = match cwd {
        Some(dir) if p.is_relative() => dir.join(p),
        _ => p,
    };
    Some(p.display().to_string())
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

/// A file this window should open: passed on the command line or via "Open with" before the
/// UI was ready.
#[tauri::command]
fn take_pending_open(window: tauri::Window, state: State<'_, AppState>) -> Option<String> {
    state.pending_open.lock().ok().and_then(|mut p| p.remove(window.label()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    let pending: HashMap<String, String> = file_arg(&args, None).map(|p| ("main".to_string(), p)).into_iter().collect();
    let app = tauri::Builder::default()
        // Must come first: a second launch hands its file to this process instead of starting
        // another one, so both windows can share the document.
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            open_from_system(app, file_arg(&argv, Some(Path::new(&cwd))));
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState { hub: Mutex::new(Hub::new()), pending_open: Mutex::new(pending), next_window: AtomicUsize::new(2) })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                let state = window.state::<AppState>();
                if let Ok(reply) = api::dispatch_view(&state.hub, window.label(), "close_view", serde_json::Value::Null) {
                    send_events(window.app_handle(), reply.events);
                }
                if let Ok(mut pending) = state.pending_open.lock() {
                    pending.remove(window.label());
                };
            }
        })
        .invoke_handler(tauri::generate_handler![
            call,
            print_page,
            app_info,
            send_feedback,
            take_pending_open,
            focus_window,
            new_window
        ])
        .build(tauri::generate_context!())
        .expect("error while building SpeedSheet");

    app.run(|handle, event| {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        if let tauri::RunEvent::Opened { urls } = &event {
            if let Some(path) = urls.iter().find_map(|u| u.to_file_path().ok()) {
                open_from_system(handle, Some(path.display().to_string()));
            }
        }
        let _ = (&handle, &event);
    });
}
