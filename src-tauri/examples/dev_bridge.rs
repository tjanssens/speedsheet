//! Browser dev bridge: serves the same API as the desktop app over HTTP on 127.0.0.1:1421,
//! so the UI can be developed and tested in a normal browser (`npm run dev`, then open
//! http://localhost:1420). Run with: `cargo run -p speedsheet --example dev_bridge -- [file]`.
//! Only for local development; it is not part of the app.
//!
//! Each browser tab is a window: it sends its id as `?view=`. Messages for a tab (edits and
//! cursors from other tabs on the same file) are fetched with `POST /_events?view=`.

use std::collections::HashMap;
use std::sync::Mutex;

use speedsheet_core::io;
use speedsheet_core::shared::Hub;

fn main() {
    let hub = Mutex::new(Hub::new());
    // The file on the command line is what the first tab sees.
    let mut initial = std::env::args().nth(1).map(|p| io::open(p.as_ref()).expect("open file"));
    let mut queues: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    let server = tiny_http::Server::http("127.0.0.1:1421").expect("bind 127.0.0.1:1421");
    eprintln!("SpeedSheet dev bridge on http://127.0.0.1:1421");
    let cors = [
        tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap(),
        tiny_http::Header::from_bytes("Access-Control-Allow-Headers", "content-type").unwrap(),
        tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
    ];
    for mut req in server.incoming_requests() {
        if req.method() == &tiny_http::Method::Options {
            let mut r = tiny_http::Response::empty(204);
            for h in &cors {
                r.add_header(h.clone());
            }
            let _ = req.respond(r);
            continue;
        }
        let url = req.url().trim_start_matches('/').to_string();
        let (cmd, query) = url.split_once('?').unwrap_or((&url, ""));
        let view = query.split('&').find_map(|kv| kv.strip_prefix("view=")).unwrap_or("main").to_string();
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let args: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
        let (status, out) = if cmd == "_events" {
            (200, serde_json::Value::Array(queues.remove(&view).unwrap_or_default()))
        } else {
            if let Some(wb) = initial.take() {
                hub.lock().unwrap().attach_new(&view, wb);
            }
            match speedsheet_lib::api::dispatch_view(&hub, &view, cmd, args) {
                Ok(reply) => {
                    for ev in reply.events {
                        queues.entry(ev.to).or_default().push(serde_json::json!({ "event": ev.name, "payload": ev.payload }));
                    }
                    (200, reply.value)
                }
                Err(e) => (400, serde_json::Value::String(e)),
            }
        };
        if cmd == "close_view" {
            queues.remove(&view);
        }
        let mut r = tiny_http::Response::from_string(out.to_string()).with_status_code(status);
        for h in &cors {
            r.add_header(h.clone());
        }
        let _ = req.respond(r);
    }
}
