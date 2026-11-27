fn main() {
    // The feedback proxy URL is baked in at compile time (see feedback-worker/README.md).
    println!("cargo:rerun-if-env-changed=SPEEDSHEET_FEEDBACK_URL");
    tauri_build::build()
}
