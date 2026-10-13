mod compile;
mod export;
mod theme;
mod workspace;

#[tauri::command]
fn app_version() -> Result<String, String> {
    Ok(env!("CARGO_PKG_VERSION").to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(compile::Compiler::default())
        .manage(workspace::Workspace::default())
        .manage(export::Exports::default())
        .invoke_handler(tauri::generate_handler![
            app_version,
            compile::compile,
            workspace::default_workspace,
            workspace::pick_folder,
            workspace::pick_typst_file,
            workspace::open_workspace,
            workspace::list_tree,
            workspace::create_entry,
            workspace::move_entry,
            workspace::delete_entry,
            workspace::read_page,
            workspace::write_page,
            theme::list_themes,
            theme::import_theme,
            theme::remove_theme,
            export::export_page,
            export::open_export,
            export::show_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
