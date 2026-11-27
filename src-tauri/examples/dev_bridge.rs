//! Browser dev bridge: serves the same API as the desktop app over HTTP on 127.0.0.1:1421,
//! so the UI can be developed and tested in a normal browser (`npm run dev`, then open
//! http://localhost:1420). Run with: `cargo run -p speedsheet --example dev_bridge -- [file]`.
//! Only for local development; it is not part of the app.

use speedsheet_core::{io, Workbook};

fn main() {
    let mut wb = match std::env::args().nth(1) {
        Some(p) => io::open(p.as_ref()).expect("open file"),
        None => Workbook::new(),
    };
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
        let cmd = req.url().trim_start_matches('/').to_string();
        let mut body = String::new();
        let _ = req.as_reader().read_to_string(&mut body);
        let args: serde_json::Value = serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
        let (status, out) = match speedsheet_lib::api::dispatch(&mut wb, &cmd, args) {
            Ok(v) => (200, v),
            Err(e) => (400, serde_json::Value::String(e)),
        };
        let mut r = tiny_http::Response::from_string(out.to_string()).with_status_code(status);
        for h in &cors {
            r.add_header(h.clone());
        }
        let _ = req.respond(r);
    }
}
