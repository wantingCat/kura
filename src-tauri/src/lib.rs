pub mod anilist_sync;
pub mod backup;
pub mod commands;
pub mod db;
pub mod dictionary;
pub mod discord;
pub mod embedded_player;
pub mod franchise;
pub mod logging;
pub mod matcher;
pub mod parser;
pub mod player;
pub mod providers;
pub mod scan;
pub mod service;
pub mod store;
pub mod subtitles;
pub mod torrent;
pub mod watcher;

use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Enable native GPU 10-bit HEVC (Main/Main10) hardware decoding and zero-copy video
    // rendering inside Windows WebView2 so 10-bit HEVC anime plays directly on the GPU
    // without CPU transcoding.
    #[cfg(target_os = "windows")]
    {
        const WV2_ARGS: &str = "--enable-features=PlatformHEVCDecoderSupport,PlatformHEVCEncoderSupport --autoplay-policy=no-user-gesture-required --ignore-gpu-blocklist --enable-gpu-rasterization --enable-zero-copy";
        match std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
            Ok(existing) if !existing.contains("PlatformHEVCDecoderSupport") => {
                std::env::set_var(
                    "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
                    format!("{existing} {WV2_ARGS}"),
                );
            }
            Err(_) => {
                std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", WV2_ARGS);
            }
            _ => {}
        }
    }

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
            logging::init(&data_dir);
            klog!(
                "[kura] Kura {} starting on {} {}",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            let _ = app.asset_protocol_scope().allow_directory(&data_dir, true);
            let conn = db::open(&data_dir.join("kura.db"))?;
            let (watcher, reload_rx) = watcher::WatcherHandle::new();
            let state = Arc::new(service::AppState::new(conn, data_dir.clone(), watcher));
            app.manage(state.clone());

            // Seed starter Japanese Yomitan dictionary in background
            std::thread::spawn(move || {
                if let Ok(dconn) = dictionary::open_dict_db(&data_dir) {
                    let _ = dictionary::ensure_starter_dictionary(&dconn);
                }
            });

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
                scan::spawn_scan(app.handle().clone(), state.clone(), None);
            }

            // Start background filesystem watcher for configured folders.
            watcher::start(app.handle().clone(), state.clone(), reload_rx);

            // Debounced writer for the automatic safety backup.
            backup::start_auto(state.clone());
            anilist_sync::start_worker(app.handle().clone(), state.clone());
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
            commands::check_mpv_status,
            commands::install_mpv,
            commands::play_episode,
            commands::get_up_next,
            commands::get_new_episodes,
            commands::get_media_track_pref,
            commands::set_media_track_pref,
            commands::rebuild_metadata,
            commands::export_backup,
            commands::preview_backup,
            commands::import_backup,
            commands::backup_status,
            commands::get_diagnostics,
            commands::get_anilist_account,
            commands::anilist_update_entry,
            commands::anilist_start_oauth,
            commands::anilist_login_token,
            commands::anilist_disconnect,
            commands::anilist_sync_now,
            commands::get_seasonal_calendar,
            commands::get_person_works,
            commands::preview_subtitle_matches,
            commands::apply_subtitle_matches,
            commands::retime_episode_subtitle,
            commands::remove_linked_subtitle,
            commands::jimaku_search,
            commands::jimaku_download,
            commands::toggle_stream_library,
            commands::browse_catalog,
            commands::search_people,
            commands::search_episode_torrents,
            commands::start_torrent_session,
            commands::list_torrent_sessions,
            commands::cancel_torrent_session,
            commands::list_torrent_extensions,
            commands::import_torrent_extension,
            commands::toggle_torrent_extension,
            commands::remove_torrent_extension,
            commands::builtin_player_get_state,
            commands::builtin_player_command,
            commands::lookup_japanese_at,
            commands::list_dictionaries,
            commands::import_yomitan_dictionary,
            commands::download_recommended_dictionaries,
            commands::remove_dictionary,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
