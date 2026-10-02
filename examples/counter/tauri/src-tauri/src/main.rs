// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // COUNTER_START lets you see config reaching the core: `COUNTER_START=42 ./counter-tauri`.
    let start = std::env::var("COUNTER_START").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    tauri::Builder::default()
        .plugin(tauri_plugin_carapace::plugin_with_queries::<counter_core::Counter, _>(counter_core::Config { start }))
        .run(tauri::generate_context!())
        .expect("error while running the Counter app");
}
