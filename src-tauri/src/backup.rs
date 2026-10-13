//! User-data backups.
//!
//! One JSON format is used for both:
//! - the automatic *safety copy* (`kura_backup.json`, written to a few locations outside the
//!   app data folder so a reinstall / wiped database can be restored on the next start), and
//! - manual *Export / Import* from Settings → Backup & Restore.
//!
//! Only user-owned data is stored — settings, libraries & folders, watch history, resume
//! progress, per-show track preferences and manual match overrides. Metadata and artwork are
//! re-downloaded by a scan, so they are never part of a backup.
//!
//! The format is forward/backward compatible: every field added after v1 is `#[serde(default)]`,
//! and older Kura builds simply ignore the fields they don't know.

use crate::db::{now, BackupLibrary};
use crate::service::AppState;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Current backup format. v1 = settings + libraries only (≤ 0.3.x).
pub const FORMAT: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Backup {
    #[serde(default)]
    pub app: String,
    pub version: u32,
    pub updated_at: i64,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub settings: HashMap<String, String>,
    #[serde(default)]
    pub libraries: Vec<BackupLibrary>,
    #[serde(default)]
    pub watched: Vec<WatchedRow>,
    #[serde(default)]
    pub progress: Vec<ProgressRow>,
    #[serde(default)]
    pub track_prefs: Vec<TrackPrefRow>,
    #[serde(default)]
    pub manual_matches: Vec<ManualMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedRow {
    pub anilist_id: i64,
    pub ep_key: String,
    pub watched_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressRow {
    pub anilist_id: i64,
    pub ep_key: String,
    pub position: f64,
    pub duration: f64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackPrefRow {
    pub anilist_id: i64,
    pub audio_pref: Option<String>,
    pub sub_pref: Option<String>,
    pub sub_fallback: Option<String>,
    pub updated_at: i64,
}

/// A "Fix match" the user made. Group keys embed the library folder path, so we store the
/// folder (`root`) separately and remap it on import if the folder moved (e.g. new drive letter).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualMatch {
    pub library: String,
    pub root: String,
    pub rest: String,
    pub folder_path: Option<String>,
    pub display_name: String,
    pub title_guess: String,
    pub title_full: String,
    pub year_hint: Option<i64>,
    pub anilist_id: i64,
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

fn rows<T>(conn: &Connection, sql: &str, f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>) -> Vec<T> {
    // Tolerant on purpose: a missing table (very old DB) just yields nothing.
    let Ok(mut stmt) = conn.prepare(sql) else { return Vec::new() };
    let Ok(iter) = stmt.query_map([], f) else { return Vec::new() };
    iter.flatten().collect()
}

/// Read every piece of user data into a [`Backup`].
pub fn snapshot(conn: &Connection) -> Backup {
    let settings: HashMap<String, String> = rows(conn, "SELECT key, value FROM settings", |r| Ok((r.get(0)?, r.get(1)?)))
        .into_iter()
        .collect();

    let libs: Vec<(i64, String, String)> =
        rows(conn, "SELECT id, name, kind FROM libraries ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)));
    let folders: Vec<(i64, String)> =
        rows(conn, "SELECT library_id, path FROM library_folders ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?)));
    let libraries: Vec<BackupLibrary> = libs
        .iter()
        .map(|(id, name, kind)| BackupLibrary {
            name: name.clone(),
            kind: kind.clone(),
            folders: folders.iter().filter(|(l, _)| l == id).map(|(_, p)| p.clone()).collect(),
        })
        .collect();

    let watched = rows(conn, "SELECT anilist_id, ep_key, watched_at FROM watch_state ORDER BY anilist_id, ep_key", |r| {
        Ok(WatchedRow { anilist_id: r.get(0)?, ep_key: r.get(1)?, watched_at: r.get(2)? })
    });
    let progress = rows(
        conn,
        "SELECT anilist_id, ep_key, position, duration, updated_at FROM watch_progress ORDER BY anilist_id, ep_key",
        |r| {
            Ok(ProgressRow {
                anilist_id: r.get(0)?,
                ep_key: r.get(1)?,
                position: r.get(2)?,
                duration: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    );
    let track_prefs = rows(
        conn,
        "SELECT anilist_id, audio_pref, sub_pref, sub_fallback, updated_at FROM media_track_prefs ORDER BY anilist_id",
        |r| {
            Ok(TrackPrefRow {
                anilist_id: r.get(0)?,
                audio_pref: r.get(1)?,
                sub_pref: r.get(2)?,
                sub_fallback: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    );

    let lib_name = |id: i64| libs.iter().find(|(l, ..)| *l == id).map(|(_, n, _)| n.clone());
    let groups: Vec<(i64, String, Option<String>, String, String, String, Option<i64>, i64)> = rows(
        conn,
        "SELECT library_id, group_key, folder_path, display_name, title_guess, title_full, year_hint, anilist_id
         FROM match_groups WHERE manual = 1 AND anilist_id IS NOT NULL ORDER BY id",
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
    );
    let mut manual_matches = Vec::new();
    for (lib, key, folder_path, display_name, title_guess, title_full, year_hint, anilist_id) in groups {
        let Some(library) = lib_name(lib) else { continue };
        // Longest matching folder wins (nested library folders).
        let root = folders
            .iter()
            .filter(|(l, p)| *l == lib && key.starts_with(&format!("{p}|")))
            .map(|(_, p)| p.clone())
            .max_by_key(|p| p.len());
        let Some(root) = root else { continue };
        let rest = key[root.len() + 1..].to_string();
        manual_matches.push(ManualMatch {
            library,
            root,
            rest,
            folder_path,
            display_name,
            title_guess,
            title_full,
            year_hint,
            anilist_id,
        });
    }

    Backup {
        app: "kura".into(),
        version: FORMAT,
        updated_at: now(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        settings,
        libraries,
        watched,
        progress,
        track_prefs,
        manual_matches,
    }
}

// ---------------------------------------------------------------------------
// Apply (import / restore)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    pub settings: bool,
    pub libraries: bool,
    pub history: bool,
    pub track_prefs: bool,
    pub matches: bool,
}

impl ImportOptions {
    pub const ALL: Self = Self { settings: true, libraries: true, history: true, track_prefs: true, matches: true };
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub settings: usize,
    pub libraries_added: usize,
    pub folders_added: usize,
    pub watched_added: usize,
    pub progress_added: usize,
    pub track_prefs: usize,
    pub matches: usize,
    pub matches_skipped: usize,
}

/// Pick which current folder of `lib_id` a backed-up folder `root` corresponds to.
fn remap_root(conn: &Connection, lib_id: i64, root: &str) -> Option<String> {
    let folders: Vec<String> = {
        let mut stmt = conn.prepare("SELECT path FROM library_folders WHERE library_id = ?1").ok()?;
        let r = stmt.query_map([lib_id], |r| r.get(0)).ok()?.flatten().collect();
        r
    };
    if folders.iter().any(|f| f == root) {
        return Some(root.to_string());
    }
    if folders.len() == 1 {
        return folders.into_iter().next();
    }
    let base = |p: &str| p.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
    let want = base(root);
    let mut same: Vec<String> = folders.into_iter().filter(|f| !want.is_empty() && base(f) == want).collect();
    if same.len() == 1 {
        same.pop()
    } else {
        None
    }
}

/// Merge a backup into the database. Never deletes anything: existing watch history wins over
/// older entries, newer progress / track preferences replace older ones.
pub fn apply(conn: &Connection, b: &Backup, o: &ImportOptions) -> Result<ImportReport> {
    let tx = conn.unchecked_transaction()?;
    let mut rep = ImportReport::default();

    if o.settings {
        for (k, v) in &b.settings {
            // A player path from another machine would break playback.
            if k == "pref.player_path" && !v.is_empty() && !Path::new(v).is_file() {
                continue;
            }
            tx.execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
            rep.settings += 1;
        }
    }

    // Libraries are matched by name; also needed (read-only) for manual matches.
    let lib_id = |tx: &Connection, name: &str| -> Result<Option<i64>> {
        Ok(tx.query_row("SELECT id FROM libraries WHERE name = ?1 ORDER BY id LIMIT 1", [name], |r| r.get(0)).optional()?)
    };
    if o.libraries {
        for lib in &b.libraries {
            let id = match lib_id(&tx, &lib.name)? {
                Some(id) => id,
                None => {
                    tx.execute(
                        "INSERT INTO libraries (name, kind, created_at) VALUES (?1, ?2, ?3)",
                        params![lib.name, lib.kind, now()],
                    )?;
                    rep.libraries_added += 1;
                    tx.last_insert_rowid()
                }
            };
            for f in &lib.folders {
                rep.folders_added +=
                    tx.execute("INSERT OR IGNORE INTO library_folders (library_id, path) VALUES (?1, ?2)", params![id, f])?;
            }
        }
    }

    if o.history {
        for w in &b.watched {
            rep.watched_added += tx.execute(
                "INSERT OR IGNORE INTO watch_state (anilist_id, ep_key, watched_at) VALUES (?1, ?2, ?3)",
                params![w.anilist_id, w.ep_key, w.watched_at],
            )?;
        }
        for p in &b.progress {
            rep.progress_added += tx.execute(
                "INSERT INTO watch_progress (anilist_id, ep_key, position, duration, updated_at)
                 SELECT ?1, ?2, ?3, ?4, ?5
                 WHERE NOT EXISTS (SELECT 1 FROM watch_state WHERE anilist_id = ?1 AND ep_key = ?2)
                 ON CONFLICT(anilist_id, ep_key) DO UPDATE SET
                    position = excluded.position, duration = excluded.duration, updated_at = excluded.updated_at
                 WHERE excluded.updated_at > watch_progress.updated_at",
                params![p.anilist_id, p.ep_key, p.position, p.duration, p.updated_at],
            )?;
        }
    }

    if o.track_prefs {
        for t in &b.track_prefs {
            rep.track_prefs += tx.execute(
                "INSERT INTO media_track_prefs (anilist_id, audio_pref, sub_pref, sub_fallback, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(anilist_id) DO UPDATE SET
                    audio_pref = excluded.audio_pref, sub_pref = excluded.sub_pref,
                    sub_fallback = excluded.sub_fallback, updated_at = excluded.updated_at
                 WHERE excluded.updated_at >= media_track_prefs.updated_at",
                params![t.anilist_id, t.audio_pref, t.sub_pref, t.sub_fallback, t.updated_at],
            )?;
        }
    }

    if o.matches {
        for m in &b.manual_matches {
            let Some(lib) = lib_id(&tx, &m.library)? else {
                rep.matches_skipped += 1;
                continue;
            };
            let Some(root) = remap_root(&tx, lib, &m.root) else {
                rep.matches_skipped += 1;
                continue;
            };
            let key = format!("{root}|{}", m.rest);
            let folder_path = m.folder_path.as_ref().map(|fp| match fp.strip_prefix(&m.root) {
                Some(tail) => format!("{root}{tail}"),
                None => fp.clone(),
            });
            let current: Option<(i64, Option<i64>)> = tx
                .query_row(
                    "SELECT id, anilist_id FROM match_groups WHERE library_id = ?1 AND group_key = ?2",
                    params![lib, key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match current {
                Some((gid, aid)) => {
                    tx.execute(
                        "UPDATE match_groups SET anilist_id = ?2, confidence = 1.0, manual = 1, attempted = 1 WHERE id = ?1",
                        params![gid, m.anilist_id],
                    )?;
                    if aid != Some(m.anilist_id) {
                        // Let the next scan re-resolve these files against the restored match.
                        tx.execute("UPDATE local_files SET anilist_id = NULL, ep_key = NULL WHERE group_id = ?1", [gid])?;
                    }
                }
                None => {
                    // Files arrive with the next scan and attach to this group by key.
                    tx.execute(
                        "INSERT INTO match_groups (library_id, group_key, folder_path, display_name, title_guess, title_full,
                            year_hint, anilist_id, confidence, manual, attempted, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1.0, 1, 1, ?9)",
                        params![lib, key, folder_path, m.display_name, m.title_guess, m.title_full, m.year_hint, m.anilist_id, now()],
                    )?;
                }
            }
            rep.matches += 1;
        }
    }

    tx.commit()?;
    Ok(rep)
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

pub fn parse(json: &str) -> Result<Backup> {
    let b: Backup = serde_json::from_str(json).context("This file isn't a Kura backup (couldn't read it as JSON).")?;
    if !b.app.is_empty() && b.app != "kura" {
        bail!("This file isn't a Kura backup.");
    }
    if b.version == 0 || b.version > FORMAT {
        bail!("This backup was made by a newer version of Kura. Update Kura and try again.");
    }
    Ok(b)
}

pub fn read_file(path: &Path) -> Result<Backup> {
    let json = std::fs::read_to_string(path).with_context(|| format!("Couldn't read {}", path.display()))?;
    parse(&json)
}

/// Write via a temp file + rename so a crash mid-write never leaves a truncated backup.
pub fn write_file(path: &Path, b: &Backup) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(b)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path).or_else(|_| {
        // rename can fail across some network shares; fall back to a direct write.
        let r = std::fs::copy(&tmp, path).map(|_| ());
        let _ = std::fs::remove_file(&tmp);
        r
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Automatic safety copy
// ---------------------------------------------------------------------------

pub fn safety_paths(data_dir: &Path) -> Vec<PathBuf> {
    let mut paths = vec![data_dir.join("kura_backup.json")];
    if cfg!(test) {
        // Never touch the real user's safety copies from unit tests.
        return paths;
    }

    #[cfg(windows)]
    {
        if let Some(user_profile) = std::env::var_os("USERPROFILE") {
            paths.push(PathBuf::from(user_profile).join(".kura").join("kura_backup.json"));
        }
        if let Some(app_data) = std::env::var_os("APPDATA") {
            paths.push(PathBuf::from(app_data).join("Kura").join("kura_backup.json"));
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            paths.push(home.join(".kura").join("kura_backup.json"));
            paths.push(home.join(".config").join("kura").join("kura_backup.json"));
        }
    }

    paths
}

/// Whether a snapshot holds anything worth protecting (default settings alone don't count).
fn meaningful(b: &Backup) -> bool {
    !b.libraries.is_empty() || !b.watched.is_empty() || !b.progress.is_empty()
}

pub fn write_safety_copy_from(b: &Backup, data_dir: &Path) {
    if !meaningful(b) {
        return;
    }
    for path in safety_paths(data_dir) {
        if let Err(e) = write_file(&path, b) {
            crate::klog!("[kura] safety backup to {} failed: {e:#}", path.display());
        }
    }
}

/// Synchronous safety copy (used at startup, before the async runtime is involved).
pub fn write_safety_copy(conn: &Connection, data_dir: &Path) {
    write_safety_copy_from(&snapshot(conn), data_dir);
}

/// On startup: fill whatever is empty in the database from the newest readable safety copy.
pub fn restore_if_needed(conn: &Connection, data_dir: &Path) -> Result<bool> {
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0) };
    let no_settings = count("SELECT COUNT(*) FROM settings WHERE key LIKE 'pref.%'") == 0;
    let no_libraries = count("SELECT COUNT(*) FROM libraries") == 0;
    let no_history = count("SELECT COUNT(*) FROM watch_state") == 0 && count("SELECT COUNT(*) FROM watch_progress") == 0;
    let no_track_prefs = count("SELECT COUNT(*) FROM media_track_prefs") == 0;
    if !(no_settings || no_libraries || no_history || no_track_prefs) {
        return Ok(false);
    }

    let newest = safety_paths(data_dir)
        .into_iter()
        .filter_map(|p| read_file(&p).ok().map(|b| (p, b)))
        .max_by_key(|(_, b)| b.updated_at);
    let Some((path, b)) = newest else { return Ok(false) };

    let opts = ImportOptions {
        settings: no_settings && !b.settings.is_empty(),
        libraries: no_libraries && !b.libraries.is_empty(),
        history: no_history && (!b.watched.is_empty() || !b.progress.is_empty()),
        track_prefs: no_track_prefs && !b.track_prefs.is_empty(),
        // Manual matches only make sense together with freshly restored libraries.
        matches: no_libraries && !b.manual_matches.is_empty(),
    };
    if !(opts.settings || opts.libraries || opts.history || opts.track_prefs || opts.matches) {
        return Ok(false);
    }
    apply(conn, &b, &opts)?;
    crate::klog!("[kura] restored user data from backup: {}", path.display());
    Ok(true)
}

static DIRTY: AtomicBool = AtomicBool::new(false);

/// Ask for a safety copy soon. Cheap — the actual write is debounced in the background.
pub fn mark_dirty() {
    DIRTY.store(true, Ordering::Relaxed);
}

/// Cheap change detector for data written outside of commands (e.g. playback progress).
fn fingerprint(conn: &Connection) -> String {
    conn.query_row(
        "SELECT
            (SELECT COUNT(*) || ':' || IFNULL(MAX(watched_at), 0) FROM watch_state) || '|' ||
            (SELECT COUNT(*) || ':' || IFNULL(MAX(updated_at), 0) FROM watch_progress) || '|' ||
            (SELECT COUNT(*) || ':' || IFNULL(MAX(updated_at), 0) FROM media_track_prefs) || '|' ||
            (SELECT COUNT(*) || ':' || IFNULL(SUM(anilist_id), 0) FROM match_groups WHERE manual = 1)",
        [],
        |r| r.get(0),
    )
    .unwrap_or_default()
}

/// Background writer: flushes within ~5 s of [`mark_dirty`], and checks for other changes
/// (watch progress, auto-watched episodes…) once a minute.
pub fn start_auto(st: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let mut last = fingerprint(&st.db());
        let mut ticks: u32 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            ticks = ticks.wrapping_add(1);
            let dirty = DIRTY.swap(false, Ordering::Relaxed);
            if !dirty && ticks % 12 != 0 {
                continue;
            }
            let (fp, snap) = {
                let conn = st.db();
                let fp = fingerprint(&conn);
                if !dirty && fp == last {
                    continue;
                }
                (fp, snapshot(&conn))
            };
            last = fp;
            let dir = st.data_dir.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || write_safety_copy_from(&snap, &dir)).await;
        }
    });
}

// ---------------------------------------------------------------------------
// Summaries for the UI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    pub name: String,
    pub folders: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub path: String,
    pub created_at: i64,
    pub app_version: String,
    pub format: u32,
    pub settings: usize,
    pub libraries: Vec<LibrarySummary>,
    pub watched: usize,
    pub shows_watched: usize,
    pub progress: usize,
    pub track_prefs: usize,
    pub manual_matches: usize,
}

pub fn summarize(path: &Path, b: &Backup) -> BackupSummary {
    let mut shows: Vec<i64> = b.watched.iter().map(|w| w.anilist_id).collect();
    shows.sort_unstable();
    shows.dedup();
    BackupSummary {
        path: path.to_string_lossy().to_string(),
        created_at: b.updated_at,
        app_version: b.app_version.clone(),
        format: b.version,
        settings: b.settings.keys().filter(|k| k.starts_with("pref.")).count(),
        libraries: b.libraries.iter().map(|l| LibrarySummary { name: l.name.clone(), folders: l.folders.len() }).collect(),
        watched: b.watched.len(),
        shows_watched: shows.len(),
        progress: b.progress.len(),
        track_prefs: b.track_prefs.len(),
        manual_matches: b.manual_matches.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate_for_tests(&conn).unwrap();
        conn
    }

    fn seed(conn: &Connection) {
        crate::db::set_setting(conn, "pref.theme", "manga").unwrap();
        conn.execute("INSERT INTO libraries (id, name, kind, created_at) VALUES (1, 'Anime', 'anime', 0)", []).unwrap();
        conn.execute("INSERT INTO library_folders (library_id, path) VALUES (1, 'D:\\Anime')", []).unwrap();
        conn.execute("INSERT INTO watch_state VALUES (21, '1', 100), (21, '2', 200)", []).unwrap();
        conn.execute("INSERT INTO watch_progress VALUES (21, '3', 600.0, 1400.0, 300)", []).unwrap();
        conn.execute("INSERT INTO media_track_prefs VALUES (21, 'eng', 'off', NULL, 50)", []).unwrap();
        conn.execute(
            "INSERT INTO match_groups (library_id, group_key, folder_path, display_name, title_guess, title_full, anilist_id, confidence, manual, attempted, created_at)
             VALUES (1, 'D:\\Anime|dir:One Piece', 'D:\\Anime\\One Piece', 'One Piece', 'One Piece', 'One Piece', 21, 1.0, 1, 1, 0)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn roundtrip_restores_everything() {
        let a = fresh();
        seed(&a);
        let b = snapshot(&a);
        assert_eq!(b.watched.len(), 2);
        assert_eq!(b.manual_matches.len(), 1);
        assert_eq!(b.manual_matches[0].root, "D:\\Anime");
        assert_eq!(b.manual_matches[0].rest, "dir:One Piece");

        let json = serde_json::to_string(&b).unwrap();
        let parsed = parse(&json).unwrap();

        let c = fresh();
        let rep = apply(&c, &parsed, &ImportOptions::ALL).unwrap();
        assert_eq!(rep.libraries_added, 1);
        assert_eq!(rep.watched_added, 2);
        assert_eq!(rep.progress_added, 1);
        assert_eq!(rep.matches, 1);
        let key: String = c.query_row("SELECT group_key FROM match_groups WHERE manual = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(key, "D:\\Anime|dir:One Piece");
        assert_eq!(crate::db::get_setting(&c, "pref.theme").unwrap().as_deref(), Some("manga"));

        // Importing twice changes nothing.
        let again = apply(&c, &parsed, &ImportOptions::ALL).unwrap();
        assert_eq!(again.libraries_added + again.folders_added + again.watched_added + again.progress_added, 0);
    }

    #[test]
    fn manual_match_follows_moved_folder() {
        let a = fresh();
        seed(&a);
        let b = snapshot(&a);

        let c = fresh();
        c.execute("INSERT INTO libraries (id, name, kind, created_at) VALUES (1, 'Anime', 'anime', 0)", []).unwrap();
        c.execute("INSERT INTO library_folders (library_id, path) VALUES (1, 'E:\\Anime')", []).unwrap();
        let opts = ImportOptions { libraries: false, ..ImportOptions::ALL };
        apply(&c, &b, &opts).unwrap();
        let (key, fp): (String, String) = c
            .query_row("SELECT group_key, folder_path FROM match_groups WHERE manual = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(key, "E:\\Anime|dir:One Piece");
        assert_eq!(fp, "E:\\Anime\\One Piece");
    }

    #[test]
    fn newer_local_progress_wins_and_watched_blocks_progress() {
        let a = fresh();
        seed(&a);
        let b = snapshot(&a);

        let c = fresh();
        c.execute("INSERT INTO watch_progress VALUES (21, '3', 900.0, 1400.0, 999)", []).unwrap();
        c.execute("INSERT INTO watch_state VALUES (21, '2', 5)", []).unwrap();
        apply(&c, &b, &ImportOptions::ALL).unwrap();
        let pos: f64 = c.query_row("SELECT position FROM watch_progress WHERE ep_key = '3'", [], |r| r.get(0)).unwrap();
        assert_eq!(pos, 900.0);
        let at: i64 = c.query_row("SELECT watched_at FROM watch_state WHERE ep_key = '2'", [], |r| r.get(0)).unwrap();
        assert_eq!(at, 5, "existing watch history is kept");
    }

    #[test]
    fn reads_v1_backups_and_rejects_garbage() {
        let v1 = r#"{"version":1,"updated_at":5,"settings":{"pref.theme":"manga"},"libraries":[{"name":"Anime","kind":"anime","folders":["C:\\A"]}]}"#;
        let b = parse(v1).unwrap();
        assert_eq!(b.libraries.len(), 1);
        assert!(b.watched.is_empty());
        assert!(parse("{\"hello\":1}").is_err());
        assert!(parse(r#"{"app":"kura","version":99,"updated_at":1}"#).is_err());
    }
}
