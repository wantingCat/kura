pub mod anilist_sync;
pub mod backup;
pub mod commands;
pub mod db;
pub mod dht;
pub mod dictionary;
pub mod discord;
pub mod embedded_player;
pub mod extensions;
pub mod franchise;
pub mod libmpv;
pub mod logging;
pub mod matcher;
pub mod mse;
pub mod onlinestream;
pub mod parser;
pub mod player;
pub mod providers;
pub mod scan;
pub mod service;
pub mod store;
pub mod subtitles;
pub mod torrent;
pub mod watcher;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;

const SINGLE_INSTANCE_ADDR: ([u8; 4], u16) = ([127, 0, 0, 1], 45872);

fn restore_main_window(app_handle: &tauri::AppHandle) {
    if let Some(w) = app_handle.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_always_on_top(true);
        let _ = w.set_focus();
        let _ = w.set_always_on_top(false);
    }
}

fn resolve_instance_lock_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(|p| PathBuf::from(p).join("io.github.kura.app").join(".instance_lock"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|p| {
            PathBuf::from(p)
                .join("Library")
                .join("Application Support")
                .join("io.github.kura.app")
                .join(".instance_lock")
        })
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .map(|p| p.join("io.github.kura.app").join(".instance_lock"))
    }
}

fn write_instance_lock_secret(path: &std::path::Path, secret: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
        {
            let _ = f.write_all(secret.as_bytes());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::write(path, secret.as_bytes());
    }
}

fn perform_graceful_shutdown(app_handle: &tauri::AppHandle) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
    if SHUTTING_DOWN.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(st) = app_handle.try_state::<Arc<service::AppState>>() {
        let st_arc = st.inner().clone();
        embedded_player::shutdown_sync(app_handle, &st_arc);
        crate::torrent::shutdown_all_sessions();
        crate::backup::write_safety_copy(&st_arc);
        let conn = st_arc.db();
        let _ = subtitles::cleanup_watched_subtitles(&conn, &st_arc.data_dir);
    } else {
        crate::torrent::shutdown_all_sessions();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Enforce authenticated single-instance IPC on desktop (`L-01`):
    // Only focus an existing instance (and exit) when both processes share the CSPRNG secret
    // stored in `.instance_lock`, preventing both focus-stealing and port-collision DoS.
    let single_instance_listener: Option<(TcpListener, String)> =
        if std::env::var_os("KURA_DATA_DIR").is_none() {
            let addr = SocketAddr::from(SINGLE_INSTANCE_ADDR);
            let lock_path = resolve_instance_lock_path();
            match TcpListener::bind(addr) {
                Ok(listener) => {
                    let secret = db::random_hex_token(16);
                    if let Some(ref lp) = lock_path {
                        write_instance_lock_secret(lp, &secret);
                    }
                    Some((listener, secret))
                }
                Err(_) => {
                    let existing_secret = lock_path
                        .as_deref()
                        .and_then(|lp| std::fs::read_to_string(lp).ok())
                        .map(|s| s.trim().to_string())
                        .filter(|s| s.len() >= 16);
                    if let Some(secret) = existing_secret {
                        if let Ok(mut stream) =
                            TcpStream::connect_timeout(&addr, Duration::from_millis(400))
                        {
                            let _ = stream.set_write_timeout(Some(Duration::from_millis(400)));
                            let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
                            let req = format!("KURA_SHOW {secret}\n");
                            let expected_ack = format!("KURA_OK {secret}");
                            if stream.write_all(req.as_bytes()).is_ok() && stream.flush().is_ok() {
                                let mut ack = [0u8; 128];
                                let n = stream.read(&mut ack).unwrap_or(0);
                                let ack_str = std::str::from_utf8(&ack[..n]).unwrap_or("").trim();
                                if ack_str == expected_ack {
                                    return;
                                }
                            }
                        }
                    }
                    None
                }
            }
        } else {
            None
        };

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    builder
        .setup(move |app| {
            #[cfg_attr(not(debug_assertions), allow(unused_mut))]
            let mut data_dir = app.path().app_data_dir()?;
            #[cfg(debug_assertions)]
            if let Ok(dir) = std::env::var("KURA_DATA_DIR") {
                data_dir = PathBuf::from(dir);
            }
            std::fs::create_dir_all(&data_dir)?;

            if let Some((listener, secret)) = single_instance_listener {
                write_instance_lock_secret(&data_dir.join(".instance_lock"), &secret);
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    let expected_req = format!("KURA_SHOW {secret}");
                    let ok_reply = format!("KURA_OK {secret}\n");
                    for stream in listener.incoming().flatten() {
                        let mut stream = stream;
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(400)));
                        let mut buf = [0u8; 128];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let msg = std::str::from_utf8(&buf[..n]).unwrap_or("").trim();
                        if msg == expected_req {
                            let _ = stream.write_all(ok_reply.as_bytes());
                            let _ = stream.flush();
                            restore_main_window(&app_handle);
                        }
                    }
                });
            }

            logging::init(&data_dir);
            klog!(
                "[kura] Kura {} starting on {} {}",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH
            );

            // Only expose `data_dir/images` (never `kura.db` or `kura_backup.json`) to `asset://` (`C-01`).
            let images_dir = data_dir.join("images");
            let _ = std::fs::create_dir_all(&images_dir);
            let _ = app.asset_protocol_scope().allow_directory(&images_dir, true);

            // Purge stale temporary torrent stream cache files from prior sessions (`C-04`).
            crate::torrent::purge_stream_cache(&data_dir);

            let conn = db::open(&data_dir.join("kura.db"))?;
            let _ = subtitles::cleanup_watched_subtitles(&conn, &data_dir);
            let (watcher, reload_rx) = watcher::WatcherHandle::new();
            let state = Arc::new(service::AppState::new(conn, data_dir.clone(), watcher));
            app.manage(state.clone());

            // Seed starter Japanese Yomitan dictionary in background (`C-07`)
            let dict_dir = data_dir.clone();
            std::thread::spawn(move || {
                let _ = dictionary::open_dict_db(&dict_dir);
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

            // Allow configured library folders in `asset://` so local `poster.jpg`/`fanart.jpg` render (`C-01`).
            commands::allow_all_library_folders_in_scope(app.handle(), &state.db_read());

            // Pick up new / removed files on startup.
            let has_folders: bool = state
                .db_read()
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
            dht::warm_up();

            // Pre-warm the built-in libmpv engine & hidden Win32 video surface shortly after
            // launch on Windows so the first click on Play opens instantaneously.
            #[cfg(windows)]
            {
                let warm_app = app.handle().clone();
                let warm_state = state.clone();
                std::thread::Builder::new()
                    .name("kura-mpv-warmup".to_string())
                    .spawn(move || {
                        std::thread::sleep(Duration::from_millis(900));
                        embedded_player::warm_up(warm_app, warm_state);
                    })
                    .ok();
            }

            // System tray icon & context menu for background episode + Jimaku monitoring
            #[cfg(desktop)]
            {
                use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

                let show_item = MenuItem::with_id(app, "show", "Open Kura", true, None::<&str>)?;
                let check_item = MenuItem::with_id(
                    app,
                    "check_updates",
                    "Check Episode & Jimaku Updates Now",
                    true,
                    None::<&str>,
                )?;
                let sep = PredefinedMenuItem::separator(app)?;
                let quit_item = MenuItem::with_id(app, "quit", "Quit Kura", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_item, &check_item, &sep, &quit_item])?;

                let mut tray_builder = TrayIconBuilder::with_id("kura-tray")
                    .tooltip("Kura · Anime Library & Jimaku Monitor")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app_handle, event| match event.id.as_ref() {
                        "show" => {
                            restore_main_window(app_handle);
                        }
                        "check_updates" => {
                            if let Some(st) = app_handle.try_state::<Arc<service::AppState>>() {
                                let st_clone = st.inner().clone();
                                let app_clone = app_handle.clone();
                                tauri::async_runtime::spawn(async move {
                                    let _ = anilist_sync::sync_now(&app_clone, &st_clone).await;
                                    anilist_sync::check_airing_notifications(&app_clone, &st_clone)
                                        .await;
                                });
                            }
                        }
                        "quit" => {
                            perform_graceful_shutdown(app_handle);
                            app_handle.exit(0);
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                        | TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        } = event
                        {
                            restore_main_window(tray.app_handle());
                        }
                    });

                if let Some(icon) = app.default_window_icon().cloned() {
                    tray_builder = tray_builder.icon(icon);
                }
                let _ = tray_builder.build(app);
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
            commands::check_mpv_status,
            commands::install_mpv,
            commands::play_episode,
            commands::get_up_next,
            commands::get_new_episodes,
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
            commands::builtin_player_command,
            commands::lookup_japanese_at,
            commands::list_dictionaries,
            commands::import_yomitan_dictionary,
            commands::download_recommended_dictionaries,
            commands::remove_dictionary,
            commands::record_word_lookup,
            commands::set_word_status,
            commands::delete_immersion_word,
            commands::get_tracked_surface_forms,
            commands::set_immersion_daily_goal,
            commands::log_manual_immersion,
            commands::get_immersion_overview,
            commands::get_anilist_immersion_lists,
            commands::set_anilist_immersion_list,
            commands::list_extensions,
            commands::get_extension_payload,
            commands::fetch_marketplace_extensions,
            commands::install_extension,
            commands::toggle_extension,
            commands::remove_extension,
            commands::set_extension_user_config,
            commands::extension_http_fetch,
            commands::get_onlinestream_mapping,
            commands::set_onlinestream_mapping,
            commands::remove_onlinestream_mapping,
            commands::start_web_stream,
            commands::prefetch_web_stream_subtitles,
            commands::get_anidb_ids,
            commands::list_torrent_files,
            commands::get_remembered_batch,
            commands::clear_remembered_batch,
            commands::check_episode_availability,
            commands::preload_next_torrent_episode,
        ])
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let mut minimize_to_tray = true;
                if let Some(st) = window.try_state::<Arc<service::AppState>>() {
                    let st_arc = st.inner().clone();
                    if let Ok(Some(val)) = db::get_setting(&st_arc.db_read(), "pref.minimize_to_tray") {
                        if val == "0" || val.eq_ignore_ascii_case("false") {
                            minimize_to_tray = false;
                        }
                    }
                    if minimize_to_tray && window.label() == "main" {
                        embedded_player::shutdown_sync(window.app_handle(), &st_arc);
                        let _ = subtitles::cleanup_watched_subtitles(&st_arc.db(), &st_arc.data_dir);
                    }
                }
                if minimize_to_tray && window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                } else if window.label() == "main" {
                    perform_graceful_shutdown(window.app_handle());
                }
            }
            tauri::WindowEvent::Moved(_)
            | tauri::WindowEvent::Resized(_)
            | tauri::WindowEvent::Focused(_)
            | tauri::WindowEvent::ScaleFactorChanged { .. } => {
                if window.label() == "main" {
                    embedded_player::sync_bounds(window.app_handle());
                }
            }
            tauri::WindowEvent::Destroyed => {
                if window.label() == "main" {
                    perform_graceful_shutdown(window.app_handle());
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
