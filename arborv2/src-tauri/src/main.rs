#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod lattice;
mod models;

use lattice::AppState;

fn main() {
    let state = AppState::new().expect("failed to initialize Arbor V2 state");

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::index_notes,
            commands::search_notes,
            commands::analyze_note_impact,
            commands::find_note_path
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Arbor V2");
}
