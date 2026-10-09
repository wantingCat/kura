//! Background filesystem watcher monitoring library folders.
//!
//! Features:
//! - Recursively watches all configured library roots (`library_folders` table).
//! - 3-second debounce window with file size stability & read lock verification (ensures
//!   active torrents or in-progress file copies aren't prematurely read).
//! - Targeted ingestion for new episodes of known series (< 10ms with zero extra HTTP calls).
//! - Automatic removal of deleted files and folders from the database.
//! - Automatic scan triggering (`scan::spawn_scan`) when brand new anime series are added.
//! - Live detection of local artwork drops (`clearlogo.png`, `fanart.jpg`, `poster.jpg`).
//! - Dynamic root reloading on library additions/removals and user toggle in Settings.

use crate::service::AppState;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Clone)]
pub struct WatcherHandle {
    reload_tx: UnboundedSender<()>,
}

impl WatcherHandle {
    pub fn new() -> (Self, UnboundedReceiver<()>) {
        let (reload_tx, reload_rx) = unbounded_channel();
        (Self { reload_tx }, reload_rx)
    }

    /// Notify the watcher to refresh its watched directory list from the database.
    pub fn reload(&self) {
        let _ = self.reload_tx.send(());
    }
}

pub fn start(app: AppHandle, st: Arc<AppState>, reload_rx: UnboundedReceiver<()>) {
    tauri::async_runtime::spawn(async move {
        run_worker(app, st, reload_rx).await;
    });
}

/// Filter out temporary files, download partials, and hidden dotfiles.
pub fn is_temp_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return true; };
    if name.starts_with('.') {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".tmp")
        || lower.ends_with(".part")
        || lower.ends_with(".!qb")
        || lower.ends_with(".crdownload")
        || lower.ends_with(".partial")
        || lower.ends_with(".download")
        || lower.ends_with(".aria2")
        || lower.ends_with(".bak")
        || lower.ends_with('~')
}

fn is_image_asset(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase()) else { return false; };
    let Some(ext) = path.extension().and_then(|s| s.to_str()).map(|s| s.to_ascii_lowercase()) else { return false; };
    let valid_exts = ["jpg", "jpeg", "png", "webp"];
    if !valid_exts.contains(&ext.as_str()) {
        return false;
    }
    let asset_names = [
        "fanart", "backdrop", "background", "art",
        "clearlogo", "logo", "clearart",
        "poster", "cover", "folder",
    ];
    asset_names.contains(&stem.as_str())
}

/// Check if a file is ready to be processed:
/// 1. Exists and size > 0
/// 2. Size matches previous observation (has stopped growing)
/// 3. File can be opened with read permissions (not locked exclusively by a downloader/copier)
fn is_file_ready(path: &Path, last_size: Option<u64>) -> (bool, u64) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (false, 0);
    };
    let len = meta.len();
    if len == 0 {
        return (false, 0);
    }
    if let Some(prev) = last_size {
        if prev != len {
            return (false, len);
        }
    }
    match std::fs::OpenOptions::new().read(true).open(path) {
        Ok(_) => (true, len),
        Err(_) => (false, len),
    }
}

struct PendingItem {
    last_event: Instant,
    last_size: Option<u64>,
}

async fn run_worker(app: AppHandle, st: Arc<AppState>, mut reload_rx: UnboundedReceiver<()>) {
    let (event_tx, mut event_rx) = unbounded_channel::<notify::Result<Event>>();

    let mut watcher = match RecommendedWatcher::new(
        move |res| {
            let _ = event_tx.send(res);
        },
        Config::default(),
    ) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[kura-watcher] failed to initialize watcher: {e}");
            return;
        }
    };

    let mut current_watched: HashSet<PathBuf> = HashSet::new();
    reload_watches(&st, &mut watcher, &mut current_watched);

    let mut pending: HashMap<PathBuf, PendingItem> = HashMap::new();
    let debounce_duration = Duration::from_secs(3);
    let mut ticker = tokio::time::interval(Duration::from_millis(500));

    loop {
        tokio::select! {
            Some(()) = reload_rx.recv() => {
                reload_watches(&st, &mut watcher, &mut current_watched);
            }
            Some(res) = event_rx.recv() => {
                if let Ok(event) = res {
                    handle_fs_event(event, &mut pending);
                }
            }
            _ = ticker.tick() => {
                if !pending.is_empty() {
                    let mut ready = Vec::new();
                    let now = Instant::now();
                    let mut to_remove = Vec::new();

                    for (path, item) in pending.iter_mut() {
                        if now.duration_since(item.last_event) < debounce_duration {
                            continue;
                        }
                        if !path.exists() {
                            // File or folder was deleted
                            to_remove.push(path.clone());
                            ready.push(path.clone());
                        } else if path.is_file() {
                            let (is_ready, size) = is_file_ready(path, item.last_size);
                            if is_ready {
                                to_remove.push(path.clone());
                                ready.push(path.clone());
                            } else {
                                item.last_size = Some(size);
                                item.last_event = now; // Give another debounce window to settle
                            }
                        } else if path.is_dir() {
                            // Directory changed: check if any video/image files were added
                            to_remove.push(path.clone());
                            for entry in walkdir::WalkDir::new(path)
                                .follow_links(true)
                                .into_iter()
                                .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
                            {
                                if let Ok(entry) = entry {
                                    let ep = entry.path();
                                    if ep.is_file() && (crate::parser::is_video_file(ep) || is_image_asset(ep)) && !is_temp_path(ep) {
                                        ready.push(ep.to_path_buf());
                                    }
                                }
                            }
                        } else {
                            to_remove.push(path.clone());
                        }
                    }

                    for path in to_remove {
                        pending.remove(&path);
                    }

                    if !ready.is_empty() {
                        process_ready_batch(&app, &st, ready).await;
                    }
                }
            }
        }
    }
}

fn reload_watches(
    st: &AppState,
    watcher: &mut RecommendedWatcher,
    current_watched: &mut HashSet<PathBuf>,
) {
    let enabled = {
        let conn = st.db();
        let val: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'pref.folder_watcher'",
                [],
                |r| r.get(0),
            )
            .ok();
        val.as_deref() != Some("0")
    };

    if !enabled {
        if !current_watched.is_empty() {
            println!("[kura-watcher] live folder watcher disabled in settings, pausing watches");
            for p in current_watched.drain() {
                let _ = watcher.unwatch(&p);
            }
        }
        return;
    }

    let roots: HashSet<PathBuf> = {
        let conn = st.db();
        let mut stmt = match conn.prepare("SELECT DISTINCT path FROM library_folders") {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[kura-watcher] db error querying library_folders: {e}");
                return;
            }
        };
        let paths = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .ok()
            .map(|rows| {
                rows.flatten()
                    .map(PathBuf::from)
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        paths
    };

    // Unwatch removed roots
    let to_remove: Vec<PathBuf> = current_watched.difference(&roots).cloned().collect();
    for p in to_remove {
        println!("[kura-watcher] unwatching folder: {}", p.display());
        let _ = watcher.unwatch(&p);
        current_watched.remove(&p);
    }

    // Watch new roots
    let to_add: Vec<PathBuf> = roots.difference(current_watched).cloned().collect();
    for p in to_add {
        println!("[kura-watcher] watching folder: {}", p.display());
        if let Err(e) = watcher.watch(&p, RecursiveMode::Recursive) {
            eprintln!("[kura-watcher] failed to watch {}: {e}", p.display());
        } else {
            current_watched.insert(p);
        }
    }
}

fn handle_fs_event(event: Event, pending: &mut HashMap<PathBuf, PendingItem>) {
    let now = Instant::now();
    for path in event.paths {
        if is_temp_path(&path) {
            continue;
        }
        if path.is_file() {
            if crate::parser::is_video_file(&path) || is_image_asset(&path) {
                pending.entry(path).and_modify(|item| item.last_event = now).or_insert(PendingItem {
                    last_event: now,
                    last_size: None,
                });
            }
        } else if path.is_dir() {
            if path.exists() {
                // Directory created or modified: scan immediate children for video or image files
                for entry in walkdir::WalkDir::new(&path)
                    .follow_links(true)
                    .into_iter()
                    .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
                {
                    let Ok(entry) = entry else { continue };
                    let ep = entry.path();
                    if ep.is_file() && (crate::parser::is_video_file(ep) || is_image_asset(ep)) && !is_temp_path(ep) {
                        pending.entry(ep.to_path_buf()).and_modify(|item| item.last_event = now).or_insert(PendingItem {
                            last_event: now,
                            last_size: None,
                        });
                    }
                }
            } else {
                // Directory removed
                pending.entry(path).and_modify(|item| item.last_event = now).or_insert(PendingItem {
                    last_event: now,
                    last_size: None,
                });
            }
        } else if !path.exists() {
            // Path was deleted or renamed
            pending.entry(path).and_modify(|item| item.last_event = now).or_insert(PendingItem {
                last_event: now,
                last_size: None,
            });
        }
    }
}

async fn process_ready_batch(app: &AppHandle, st: &Arc<AppState>, paths: Vec<PathBuf>) {
    let mut deleted_paths = Vec::new();
    let mut video_paths = Vec::new();
    let mut image_paths = Vec::new();

    for p in paths {
        if !p.exists() {
            deleted_paths.push(p);
        } else if p.is_file() {
            if crate::parser::is_video_file(&p) {
                video_paths.push(p);
            } else if is_image_asset(&p) {
                image_paths.push(p);
            }
        }
    }

    let mut changed = false;
    let mut needs_scan = false;

    // 1. Process deletions
    if !deleted_paths.is_empty() {
        let conn = st.db();
        for p in deleted_paths {
            let p_str = p.to_string_lossy().to_string();
            let rows_affected = conn.execute("DELETE FROM local_files WHERE path = ?1", [&p_str]).unwrap_or(0);
            if rows_affected > 0 {
                changed = true;
                println!("[kura-watcher] removed deleted file: {p_str}");
            } else {
                // Handle deleted directory: remove all files prefixed by directory path
                let dir_pattern1 = format!("{p_str}/%");
                let dir_pattern2 = format!("{p_str}\\%");
                let dir_deleted = conn
                    .execute(
                        "DELETE FROM local_files WHERE path = ?1 OR path LIKE ?2 OR path LIKE ?3",
                        rusqlite::params![p_str, dir_pattern1, dir_pattern2],
                    )
                    .unwrap_or(0);
                if dir_deleted > 0 {
                    changed = true;
                    println!("[kura-watcher] removed {dir_deleted} files for deleted folder: {p_str}");
                }
            }
        }
    }

    // 2. Process local image assets (fanart, ClearLogos, posters)
    if !image_paths.is_empty() {
        for img in image_paths {
            if let Some(parent) = img.parent() {
                let p_str = parent.to_string_lossy().to_string();
                let anilist_id: Option<i64> = {
                    let conn = st.db();
                    conn.query_row(
                        "SELECT anilist_id FROM local_files WHERE path LIKE ?1 || '%' AND anilist_id IS NOT NULL LIMIT 1",
                        [&p_str],
                        |r| r.get(0),
                    )
                    .ok()
                };
                if let Some(id) = anilist_id {
                    let dirs = vec![parent.to_path_buf()];
                    let local = crate::scan::scan_folder_assets(&dirs);
                    let conn = st.db();
                    if let Some(poster) = local.poster {
                        let _ = conn.execute(
                            "UPDATE media SET cover_path = ?2 WHERE anilist_id = ?1",
                            rusqlite::params![id, poster.to_string_lossy().to_string()],
                        );
                        changed = true;
                    }
                    if let Some(backdrop) = local.backdrop {
                        let _ = conn.execute(
                            "UPDATE media SET banner_path = ?2 WHERE anilist_id = ?1",
                            rusqlite::params![id, backdrop.to_string_lossy().to_string()],
                        );
                        changed = true;
                    }
                    if let Some(logo) = local.logo {
                        let _ = conn.execute(
                            "UPDATE media SET logo_path = ?2 WHERE anilist_id = ?1",
                            rusqlite::params![id, logo.to_string_lossy().to_string()],
                        );
                        changed = true;
                    }
                }
            }
        }
    }

    // 3. Process video files (targeted ingestion for existing series or scan trigger for new ones)
    if !video_paths.is_empty() {
        let folders: Vec<(i64, PathBuf)> = {
            let conn = st.db();
            let mut stmt = match conn.prepare("SELECT library_id, path FROM library_folders") {
                Ok(s) => s,
                Err(_) => return,
            };
            stmt.query_map([], |r| Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?))))
                .ok()
                .map(|rows| rows.flatten().collect())
                .unwrap_or_default()
        };

        for path in video_paths {
            let matched_folder = folders.iter().find(|(_, root)| path.starts_with(root));
            let Some((lib_id, root)) = matched_folder else {
                continue;
            };

            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let size = meta.len() as i64;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let path_s = path.to_string_lossy().to_string();

            // Skip if identical size & mtime and already mapped to an episode
            let existing: Option<(i64, i64, Option<String>)> = {
                let conn = st.db();
                conn.query_row(
                    "SELECT size, mtime, ep_key FROM local_files WHERE path = ?1",
                    [&path_s],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .ok()
            };

            if let Some((ex_size, ex_mtime, Some(_))) = existing {
                if ex_size == size && ex_mtime == mtime {
                    continue;
                }
            }

            let file_name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let parsed = crate::parser::parse_filename(&file_name);
            let Some(group) = crate::scan::compute_group(root, &path, &parsed) else {
                continue;
            };

            let group_row: Option<(i64, Option<i64>)> = {
                let conn = st.db();
                conn.query_row(
                    "SELECT id, anilist_id FROM match_groups WHERE library_id = ?1 AND group_key = ?2",
                    rusqlite::params![lib_id, group.key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok()
            };

            match group_row {
                Some((group_id, Some(anilist_id))) => {
                    // Fast path: targeted ingestion (<10ms)
                    let is_extra = parsed.extra || group.extra_folder;
                    let special_hint = if is_extra {
                        Some("extra")
                    } else if group.special_folder {
                        Some("special")
                    } else {
                        None
                    };

                    let insert_res: rusqlite::Result<i64> = (|| {
                        let conn = st.db();
                        conn.execute(
                            "INSERT INTO local_files (library_id, group_id, path, size, mtime, parsed, season_hint, special_hint, anilist_id, ep_key, added_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9)
                             ON CONFLICT(path) DO UPDATE SET
                                library_id = excluded.library_id, group_id = excluded.group_id, size = excluded.size,
                                mtime = excluded.mtime, parsed = excluded.parsed, season_hint = excluded.season_hint,
                                special_hint = excluded.special_hint",
                            rusqlite::params![
                                lib_id,
                                group_id,
                                path_s,
                                size,
                                mtime,
                                serde_json::to_string(&parsed).unwrap_or_default(),
                                group.season_hint,
                                special_hint,
                                crate::db::now()
                            ],
                        )?;
                        conn.query_row("SELECT id FROM local_files WHERE path = ?1", [&path_s], |r| r.get(0))
                    })();

                    if let Ok(file_id) = insert_res {
                        let resolve_res = crate::service::resolve_file(
                            st,
                            anilist_id,
                            crate::service::FileHints {
                                season: group.season_hint,
                                special_folder: group.special_folder,
                                extra_folder: is_extra,
                                parsed: &parsed,
                            },
                        )
                        .await;

                        match resolve_res {
                            Ok(Some((resolved_id, ep_key))) => {
                                {
                                    let conn = st.db();
                                    let _ = conn.execute(
                                        "UPDATE local_files SET anilist_id = ?2, ep_key = ?3 WHERE id = ?1",
                                        rusqlite::params![file_id, resolved_id, ep_key],
                                    );
                                }
                                println!("[kura-watcher] targeted ingestion: {path_s} -> ep {ep_key} (anilist {resolved_id})");
                                let _ = crate::service::ensure_episodes(st, resolved_id).await;
                                changed = true;
                            }
                            Ok(None) => {
                                let conn = st.db();
                                let _ = conn.execute(
                                    "UPDATE local_files SET anilist_id = ?2, ep_key = NULL WHERE id = ?1",
                                    rusqlite::params![file_id, anilist_id],
                                );
                                changed = true;
                            }
                            Err(e) => eprintln!("[kura-watcher] resolve error for {path_s}: {e}"),
                        }
                    }
                }
                _ => {
                    // New anime series or unmatched group: schedule background scan pass
                    println!("[kura-watcher] new series detected: '{}', queuing background scan", group.display_name);
                    needs_scan = true;
                }
            }
        }
    }

    if changed {
        let _ = st.db().execute(
            "DELETE FROM match_groups WHERE id NOT IN (SELECT DISTINCT group_id FROM local_files WHERE group_id IS NOT NULL)",
            [],
        );
        let _ = app.emit("library-changed", ());
    }

    if needs_scan {
        crate::scan::spawn_scan(app.clone(), st.clone(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_temp_path() {
        assert!(is_temp_path(Path::new("Frieren - 01.mkv.part")));
        assert!(is_temp_path(Path::new("Frieren - 01.mkv.!qb")));
        assert!(is_temp_path(Path::new("download.crdownload")));
        assert!(is_temp_path(Path::new("episode.partial")));
        assert!(is_temp_path(Path::new("tempfile.tmp")));
        assert!(is_temp_path(Path::new(".DS_Store")));
        assert!(is_temp_path(Path::new(".hidden_video.mkv")));

        assert!(!is_temp_path(Path::new("[SubsPlease] Sousou no Frieren - 01 (1080p).mkv")));
        assert!(!is_temp_path(Path::new("Oshi no Ko S02E01.mp4")));
        assert!(!is_temp_path(Path::new("clearlogo.png")));
    }

    #[test]
    fn test_is_image_asset() {
        assert!(is_image_asset(Path::new("fanart.jpg")));
        assert!(is_image_asset(Path::new("FANART.PNG")));
        assert!(is_image_asset(Path::new("backdrop.webp")));
        assert!(is_image_asset(Path::new("clearlogo.png")));
        assert!(is_image_asset(Path::new("logo.png")));
        assert!(is_image_asset(Path::new("poster.jpg")));
        assert!(is_image_asset(Path::new("cover.jpeg")));
        assert!(is_image_asset(Path::new("folder.jpg")));

        assert!(!is_image_asset(Path::new("episode_thumbnail_01.jpg")));
        assert!(!is_image_asset(Path::new("random_photo.png")));
        assert!(!is_image_asset(Path::new("video.mkv")));
        assert!(!is_image_asset(Path::new("subtitles.ass")));
    }

    #[test]
    fn test_file_readiness_lifecycle() {
        let temp_dir = std::env::temp_dir().join("kura_watcher_test");
        let _ = std::fs::create_dir_all(&temp_dir);
        let test_file = temp_dir.join("test_episode.mkv");

        // 1. Zero-byte file: not ready
        std::fs::write(&test_file, b"").unwrap();
        let (ready, len) = is_file_ready(&test_file, None);
        assert!(!ready);
        assert_eq!(len, 0);

        // 2. Growing file: size mismatch with previous observation
        std::fs::write(&test_file, b"video data chunk 1").unwrap();
        let (ready, len) = is_file_ready(&test_file, Some(5)); // prev size was 5, now 18
        assert!(!ready);
        assert_eq!(len, 18);

        // 3. Stable file: size matches previous observation and readable
        let (ready, len) = is_file_ready(&test_file, Some(18));
        assert!(ready);
        assert_eq!(len, 18);

        let _ = std::fs::remove_file(&test_file);
        let _ = std::fs::remove_dir(&temp_dir);
    }
}

