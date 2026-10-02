const COMMANDS: &[&str] = &["dispatch", "state", "schema_hash", "attach", "query"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
