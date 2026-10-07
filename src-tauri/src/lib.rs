pub mod commands;
pub mod db;
pub mod franchise;
pub mod matcher;
pub mod parser;
pub mod player;
pub mod providers;
pub mod scan;
pub mod service;
pub mod store;

use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    builder
        .setup(|app| {
            #[cfg_attr(not(debug_assertions), allow(unused_mut))]
            let mut data_dir = app.path().app_data_dir()?;
            #[cfg(debug_assertions)]
            if let Ok(dir) = std::env::var("KURA_DATA_DIR") {
                data_dir = std::path::PathBuf::from(dir);
            }
            std::fs::create_dir_all(&data_dir)?;
            let _ = app.asset_protocol_scope().allow_directory(&data_dir, true);
            let conn = db::open(&data_dir.join("kura.db"))?;
            let state = Arc::new(service::AppState::new(conn, data_dir));
            app.manage(state.clone());

            // Development helper: seed a library from an env var (debug builds only).
            #[cfg(debug_assertions)]
            if let Ok(dev_path) = std::env::var("KURA_DEV_LIBRARY") {
                let conn = state.db();
                let empty: bool = conn
                    .query_row("SELECT NOT EXISTS(SELECT 1 FROM libraries)", [], |r| r.get(0))
                    .unwrap_or(false);
                if empty {
                    let _ = conn.execute(
                        "INSERT INTO libraries (name, kind, created_at) VALUES ('Anime', 'anime', ?1)",
                        [db::now()],
                    );
                    let id = conn.last_insert_rowid();
                    let _ = conn.execute(
                        "INSERT INTO library_folders (library_id, path) VALUES (?1, ?2)",
                        rusqlite::params![id, dev_path],
                    );
                }
            }

            // Pick up new / removed files on startup.
            let has_folders: bool = state
                .db()
                .query_row("SELECT EXISTS(SELECT 1 FROM library_folders)", [], |r| r.get(0))
                .unwrap_or(false);
            if has_folders {
                scan::spawn_scan(app.handle().clone(), state, None);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_libraries,
            commands::create_library,
            commands::rename_library,
            commands::delete_library,
            commands::add_folder,
            commands::remove_folder,
            commands::start_scan,
            commands::is_scanning,
            commands::get_library,
            commands::get_unmatched,
            commands::get_media_detail,
            commands::set_watched,
            commands::search_anilist,
            commands::fix_match,
            commands::refresh_media,
            commands::open_file,
            commands::reveal_file,
            commands::get_prefs,
            commands::set_pref,
            commands::detect_players,
            commands::play_episode,
            commands::get_up_next,
            commands::get_new_episodes,
            commands::get_media_track_pref,
            commands::set_media_track_pref,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
