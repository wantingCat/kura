//! Tauri commands exposed to the frontend.

use crate::backup;
use crate::db::{self, now};
use crate::franchise;
use crate::matcher;
use crate::parser;
use crate::player;
use crate::scan;
use crate::service::AppState;
use crate::store;
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

type St<'a> = State<'a, Arc<AppState>>;
type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn json_list(s: Option<String>) -> Vec<String> {
    s.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Libraries
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    id: i64,
    path: String,
    exists: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    id: i64,
    name: String,
    kind: String,
    folders: Vec<Folder>,
    file_count: i64,
    media_count: i64,
}

#[tauri::command]
pub async fn list_libraries(st: St<'_>) -> CmdResult<Vec<Library>> {
    // Read everything from SQLite first and release the lock before touching the disk.
    let (libs, folder_rows) = {
        let conn = st.db();
        let mut stmt = conn
            .prepare(
                "SELECT l.id, l.name, l.kind,
                    (SELECT COUNT(*) FROM local_files f WHERE f.library_id = l.id),
                    (SELECT COUNT(DISTINCT anilist_id) FROM local_files f WHERE f.library_id = l.id AND anilist_id IS NOT NULL)
                 FROM libraries l ORDER BY l.id",
            )
            .map_err(err)?;
        let libs: Vec<(i64, String, String, i64, i64)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        let mut fs = conn
            .prepare("SELECT id, library_id, path FROM library_folders ORDER BY id")
            .map_err(err)?;
        let folder_rows: Vec<(i64, i64, String)> = fs
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        (libs, folder_rows)
    };

    // Checking a folder on a sleeping external HDD or an unreachable network drive can block
    // for many seconds. Check all folders in parallel and give up after a short timeout
    // (treated as "present" so we don't flash a false "folder missing" warning).
    let tasks: Vec<_> = folder_rows
        .iter()
        .map(|(_, _, path)| {
            let path = path.clone();
            tauri::async_runtime::spawn_blocking(move || std::path::Path::new(&path).is_dir())
        })
        .collect();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1500);
    let mut exists = Vec::with_capacity(tasks.len());
    for task in tasks {
        exists.push(match tokio::time::timeout_at(deadline, task).await {
            Ok(Ok(ok)) => ok,
            _ => true,
        });
    }

    let mut out = Vec::new();
    for (id, name, kind, file_count, media_count) in libs {
        let folders = folder_rows
            .iter()
            .zip(&exists)
            .filter(|((_, lib, _), _)| *lib == id)
            .map(|((fid, _, path), ok)| Folder {
                id: *fid,
                path: path.clone(),
                exists: *ok,
            })
            .collect();
        out.push(Library {
            id,
            name,
            kind,
            folders,
            file_count,
            media_count,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn create_library(st: St<'_>, name: String, folders: Vec<String>) -> CmdResult<i64> {
    let conn = st.db();
    conn.execute(
        "INSERT INTO libraries (name, kind, created_at) VALUES (?1, 'anime', ?2)",
        params![name.trim(), now()],
    )
    .map_err(err)?;
    let id = conn.last_insert_rowid();
    for f in folders {
        conn.execute(
            "INSERT OR IGNORE INTO library_folders (library_id, path) VALUES (?1, ?2)",
            params![id, f],
        )
        .map_err(err)?;
    }
    st.watcher.reload();
    drop(conn);
    crate::backup::mark_dirty();
    Ok(id)
}

#[tauri::command]
pub async fn rename_library(st: St<'_>, id: i64, name: String) -> CmdResult<()> {
    st.db()
        .execute(
            "UPDATE libraries SET name = ?2 WHERE id = ?1",
            params![id, name.trim()],
        )
        .map_err(err)?;
    crate::backup::mark_dirty();
    Ok(())
}

#[tauri::command]
pub async fn delete_library(app: AppHandle, st: St<'_>, id: i64) -> CmdResult<()> {
    st.db()
        .execute("DELETE FROM libraries WHERE id = ?1", [id])
        .map_err(err)?;
    st.watcher.reload();
    let _ = app.emit("library-changed", ());
    crate::backup::mark_dirty();
    Ok(())
}

#[tauri::command]
pub async fn add_folder(st: St<'_>, library_id: i64, path: String) -> CmdResult<()> {
    st.db()
        .execute(
            "INSERT OR IGNORE INTO library_folders (library_id, path) VALUES (?1, ?2)",
            params![library_id, path],
        )
        .map_err(err)?;
    st.watcher.reload();
    crate::backup::mark_dirty();
    Ok(())
}

#[tauri::command]
pub async fn remove_folder(app: AppHandle, st: St<'_>, folder_id: i64) -> CmdResult<()> {
    let conn = st.db();
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT library_id, path FROM library_folders WHERE id = ?1",
            [folder_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?;
    if let Some((lib, path)) = row {
        conn.execute("DELETE FROM library_folders WHERE id = ?1", [folder_id])
            .map_err(err)?;
        conn.execute(
            "DELETE FROM local_files WHERE library_id = ?1 AND substr(path, 1, length(?2)) = ?2",
            params![lib, path],
        )
        .map_err(err)?;
        conn.execute("DELETE FROM match_groups WHERE id NOT IN (SELECT DISTINCT group_id FROM local_files WHERE group_id IS NOT NULL)", [])
            .map_err(err)?;
        st.watcher.reload();
    }
    let _ = app.emit("library-changed", ());
    drop(conn);
    crate::backup::mark_dirty();
    Ok(())
}

#[tauri::command]
pub async fn start_scan(app: AppHandle, st: St<'_>, library_id: Option<i64>) -> CmdResult<()> {
    scan::spawn_scan(app, st.inner().clone(), library_id);
    Ok(())
}

#[tauri::command]
pub async fn is_scanning(st: St<'_>) -> CmdResult<bool> {
    Ok(st.scanning.load(std::sync::atomic::Ordering::SeqCst))
}

// ---------------------------------------------------------------------------
// Library browsing
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCard {
    anilist_id: i64,
    title_romaji: Option<String>,
    title_english: Option<String>,
    title_native: Option<String>,
    synonyms: Vec<String>,
    format: Option<String>,
    status: Option<String>,
    episodes: Option<i64>,
    season: Option<String>,
    season_year: Option<i64>,
    genres: Vec<String>,
    tags: Vec<String>,
    studios: Vec<String>,
    average_score: Option<i64>,
    cover_url: Option<String>,
    cover_path: Option<String>,
    cover_color: Option<String>,
    banner_url: Option<String>,
    banner_path: Option<String>,
    logo_url: Option<String>,
    logo_path: Option<String>,
    description: Option<String>,
    owned_count: i64,
    watched_count: i64,
    added_at: i64,
    last_watched_at: Option<i64>,
    needs_review: bool,
    library_ids: Vec<i64>,
    /// AniList id of the first owned entry in this franchise (itself if standalone).
    franchise_id: i64,
    franchise_index: usize,
    franchise_size: usize,
    anilist_status: Option<String>,
}

#[tauri::command]
pub async fn get_library(st: St<'_>, library_id: Option<i64>) -> CmdResult<Vec<MediaCard>> {
    let conn = st.db();
    let mut stmt = conn
        .prepare(
            "SELECT m.anilist_id, m.title_romaji, m.title_english, m.title_native, m.synonyms, m.format, m.status,
                    m.episodes, m.season, m.season_year, m.genres, m.average_score, m.cover_url, m.cover_path,
                    m.cover_color, m.banner_url, m.banner_path, m.description,
                    COUNT(DISTINCT CASE WHEN f.ep_key NOT LIKE 'S%' THEN f.ep_key END) AS owned,
                    MIN(f.added_at) AS added,
                    MIN(CASE WHEN g.manual = 1 THEN 1.0 ELSE COALESCE(g.confidence, 1.0) END) AS conf,
                    GROUP_CONCAT(DISTINCT f.library_id) AS libs,
                    MAX(
                      (SELECT COUNT(*) FROM watch_state w WHERE w.anilist_id = m.anilist_id AND w.ep_key NOT LIKE 'S%' AND w.ep_key NOT LIKE 'extra:%'),
                      COALESCE((SELECT CASE WHEN ae.status = 'COMPLETED' THEN MAX(ae.progress, COALESCE(m.episodes, 1)) ELSE ae.progress END FROM anilist_entries ae WHERE ae.anilist_id = m.anilist_id), 0)
                    ) AS watched,
                    (SELECT MAX(watched_at) FROM watch_state w WHERE w.anilist_id = m.anilist_id) AS last_watched,
                    NULLIF(m.logo_url, ''), m.logo_path, m.tags, m.studios,
                    (SELECT ae.status FROM anilist_entries ae WHERE ae.anilist_id = m.anilist_id) AS al_status
             FROM media m
             JOIN local_files f ON f.anilist_id = m.anilist_id
             LEFT JOIN match_groups g ON g.id = f.group_id
             WHERE (?1 IS NULL OR f.library_id = ?1)
             GROUP BY m.anilist_id",
        )
        .map_err(err)?;
    let franchises = franchise::load(&conn).map_err(err)?;
    let mut rows = stmt
        .query_map([library_id], |r: &Row| {
            let conf: f64 = r.get(20)?;
            let libs: Option<String> = r.get(21)?;
            Ok(MediaCard {
                anilist_id: r.get(0)?,
                title_romaji: r.get(1)?,
                title_english: r.get(2)?,
                title_native: r.get(3)?,
                synonyms: json_list(r.get(4)?),
                format: r.get(5)?,
                status: r.get(6)?,
                episodes: r.get(7)?,
                season: r.get(8)?,
                season_year: r.get(9)?,
                genres: json_list(r.get(10)?),
                tags: json_list(r.get(26)?),
                studios: json_list(r.get(27)?),
                average_score: r.get(11)?,
                cover_url: r.get(12)?,
                cover_path: r.get(13)?,
                cover_color: r.get(14)?,
                banner_url: r.get(15)?,
                banner_path: r.get(16)?,
                description: r.get(17)?,
                owned_count: r.get(18)?,
                added_at: r.get::<_, Option<i64>>(19)?.unwrap_or(0),
                needs_review: conf < matcher::REVIEW_THRESHOLD,
                library_ids: libs
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|s| s.parse().ok())
                    .collect(),
                watched_count: r.get(22)?,
                last_watched_at: r.get(23)?,
                logo_url: r.get(24)?,
                logo_path: r.get(25)?,
                franchise_id: 0,
                franchise_index: 0,
                franchise_size: 1,
                anilist_status: r.get(28)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    for c in &mut rows {
        match franchises.get(&c.anilist_id) {
            Some(p) => {
                c.franchise_id = p.root;
                c.franchise_index = p.index;
                c.franchise_size = p.size;
            }
            None => c.franchise_id = c.anilist_id,
        }
    }
    let app_mode = db::get_setting(&conn, "pref.app_mode")
        .ok()
        .flatten()
        .unwrap_or_else(|| "local".to_string());
    if app_mode == "streaming" && library_id.is_none() {
        let existing_ids: std::collections::HashSet<i64> =
            rows.iter().map(|c| c.anilist_id).collect();
        if let Ok(mut s_stmt) = conn.prepare(
            "SELECT m.anilist_id, m.title_romaji, m.title_english, m.title_native, m.synonyms, m.format, m.status,
                    m.episodes, m.season, m.season_year, m.genres, m.average_score, m.cover_url, m.cover_path,
                    m.cover_color, m.banner_url, m.banner_path, m.description,
                    COALESCE(sl.added_at, ae.updated_at, m.fetched_at) AS added,
                    MAX(
                      (SELECT COUNT(*) FROM watch_state w WHERE w.anilist_id = m.anilist_id AND w.ep_key NOT LIKE 'S%' AND w.ep_key NOT LIKE 'extra:%'),
                      CASE WHEN ae.status = 'COMPLETED' THEN MAX(COALESCE(ae.progress, 0), COALESCE(m.episodes, 1)) ELSE COALESCE(ae.progress, 0) END
                    ) AS watched,
                    (SELECT MAX(watched_at) FROM watch_state w WHERE w.anilist_id = m.anilist_id) AS last_watched,
                    NULLIF(m.logo_url, ''), m.logo_path, m.tags, m.studios,
                    (SELECT ae.status FROM anilist_entries ae WHERE ae.anilist_id = m.anilist_id) AS al_status
             FROM media m
             LEFT JOIN stream_library sl ON sl.anilist_id = m.anilist_id
             LEFT JOIN anilist_entries ae ON ae.anilist_id = m.anilist_id
             WHERE sl.anilist_id IS NOT NULL
                OR (ae.anilist_id IS NOT NULL AND ae.status IN ('CURRENT', 'PLANNING', 'COMPLETED', 'PAUSED', 'DROPPED', 'REPEATING'))",
        ) {
            if let Ok(extra_iter) = s_stmt.query_map([], |r: &Row| {
                let id: i64 = r.get(0)?;
                Ok(MediaCard {
                    anilist_id: id,
                    title_romaji: r.get(1)?,
                    title_english: r.get(2)?,
                    title_native: r.get(3)?,
                    synonyms: json_list(r.get(4)?),
                    format: r.get(5)?,
                    status: r.get(6)?,
                    episodes: r.get(7)?,
                    season: r.get(8)?,
                    season_year: r.get(9)?,
                    genres: json_list(r.get(10)?),
                    average_score: r.get(11)?,
                    cover_url: r.get(12)?,
                    cover_path: r.get(13)?,
                    cover_color: r.get(14)?,
                    banner_url: r.get(15)?,
                    banner_path: r.get(16)?,
                    description: r.get(17)?,
                    owned_count: 0,
                    added_at: r.get::<_, Option<i64>>(18)?.unwrap_or(0),
                    watched_count: r.get::<_, Option<i64>>(19)?.unwrap_or(0),
                    last_watched_at: r.get(20)?,
                    logo_url: r.get(21)?,
                    logo_path: r.get(22)?,
                    tags: json_list(r.get(23)?),
                    studios: json_list(r.get(24)?),
                    needs_review: false,
                    library_ids: Vec::new(),
                    franchise_id: id,
                    franchise_index: 0,
                    franchise_size: 1,
                    anilist_status: r.get(25)?,
                })
            }) {
                for card in extra_iter.flatten() {
                    if !existing_ids.contains(&card.anilist_id) {
                        rows.push(card);
                    }
                }
            }
        }
    }
    for c in &mut rows {
        match franchises.get(&c.anilist_id) {
            Some(p) => {
                c.franchise_id = p.root;
                c.franchise_index = p.index;
                c.franchise_size = p.size;
            }
            None => c.franchise_id = c.anilist_id,
        }
    }
    Ok(rows)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmatchedGroup {
    id: i64,
    library_id: i64,
    display_name: String,
    folder_path: Option<String>,
    title_guess: String,
    file_count: i64,
    confidence: Option<f64>,
    attempted: bool,
    sample_files: Vec<String>,
}

#[tauri::command]
pub async fn get_unmatched(st: St<'_>) -> CmdResult<Vec<UnmatchedGroup>> {
    let conn = st.db();
    let mut stmt = conn
        .prepare(
            "SELECT g.id, g.library_id, g.display_name, g.folder_path, g.title_guess, COUNT(f.id), g.confidence, g.attempted
             FROM match_groups g JOIN local_files f ON f.group_id = g.id
             WHERE g.anilist_id IS NULL
             GROUP BY g.id ORDER BY g.display_name COLLATE NOCASE",
        )
        .map_err(err)?;
    let mut groups: Vec<UnmatchedGroup> = stmt
        .query_map([], |r| {
            Ok(UnmatchedGroup {
                id: r.get(0)?,
                library_id: r.get(1)?,
                display_name: r.get(2)?,
                folder_path: r.get(3)?,
                title_guess: r.get(4)?,
                file_count: r.get(5)?,
                confidence: r.get(6)?,
                attempted: r.get::<_, i64>(7)? != 0,
                sample_files: Vec::new(),
            })
        })
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    for g in &mut groups {
        let mut s = conn
            .prepare("SELECT path FROM local_files WHERE group_id = ?1 ORDER BY path LIMIT 3")
            .map_err(err)?;
        g.sample_files = s
            .query_map([g.id], |r| r.get::<_, String>(0))
            .map_err(err)?
            .filter_map(|r| r.ok())
            .map(|p| {
                std::path::Path::new(&p)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or(p)
            })
            .collect();
    }
    Ok(groups)
}

// ---------------------------------------------------------------------------
// Detail page
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
    id: i64,
    path: String,
    file_name: String,
    size: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeRow {
    ep_key: String,
    number: i64,
    is_special: bool,
    title_en: Option<String>,
    title_ja: Option<String>,
    title_romaji: Option<String>,
    overview: Option<String>,
    air_date: Option<String>,
    runtime: Option<i64>,
    thumb_url: Option<String>,
    thumb_path: Option<String>,
    filler: bool,
    recap: bool,
    files: Vec<FileRef>,
    watched_at: Option<i64>,
    /// Saved playback position / duration in seconds (resume point).
    progress_pos: Option<f64>,
    progress_dur: Option<f64>,
    subtitles: Vec<crate::subtitles::LinkedSubtitle>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationCard {
    related_id: i64,
    relation_type: String,
    title: Option<String>,
    format: Option<String>,
    status: Option<String>,
    episodes: Option<i64>,
    season_year: Option<i64>,
    cover_url: Option<String>,
    cover_path: Option<String>,
    owned_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupRef {
    id: i64,
    display_name: String,
    folder_path: Option<String>,
    confidence: Option<f64>,
    manual: bool,
}

/// One owned entry of the franchise this title belongs to, in release order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FranchiseEntry {
    anilist_id: i64,
    title_romaji: Option<String>,
    title_english: Option<String>,
    title_native: Option<String>,
    format: Option<String>,
    status: Option<String>,
    season: Option<String>,
    season_year: Option<i64>,
    episodes: Option<i64>,
    owned_count: i64,
    watched_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraFile {
    pub id: i64,
    pub path: String,
    pub file_name: String,
    pub title: String,
    pub kind: String,
    pub size: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterCard {
    pub character_id: i64,
    pub role: String,
    pub name_full: String,
    pub name_native: Option<String>,
    pub image_url: Option<String>,
    pub va_id: Option<i64>,
    pub va_name_full: Option<String>,
    pub va_name_native: Option<String>,
    pub va_image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaffCard {
    pub staff_id: i64,
    pub role: String,
    pub name_full: String,
    pub name_native: Option<String>,
    pub image_url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDetail {
    anilist_id: i64,
    id_mal: Option<i64>,
    title_romaji: Option<String>,
    title_english: Option<String>,
    title_native: Option<String>,
    synonyms: Vec<String>,
    format: Option<String>,
    status: Option<String>,
    episodes: Option<i64>,
    duration: Option<i64>,
    season: Option<String>,
    season_year: Option<u32>,
    start_date: Option<String>,
    end_date: Option<String>,
    description: Option<String>,
    genres: Vec<String>,
    tags: Vec<String>,
    studios: Vec<String>,
    cover_url: Option<String>,
    cover_path: Option<String>,
    cover_color: Option<String>,
    banner_url: Option<String>,
    banner_path: Option<String>,
    logo_url: Option<String>,
    logo_path: Option<String>,
    average_score: Option<i64>,
    next_airing_episode: Option<i64>,
    next_airing_at: Option<i64>,
    episodes_source: Option<String>,
    episode_list: Vec<EpisodeRow>,
    relations: Vec<RelationCard>,
    extras: Vec<ExtraFile>,
    other_files: Vec<FileRef>,
    groups: Vec<GroupRef>,
    franchise: Vec<FranchiseEntry>,
    track_pref: Option<db::MediaTrackPref>,
    anilist_status: Option<String>,
    anilist_progress: Option<i64>,
    anilist_score: Option<f64>,
    characters: Vec<CharacterCard>,
    staff: Vec<StaffCard>,
    in_stream_library: bool,
}

fn file_ref(id: i64, path: String, size: i64) -> FileRef {
    let file_name = std::path::Path::new(&path)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    FileRef {
        id,
        path,
        file_name,
        size,
    }
}

#[tauri::command]
pub async fn get_media_detail(app: AppHandle, st: St<'_>, anilist_id: i64) -> CmdResult<MediaDetail> {
    let (has_any_row, has_full_media, has_episodes, has_credits) = {
        let conn = st.db();
        let _ = conn.execute_batch(db::SCHEMA_V5);
        let _ = conn.execute_batch(db::SCHEMA_V6);
        let any: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM media WHERE anilist_id = ?1)",
                [anilist_id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        let hm: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM media WHERE anilist_id = ?1 AND fetched_at > 0)",
                [anilist_id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        let he: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM episodes WHERE anilist_id = ?1)",
                [anilist_id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        let hc: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM media_characters WHERE anilist_id = ?1)",
                [anilist_id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        (any, hm, he, hc)
    };

    // Only block if we have zero row in SQLite for this `anilist_id`.
    // If we already have a seeded row (from Library, Discover, Seasonal Calendar, or AniList sync),
    // return immediately in ~1ms and hydrate missing episode titles/credits in the background!
    if !has_any_row {
        crate::service::ensure_media(&st, anilist_id)
            .await
            .map_err(err)?;
    }

    if !has_full_media || !has_episodes || !has_credits {
        let st_bg = st.inner().clone();
        let app_bg = app.clone();
        let need_media = has_any_row && !has_full_media;
        let need_eps = !has_episodes;
        let need_creds = !has_credits && has_full_media;
        tokio::spawn(async move {
            let mut changed = false;
            if need_media {
                if crate::service::ensure_media(&st_bg, anilist_id).await.is_ok() {
                    changed = true;
                }
            }
            if need_eps {
                if crate::service::ensure_episodes(&st_bg, anilist_id).await.is_ok() {
                    changed = true;
                }
            }
            if need_creds {
                if crate::service::ensure_credits(&st_bg, anilist_id).await.is_ok() {
                    changed = true;
                }
            }
            if changed {
                let _ = app_bg.emit("library-changed", ());
            }
        });
    }

    let conn = st.db();
    crate::anilist_sync::reconcile_media_entry(&conn, anilist_id, None);
    let mut d = conn
        .query_row(
            "SELECT anilist_id, id_mal, title_romaji, title_english, title_native, synonyms, format, status, episodes,
                    duration, season, season_year, start_date, end_date, description, genres, tags, studios, cover_url,
                    cover_path, cover_color, banner_url, banner_path, average_score, next_airing_episode, next_airing_at,
                    episodes_source, NULLIF(logo_url, ''), logo_path
             FROM media WHERE anilist_id = ?1",
            [anilist_id],
            |r| {
                Ok(MediaDetail {
                    anilist_id: r.get(0)?,
                    id_mal: r.get(1)?,
                    title_romaji: r.get(2)?,
                    title_english: r.get(3)?,
                    title_native: r.get(4)?,
                    synonyms: json_list(r.get(5)?),
                    format: r.get(6)?,
                    status: r.get(7)?,
                    episodes: r.get(8)?,
                    duration: r.get(9)?,
                    season: r.get(10)?,
                    season_year: r.get(11)?,
                    start_date: r.get(12)?,
                    end_date: r.get(13)?,
                    description: r.get(14)?,
                    genres: json_list(r.get(15)?),
                    tags: json_list(r.get(16)?),
                    studios: json_list(r.get(17)?),
                    cover_url: r.get(18)?,
                    cover_path: r.get(19)?,
                    cover_color: r.get(20)?,
                    banner_url: r.get(21)?,
                    banner_path: r.get(22)?,
                    average_score: r.get(23)?,
                    next_airing_episode: r.get(24)?,
                    next_airing_at: r.get(25)?,
                    episodes_source: r.get(26)?,
                    logo_url: r.get(27)?,
                    logo_path: r.get(28)?,
                    episode_list: Vec::new(),
                    relations: Vec::new(),
                    extras: Vec::new(),
                    other_files: Vec::new(),
                    groups: Vec::new(),
                    franchise: Vec::new(),
                    track_pref: None,
                    anilist_status: None,
                    anilist_progress: None,
                    anilist_score: None,
                    characters: Vec::new(),
                    staff: Vec::new(),
                    in_stream_library: false,
                })
            },
        )
        .map_err(err)?;

    // Files for this entry
    let mut files_by_key: HashMap<String, Vec<FileRef>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, path, size, ep_key, special_hint, parsed FROM local_files WHERE anilist_id = ?1 ORDER BY path")
            .map_err(err)?;
        let rows = stmt
            .query_map([anilist_id], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(err)?;
        for row in rows.flatten() {
            let (id, path, size, key, special_hint, parsed_str) = row;
            let is_extra =
                special_hint.as_deref() == Some("extra") || parsed_str.contains("\"extra\":true");
            if is_extra {
                let file_name = std::path::Path::new(&path)
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                let (kind, title) = parser::classify_extra(&file_name);
                let kind_str = match kind {
                    parser::ExtraKind::Opening => "opening",
                    parser::ExtraKind::Ending => "ending",
                    parser::ExtraKind::Trailer => "trailer",
                    parser::ExtraKind::Pv => "pv",
                    parser::ExtraKind::Bonus => "bonus",
                    parser::ExtraKind::Other => "other",
                };
                d.extras.push(ExtraFile {
                    id,
                    path,
                    file_name,
                    title,
                    kind: kind_str.to_string(),
                    size,
                });
            } else {
                match key {
                    Some(k) => files_by_key
                        .entry(k)
                        .or_default()
                        .push(file_ref(id, path, size)),
                    None => d.other_files.push(file_ref(id, path, size)),
                }
            }
        }
        d.extras.sort_by(|a, b| {
            let order = |k: &str| match k {
                "opening" => 1,
                "ending" => 2,
                "trailer" => 3,
                "pv" => 4,
                "bonus" => 5,
                _ => 6,
            };
            order(&a.kind)
                .cmp(&order(&b.kind))
                .then_with(|| a.title.cmp(&b.title))
        });
    }

    let mut watched: HashMap<String, i64> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT ep_key, watched_at FROM watch_state WHERE anilist_id = ?1")
            .map_err(err)?;
        for row in stmt
            .query_map([anilist_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(err)?
            .flatten()
        {
            watched.insert(row.0, row.1);
        }
    }
    let mut progress: HashMap<String, (f64, f64)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT ep_key, position, duration FROM watch_progress WHERE anilist_id = ?1")
            .map_err(err)?;
        for row in stmt
            .query_map([anilist_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, f64>(2)?,
                ))
            })
            .map_err(err)?
            .flatten()
        {
            progress.insert(row.0, (row.1, row.2));
        }
    }

    {
        let mut stmt = conn
            .prepare(
                "SELECT ep_key, number, is_special, title_en, title_ja, title_romaji, overview, air_date, runtime,
                        thumb_url, thumb_path, filler, recap
                 FROM episodes WHERE anilist_id = ?1",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([anilist_id], |r| {
                Ok(EpisodeRow {
                    ep_key: r.get(0)?,
                    number: r.get(1)?,
                    is_special: r.get::<_, i64>(2)? != 0,
                    title_en: r.get(3)?,
                    title_ja: r.get(4)?,
                    title_romaji: r.get(5)?,
                    overview: r.get(6)?,
                    air_date: r.get(7)?,
                    runtime: r.get(8)?,
                    thumb_url: r.get(9)?,
                    thumb_path: r.get(10)?,
                    filler: r.get::<_, i64>(11)? != 0,
                    recap: r.get::<_, i64>(12)? != 0,
                    files: Vec::new(),
                    watched_at: None,
                    subtitles: Vec::new(),
                    progress_pos: None,
                    progress_dur: None,
                })
            })
            .map_err(err)?;
        for row in rows.flatten() {
            d.episode_list.push(row);
        }
    }

    // Files mapped to keys that have no metadata row (e.g. episode 13 of a "12 episode" show).
    let known: std::collections::HashSet<String> =
        d.episode_list.iter().map(|e| e.ep_key.clone()).collect();
    for key in files_by_key.keys() {
        if !known.contains(key) {
            let is_special = key.starts_with('S');
            let number = key.trim_start_matches('S').parse().unwrap_or(0);
            d.episode_list.push(EpisodeRow {
                ep_key: key.clone(),
                number,
                is_special,
                title_en: None,
                title_ja: None,
                title_romaji: None,
                overview: None,
                air_date: None,
                runtime: None,
                thumb_url: None,
                thumb_path: None,
                filler: false,
                recap: false,
                files: Vec::new(),
                watched_at: None,
                subtitles: Vec::new(),
                progress_pos: None,
                progress_dur: None,
            });
        }
    }
    for e in &mut d.episode_list {
        e.files = files_by_key.remove(&e.ep_key).unwrap_or_default();
        e.watched_at = watched.get(&e.ep_key).copied();
        if let Some((p, dur)) = progress.get(&e.ep_key) {
            e.progress_pos = Some(*p);
            e.progress_dur = Some(*dur);
        }
    }
    // Specials are only interesting if owned or described.
    d.episode_list.retain(|e| {
        !e.is_special || !e.files.is_empty() || e.title_en.is_some() || e.title_ja.is_some()
    });
    let max_watched_ep: i64 = watched
        .keys()
        .filter(|k| !k.starts_with('S') && !k.starts_with("extra:"))
        .filter_map(|k| k.parse::<i64>().ok())
        .max()
        .unwrap_or(0);
    let has_any_regular = d.episode_list.iter().any(|e| !e.is_special);
    let expected_regular = d
        .episodes
        .or_else(|| d.next_airing_episode.map(|n| (n - 1).max(1)))
        .unwrap_or(if d.format.as_deref() == Some("MOVIE") {
            1
        } else if has_any_regular {
            0
        } else {
            12
        })
        .max(max_watched_ep)
        .clamp(0, 2000);
    if expected_regular > 0 {
        let existing_regular: std::collections::HashSet<i64> = d
            .episode_list
            .iter()
            .filter(|e| !e.is_special)
            .map(|e| e.number)
            .collect();
        for n in 1..=expected_regular {
            if !existing_regular.contains(&n) {
                let key = n.to_string();
                let (progress_pos, progress_dur) = progress
                    .get(&key)
                    .map(|(p, dur)| (Some(*p), Some(*dur)))
                    .unwrap_or((None, None));
                d.episode_list.push(EpisodeRow {
                    ep_key: key.clone(),
                    number: n,
                    is_special: false,
                    title_en: None,
                    title_ja: None,
                    title_romaji: None,
                    overview: None,
                    air_date: None,
                    runtime: d.duration,
                    thumb_url: None,
                    thumb_path: None,
                    filler: false,
                    recap: false,
                    files: Vec::new(),
                    watched_at: watched.get(&key).copied(),
                    progress_pos,
                    progress_dur,
                    subtitles: Vec::new(),
                });
            }
        }
    }
    d.episode_list.sort_by(|a, b| {
        a.is_special
            .cmp(&b.is_special)
            .then(a.number.cmp(&b.number))
    });
    {
        let mut subs_by_ep: HashMap<String, Vec<crate::subtitles::LinkedSubtitle>> = HashMap::new();
        for s in crate::subtitles::list_linked_subtitles(&conn, anilist_id).unwrap_or_default() {
            subs_by_ep.entry(s.ep_key.clone()).or_default().push(s);
        }
        for e in &mut d.episode_list {
            if let Some(subs) = subs_by_ep.remove(&e.ep_key) {
                e.subtitles = subs;
            }
        }
    }

    // Relations
    {
        let mut stmt = conn
            .prepare(
                "SELECT r.related_id, r.relation_type, COALESCE(m.title_english, m.title_romaji, r.title), r.format, r.status,
                        r.episodes, r.season_year, r.cover_url, m.cover_path,
                        (SELECT COUNT(*) FROM local_files f WHERE f.anilist_id = r.related_id)
                 FROM media_relations r LEFT JOIN media m ON m.anilist_id = r.related_id
                 WHERE r.anilist_id = ?1 AND COALESCE(r.media_type, 'ANIME') = 'ANIME'",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([anilist_id], |r| {
                Ok(RelationCard {
                    related_id: r.get(0)?,
                    relation_type: r.get(1)?,
                    title: r.get(2)?,
                    format: r.get(3)?,
                    status: r.get(4)?,
                    episodes: r.get(5)?,
                    season_year: r.get(6)?,
                    cover_url: r.get(7)?,
                    cover_path: r.get(8)?,
                    owned_count: r.get(9)?,
                })
            })
            .map_err(err)?;
        let order = |t: &str| match t {
            "PREQUEL" => 0,
            "SEQUEL" => 1,
            "PARENT" => 2,
            "SIDE_STORY" => 3,
            "SPIN_OFF" => 4,
            "ALTERNATIVE" => 5,
            "SUMMARY" => 6,
            _ => 9,
        };
        let mut rels: Vec<RelationCard> = rows
            .flatten()
            .filter(|r| r.relation_type != "CHARACTER")
            .collect();
        rels.sort_by(|a, b| {
            order(&a.relation_type)
                .cmp(&order(&b.relation_type))
                .then(a.season_year.cmp(&b.season_year))
        });
        d.relations = rels;
    }

    // Match groups contributing files
    {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT g.id, g.display_name, g.folder_path, g.confidence, g.manual
                 FROM match_groups g JOIN local_files f ON f.group_id = g.id WHERE f.anilist_id = ?1",
            )
            .map_err(err)?;
        d.groups = stmt
            .query_map([anilist_id], |r| {
                Ok(GroupRef {
                    id: r.get(0)?,
                    display_name: r.get(1)?,
                    folder_path: r.get(2)?,
                    confidence: r.get(3)?,
                    manual: r.get::<_, i64>(4)? != 0,
                })
            })
            .map_err(err)?
            .flatten()
            .collect();
    }

    // Other owned seasons / movies / OVAs of the same franchise
    {
        let franchises = franchise::load(&conn).map_err(err)?;
        if let Some(me) = franchises.get(&anilist_id).filter(|p| p.size > 1) {
            let mut members: Vec<(usize, i64)> = franchises
                .iter()
                .filter(|(_, p)| p.root == me.root)
                .map(|(id, p)| (p.index, *id))
                .collect();
            members.sort();
            let mut stmt = conn
                .prepare(
                    "SELECT m.anilist_id, m.title_romaji, m.title_english, m.title_native, m.format, m.status, m.season,
                            m.season_year, m.episodes,
                            (SELECT COUNT(DISTINCT f.ep_key) FROM local_files f
                              WHERE f.anilist_id = m.anilist_id AND f.ep_key NOT LIKE 'S%'),
                            MAX(
                              (SELECT COUNT(*) FROM watch_state w WHERE w.anilist_id = m.anilist_id AND w.ep_key NOT LIKE 'S%' AND w.ep_key NOT LIKE 'extra:%'),
                              COALESCE((SELECT CASE WHEN ae.status = 'COMPLETED' THEN MAX(ae.progress, COALESCE(m.episodes, 1)) ELSE ae.progress END FROM anilist_entries ae WHERE ae.anilist_id = m.anilist_id), 0)
                            )
                     FROM media m WHERE m.anilist_id = ?1",
                )
                .map_err(err)?;
            for (_, id) in members {
                let entry = stmt
                    .query_row([id], |r| {
                        Ok(FranchiseEntry {
                            anilist_id: r.get(0)?,
                            title_romaji: r.get(1)?,
                            title_english: r.get(2)?,
                            title_native: r.get(3)?,
                            format: r.get(4)?,
                            status: r.get(5)?,
                            season: r.get(6)?,
                            season_year: r.get(7)?,
                            episodes: r.get(8)?,
                            owned_count: r.get(9)?,
                            watched_count: r.get(10)?,
                        })
                    })
                    .optional()
                    .map_err(err)?;
                d.franchise.extend(entry);
            }
        }
    }
    d.track_pref = db::get_media_track_pref(&conn, anilist_id).map_err(err)?;
    let _ = conn.execute_batch(db::SCHEMA_V7);
    d.in_stream_library = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM stream_library WHERE anilist_id = ?1)",
            [anilist_id],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if let Some(rem) = crate::anilist_sync::get_remote_entry(&conn, anilist_id) {
        d.anilist_status = Some(rem.status);
        d.anilist_progress = Some(rem.progress);
        d.anilist_score = rem.score;
    }
    {
        let _ = conn.execute_batch(db::SCHEMA_V6);
        if let Ok(mut stmt) = conn.prepare(
            "SELECT character_id, role, name_full, name_native, image_url,
                    va_id, va_name_full, va_name_native, va_image_url
             FROM media_characters WHERE anilist_id = ?1 ORDER BY sort_order, character_id",
        ) {
            if let Ok(rows) = stmt.query_map([anilist_id], |r| {
                Ok(CharacterCard {
                    character_id: r.get(0)?,
                    role: r.get(1)?,
                    name_full: r.get(2)?,
                    name_native: r.get(3)?,
                    image_url: r.get(4)?,
                    va_id: r.get(5)?,
                    va_name_full: r.get(6)?,
                    va_name_native: r.get(7)?,
                    va_image_url: r.get(8)?,
                })
            }) {
                d.characters = rows.flatten().collect();
            }
        }
        if let Ok(mut stmt) = conn.prepare(
            "SELECT staff_id, role, name_full, name_native, image_url
             FROM media_staff WHERE anilist_id = ?1 ORDER BY sort_order, staff_id",
        ) {
            if let Ok(rows) = stmt.query_map([anilist_id], |r| {
                Ok(StaffCard {
                    staff_id: r.get(0)?,
                    role: r.get(1)?,
                    name_full: r.get(2)?,
                    name_native: r.get(3)?,
                    image_url: r.get(4)?,
                })
            }) {
                d.staff = rows.flatten().collect();
            }
        }
    }
    let is_streaming_mode = db::get_setting(&conn, "pref.app_mode")
        .ok()
        .flatten()
        .as_deref()
        == Some("streaming");
    drop(conn);

    if is_streaming_mode {
        let remote_prog = d.anilist_progress.unwrap_or(0);
        let next_ep = d
            .episode_list
            .iter()
            .find(|e| !e.is_special && e.watched_at.is_none() && e.files.is_empty() && e.progress_pos.unwrap_or(0.0) > 0.0)
            .or_else(|| {
                d.episode_list
                    .iter()
                    .find(|e| !e.is_special && e.watched_at.is_none() && e.files.is_empty() && e.number > remote_prog)
            })
            .or_else(|| d.episode_list.iter().find(|e| !e.is_special && e.files.is_empty()))
            .map(|e| e.number);

        if let Some(ep_num) = next_ep.filter(|&n| n > 0) {
            let st_pf = st.inner().clone();
            tokio::spawn(async move {
                let _ = crate::torrent::search_episode_torrents(&st_pf, anilist_id, ep_num, None).await;
            });
        }
    }

    Ok(d)
}

// ---------------------------------------------------------------------------
// Watch state
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn set_watched(
    app: AppHandle,
    st: St<'_>,
    anilist_id: i64,
    ep_keys: Vec<String>,
    watched: bool,
) -> CmdResult<()> {
    let mut conn = st.db();
    let tx = conn.transaction().map_err(err)?;
    for key in ep_keys {
        if watched {
            tx.execute(
                "INSERT OR IGNORE INTO watch_state (anilist_id, ep_key, watched_at) VALUES (?1, ?2, ?3)",
                params![anilist_id, key, now()],
            )
            .map_err(err)?;
        } else {
            tx.execute(
                "DELETE FROM watch_state WHERE anilist_id = ?1 AND ep_key = ?2",
                params![anilist_id, key],
            )
            .map_err(err)?;
        }
        // Either way the saved resume point is no longer meaningful.
        tx.execute(
            "DELETE FROM watch_progress WHERE anilist_id = ?1 AND ep_key = ?2",
            params![anilist_id, key],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)?;
    drop(conn);
    if watched {
        crate::anilist_sync::on_episode_watched(&app, &st, anilist_id);
    } else {
        crate::anilist_sync::on_episode_unwatched(&app, &st, anilist_id);
    }
    let _ = app.emit("library-changed", ());
    crate::backup::mark_dirty();
    Ok(())
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    anilist_id: i64,
    title_romaji: Option<String>,
    title_english: Option<String>,
    title_native: Option<String>,
    format: Option<String>,
    status: Option<String>,
    episodes: Option<i64>,
    season_year: Option<i64>,
    cover_url: Option<String>,
}

#[tauri::command]
pub async fn search_anilist(st: St<'_>, query: String) -> CmdResult<Vec<SearchResult>> {
    let res = st.providers.anilist_search(&query, 15).await.map_err(err)?;
    Ok(res
        .into_iter()
        .map(|m| SearchResult {
            anilist_id: m.id,
            title_romaji: m.title.romaji,
            title_english: m.title.english,
            title_native: m.title.native,
            format: m.format,
            status: m.status,
            episodes: m.episodes,
            season_year: m.season_year,
            cover_url: m.cover_image.and_then(|c| c.large),
        })
        .collect())
}

/// Re-point match groups at a different AniList entry. Either a single group, or every group feeding `from_anilist_id`.
#[tauri::command]
pub async fn fix_match(
    app: AppHandle,
    st: St<'_>,
    new_anilist_id: i64,
    group_id: Option<i64>,
    from_anilist_id: Option<i64>,
) -> CmdResult<()> {
    let m = st
        .providers
        .anilist_media(new_anilist_id)
        .await
        .map_err(err)?;
    {
        let conn = st.db();
        store::upsert_media(&conn, &m).map_err(err)?;
        let groups: Vec<i64> = if let Some(g) = group_id {
            vec![g]
        } else if let Some(from) = from_anilist_id {
            let mut stmt = conn
                .prepare("SELECT DISTINCT group_id FROM local_files WHERE anilist_id = ?1 AND group_id IS NOT NULL")
                .map_err(err)?;
            let rows = stmt
                .query_map([from], |r| r.get(0))
                .map_err(err)?
                .flatten()
                .collect();
            rows
        } else {
            return Err("nothing to fix".into());
        };
        for g in groups {
            conn.execute(
                "UPDATE match_groups SET anilist_id = ?2, confidence = 1.0, manual = 1, attempted = 1 WHERE id = ?1",
                params![g, new_anilist_id],
            )
            .map_err(err)?;
            conn.execute(
                "UPDATE local_files SET anilist_id = NULL, ep_key = NULL WHERE group_id = ?1",
                [g],
            )
            .map_err(err)?;
        }
    }
    let _ = app.emit("library-changed", ());
    crate::backup::mark_dirty();
    scan::spawn_scan(app, st.inner().clone(), None);
    Ok(())
}

/// Re-fetch metadata + episodes for one entry.
#[tauri::command]
pub async fn refresh_media(app: AppHandle, st: St<'_>, anilist_id: i64) -> CmdResult<()> {
    let token = {
        let conn = st.db();
        db::get_setting(&conn, "anilist.token")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
    };
    let m = st
        .providers
        .anilist_media_authed(anilist_id, token.as_deref())
        .await
        .map_err(err)?;
    store::upsert_media(&st.db(), &m).map_err(err)?;
    let lite = db::media_lite(&st.db(), anilist_id)
        .map_err(err)?
        .ok_or("missing media")?;
    crate::service::fetch_episodes(&st, &lite)
        .await
        .map_err(err)?;
    let _ = app.emit("library-changed", ());
    scan::spawn_scan(app, st.inner().clone(), None);
    Ok(())
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

fn known_file(st: &AppState, path: &str) -> CmdResult<()> {
    let ok: Option<i64> = st
        .db()
        .query_row("SELECT id FROM local_files WHERE path = ?1", [path], |r| {
            r.get(0)
        })
        .optional()
        .map_err(err)?;
    ok.map(|_| ()).ok_or_else(|| "unknown file".to_string())
}

#[tauri::command]
pub async fn open_file(st: St<'_>, path: String) -> CmdResult<()> {
    known_file(&st, &path)?;
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(err)
}

#[tauri::command]
pub async fn reveal_file(st: St<'_>, path: String) -> CmdResult<()> {
    known_file(&st, &path)?;
    tauri_plugin_opener::reveal_item_in_dir(&path).map_err(err)
}

// ---------------------------------------------------------------------------
// Playback
// ---------------------------------------------------------------------------

// Detection probes the registry, PATH and several folders and may spawn `where.exe`;
// run it on a blocking thread so it can never freeze the window.
#[tauri::command]
pub async fn detect_players() -> Vec<player::Detected> {
    tauri::async_runtime::spawn_blocking(player::detect_all)
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn check_mpv_status() -> CmdResult<player::MpvStatus> {
    tauri::async_runtime::spawn_blocking(player::check_mpv_status)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn install_mpv(app: AppHandle, st: St<'_>) -> CmdResult<player::InstallMpvResult> {
    player::install_mpv(app, st.inner().clone())
        .await
        .map_err(err)
}

/// Play an episode with the configured external player, tracking progress where possible.
#[tauri::command]
pub async fn play_episode(
    app: AppHandle,
    st: St<'_>,
    anilist_id: i64,
    ep_key: String,
    path: String,
) -> CmdResult<()> {
    known_file(&st, &path)?;
    player::play(
        app,
        st.inner().clone(),
        player::Ep {
            anilist_id,
            ep_key,
            path,
        },
    )
    .map_err(|e| {
        crate::klog!("[kura] play failed: {e:#}");
        err(e)
    })
}

/// An episode to surface on the home page (continue watching / new episode).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpNextItem {
    anilist_id: i64,
    ep_key: String,
    number: i64,
    path: String,
    title_romaji: Option<String>,
    title_english: Option<String>,
    title_native: Option<String>,
    format: Option<String>,
    episodes: Option<i64>,
    cover_url: Option<String>,
    cover_path: Option<String>,
    cover_color: Option<String>,
    banner_url: Option<String>,
    banner_path: Option<String>,
    logo_url: Option<String>,
    logo_path: Option<String>,
    episode_title: Option<String>,
    thumb_url: Option<String>,
    thumb_path: Option<String>,
    runtime: Option<i64>,
    /// Seconds; 0 when the episode hasn't been started.
    position: f64,
    duration: f64,
    owned_count: i64,
    watched_count: i64,
    /// Last activity (continue watching) or when the file was added (new episodes).
    at: i64,
    /// New episodes only: how many unwatched files were added recently.
    new_count: i64,
}

fn up_next_item(conn: &rusqlite::Connection, ep: &player::Ep, at: i64) -> Option<UpNextItem> {
    let (pos, dur) = player::saved_progress(conn, ep.anilist_id, &ep.ep_key).unwrap_or((0.0, 0.0));
    conn.query_row(
        "SELECT m.title_romaji, m.title_english, m.title_native, m.format, m.episodes, m.cover_url, m.cover_path,
                m.cover_color, m.banner_url, m.banner_path,
                COALESCE(e.title_en, e.title_romaji, e.title_ja), e.thumb_url, e.thumb_path, e.runtime,
                (SELECT COUNT(DISTINCT f.ep_key) FROM local_files f WHERE f.anilist_id = m.anilist_id AND f.ep_key NOT LIKE 'S%'),
                (SELECT COUNT(*) FROM watch_state w WHERE w.anilist_id = m.anilist_id AND w.ep_key NOT LIKE 'S%'),
                NULLIF(m.logo_url, ''), m.logo_path
         FROM media m LEFT JOIN episodes e ON e.anilist_id = m.anilist_id AND e.ep_key = ?2
         WHERE m.anilist_id = ?1",
        params![ep.anilist_id, ep.ep_key],
        |r| {
            Ok(UpNextItem {
                anilist_id: ep.anilist_id,
                ep_key: ep.ep_key.clone(),
                number: ep.ep_key.trim_start_matches('S').parse().unwrap_or(0),
                path: ep.path.clone(),
                title_romaji: r.get(0)?,
                title_english: r.get(1)?,
                title_native: r.get(2)?,
                format: r.get(3)?,
                episodes: r.get(4)?,
                cover_url: r.get(5)?,
                cover_path: r.get(6)?,
                cover_color: r.get(7)?,
                banner_url: r.get(8)?,
                banner_path: r.get(9)?,
                episode_title: r.get(10)?,
                thumb_url: r.get(11)?,
                thumb_path: r.get(12)?,
                runtime: r.get(13)?,
                position: pos,
                duration: dur,
                owned_count: r.get(14)?,
                watched_count: r.get(15)?,
                logo_url: r.get(16)?,
                logo_path: r.get(17)?,
                at,
                new_count: 0,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

fn is_watched(conn: &rusqlite::Connection, ep: &player::Ep) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM watch_state WHERE anilist_id = ?1 AND ep_key = ?2)",
        params![ep.anilist_id, ep.ep_key],
        |r| r.get(0),
    )
    .unwrap_or(false)
}

/// What to resume / watch next, most recently active first. One entry per franchise.
#[tauri::command]
pub async fn get_up_next(st: St<'_>, limit: Option<usize>) -> CmdResult<Vec<UpNextItem>> {
    let limit = limit.unwrap_or(6);
    let conn = st.db();
    let mut stmt = conn
        .prepare(
            "SELECT anilist_id, MAX(t) AS last FROM (
                SELECT anilist_id, watched_at AS t FROM watch_state
                UNION ALL SELECT anilist_id, updated_at AS t FROM watch_progress
             ) GROUP BY anilist_id ORDER BY last DESC LIMIT 60",
        )
        .map_err(err)?;
    let active: Vec<(i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(err)?
        .flatten()
        .collect();
    let franchises = franchise::load(&conn).map_err(err)?;
    let root = |id: i64| franchises.get(&id).map(|p| p.root).unwrap_or(id);

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (id, at) in active {
        if out.len() >= limit {
            break;
        }
        if !seen.insert(root(id)) {
            continue; // a later season of this franchise is already listed
        }
        // 1. An episode left part-way through.
        let partial: Option<player::Ep> = conn
            .query_row(
                "SELECT p.ep_key, f.path FROM watch_progress p
                 JOIN local_files f ON f.anilist_id = p.anilist_id AND f.ep_key = p.ep_key
                 WHERE p.anilist_id = ?1 ORDER BY p.updated_at DESC, f.path LIMIT 1",
                [id],
                |r| {
                    Ok(player::Ep {
                        anilist_id: id,
                        ep_key: r.get(0)?,
                        path: r.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(err)?;
        let ep = match partial {
            Some(e) => Some(e),
            None => {
                // 2. The next unwatched owned episode after the furthest watched one.
                let last: Option<String> = conn
                    .query_row(
                        "SELECT ep_key FROM watch_state WHERE anilist_id = ?1 AND ep_key NOT LIKE 'S%'
                         ORDER BY CAST(ep_key AS INTEGER) DESC LIMIT 1",
                        [id],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(err)?;
                let mut cur = player::Ep {
                    anilist_id: id,
                    ep_key: last.unwrap_or_else(|| "0".into()),
                    path: String::new(),
                };
                let mut found = None;
                for _ in 0..500 {
                    match player::next_episode(&conn, &cur) {
                        Some(n) if is_watched(&conn, &n) => cur = n,
                        Some(n) => {
                            found = Some(n);
                            break;
                        }
                        None => break,
                    }
                }
                found
            }
        };
        if let Some(item) = ep.and_then(|e| up_next_item(&conn, &e, at)) {
            out.push(item);
        }
    }
    Ok(out)
}

/// Unwatched episodes that showed up after a show was already in the library (last 30 days).
#[tauri::command]
pub async fn get_new_episodes(st: St<'_>, limit: Option<usize>) -> CmdResult<Vec<UpNextItem>> {
    let limit = limit.unwrap_or(6) as i64;
    let since = now() - 30 * 86_400;
    let conn = st.db();
    // A file is "new" if it arrived more than an hour after the first file of the same title.
    let new_filter = "f.anilist_id IS NOT NULL AND f.ep_key IS NOT NULL AND f.added_at > ?1
         AND f.added_at > (SELECT MIN(g.added_at) FROM local_files g WHERE g.anilist_id = f.anilist_id) + 3600
         AND NOT EXISTS (SELECT 1 FROM watch_state w WHERE w.anilist_id = f.anilist_id AND w.ep_key = f.ep_key)";
    let mut stmt = conn
        .prepare(&format!(
            "SELECT f.anilist_id, MAX(f.added_at), COUNT(DISTINCT f.ep_key) FROM local_files f WHERE {new_filter}
             GROUP BY f.anilist_id ORDER BY MAX(f.added_at) DESC LIMIT ?2"
        ))
        .map_err(err)?;
    let rows: Vec<(i64, i64, i64)> = stmt
        .query_map(params![since, limit], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .map_err(err)?
        .flatten()
        .collect();
    let mut first = conn
        .prepare(&format!(
            "SELECT f.ep_key, f.path FROM local_files f WHERE f.anilist_id = ?2 AND {new_filter}
             ORDER BY f.ep_key LIKE 'S%', CAST(f.ep_key AS INTEGER), f.path LIMIT 1"
        ))
        .map_err(err)?;
    let mut out = Vec::new();
    for (id, at, count) in rows {
        let ep: Option<player::Ep> = first
            .query_row(params![since, id], |r| {
                Ok(player::Ep {
                    anilist_id: id,
                    ep_key: r.get(0)?,
                    path: r.get(1)?,
                })
            })
            .optional()
            .map_err(err)?;
        if let Some(mut item) = ep.and_then(|e| up_next_item(&conn, &e, at)) {
            item.new_count = count;
            out.push(item);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

/// UI preferences, stored as `pref.<key>` rows in the settings table.
#[tauri::command]
pub async fn get_prefs(st: St<'_>) -> CmdResult<HashMap<String, String>> {
    let conn = st.db();
    let _ = db::seed_default_prefs_if_missing(&conn);
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings WHERE key LIKE 'pref.%'")
        .map_err(err)?;
    let mut rows: HashMap<String, String> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(err)?
        .flatten()
        .map(|(k, v)| (k.trim_start_matches("pref.").to_string(), v))
        .collect();

    // Guarantee default values for all known preferences so nothing reverts on updates
    let defaults = [
        ("theme", "comic"),
        ("player", "mpv"),
        ("audio_lang", "jpn"),
        ("sub_lang", "jpn"),
        ("sub_fallback", "eng"),
        ("watched_threshold", "90"),
        ("resume", "1"),
        ("autoplay", "1"),
        ("group_seasons", "1"),
        ("auto_update", "1"),
        ("folder_watcher", "1"),
        ("discord_rpc", "1"),
        ("discord_spoilers", "0"),
        ("discord_buttons", "1"),
        ("mpc_port", "13579"), ("minimize_to_tray", "1"),
    ];
    for (k, v) in defaults {
        rows.entry(k.to_string()).or_insert_with(|| v.to_string());
    }

    Ok(rows)
}

#[tauri::command]
pub async fn set_pref(st: St<'_>, key: String, value: String) -> CmdResult<()> {
    if key == "discord_rpc" && value == "0" {
        st.discord.clear();
    }
    db::set_setting(&st.db(), &format!("pref.{key}"), &value).map_err(err)?;
    if key == "folder_watcher" {
        st.watcher.reload();
    }
    crate::backup::mark_dirty();
    Ok(())
}

#[tauri::command]
pub async fn get_media_track_pref(
    st: St<'_>,
    anilist_id: i64,
) -> CmdResult<Option<db::MediaTrackPref>> {
    db::get_media_track_pref(&st.db(), anilist_id).map_err(err)
}

#[tauri::command]
pub async fn set_media_track_pref(
    app: AppHandle,
    st: St<'_>,
    anilist_id: i64,
    pref: Option<db::MediaTrackPref>,
) -> CmdResult<()> {
    db::set_media_track_pref(&st.db(), anilist_id, pref).map_err(err)?;
    let _ = app.emit("library-changed", ());
    crate::backup::mark_dirty();
    Ok(())
}

/// Wipes cached metadata, episode guides, and downloaded artwork while strictly preserving:
/// - watch_state (watched / unwatched history)
/// - watch_progress (resume times and playback progress)
/// - media_track_prefs (custom audio/subtitle language priorities)
/// - libraries & library_folders (configured folders)
/// - settings (user preferences, themes, fanart key)
#[tauri::command]
pub async fn rebuild_metadata(app: AppHandle, st: St<'_>) -> CmdResult<()> {
    // 1. Clear cached media, episodes, media_relations
    {
        let conn = st.db();
        conn.execute_batch(
            "DELETE FROM episodes;
             DELETE FROM media_relations;
             DELETE FROM media;
             UPDATE match_groups SET attempted = 0 WHERE manual = 0;
             UPDATE local_files SET ep_key = NULL WHERE anilist_id IS NULL;",
        )
        .map_err(err)?;
    }

    // 2. Clean up cached remote images
    let img_dir = st.data_dir.join("images");
    for sub in ["covers", "banners", "logos", "thumbs"] {
        let p = img_dir.join(sub);
        if p.exists() {
            let _ = std::fs::remove_dir_all(&p);
            let _ = std::fs::create_dir_all(&p);
        }
    }

    // 3. Emit library changed and spawn full scan
    let _ = app.emit("library-changed", ());
    scan::spawn_scan(app, st.inner().clone(), None);
    Ok(())
}

// ---------------------------------------------------------------------------
// Backup & restore
// ---------------------------------------------------------------------------

/// Export all user data to a JSON file chosen by the user.
#[tauri::command]
pub async fn export_backup(st: St<'_>, path: String) -> CmdResult<backup::BackupSummary> {
    let snap = backup::snapshot(&st.db());
    tauri::async_runtime::spawn_blocking(move || {
        let p = std::path::PathBuf::from(&path);
        backup::write_file(&p, &snap).map_err(|e| format!("{e:#}"))?;
        Ok(backup::summarize(&p, &snap))
    })
    .await
    .map_err(err)?
}

/// Read a backup file and describe what's inside, without changing anything.
#[tauri::command]
pub async fn preview_backup(path: String) -> CmdResult<backup::BackupSummary> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = std::path::PathBuf::from(&path);
        let b = backup::read_file(&p).map_err(|e| format!("{e:#}"))?;
        Ok(backup::summarize(&p, &b))
    })
    .await
    .map_err(err)?
}

/// Merge a backup file into the library. Nothing is deleted.
#[tauri::command]
pub async fn import_backup(
    app: AppHandle,
    st: St<'_>,
    path: String,
    options: backup::ImportOptions,
) -> CmdResult<backup::ImportReport> {
    let b = tauri::async_runtime::spawn_blocking(move || {
        backup::read_file(std::path::Path::new(&path))
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let report = backup::apply(&st.db(), &b, &options).map_err(|e| format!("{e:#}"))?;

    if options.settings {
        let rpc_off = db::get_setting(&st.db(), "pref.discord_rpc")
            .ok()
            .flatten()
            .as_deref()
            == Some("0");
        if rpc_off {
            st.discord.clear();
        }
    }
    st.watcher.reload();
    crate::backup::mark_dirty();
    let _ = app.emit("library-changed", ());
    if options.libraries || options.matches {
        scan::spawn_scan(app, st.inner().clone(), None);
    }
    Ok(report)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyCopy {
    path: String,
    /// Unix seconds of the last write, if the file exists.
    modified: Option<i64>,
}

/// Where the automatic safety copies live and when they were last written.
#[tauri::command]
pub async fn backup_status(st: St<'_>) -> CmdResult<Vec<SafetyCopy>> {
    let dir = st.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        backup::safety_paths(&dir)
            .into_iter()
            .map(|p| {
                let modified = std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64);
                SafetyCopy {
                    path: p.to_string_lossy().to_string(),
                    modified,
                }
            })
            .collect()
    })
    .await
    .map_err(err)
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    /// Plain-text report, ready to paste into a GitHub issue.
    report: String,
    log_path: String,
    players: Vec<player::Detected>,
    log_tail: Vec<String>,
}

/// Check folders in parallel with a shared deadline. `None` = didn't answer in time.
async fn folders_reachable(paths: &[String]) -> Vec<Option<bool>> {
    let tasks: Vec<_> = paths
        .iter()
        .map(|p| {
            let p = p.clone();
            tauri::async_runtime::spawn_blocking(move || std::path::Path::new(&p).is_dir())
        })
        .collect();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(1500);
    let mut out = Vec::with_capacity(tasks.len());
    for t in tasks {
        out.push(match tokio::time::timeout_at(deadline, t).await {
            Ok(Ok(ok)) => Some(ok),
            _ => None,
        });
    }
    out
}

#[tauri::command]
pub async fn get_diagnostics(app: AppHandle, st: St<'_>) -> CmdResult<Diagnostics> {
    use std::fmt::Write as _;

    struct Snap {
        libs: Vec<(i64, String)>,
        folders: Vec<(i64, String)>,
        files: i64,
        matched: i64,
        unmatched: i64,
        watched: i64,
        progress: i64,
        settings: Vec<(String, String)>,
    }
    let snap = {
        let conn = st.db();
        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0) };
        let list = |sql: &str| -> Vec<(i64, String)> {
            conn.prepare(sql)
                .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect())
                .unwrap_or_default()
        };
        let settings: Vec<(String, String)> = conn
            .prepare("SELECT key, value FROM settings WHERE key LIKE 'pref.%' ORDER BY key")
            .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect())
            .unwrap_or_default();
        Snap {
            libs: list("SELECT id, name FROM libraries ORDER BY id"),
            folders: list("SELECT library_id, path FROM library_folders ORDER BY id"),
            files: count("SELECT COUNT(*) FROM local_files"),
            matched: count(
                "SELECT COUNT(DISTINCT anilist_id) FROM local_files WHERE anilist_id IS NOT NULL",
            ),
            unmatched: count("SELECT COUNT(*) FROM match_groups WHERE anilist_id IS NULL"),
            watched: count("SELECT COUNT(*) FROM watch_state"),
            progress: count("SELECT COUNT(*) FROM watch_progress"),
            settings,
        }
    };

    let folder_paths: Vec<String> = snap.folders.iter().map(|(_, p)| p.clone()).collect();
    let reachable = folders_reachable(&folder_paths).await;
    let players = tauri::async_runtime::spawn_blocking(player::detect_all)
        .await
        .unwrap_or_default();
    let data_dir = st.data_dir.clone();
    let (db_size, log_tail) = {
        let dir = data_dir.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let db = std::fs::metadata(dir.join("kura.db"))
                .map(|m| m.len())
                .unwrap_or(0);
            (db, crate::logging::tail(&dir, 120))
        })
        .await
        .map_err(err)?
    };

    let mut r = String::new();
    let _ = writeln!(
        r,
        "Kura diagnostics — {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    );
    let _ = writeln!(
        r,
        "App: Kura {} ({})",
        app.package_info().version,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    let _ = writeln!(r, "OS: {} {}", std::env::consts::OS, std::env::consts::ARCH);
    let _ = writeln!(
        r,
        "WebView: {}",
        tauri::webview_version().unwrap_or_else(|_| "unknown".into())
    );
    let _ = writeln!(r, "Data folder: {}", data_dir.display());
    let _ = writeln!(r, "Database: {:.1} MB", db_size as f64 / 1_048_576.0);

    let _ = writeln!(r, "\n[Library]");
    for (id, name) in &snap.libs {
        let _ = writeln!(r, "- {name}");
        for ((lib, path), ok) in snap.folders.iter().zip(&reachable) {
            if lib == id {
                let state = match ok {
                    Some(true) => "ok",
                    Some(false) => "NOT FOUND",
                    None => "no response (sleeping or network drive?)",
                };
                let _ = writeln!(r, "    {path}  [{state}]");
            }
        }
    }
    let _ = writeln!(
        r,
        "Files: {} · matched shows: {} · unmatched groups: {}",
        snap.files, snap.matched, snap.unmatched
    );
    let _ = writeln!(
        r,
        "Watched episodes: {} · resume points: {}",
        snap.watched, snap.progress
    );

    let _ = writeln!(r, "\n[Playback]");
    if players.is_empty() {
        let _ = writeln!(r, "Detected players: none");
    }
    for p in &players {
        let _ = writeln!(r, "Detected {}: {}", p.kind, p.path);
    }

    let _ = writeln!(r, "\n[Settings]");
    for (k, v) in &snap.settings {
        let shown = if (k.contains("api_key") || k.contains("token")) && !v.is_empty() {
            "(set, hidden)"
        } else {
            v.as_str()
        };
        let _ = writeln!(r, "{} = {}", k.trim_start_matches("pref."), shown);
    }

    let _ = writeln!(r, "\n[Recent log]");
    for l in log_tail
        .iter()
        .rev()
        .take(60)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let _ = writeln!(r, "{l}");
    }

    Ok(Diagnostics {
        report: r,
        log_path: crate::logging::log_path(&data_dir)
            .to_string_lossy()
            .to_string(),
        players,
        log_tail,
    })
}

// ---------------------------------------------------------------------------
// AniList Two-Way Sync & Seasonal Airing Calendar
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_anilist_account(st: St<'_>) -> CmdResult<crate::anilist_sync::AnilistAccount> {
    Ok(crate::anilist_sync::get_account(&st.db()))
}

#[tauri::command]
pub async fn anilist_update_entry(
    app: AppHandle,
    st: St<'_>,
    anilist_id: i64,
    status: Option<String>,
    progress: Option<i64>,
    score: Option<f64>,
) -> CmdResult<crate::anilist_sync::AnilistRemoteEntry> {
    crate::anilist_sync::update_entry(&app, st.inner(), anilist_id, status, progress, score)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn anilist_start_oauth(
    app: AppHandle,
    st: St<'_>,
    client_id: Option<String>,
) -> CmdResult<String> {
    crate::anilist_sync::start_oauth_flow(app, st.inner().clone(), client_id).map_err(err)
}

#[tauri::command]
pub async fn anilist_login_token(
    app: AppHandle,
    st: St<'_>,
    token: String,
) -> CmdResult<crate::anilist_sync::AnilistAccount> {
    crate::anilist_sync::connect_with_token(&app, st.inner(), &token)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn anilist_disconnect(
    app: AppHandle,
    st: St<'_>,
) -> CmdResult<crate::anilist_sync::AnilistAccount> {
    let acct = {
        let conn = st.db();
        crate::anilist_sync::disconnect(&conn).map_err(err)?;
        crate::anilist_sync::get_account(&conn)
    };
    let _ = app.emit("anilist-changed", &acct);
    Ok(acct)
}

#[tauri::command]
pub async fn anilist_sync_now(
    app: AppHandle,
    st: St<'_>,
) -> CmdResult<crate::anilist_sync::AnilistSyncReport> {
    crate::anilist_sync::sync_now(&app, st.inner())
        .await
        .map_err(err)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SeasonalItem {
    pub anilist_id: i64,
    pub link_id: i64,
    pub title_romaji: Option<String>,
    pub title_english: Option<String>,
    pub title_native: Option<String>,
    pub format: Option<String>,
    pub status: Option<String>,
    pub episodes: Option<i64>,
    pub duration: Option<i64>,
    pub season: Option<String>,
    pub season_year: Option<i64>,
    pub average_score: Option<i64>,
    pub genres: Vec<String>,
    pub studios: Vec<String>,
    pub start_date: Option<String>,
    pub cover_url: Option<String>,
    pub cover_path: Option<String>,
    pub cover_color: Option<String>,
    pub banner_url: Option<String>,
    pub next_airing_episode: Option<i64>,
    pub next_airing_at: Option<i64>,
    pub in_library: bool,
    pub owned_count: i64,
    pub watched_count: i64,
    pub anilist_status: Option<String>,
    pub anilist_progress: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeasonalCalendar {
    pub season: String,
    pub year: i32,
    pub fetched_at: i64,
    pub items: Vec<SeasonalItem>,
}

fn current_season_and_year() -> (String, i32) {
    use chrono::Datelike;
    let today = chrono::Local::now().date_naive();
    let season = match today.month() {
        1..=3 => "WINTER",
        4..=6 => "SPRING",
        7..=9 => "SUMMER",
        _ => "FALL",
    };
    (season.to_string(), today.year())
}

#[tauri::command]
pub async fn get_seasonal_calendar(
    st: St<'_>,
    season: Option<String>,
    year: Option<i32>,
    refresh: Option<bool>,
) -> CmdResult<SeasonalCalendar> {
    let (cur_season, cur_year) = current_season_and_year();
    let season = season
        .map(|s| s.trim().to_uppercase())
        .filter(|s| matches!(s.as_str(), "WINTER" | "SPRING" | "SUMMER" | "FALL"))
        .unwrap_or_else(|| cur_season.clone());
    let year = year.unwrap_or(cur_year);
    let is_current_season = season == cur_season && year == cur_year;
    let key = format!("{season}_{year}");
    let force = refresh.unwrap_or(false);

    let cached: Option<(String, i64)> = {
        let conn = st.db();
        let _ = conn.execute_batch(db::SCHEMA_V5);
        conn.query_row(
            "SELECT payload, fetched_at FROM seasonal_cache WHERE season_key = ?1",
            [&key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?
    };

    let (raw_list, fetched_at): (Vec<crate::providers::AlSeasonalMedia>, i64) = match cached {
        Some((ref payload, ts)) if !force && (now() - ts < 3 * 3600) && payload.len() > 4 => {
            let parsed = serde_json::from_str(payload).unwrap_or_default();
            (parsed, ts)
        }
        _ => match st.providers.anilist_season(&season, year).await {
            Ok(list) => {
                let ts = now();
                if let Ok(json) = serde_json::to_string(&list) {
                    let _ = st.db().execute(
                        "INSERT INTO seasonal_cache (season_key, payload, fetched_at) VALUES (?1, ?2, ?3)
                         ON CONFLICT(season_key) DO UPDATE SET payload = excluded.payload, fetched_at = excluded.fetched_at",
                        params![key, json, ts],
                    );
                }
                (list, ts)
            }
            Err(e) => {
                crate::klog!("[kura] seasonal calendar fetch failed for {key}: {e}");
                if let Some((payload, ts)) = cached {
                    (serde_json::from_str(&payload).unwrap_or_default(), ts)
                } else {
                    (Vec::new(), now())
                }
            }
        },
    };

    let conn = st.db();
    let franchises = franchise::load(&conn).unwrap_or_default();

    // Local library counts + cover paths + airing overrides
    let mut local_map: HashMap<i64, (i64, i64, Option<String>, Option<i64>, Option<i64>)> =
        HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT m.anilist_id,
                        (SELECT COUNT(DISTINCT f.ep_key) FROM local_files f WHERE f.anilist_id = m.anilist_id AND f.ep_key NOT LIKE 'S%' AND f.ep_key NOT LIKE 'extra:%'),
                        (SELECT COUNT(*) FROM watch_state w WHERE w.anilist_id = m.anilist_id AND w.ep_key NOT LIKE 'S%' AND w.ep_key NOT LIKE 'extra:%'),
                        m.cover_path,
                        m.next_airing_episode,
                        m.next_airing_at
                 FROM media m",
            )
            .map_err(err)?;
        for row in stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                ))
            })
            .map_err(err)?
            .flatten()
        {
            local_map.insert(row.0, (row.1, row.2, row.3, row.4, row.5));
        }
    }

    let mut remote_map: HashMap<i64, (String, i64)> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT anilist_id, status, progress FROM anilist_entries")
            .map_err(err)?;
        for row in stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(err)?
            .flatten()
        {
            remote_map.insert(row.0, (row.1, row.2));
        }
    }

    let mut seen_ids = std::collections::HashSet::new();
    let mut items: Vec<SeasonalItem> = Vec::new();

    for m in raw_list {
        seen_ids.insert(m.id);
        let loc = local_map.get(&m.id);
        let owned_count = loc.map(|x| x.0).unwrap_or(0);
        let watched_count = loc.map(|x| x.1).unwrap_or(0);
        let cover_path = loc.and_then(|x| x.2.clone());
        let in_library = owned_count > 0;
        let link_id = franchises.get(&m.id).map(|f| f.root).unwrap_or(m.id);
        let rem = remote_map.get(&m.id);
        let next_ep = m
            .next_airing_episode
            .as_ref()
            .map(|a| a.episode)
            .or_else(|| loc.and_then(|x| x.3));
        let next_at = m
            .next_airing_episode
            .as_ref()
            .map(|a| a.airing_at)
            .or_else(|| loc.and_then(|x| x.4));

        items.push(SeasonalItem {
            anilist_id: m.id,
            link_id,
            title_romaji: m.title.romaji,
            title_english: m.title.english,
            title_native: m.title.native,
            format: m.format,
            status: m.status,
            episodes: m.episodes,
            duration: m.duration,
            season: m.season,
            season_year: m.season_year,
            average_score: m.average_score,
            genres: m.genres,
            studios: m
                .studios
                .map(|s| s.nodes.into_iter().map(|n| n.name).collect())
                .unwrap_or_default(),
            start_date: m.start_date.and_then(|d| d.to_string_opt()),
            cover_url: m
                .cover_image
                .as_ref()
                .and_then(|c| c.extra_large.clone().or_else(|| c.large.clone())),
            cover_path,
            cover_color: m.cover_image.and_then(|c| c.color),
            banner_url: m.banner_image,
            next_airing_episode: next_ep,
            next_airing_at: next_at,
            in_library,
            owned_count,
            watched_count,
            anilist_status: rem.map(|r| r.0.clone()),
            anilist_progress: rem.map(|r| r.1),
        });
    }

    // Also include local library shows for this season (or continuing airing shows in the current season)
    // so offline users and 2-cour continuing series always appear.
    {
        let mut stmt = conn
            .prepare(
                "SELECT m.anilist_id, m.title_romaji, m.title_english, m.title_native, m.format, m.status,
                        m.episodes, m.duration, m.season, m.season_year, m.average_score, m.genres, m.studios,
                        m.start_date, m.cover_url, m.cover_path, m.cover_color, m.banner_url,
                        m.next_airing_episode, m.next_airing_at
                 FROM media m
                 WHERE (m.season = ?1 AND m.season_year = ?2)
                    OR (?3 = 1 AND m.status = 'RELEASING' AND EXISTS(SELECT 1 FROM local_files f WHERE f.anilist_id = m.anilist_id))",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![season, year, i64::from(is_current_season)], |r| {
                let id: i64 = r.get(0)?;
                let loc = local_map.get(&id);
                let owned_count = loc.map(|x| x.0).unwrap_or(0);
                let watched_count = loc.map(|x| x.1).unwrap_or(0);
                let rem = remote_map.get(&id);
                let link_id = franchises.get(&id).map(|f| f.root).unwrap_or(id);
                Ok(SeasonalItem {
                    anilist_id: id,
                    link_id,
                    title_romaji: r.get(1)?,
                    title_english: r.get(2)?,
                    title_native: r.get(3)?,
                    format: r.get(4)?,
                    status: r.get(5)?,
                    episodes: r.get(6)?,
                    duration: r.get(7)?,
                    season: r.get(8)?,
                    season_year: r.get(9)?,
                    average_score: r.get(10)?,
                    genres: json_list(r.get(11)?),
                    studios: json_list(r.get(12)?),
                    start_date: r.get(13)?,
                    cover_url: r.get(14)?,
                    cover_path: r.get(15)?,
                    cover_color: r.get(16)?,
                    banner_url: r.get(17)?,
                    next_airing_episode: r.get(18)?,
                    next_airing_at: r.get(19)?,
                    in_library: owned_count > 0,
                    owned_count,
                    watched_count,
                    anilist_status: rem.map(|x| x.0.clone()),
                    anilist_progress: rem.map(|x| x.1),
                })
            })
            .map_err(err)?;
        for item in rows.flatten() {
            if seen_ids.insert(item.anilist_id) {
                items.push(item);
            }
        }
    }

    Ok(SeasonalCalendar {
        season,
        year,
        fetched_at,
        items,
    })
}

// ---------------------------------------------------------------------------
// v0.5: Characters, Seiyuu & Staff Person Works
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonWorkItem {
    pub anilist_id: i64,
    pub title_romaji: Option<String>,
    pub title_english: Option<String>,
    pub title_native: Option<String>,
    pub format: Option<String>,
    pub season_year: Option<i64>,
    pub cover_url: Option<String>,
    pub cover_path: Option<String>,
    pub role: String,
    pub character_name: Option<String>,
    pub character_image: Option<String>,
    pub in_library: bool,
    pub owned_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonDetail {
    pub staff_id: i64,
    pub name_full: String,
    pub name_native: Option<String>,
    pub image_url: Option<String>,
    pub occupations: Vec<String>,
    pub works: Vec<PersonWorkItem>,
}

#[tauri::command]
pub async fn get_person_works(st: St<'_>, staff_id: i64) -> CmdResult<PersonDetail> {
    let (mut name_full, mut name_native, mut image_url, local_map, mut works) = {
        let conn = st.db();
        let _ = conn.execute_batch(db::SCHEMA_V6);

        let mut local_map: HashMap<i64, (i64, Option<String>)> = HashMap::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT m.anilist_id,
                            (SELECT COUNT(DISTINCT f.ep_key) FROM local_files f WHERE f.anilist_id = m.anilist_id AND f.ep_key NOT LIKE 'S%'),
                            m.cover_path
                     FROM media m",
                )
                .map_err(err)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(err)?;
            for row in rows.flatten() {
                local_map.insert(row.0, (row.1, row.2));
            }
        }

        let mut nf = String::new();
        let mut nn = None;
        let mut img = None;
        let mut local_works: Vec<PersonWorkItem> = Vec::new();

        // 1. Check voice actor roles in local DB
        {
            let mut stmt = conn
                .prepare(
                    "SELECT c.anilist_id, m.title_romaji, m.title_english, m.title_native, m.format, m.season_year,
                            m.cover_url, m.cover_path, c.role, c.name_full, c.image_url,
                            c.va_name_full, c.va_name_native, c.va_image_url
                     FROM media_characters c
                     JOIN media m ON m.anilist_id = c.anilist_id
                     WHERE c.va_id = ?1
                     ORDER BY COALESCE(m.season_year, 0) DESC",
                )
                .map_err(err)?;
            let rows = stmt
                .query_map([staff_id], |r| {
                    let aid: i64 = r.get(0)?;
                    let owned = local_map.get(&aid).map(|x| x.0).unwrap_or(0);
                    Ok((
                        PersonWorkItem {
                            anilist_id: aid,
                            title_romaji: r.get(1)?,
                            title_english: r.get(2)?,
                            title_native: r.get(3)?,
                            format: r.get(4)?,
                            season_year: r.get(5)?,
                            cover_url: r.get(6)?,
                            cover_path: r.get(7)?,
                            role: r.get(8)?,
                            character_name: r.get(9)?,
                            character_image: r.get(10)?,
                            in_library: owned > 0,
                            owned_count: owned,
                        },
                        r.get::<_, Option<String>>(11)?,
                        r.get::<_, Option<String>>(12)?,
                        r.get::<_, Option<String>>(13)?,
                    ))
                })
                .map_err(err)?;
            for (item, v_nf, v_nn, v_img) in rows.flatten() {
                if nf.is_empty() {
                    if let Some(s) = v_nf {
                        nf = s;
                    }
                    nn = v_nn;
                    img = v_img;
                }
                local_works.push(item);
            }
        }

        // 2. Check staff roles in local DB
        {
            let mut stmt = conn
                .prepare(
                    "SELECT s.anilist_id, m.title_romaji, m.title_english, m.title_native, m.format, m.season_year,
                            m.cover_url, m.cover_path, s.role, s.name_full, s.name_native, s.image_url
                     FROM media_staff s
                     JOIN media m ON m.anilist_id = s.anilist_id
                     WHERE s.staff_id = ?1
                     ORDER BY COALESCE(m.season_year, 0) DESC",
                )
                .map_err(err)?;
            let rows = stmt
                .query_map([staff_id], |r| {
                    let aid: i64 = r.get(0)?;
                    let owned = local_map.get(&aid).map(|x| x.0).unwrap_or(0);
                    Ok((
                        PersonWorkItem {
                            anilist_id: aid,
                            title_romaji: r.get(1)?,
                            title_english: r.get(2)?,
                            title_native: r.get(3)?,
                            format: r.get(4)?,
                            season_year: r.get(5)?,
                            cover_url: r.get(6)?,
                            cover_path: r.get(7)?,
                            role: r.get(8)?,
                            character_name: None,
                            character_image: None,
                            in_library: owned > 0,
                            owned_count: owned,
                        },
                        r.get::<_, String>(9)?,
                        r.get::<_, Option<String>>(10)?,
                        r.get::<_, Option<String>>(11)?,
                    ))
                })
                .map_err(err)?;
            for (item, s_nf, s_nn, s_img) in rows.flatten() {
                if nf.is_empty() {
                    nf = s_nf;
                    nn = s_nn;
                    img = s_img;
                }
                if !local_works
                    .iter()
                    .any(|w| w.anilist_id == item.anilist_id && w.role == item.role)
                {
                    local_works.push(item);
                }
            }
        }

        (nf, nn, img, local_map, local_works)
    };

    let mut occupations = Vec::new();
    // 3. Also query AniList for full filmography so non-cached credits are visible too
    if let Ok(remote) = st.providers.anilist_person_detail(staff_id).await {
        if let Some(s) = remote["name"]["full"].as_str() {
            name_full = s.to_string();
        }
        if let Some(s) = remote["name"]["native"].as_str() {
            name_native = Some(s.to_string());
        }
        if let Some(s) = remote["image"]["large"].as_str() {
            image_url = Some(s.to_string());
        }
        if let Some(arr) = remote["primaryOccupations"].as_array() {
            occupations = arr
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
        }
        if let Some(edges) = remote["characterMedia"]["edges"].as_array() {
            for e in edges {
                let node = &e["node"];
                let Some(aid) = node["id"].as_i64() else {
                    continue;
                };
                if node["type"].as_str().unwrap_or("ANIME") != "ANIME" {
                    continue;
                }
                if works.iter().any(|w| w.anilist_id == aid) {
                    continue;
                }
                let loc = local_map.get(&aid);
                let owned = loc.map(|x| x.0).unwrap_or(0);
                let ch = e["characters"].as_array().and_then(|a| a.first());
                works.push(PersonWorkItem {
                    anilist_id: aid,
                    title_romaji: node["title"]["romaji"].as_str().map(String::from),
                    title_english: node["title"]["english"].as_str().map(String::from),
                    title_native: node["title"]["native"].as_str().map(String::from),
                    format: node["format"].as_str().map(String::from),
                    season_year: node["seasonYear"].as_i64(),
                    cover_url: node["coverImage"]["large"].as_str().map(String::from),
                    cover_path: loc.and_then(|x| x.1.clone()),
                    role: e["characterRole"].as_str().unwrap_or("MAIN").to_string(),
                    character_name: ch.and_then(|c| c["name"]["full"].as_str().map(String::from)),
                    character_image: ch
                        .and_then(|c| c["image"]["large"].as_str().map(String::from)),
                    in_library: owned > 0,
                    owned_count: owned,
                });
            }
        }
        if let Some(edges) = remote["staffMedia"]["edges"].as_array() {
            for e in edges {
                let node = &e["node"];
                let Some(aid) = node["id"].as_i64() else {
                    continue;
                };
                if works.iter().any(|w| w.anilist_id == aid) {
                    continue;
                }
                let loc = local_map.get(&aid);
                let owned = loc.map(|x| x.0).unwrap_or(0);
                works.push(PersonWorkItem {
                    anilist_id: aid,
                    title_romaji: node["title"]["romaji"].as_str().map(String::from),
                    title_english: node["title"]["english"].as_str().map(String::from),
                    title_native: node["title"]["native"].as_str().map(String::from),
                    format: node["format"].as_str().map(String::from),
                    season_year: node["seasonYear"].as_i64(),
                    cover_url: node["coverImage"]["large"].as_str().map(String::from),
                    cover_path: loc.and_then(|x| x.1.clone()),
                    role: e["staffRole"].as_str().unwrap_or("Staff").to_string(),
                    character_name: None,
                    character_image: None,
                    in_library: owned > 0,
                    owned_count: owned,
                });
            }
        }
    }

    works.sort_by(|a, b| {
        b.in_library
            .cmp(&a.in_library)
            .then_with(|| b.season_year.unwrap_or(0).cmp(&a.season_year.unwrap_or(0)))
    });

    Ok(PersonDetail {
        staff_id,
        name_full,
        name_native,
        image_url,
        occupations,
        works,
    })
}

// ---------------------------------------------------------------------------
// v0.5: Smart Subtitle Matcher, Auto-Retimer & Jimaku Fetcher
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn preview_subtitle_matches(
    st: St<'_>,
    anilist_id: i64,
    default_lang: Option<String>,
) -> CmdResult<crate::subtitles::SubtitleMatchPreview> {
    let lang = default_lang.unwrap_or_else(|| "ja".to_string());
    crate::subtitles::preview_subtitle_matches(&st.db(), anilist_id, None, &lang).map_err(err)
}

#[tauri::command]
pub async fn apply_subtitle_matches(
    app: AppHandle,
    st: St<'_>,
    anilist_id: i64,
    mode: String,
    items: Vec<crate::subtitles::SubtitleMatchItem>,
    auto_retime: bool,
) -> CmdResult<crate::subtitles::SubtitleApplyReport> {
    let st_arc = st.inner().clone();
    let res = tokio::task::spawn_blocking(move || {
        crate::subtitles::apply_subtitle_matches(&st_arc, anilist_id, &mode, items, auto_retime)
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    let _ = app.emit("library-changed", ());
    Ok(res)
}

#[tauri::command]
pub async fn retime_episode_subtitle(
    app: AppHandle,
    st: St<'_>,
    anilist_id: i64,
    ep_key: String,
    subtitle_path: String,
) -> CmdResult<crate::subtitles::RetimeReport> {
    let video_path: String = {
        let conn = st.db();
        conn.query_row(
            "SELECT path FROM local_files WHERE anilist_id = ?1 AND ep_key = ?2 ORDER BY id LIMIT 1",
            params![anilist_id, ep_key],
            |r| r.get(0),
        )
        .map_err(|_| "No local video file found for this episode to align against.".to_string())?
    };

    let sub_pb = std::path::PathBuf::from(&subtitle_path);
    if !sub_pb.is_file() {
        return Err("Subtitle file does not exist on disk.".into());
    }
    let ext = sub_pb
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("srt")
        .to_ascii_lowercase();
    let stem = sub_pb
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("sub")
        .trim_end_matches(".retimed");

    let cache_dir = st.data_dir.join("subtitles").join(anilist_id.to_string());
    let _ = std::fs::create_dir_all(&cache_dir);
    let out_pb = cache_dir.join(format!("ep{}_{}.retimed.{}", ep_key, stem, ext));
    let vid_pb = std::path::PathBuf::from(&video_path);

    let sub_clone = sub_pb.clone();
    let out_clone = out_pb.clone();
    let mut report = tokio::task::spawn_blocking(move || {
        crate::subtitles::retime_subtitle_file(&sub_clone, &vid_pb, &out_clone)
    })
    .await
    .map_err(err)?
    .map_err(err)?;

    {
        let conn = st.db();
        let lang = crate::subtitles::detect_sub_language(stem).unwrap_or_else(|| "ja".into());
        let cc = crate::subtitles::detect_cc_tag(stem);
        // Replace old un-retimed link if it pointed to `subtitle_path`
        let _ = conn.execute(
            "DELETE FROM linked_subtitles WHERE anilist_id = ?1 AND ep_key = ?2 AND path = ?3",
            params![anilist_id, ep_key, subtitle_path],
        );
        let _ = crate::subtitles::upsert_linked_subtitle(
            &conn,
            anilist_id,
            &ep_key,
            &report.output_path,
            &lang,
            cc,
            "retimed",
            true,
            Some(report.offset_ms),
        );
    }

    report.loaded_into_mpv =
        crate::player::try_hotload_subtitle_in_mpv(&st, anilist_id, &ep_key, &report.output_path)
            .await;
    let _ = app.emit("library-changed", ());
    Ok(report)
}

#[tauri::command]
pub async fn remove_linked_subtitle(app: AppHandle, st: St<'_>, id: i64) -> CmdResult<()> {
    crate::subtitles::remove_linked_subtitle(&st.db(), id).map_err(err)?;
    let _ = app.emit("library-changed", ());
    Ok(())
}

#[tauri::command]
pub async fn jimaku_search(
    st: St<'_>,
    anilist_id: i64,
    episode: Option<i64>,
) -> CmdResult<crate::subtitles::JimakuSearchResponse> {
    crate::subtitles::jimaku_search_files(&st, anilist_id, episode)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn jimaku_download(
    app: AppHandle,
    st: St<'_>,
    anilist_id: i64,
    ep_key: String,
    file_name: String,
    file_url: String,
    auto_retime: bool,
) -> CmdResult<crate::subtitles::JimakuDownloadResult> {
    let res = crate::subtitles::jimaku_download_and_process(
        &st,
        anilist_id,
        &ep_key,
        &file_name,
        &file_url,
        auto_retime,
    )
    .await
    .map_err(err)?;
    let _ = app.emit("library-changed", ());
    Ok(res)
}


// ---------------------------------------------------------------------------
// Phase 8: Streaming & Hybrid Mode, Catalog Browser, and Torrent Engine
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonSearchResult {
    pub staff_id: i64,
    pub name_full: String,
    pub name_native: Option<String>,
    pub image_url: Option<String>,
    pub occupations: Vec<String>,
}

#[tauri::command]
pub async fn toggle_stream_library(
    app: AppHandle,
    st: St<'_>,
    anilist_id: i64,
) -> CmdResult<bool> {
    let has_media: bool = {
        let conn = st.db();
        let _ = conn.execute_batch(db::SCHEMA_V7);
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM media WHERE anilist_id = ?1)",
            [anilist_id],
            |r| r.get(0),
        )
        .unwrap_or(false)
    };
    if !has_media {
        crate::service::ensure_media(&st, anilist_id)
            .await
            .map_err(err)?;
    }

    let now_in_lib = {
        let conn = st.db();
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM stream_library WHERE anilist_id = ?1)",
                [anilist_id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if exists {
            conn.execute("DELETE FROM stream_library WHERE anilist_id = ?1", [anilist_id])
                .map_err(err)?;
            false
        } else {
            conn.execute(
                "INSERT OR REPLACE INTO stream_library (anilist_id, added_at) VALUES (?1, ?2)",
                params![anilist_id, db::now()],
            )
            .map_err(err)?;
            true
        }
    };

    let _ = app.emit("library-changed", ());
    Ok(now_in_lib)
}

#[tauri::command]
pub async fn browse_catalog(
    st: St<'_>,
    search: Option<String>,
    genre: Option<String>,
    format: Option<String>,
    status: Option<String>,
    season: Option<String>,
    year: Option<i64>,
    sort: Option<String>,
    page: Option<u32>,
) -> CmdResult<Vec<SeasonalItem>> {
    let media = st
        .providers
        .anilist_browse(
            search.as_deref(),
            genre.as_deref(),
            format.as_deref(),
            status.as_deref(),
            season.as_deref(),
            year,
            sort.as_deref(),
            page.unwrap_or(1),
            30,
        )
        .await
        .map_err(err)?;

    let conn = st.db();
    let _ = conn.execute_batch(db::SCHEMA_V5);
    let _ = conn.execute_batch(db::SCHEMA_V7);

    let mut items = Vec::with_capacity(media.len());
    for m in media {
        let id = m.id;
        let owned_count: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT ep_key) FROM local_files WHERE anilist_id = ?1 AND ep_key NOT LIKE 'S%'",
                [id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let in_stream: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM stream_library WHERE anilist_id = ?1)",
                [id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        let watched_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM watch_state WHERE anilist_id = ?1 AND ep_key NOT LIKE 'S%'",
                [id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let cover_path: Option<String> = conn
            .query_row(
                "SELECT cover_path FROM media WHERE anilist_id = ?1",
                [id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        let rem = crate::anilist_sync::get_remote_entry(&conn, id);

        items.push(SeasonalItem {
            anilist_id: id,
            link_id: id,
            title_romaji: m.title.romaji,
            title_english: m.title.english,
            title_native: m.title.native,
            format: m.format,
            status: m.status,
            episodes: m.episodes,
            duration: m.duration,
            season: m.season,
            season_year: m.season_year,
            average_score: m.average_score,
            genres: m.genres,
            studios: m
                .studios
                .map(|s| s.nodes.into_iter().map(|n| n.name).collect())
                .unwrap_or_default(),
            start_date: m.start_date.and_then(|d| d.to_string_opt()),
            cover_url: m
                .cover_image
                .as_ref()
                .and_then(|c| c.extra_large.clone().or_else(|| c.large.clone())),
            cover_path,
            cover_color: m.cover_image.and_then(|c| c.color),
            banner_url: m.banner_image,
            next_airing_episode: m.next_airing_episode.as_ref().map(|a| a.episode),
            next_airing_at: m.next_airing_episode.as_ref().map(|a| a.airing_at),
            in_library: owned_count > 0 || in_stream,
            owned_count,
            watched_count,
            anilist_status: rem.as_ref().map(|r| r.status.clone()),
            anilist_progress: rem.as_ref().map(|r| r.progress),
        });
    }

    Ok(items)
}

#[tauri::command]
pub async fn search_people(st: St<'_>, query: String) -> CmdResult<Vec<PersonSearchResult>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let val = st
        .providers
        .anilist_search_people(&query)
        .await
        .map_err(err)?;
    let mut out = Vec::new();
    if let Some(arr) = val.as_array() {
        for item in arr {
            let Some(staff_id) = item["id"].as_i64() else {
                continue;
            };
            let name_full = item["name"]["full"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string();
            let name_native = item["name"]["native"].as_str().map(|s| s.to_string());
            let image_url = item["image"]["large"].as_str().map(|s| s.to_string());
            let occupations = item["primaryOccupations"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            out.push(PersonSearchResult {
                staff_id,
                name_full,
                name_native,
                image_url,
                occupations,
            });
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn search_episode_torrents(
    st: St<'_>,
    anilist_id: i64,
    episode: i64,
    custom_query: Option<String>,
) -> CmdResult<Vec<crate::torrent::TorrentCandidate>> {
    crate::torrent::search_episode_torrents(&st, anilist_id, episode, custom_query)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn start_torrent_session(
    app: AppHandle,
    st: St<'_>,
    anilist_id: i64,
    ep_key: String,
    episode_num: Option<i64>,
    candidate: crate::torrent::TorrentCandidate,
    mode_override: Option<String>,
) -> CmdResult<crate::torrent::TorrentSessionStatus> {
    crate::torrent::start_torrent_session(
        app,
        st.inner().clone(),
        anilist_id,
        ep_key,
        episode_num,
        candidate,
        mode_override,
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn list_torrent_sessions() -> CmdResult<Vec<crate::torrent::TorrentSessionStatus>> {
    Ok(crate::torrent::list_active_sessions())
}

#[tauri::command]
pub async fn cancel_torrent_session(session_id: String) -> CmdResult<()> {
    crate::torrent::cancel_session(&session_id);
    Ok(())
}

#[tauri::command]
pub async fn list_torrent_extensions(
    st: St<'_>,
) -> CmdResult<Vec<crate::torrent::TorrentExtension>> {
    let conn = st.db();
    Ok(crate::torrent::list_torrent_extensions(&conn))
}

#[tauri::command]
pub async fn import_torrent_extension(
    st: St<'_>,
    input: String,
) -> CmdResult<Vec<crate::torrent::TorrentExtension>> {
    crate::torrent::import_torrent_extension(st.inner(), &input)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn toggle_torrent_extension(
    st: St<'_>,
    id: String,
    enabled: bool,
) -> CmdResult<Vec<crate::torrent::TorrentExtension>> {
    crate::torrent::toggle_torrent_extension(st.inner(), &id, enabled).map_err(err)
}

#[tauri::command]
pub async fn remove_torrent_extension(
    st: St<'_>,
    id: String,
) -> CmdResult<Vec<crate::torrent::TorrentExtension>> {
    crate::torrent::remove_torrent_extension(st.inner(), &id).map_err(err)
}

// ---------------------------------------------------------------------------
// Built-in Player & Yomitan Dictionary Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn builtin_player_get_state() -> CmdResult<crate::embedded_player::BuiltinPlayerState> {
    Ok(crate::embedded_player::get_state())
}

#[tauri::command]
pub async fn builtin_player_command(
    app: AppHandle,
    st: St<'_>,
    command: crate::embedded_player::BuiltinCommand,
) -> CmdResult<crate::embedded_player::BuiltinPlayerState> {
    crate::embedded_player::dispatch_command(app, st.inner().clone(), command)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn lookup_japanese_at(
    st: St<'_>,
    text: String,
    char_offset: usize,
) -> CmdResult<crate::dictionary::LookupResult> {
    let data_dir = st.data_dir.clone();
    let st_arc = st.inner().clone();
    tokio::task::spawn_blocking(move || {
        let dict_conn = crate::dictionary::open_dict_db(&data_dir)?;
        let mut res = crate::dictionary::lookup_at(&dict_conn, &text, char_offset)?;
        if !res.terms.is_empty() {
            let kura_conn = st_arc.db();
            if let Ok(mut stmt) = kura_conn.prepare(
                "SELECT status, lookup_count FROM immersion_words WHERE expression = ?1 AND reading = ?2",
            ) {
                for term in &mut res.terms {
                    if let Ok((status, count)) = stmt.query_row(
                        rusqlite::params![&term.expression, &term.reading],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                    ) {
                        term.word_status = Some(status);
                        term.lookup_count = count;
                    }
                }
            };
        }
        Ok::<_, anyhow::Error>(res)
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn list_dictionaries(
    st: St<'_>,
) -> CmdResult<Vec<crate::dictionary::DictionaryInfo>> {
    let data_dir = st.data_dir.clone();
    tokio::task::spawn_blocking(move || {
        let conn = crate::dictionary::open_dict_db(&data_dir)?;
        crate::dictionary::list_dictionaries(&conn)
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn import_yomitan_dictionary(
    st: St<'_>,
    zip_path: String,
) -> CmdResult<crate::dictionary::DictionaryInfo> {
    let data_dir = st.data_dir.clone();
    tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(&zip_path)?;
        let mut conn = crate::dictionary::open_dict_db(&data_dir)?;
        crate::dictionary::import_yomitan_zip_bytes(&mut conn, &bytes)
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
pub async fn download_recommended_dictionaries(
    app: AppHandle,
    st: St<'_>,
) -> CmdResult<Vec<crate::dictionary::DictionaryInfo>> {
    crate::dictionary::download_recommended_dictionaries(&app, &st.data_dir)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn remove_dictionary(st: St<'_>, id: i64) -> CmdResult<()> {
    let data_dir = st.data_dir.clone();
    tokio::task::spawn_blocking(move || {
        let conn = crate::dictionary::open_dict_db(&data_dir)?;
        crate::dictionary::remove_dictionary(&conn, id)
    })
    .await
    .map_err(err)?
    .map_err(err)
}

// ---------------------------------------------------------------------------
// Japanese Immersion Suite Commands (Vocabulary Bank & Immersion Time Analytics)
// ---------------------------------------------------------------------------

static LAST_LOOKUP_DEDUP: std::sync::OnceLock<
    std::sync::Mutex<Option<(String, String, String, std::time::Instant)>>,
> = std::sync::OnceLock::new();

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordWordLookupInput {
    pub expression: String,
    pub reading: String,
    pub matched_text: String,
    pub furigana: Vec<crate::dictionary::FuriganaSegment>,
    pub glossaries: Vec<String>,
    pub pitches: Vec<crate::dictionary::PitchAccent>,
    pub frequencies: Vec<String>,
    pub pos_tags: String,
    pub anilist_id: Option<i64>,
    pub ep_key: Option<String>,
    pub sentence: Option<String>,
    pub position_sec: Option<f64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetWordStatusInput {
    pub expression: String,
    pub reading: String,
    pub status: String,
    pub matched_text: Option<String>,
    pub furigana: Option<Vec<crate::dictionary::FuriganaSegment>>,
    pub glossaries: Option<Vec<String>>,
    pub pitches: Option<Vec<crate::dictionary::PitchAccent>>,
    pub frequencies: Option<Vec<String>>,
    pub pos_tags: Option<String>,
    pub anilist_id: Option<i64>,
    pub ep_key: Option<String>,
    pub sentence: Option<String>,
    pub position_sec: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WordStatusResult {
    pub id: i64,
    pub expression: String,
    pub reading: String,
    pub status: String,
    pub lookup_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImmersionContextItem {
    pub id: i64,
    pub anilist_id: Option<i64>,
    pub ep_key: Option<String>,
    pub anime_title: Option<String>,
    pub cover_url: Option<String>,
    pub cover_path: Option<String>,
    pub sentence: String,
    pub matched_surface: String,
    pub position_sec: Option<f64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImmersionWordItem {
    pub id: i64,
    pub expression: String,
    pub reading: String,
    pub matched_forms: Vec<String>,
    pub status: String,
    pub lookup_count: i64,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub furigana: Vec<crate::dictionary::FuriganaSegment>,
    pub glossaries: Vec<String>,
    pub pitches: Vec<crate::dictionary::PitchAccent>,
    pub frequencies: Vec<String>,
    pub pos_tags: String,
    pub contexts: Vec<ImmersionContextItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImmersionDayPoint {
    pub day_key: String,
    pub label: String,
    pub active_seconds: i64,
    pub lookup_count: i64,
    pub words_tracked: i64,
    pub episodes_completed: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImmersionAnimeSummary {
    pub anilist_id: i64,
    pub title: String,
    pub cover_url: Option<String>,
    pub cover_path: Option<String>,
    pub active_seconds: i64,
    pub episodes_watched: i64,
    pub words_looked_up: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImmersionSessionItem {
    pub id: i64,
    pub day_key: String,
    pub anilist_id: i64,
    pub ep_key: String,
    pub source_label: String,
    pub cover_url: Option<String>,
    pub cover_path: Option<String>,
    pub active_seconds: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImmersionOverview {
    pub today_immersion_sec: i64,
    pub week_immersion_sec: i64,
    pub month_immersion_sec: i64,
    pub total_immersion_sec: i64,
    pub daily_goal_minutes: i64,
    pub current_streak_days: i64,
    pub best_streak_days: i64,
    pub total_episodes_completed: i64,
    pub total_lookups: i64,
    pub unique_words_count: i64,
    pub learning_words_count: i64,
    pub known_words_count: i64,
    pub ignored_words_count: i64,
    pub daily_activity: Vec<ImmersionDayPoint>,
    pub top_anime: Vec<ImmersionAnimeSummary>,
    pub recent_sessions: Vec<ImmersionSessionItem>,
    pub words: Vec<ImmersionWordItem>,
    pub anilist_immersion_list: Option<String>,
    pub anilist_immersion_shows_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnilistListOption {
    pub name: String,
    pub is_custom_list: bool,
    pub entry_count: usize,
    pub total_episodes: i64,
    pub estimated_hours: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnilistImmersionListsResponse {
    pub selected_list: Option<String>,
    pub synced_show_count: i64,
    pub synced_hours: f64,
    pub lists: Vec<AnilistListOption>,
}

fn save_word_context_if_present(
    conn: &rusqlite::Connection,
    word_id: i64,
    anilist_id: Option<i64>,
    ep_key: Option<&str>,
    sentence: Option<&str>,
    matched_surface: &str,
    position_sec: Option<f64>,
    now_ts: i64,
) {
    let Some(sent_raw) = sentence else { return };
    let sent = sent_raw.trim();
    if sent.is_empty() {
        return;
    }
    let _ = conn.execute(
        "INSERT INTO immersion_contexts (word_id, anilist_id, ep_key, sentence, matched_surface, position_sec, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(word_id, sentence) DO UPDATE SET
            anilist_id = COALESCE(excluded.anilist_id, immersion_contexts.anilist_id),
            ep_key = COALESCE(excluded.ep_key, immersion_contexts.ep_key),
            matched_surface = CASE WHEN excluded.matched_surface != '' THEN excluded.matched_surface ELSE immersion_contexts.matched_surface END,
            position_sec = COALESCE(excluded.position_sec, immersion_contexts.position_sec),
            created_at = excluded.created_at",
        rusqlite::params![word_id, anilist_id, ep_key, sent, matched_surface, position_sec, now_ts],
    );
    // Keep up to 8 most recent context sentences per word
    let _ = conn.execute(
        "DELETE FROM immersion_contexts
         WHERE word_id = ?1 AND id NOT IN (
             SELECT id FROM immersion_contexts WHERE word_id = ?1 ORDER BY created_at DESC LIMIT 8
         )",
        [word_id],
    );
}

#[tauri::command]
pub async fn record_word_lookup(
    st: St<'_>,
    input: RecordWordLookupInput,
) -> CmdResult<WordStatusResult> {
    let expr = input.expression.trim().to_string();
    let read = if input.reading.trim().is_empty() {
        expr.clone()
    } else {
        input.reading.trim().to_string()
    };
    if expr.is_empty() {
        return Err("Empty word expression".into());
    }

    let sent_clean = input.sentence.clone().unwrap_or_default().trim().to_string();
    let is_duplicate = {
        let mtx = LAST_LOOKUP_DEDUP.get_or_init(|| std::sync::Mutex::new(None));
        let mut g = mtx.lock().unwrap_or_else(|e| e.into_inner());
        let now_inst = std::time::Instant::now();
        let dup = match g.as_ref() {
            Some((prev_e, prev_r, prev_s, prev_t)) => {
                prev_e == &expr
                    && prev_r == &read
                    && ((prev_s == &sent_clean && now_inst.duration_since(*prev_t).as_secs() < 30)
                        || now_inst.duration_since(*prev_t).as_secs() < 6)
            }
            None => false,
        };
        if !dup {
            *g = Some((expr.clone(), read.clone(), sent_clean.clone(), now_inst));
        }
        dup
    };

    let conn = st.db();
    let now_ts = db::now();

    if is_duplicate {
        if let Ok((id, status, count)) = conn.query_row(
            "SELECT id, status, lookup_count FROM immersion_words WHERE expression = ?1 AND reading = ?2",
            rusqlite::params![&expr, &read],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
        ) {
            return Ok(WordStatusResult {
                id,
                expression: expr,
                reading: read,
                status,
                lookup_count: count,
            });
        }
    }

    let furi_json = serde_json::to_string(&input.furigana).unwrap_or_else(|_| "[]".into());
    let gloss_json = serde_json::to_string(&input.glossaries).unwrap_or_else(|_| "[]".into());
    let pitch_json = serde_json::to_string(&input.pitches).unwrap_or_else(|_| "[]".into());
    let freq_json = serde_json::to_string(&input.frequencies).unwrap_or_else(|_| "[]".into());
    let surface = input.matched_text.trim().to_string();

    let existing: Option<(i64, String, i64, String)> = conn
        .query_row(
            "SELECT id, status, lookup_count, matched_forms FROM immersion_words WHERE expression = ?1 AND reading = ?2",
            rusqlite::params![&expr, &read],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .ok();

    let (word_id, status, new_count) = if let Some((id, st_val, count, forms_str)) = existing {
        let mut forms: Vec<String> = serde_json::from_str(&forms_str).unwrap_or_default();
        if !surface.is_empty() && !forms.contains(&surface) {
            forms.push(surface.clone());
        }
        let forms_json = serde_json::to_string(&forms).unwrap_or_else(|_| "[]".into());
        let next_count = count + 1;
        conn.execute(
            "UPDATE immersion_words SET
                lookup_count = ?1,
                last_seen_at = ?2,
                matched_forms = ?3,
                furigana_json = CASE WHEN ?4 != '[]' THEN ?4 ELSE furigana_json END,
                glossary_json = CASE WHEN ?5 != '[]' THEN ?5 ELSE glossary_json END,
                pitch_json = CASE WHEN ?6 != '[]' THEN ?6 ELSE pitch_json END,
                freq_json = CASE WHEN ?7 != '[]' THEN ?7 ELSE freq_json END,
                pos_tags = CASE WHEN ?8 != '' THEN ?8 ELSE pos_tags END
             WHERE id = ?9",
            rusqlite::params![
                next_count,
                now_ts,
                forms_json,
                furi_json,
                gloss_json,
                pitch_json,
                freq_json,
                input.pos_tags,
                id
            ],
        )
        .map_err(err)?;
        (id, st_val, next_count)
    } else {
        let forms = if surface.is_empty() {
            vec![expr.clone()]
        } else {
            vec![surface.clone()]
        };
        let forms_json = serde_json::to_string(&forms).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "INSERT INTO immersion_words
             (expression, reading, matched_forms, status, lookup_count, first_seen_at, last_seen_at, furigana_json, glossary_json, pitch_json, freq_json, pos_tags)
             VALUES (?1, ?2, ?3, 'learning', 1, ?4, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                &expr,
                &read,
                forms_json,
                now_ts,
                furi_json,
                gloss_json,
                pitch_json,
                freq_json,
                input.pos_tags
            ],
        )
        .map_err(err)?;
        let id = conn.last_insert_rowid();
        let _ = conn.execute(
            "INSERT INTO immersion_daily (day_key, active_seconds, lookup_count, words_tracked, episodes_completed)
             VALUES (?1, 0, 0, 1, 0)
             ON CONFLICT(day_key) DO UPDATE SET words_tracked = words_tracked + 1",
            [db::today_day_key()],
        );
        (id, "learning".to_string(), 1)
    };

    let _ = conn.execute(
        "INSERT INTO immersion_daily (day_key, active_seconds, lookup_count, words_tracked, episodes_completed)
         VALUES (?1, 0, 1, 0, 0)
         ON CONFLICT(day_key) DO UPDATE SET lookup_count = lookup_count + 1",
        [db::today_day_key()],
    );

    save_word_context_if_present(
        &conn,
        word_id,
        input.anilist_id,
        input.ep_key.as_deref(),
        input.sentence.as_deref(),
        &surface,
        input.position_sec,
        now_ts,
    );

    crate::backup::mark_dirty();

    Ok(WordStatusResult {
        id: word_id,
        expression: expr,
        reading: read,
        status,
        lookup_count: new_count,
    })
}

#[tauri::command]
pub async fn set_word_status(
    st: St<'_>,
    input: SetWordStatusInput,
) -> CmdResult<WordStatusResult> {
    let expr = input.expression.trim().to_string();
    let read = if input.reading.trim().is_empty() {
        expr.clone()
    } else {
        input.reading.trim().to_string()
    };
    let status = match input.status.trim().to_ascii_lowercase().as_str() {
        "known" => "known",
        "ignored" | "ignore" => "ignored",
        _ => "learning",
    }
    .to_string();

    let conn = st.db();
    let now_ts = db::now();
    let surface = input.matched_text.clone().unwrap_or_default().trim().to_string();

    let existing: Option<(i64, i64, String)> = conn
        .query_row(
            "SELECT id, lookup_count, matched_forms FROM immersion_words WHERE expression = ?1 AND reading = ?2",
            rusqlite::params![&expr, &read],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)),
        )
        .ok();

    let (word_id, count) = if let Some((id, lookup_count, forms_str)) = existing {
        let mut forms: Vec<String> = serde_json::from_str(&forms_str).unwrap_or_default();
        if !surface.is_empty() && !forms.contains(&surface) {
            forms.push(surface.clone());
        }
        let forms_json = serde_json::to_string(&forms).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "UPDATE immersion_words SET status = ?1, matched_forms = ?2, last_seen_at = ?3 WHERE id = ?4",
            rusqlite::params![&status, forms_json, now_ts, id],
        )
        .map_err(err)?;
        (id, lookup_count)
    } else {
        let forms = if surface.is_empty() {
            vec![expr.clone()]
        } else {
            vec![surface.clone()]
        };
        let forms_json = serde_json::to_string(&forms).unwrap_or_else(|_| "[]".into());
        let furi_json = input
            .furigana
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_else(|| "[]".into());
        let gloss_json = input
            .glossaries
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_else(|| "[]".into());
        let pitch_json = input
            .pitches
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_else(|| "[]".into());
        let freq_json = input
            .frequencies
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_else(|| "[]".into());
        let pos_tags = input.pos_tags.clone().unwrap_or_default();

        conn.execute(
            "INSERT INTO immersion_words
             (expression, reading, matched_forms, status, lookup_count, first_seen_at, last_seen_at, furigana_json, glossary_json, pitch_json, freq_json, pos_tags)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                &expr,
                &read,
                forms_json,
                &status,
                now_ts,
                furi_json,
                gloss_json,
                pitch_json,
                freq_json,
                pos_tags
            ],
        )
        .map_err(err)?;
        let id = conn.last_insert_rowid();
        let _ = conn.execute(
            "INSERT INTO immersion_daily (day_key, active_seconds, lookup_count, words_tracked, episodes_completed)
             VALUES (?1, 0, 1, 1, 0)
             ON CONFLICT(day_key) DO UPDATE SET lookup_count = lookup_count + 1, words_tracked = words_tracked + 1",
            [db::today_day_key()],
        );
        (id, 1)
    };

    save_word_context_if_present(
        &conn,
        word_id,
        input.anilist_id,
        input.ep_key.as_deref(),
        input.sentence.as_deref(),
        &surface,
        input.position_sec,
        now_ts,
    );

    crate::backup::mark_dirty();

    Ok(WordStatusResult {
        id: word_id,
        expression: expr,
        reading: read,
        status,
        lookup_count: count,
    })
}

#[tauri::command]
pub async fn delete_immersion_word(st: St<'_>, id: i64) -> CmdResult<()> {
    let conn = st.db();
    conn.execute("DELETE FROM immersion_words WHERE id = ?1", [id])
        .map_err(err)?;
    crate::backup::mark_dirty();
    Ok(())
}

#[tauri::command]
pub async fn get_tracked_surface_forms(st: St<'_>) -> CmdResult<Vec<String>> {
    let conn = st.db();
    let mut stmt = conn
        .prepare("SELECT expression, reading, matched_forms FROM immersion_words WHERE status = 'learning'")
        .map_err(err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(err)?;

    let mut set = std::collections::HashSet::new();
    for row in rows.flatten() {
        let (expr, read, forms_str) = row;
        if !expr.trim().is_empty() {
            set.insert(expr);
        }
        if !read.trim().is_empty() && read.chars().count() >= 2 {
            set.insert(read);
        }
        if let Ok(forms) = serde_json::from_str::<Vec<String>>(&forms_str) {
            for f in forms {
                if !f.trim().is_empty() {
                    set.insert(f);
                }
            }
        }
    }
    let mut out: Vec<String> = set.into_iter().collect();
    out.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()));
    Ok(out)
}

#[tauri::command]
pub async fn set_immersion_daily_goal(st: St<'_>, minutes: i64) -> CmdResult<i64> {
    let clamped = minutes.clamp(5, 720);
    let conn = st.db();
    db::set_setting(&conn, "pref.immersion_daily_goal", &clamped.to_string()).map_err(err)?;
    crate::backup::mark_dirty();
    Ok(clamped)
}

#[tauri::command]
pub async fn log_manual_immersion(
    st: St<'_>,
    minutes: i64,
    label: String,
    anilist_id: Option<i64>,
    ep_key: Option<String>,
) -> CmdResult<()> {
    let mins = minutes.clamp(1, 720);
    let conn = st.db();
    let clean_label = if label.trim().is_empty() {
        "Manual Immersion"
    } else {
        label.trim()
    };
    let ek = ep_key.unwrap_or_else(|| format!("manual:{}", db::now()));
    db::record_immersion_time(&conn, anilist_id, Some(&ek), Some(clean_label), mins * 60)
        .map_err(err)?;
    crate::backup::mark_dirty();
    Ok(())
}

#[tauri::command]
pub async fn get_immersion_overview(st: St<'_>) -> CmdResult<ImmersionOverview> {
    let conn = st.db();
    let _ = db::backfill_immersion_from_watch_state(&conn);

    let daily_goal_minutes: i64 = db::get_setting(&conn, "pref.immersion_daily_goal")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(60)
        .clamp(5, 720);

    // Load all daily rows into a map
    let mut daily_map: std::collections::HashMap<String, (i64, i64, i64, i64)> =
        std::collections::HashMap::new();
    let mut total_immersion_sec = 0i64;
    if let Ok(mut stmt) = conn.prepare(
        "SELECT day_key, active_seconds, lookup_count, words_tracked, episodes_completed FROM immersion_daily",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        }) {
            for row in rows.flatten() {
                total_immersion_sec += row.1.max(0);
                daily_map.insert(row.0, (row.1.max(0), row.2.max(0), row.3.max(0), row.4.max(0)));
            }
        }
    }

    let now_local = chrono::Local::now().date_naive();
    let today_key = now_local.format("%Y-%m-%d").to_string();
    let today_immersion_sec = daily_map.get(&today_key).map(|x| x.0).unwrap_or(0);

    let mut week_immersion_sec = 0i64;
    let mut month_immersion_sec = 0i64;
    let mut daily_activity = Vec::with_capacity(30);

    for days_ago in (0..30).rev() {
        let d = now_local - chrono::Days::new(days_ago);
        let key = d.format("%Y-%m-%d").to_string();
        let label = d.format("%b %d").to_string();
        let (act_sec, l_cnt, w_trk, ep_cmp) = daily_map.get(&key).copied().unwrap_or((0, 0, 0, 0));
        if days_ago < 7 {
            week_immersion_sec += act_sec;
        }
        month_immersion_sec += act_sec;
        daily_activity.push(ImmersionDayPoint {
            day_key: key,
            label,
            active_seconds: act_sec,
            lookup_count: l_cnt,
            words_tracked: w_trk,
            episodes_completed: ep_cmp,
        });
    }

    // Calculate current streak and best streak
    let is_active_day = |key: &str| -> bool {
        daily_map
            .get(key)
            .map(|(sec, lookups, _, eps)| *sec >= 60 || *lookups > 0 || *eps > 0)
            .unwrap_or(false)
    };

    let mut current_streak_days = 0i64;
    let mut cursor = now_local;
    if !is_active_day(&cursor.format("%Y-%m-%d").to_string()) {
        cursor = cursor - chrono::Days::new(1);
    }
    while is_active_day(&cursor.format("%Y-%m-%d").to_string()) {
        current_streak_days += 1;
        cursor = cursor - chrono::Days::new(1);
    }

    let mut active_dates: Vec<chrono::NaiveDate> = daily_map
        .iter()
        .filter(|(_, (sec, lookups, _, eps))| *sec >= 60 || *lookups > 0 || *eps > 0)
        .filter_map(|(k, _)| chrono::NaiveDate::parse_from_str(k, "%Y-%m-%d").ok())
        .collect();
    active_dates.sort();
    let mut best_streak_days = 0i64;
    let mut run = 0i64;
    let mut prev_date: Option<chrono::NaiveDate> = None;
    for d in active_dates {
        if let Some(p) = prev_date {
            if d == p + chrono::Days::new(1) {
                run += 1;
            } else if d != p {
                run = 1;
            }
        } else {
            run = 1;
        }
        if run > best_streak_days {
            best_streak_days = run;
        }
        prev_date = Some(d);
    }

    let anilist_immersion_list = db::get_setting(&conn, "pref.anilist_immersion_list")
        .ok()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut total_episodes_completed: i64 = daily_map.values().map(|(_, _, _, eps)| *eps).sum();

    // Real-time sessions grouped by anilist_id
    let mut session_sec_by_aid: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    let mut top_anime_map: std::collections::HashMap<String, ImmersionAnimeSummary> =
        std::collections::HashMap::new();

    if let Ok(mut stmt) = conn.prepare(
        "SELECT s.anilist_id,
                MAX(COALESCE(m.title_english, m.title_romaji, m.title_native, NULLIF(s.source_label, ''), 'Unknown Anime')),
                m.cover_url,
                m.cover_path,
                SUM(s.active_seconds) AS total_sec,
                COUNT(DISTINCT CASE WHEN s.ep_key != '' THEN s.ep_key END) AS ep_cnt,
                (SELECT COUNT(DISTINCT c.word_id) FROM immersion_contexts c WHERE c.anilist_id = s.anilist_id AND s.anilist_id > 0) AS word_cnt
         FROM immersion_sessions s
         LEFT JOIN media m ON m.anilist_id = s.anilist_id
         GROUP BY CASE WHEN s.anilist_id > 0 THEN CAST(s.anilist_id AS TEXT) ELSE s.source_label END
         HAVING total_sec > 0",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok(ImmersionAnimeSummary {
                anilist_id: r.get::<_, i64>(0)?,
                title: r.get::<_, String>(1)?,
                cover_url: r.get(2)?,
                cover_path: r.get(3)?,
                active_seconds: r.get::<_, i64>(4)?,
                episodes_watched: r.get::<_, i64>(5)?,
                words_looked_up: r.get::<_, i64>(6).unwrap_or(0),
            })
        }) {
            for item in rows.flatten() {
                if item.anilist_id > 0 {
                    session_sec_by_aid.insert(item.anilist_id, item.active_seconds);
                    top_anime_map.insert(format!("aid:{}", item.anilist_id), item);
                } else {
                    top_anime_map.insert(format!("lbl:{}", item.title), item);
                }
            }
        }
    }

    // Merge shows from the user's selected AniList Japanese Immersion List (if configured)
    let mut anilist_immersion_shows_count = 0i64;
    if anilist_immersion_list.is_some() {
        let _ = conn.execute_batch(db::SCHEMA_V8);
        if let Ok(mut stmt) = conn.prepare(
            "SELECT a.anilist_id,
                    COALESCE(m.title_english, m.title_romaji, m.title_native, a.title),
                    COALESCE(m.cover_url, a.cover_url),
                    m.cover_path,
                    a.episodes_watched,
                    a.active_seconds,
                    (SELECT COUNT(DISTINCT c.word_id) FROM immersion_contexts c WHERE c.anilist_id = a.anilist_id) AS word_cnt
             FROM immersion_anilist_shows a
             LEFT JOIN media m ON m.anilist_id = a.anilist_id
             WHERE a.active_seconds > 0",
        ) {
            if let Ok(rows) = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6).unwrap_or(0),
                ))
            }) {
                for (aid, title, cover_url, cover_path, eps_w, act_sec, word_cnt) in rows.flatten() {
                    anilist_immersion_shows_count += 1;
                    let already_counted_sec = session_sec_by_aid.get(&aid).copied().unwrap_or(0);
                    let extra_sec = (act_sec - already_counted_sec).max(0);
                    total_immersion_sec += extra_sec;
                    total_episodes_completed += eps_w.max(0);

                    let key = format!("aid:{aid}");
                    if let Some(existing) = top_anime_map.get_mut(&key) {
                        existing.active_seconds = existing.active_seconds.max(act_sec);
                        existing.episodes_watched = existing.episodes_watched.max(eps_w);
                        if existing.cover_url.is_none() {
                            existing.cover_url = cover_url;
                        }
                    } else {
                        top_anime_map.insert(
                            key,
                            ImmersionAnimeSummary {
                                anilist_id: aid,
                                title,
                                cover_url,
                                cover_path,
                                active_seconds: act_sec,
                                episodes_watched: eps_w,
                                words_looked_up: word_cnt,
                            },
                        );
                    }
                }
            }
        }
    }

    let mut top_anime: Vec<ImmersionAnimeSummary> = top_anime_map.into_values().collect();
    top_anime.sort_by(|a, b| b.active_seconds.cmp(&a.active_seconds));
    top_anime.truncate(16);

    // Recent sessions
    let mut recent_sessions = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT s.id, s.day_key, s.anilist_id, s.ep_key,
                COALESCE(m.title_english, m.title_romaji, m.title_native, NULLIF(s.source_label, ''), 'Immersion Session'),
                m.cover_url, m.cover_path, s.active_seconds, s.updated_at
         FROM immersion_sessions s
         LEFT JOIN media m ON m.anilist_id = s.anilist_id
         WHERE s.active_seconds > 0
         ORDER BY s.updated_at DESC
         LIMIT 15",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok(ImmersionSessionItem {
                id: r.get(0)?,
                day_key: r.get(1)?,
                anilist_id: r.get(2)?,
                ep_key: r.get(3)?,
                source_label: r.get(4)?,
                cover_url: r.get(5)?,
                cover_path: r.get(6)?,
                active_seconds: r.get(7)?,
                updated_at: r.get(8)?,
            })
        }) {
            for item in rows.flatten() {
                recent_sessions.push(item);
            }
        }
    }

    // Load all contexts grouped by word_id
    let mut contexts_by_word: std::collections::HashMap<i64, Vec<ImmersionContextItem>> =
        std::collections::HashMap::new();
    if let Ok(mut ctx_stmt) = conn.prepare(
        "SELECT c.id, c.word_id, c.anilist_id, c.ep_key,
                COALESCE(m.title_english, m.title_romaji, m.title_native),
                m.cover_url, m.cover_path,
                c.sentence, c.matched_surface, c.position_sec, c.created_at
         FROM immersion_contexts c
         LEFT JOIN media m ON m.anilist_id = c.anilist_id
         ORDER BY c.created_at DESC",
    ) {
        if let Ok(rows) = ctx_stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(1)?,
                ImmersionContextItem {
                    id: r.get(0)?,
                    anilist_id: r.get(2)?,
                    ep_key: r.get(3)?,
                    anime_title: r.get(4)?,
                    cover_url: r.get(5)?,
                    cover_path: r.get(6)?,
                    sentence: r.get(7)?,
                    matched_surface: r.get(8)?,
                    position_sec: r.get(9)?,
                    created_at: r.get(10)?,
                },
            ))
        }) {
            for (word_id, ctx) in rows.flatten() {
                contexts_by_word.entry(word_id).or_default().push(ctx);
            }
        }
    }

    // Load all words ordered by lookup_count DESC, last_seen_at DESC
    let mut words = Vec::new();
    let mut total_lookups = 0i64;
    let mut unique_words_count = 0i64;
    let mut learning_words_count = 0i64;
    let mut known_words_count = 0i64;
    let mut ignored_words_count = 0i64;

    if let Ok(mut w_stmt) = conn.prepare(
        "SELECT id, expression, reading, matched_forms, status, lookup_count,
                first_seen_at, last_seen_at, furigana_json, glossary_json,
                pitch_json, freq_json, pos_tags
         FROM immersion_words
         ORDER BY lookup_count DESC, last_seen_at DESC",
    ) {
        if let Ok(rows) = w_stmt.query_map([], |r| {
            let id: i64 = r.get(0)?;
            let expression: String = r.get(1)?;
            let reading: String = r.get(2)?;
            let forms_json: String = r.get(3)?;
            let status: String = r.get(4)?;
            let lookup_count: i64 = r.get(5)?;
            let first_seen_at: i64 = r.get(6)?;
            let last_seen_at: i64 = r.get(7)?;
            let furi_json: String = r.get(8)?;
            let gloss_json: String = r.get(9)?;
            let pitch_json: String = r.get(10)?;
            let freq_json: String = r.get(11)?;
            let pos_tags: String = r.get(12)?;

            let matched_forms: Vec<String> = serde_json::from_str(&forms_json).unwrap_or_default();
            let furigana: Vec<crate::dictionary::FuriganaSegment> =
                serde_json::from_str(&furi_json).unwrap_or_default();
            let glossaries: Vec<String> = serde_json::from_str(&gloss_json).unwrap_or_default();
            let pitches: Vec<crate::dictionary::PitchAccent> =
                serde_json::from_str(&pitch_json).unwrap_or_default();
            let frequencies: Vec<String> = serde_json::from_str(&freq_json).unwrap_or_default();

            Ok(ImmersionWordItem {
                id,
                expression,
                reading,
                matched_forms,
                status,
                lookup_count,
                first_seen_at,
                last_seen_at,
                furigana,
                glossaries,
                pitches,
                frequencies,
                pos_tags,
                contexts: Vec::new(),
            })
        }) {
            for mut item in rows.flatten() {
                match item.status.as_str() {
                    "known" => {
                        known_words_count += 1;
                        unique_words_count += 1;
                        total_lookups += item.lookup_count;
                    }
                    "ignored" => {
                        ignored_words_count += 1;
                    }
                    _ => {
                        learning_words_count += 1;
                        unique_words_count += 1;
                        total_lookups += item.lookup_count;
                    }
                }
                if let Some(ctxs) = contexts_by_word.remove(&item.id) {
                    item.contexts = ctxs;
                }
                words.push(item);
            }
        }
    }

    Ok(ImmersionOverview {
        today_immersion_sec,
        week_immersion_sec,
        month_immersion_sec,
        total_immersion_sec,
        daily_goal_minutes,
        current_streak_days,
        best_streak_days,
        total_episodes_completed,
        total_lookups,
        unique_words_count,
        learning_words_count,
        known_words_count,
        ignored_words_count,
        daily_activity,
        top_anime,
        recent_sessions,
        words,
        anilist_immersion_list,
        anilist_immersion_shows_count,
    })
}

fn entry_immersion_episodes_and_mins(e: &crate::providers::AlListEntry) -> (i64, i64, String, Option<String>) {
    let total_eps = e.media.as_ref().and_then(|m| m.episodes).unwrap_or(0);
    let eps = if e.status == "COMPLETED" && total_eps > 0 {
        e.progress.max(total_eps)
    } else {
        e.progress.max(0)
    };
    let dur_mins = e
        .media
        .as_ref()
        .and_then(|m| m.duration)
        .filter(|&d| d > 0 && d <= 240)
        .unwrap_or(24);
    let title = e
        .media
        .as_ref()
        .and_then(|m| {
            m.title
                .english
                .clone()
                .or_else(|| m.title.romaji.clone())
                .or_else(|| m.title.native.clone())
        })
        .unwrap_or_else(|| format!("Anime #{}", e.media_id));
    let cover = e
        .media
        .as_ref()
        .and_then(|m| m.cover_image.as_ref())
        .and_then(|c| c.extra_large.clone().or_else(|| c.large.clone()));
    (eps, dur_mins, title, cover)
}

fn apply_anilist_immersion_group_to_db(
    conn: &rusqlite::Connection,
    list_name: &str,
    entries: &[crate::providers::AlListEntry],
) -> anyhow::Result<()> {
    conn.execute_batch(db::SCHEMA_V8)?;
    conn.execute("DELETE FROM immersion_anilist_shows", [])?;
    let now_ts = db::now();
    for e in entries {
        let (eps, dur_mins, title, cover) = entry_immersion_episodes_and_mins(e);
        if eps <= 0 {
            continue;
        }
        let active_sec = eps * dur_mins * 60;
        conn.execute(
            "INSERT INTO immersion_anilist_shows
             (anilist_id, list_name, title, cover_url, episodes_watched, duration_mins, active_seconds, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(anilist_id) DO UPDATE SET
                list_name = excluded.list_name,
                title = excluded.title,
                cover_url = COALESCE(excluded.cover_url, immersion_anilist_shows.cover_url),
                episodes_watched = excluded.episodes_watched,
                duration_mins = excluded.duration_mins,
                active_seconds = excluded.active_seconds,
                updated_at = excluded.updated_at",
            rusqlite::params![
                e.media_id,
                list_name,
                title,
                cover,
                eps,
                dur_mins,
                active_sec,
                now_ts
            ],
        )?;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_anilist_immersion_lists(
    st: St<'_>,
) -> CmdResult<AnilistImmersionListsResponse> {
    let (token, user_id, selected_list, mut synced_show_count, mut synced_hours) = {
        let conn = st.db();
        let _ = conn.execute_batch(db::SCHEMA_V8);
        let token = db::get_setting(&conn, "anilist.token")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty());
        let user_id = db::get_setting(&conn, "anilist.user_id")
            .ok()
            .flatten()
            .and_then(|s| s.parse::<i64>().ok());
        let selected = db::get_setting(&conn, "pref.anilist_immersion_list")
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let (cnt, sec): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(active_seconds), 0) FROM immersion_anilist_shows",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));
        (token, user_id, selected, cnt, (sec as f64) / 3600.0)
    };

    let (Some(tok), Some(uid)) = (token, user_id) else {
        return Ok(AnilistImmersionListsResponse {
            selected_list,
            synced_show_count,
            synced_hours,
            lists: Vec::new(),
        });
    };

    let groups = st
        .providers
        .anilist_user_lists_grouped(&tok, uid)
        .await
        .map_err(err)?;

    // If a list is currently selected, refresh its cached entries automatically
    if let Some(ref sel_name) = selected_list {
        if let Some(g) = groups.iter().find(|g| g.name == *sel_name) {
            let conn = st.db();
            let _ = apply_anilist_immersion_group_to_db(&conn, sel_name, &g.entries);
            let (cnt, sec): (i64, i64) = conn
                .query_row(
                    "SELECT COUNT(*), COALESCE(SUM(active_seconds), 0) FROM immersion_anilist_shows",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap_or((0, 0));
            synced_show_count = cnt;
            synced_hours = (sec as f64) / 3600.0;
        }
    }

    let lists = groups
        .into_iter()
        .map(|g| {
            let mut total_episodes = 0i64;
            let mut total_sec = 0i64;
            for e in &g.entries {
                let (eps, mins, _, _) = entry_immersion_episodes_and_mins(e);
                total_episodes += eps;
                total_sec += eps * mins * 60;
            }
            AnilistListOption {
                name: g.name,
                is_custom_list: g.is_custom_list,
                entry_count: g.entries.len(),
                total_episodes,
                estimated_hours: ((total_sec as f64) / 360.0).round() / 10.0,
            }
        })
        .collect();

    Ok(AnilistImmersionListsResponse {
        selected_list,
        synced_show_count,
        synced_hours: (synced_hours * 10.0).round() / 10.0,
        lists,
    })
}

#[tauri::command]
pub async fn set_anilist_immersion_list(
    st: St<'_>,
    list_name: Option<String>,
) -> CmdResult<AnilistImmersionListsResponse> {
    let clean = list_name
        .unwrap_or_default()
        .trim()
        .to_string();

    {
        let conn = st.db();
        let _ = conn.execute_batch(db::SCHEMA_V8);
        db::set_setting(&conn, "pref.anilist_immersion_list", &clean).map_err(err)?;
        if clean.is_empty() {
            let _ = conn.execute("DELETE FROM immersion_anilist_shows", []);
        }
    }
    crate::backup::mark_dirty();

    get_anilist_immersion_lists(st).await
}
