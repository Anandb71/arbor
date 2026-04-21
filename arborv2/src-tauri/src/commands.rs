use crate::lattice::AppState;
use crate::models::{DashboardSnapshot, ImpactReport, PathReport, SearchResponse};
use tauri::State;

#[tauri::command]
pub fn bootstrap(state: State<'_, AppState>) -> Result<DashboardSnapshot, String> {
    state.bootstrap().map_err(|error| error.to_string())
}

#[tauri::command]
pub fn index_notes(
    state: State<'_, AppState>,
    notes_path: Option<String>,
) -> Result<DashboardSnapshot, String> {
    state
        .index_notes(notes_path)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn search_notes(
    state: State<'_, AppState>,
    query: String,
    limit: usize,
) -> Result<SearchResponse, String> {
    state
        .search_notes(&query, limit)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn analyze_note_impact(
    state: State<'_, AppState>,
    node: String,
    max_depth: usize,
) -> Result<ImpactReport, String> {
    state
        .analyze_note_impact(&node, max_depth)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn find_note_path(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<PathReport, String> {
    state
        .find_note_path(&from, &to)
        .map_err(|error| error.to_string())
}
