//! Tauri commands exposed to the frontend.

use crate::db::{self, now};
use crate::matcher;
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
    s.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
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
pub fn list_libraries(st: St) -> CmdResult<Vec<Library>> {
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
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let mut out = Vec::new();
    for (id, name, kind, file_count, media_count) in libs {
        let mut fs = conn.prepare("SELECT id, path FROM library_folders WHERE library_id = ?1 ORDER BY id").map_err(err)?;
        let folders = fs
            .query_map([id], |r| {
                let path: String = r.get(1)?;
                Ok(Folder { id: r.get(0)?, exists: std::path::Path::new(&path).is_dir(), path })
            })
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        out.push(Library { id, name, kind, folders, file_count, media_count });
    }
    Ok(out)
}

#[tauri::command]
pub fn create_library(st: St, name: String, folders: Vec<String>) -> CmdResult<i64> {
    let conn = st.db();
    conn.execute("INSERT INTO libraries (name, kind, created_at) VALUES (?1, 'anime', ?2)", params![name.trim(), now()])
        .map_err(err)?;
    let id = conn.last_insert_rowid();
    for f in folders {
        conn.execute("INSERT OR IGNORE INTO library_folders (library_id, path) VALUES (?1, ?2)", params![id, f])
            .map_err(err)?;
    }
    Ok(id)
}

#[tauri::command]
pub fn rename_library(st: St, id: i64, name: String) -> CmdResult<()> {
    st.db().execute("UPDATE libraries SET name = ?2 WHERE id = ?1", params![id, name.trim()]).map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn delete_library(app: AppHandle, st: St, id: i64) -> CmdResult<()> {
    st.db().execute("DELETE FROM libraries WHERE id = ?1", [id]).map_err(err)?;
    let _ = app.emit("library-changed", ());
    Ok(())
}

#[tauri::command]
pub fn add_folder(st: St, library_id: i64, path: String) -> CmdResult<()> {
    st.db()
        .execute("INSERT OR IGNORE INTO library_folders (library_id, path) VALUES (?1, ?2)", params![library_id, path])
        .map_err(err)?;
    Ok(())
}

#[tauri::command]
pub fn remove_folder(app: AppHandle, st: St, folder_id: i64) -> CmdResult<()> {
    let conn = st.db();
    let row: Option<(i64, String)> = conn
        .query_row("SELECT library_id, path FROM library_folders WHERE id = ?1", [folder_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(err)?;
    if let Some((lib, path)) = row {
        conn.execute("DELETE FROM library_folders WHERE id = ?1", [folder_id]).map_err(err)?;
        conn.execute(
            "DELETE FROM local_files WHERE library_id = ?1 AND substr(path, 1, length(?2)) = ?2",
            params![lib, path],
        )
        .map_err(err)?;
        conn.execute("DELETE FROM match_groups WHERE id NOT IN (SELECT DISTINCT group_id FROM local_files WHERE group_id IS NOT NULL)", [])
            .map_err(err)?;
    }
    let _ = app.emit("library-changed", ());
    Ok(())
}

#[tauri::command]
pub fn start_scan(app: AppHandle, st: St, library_id: Option<i64>) -> CmdResult<()> {
    scan::spawn_scan(app, st.inner().clone(), library_id);
    Ok(())
}

#[tauri::command]
pub fn is_scanning(st: St) -> bool {
    st.scanning.load(std::sync::atomic::Ordering::SeqCst)
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
    average_score: Option<i64>,
    cover_url: Option<String>,
    cover_path: Option<String>,
    cover_color: Option<String>,
    banner_url: Option<String>,
    banner_path: Option<String>,
    description: Option<String>,
    owned_count: i64,
    watched_count: i64,
    added_at: i64,
    last_watched_at: Option<i64>,
    needs_review: bool,
    library_ids: Vec<i64>,
}

#[tauri::command]
pub fn get_library(st: St, library_id: Option<i64>) -> CmdResult<Vec<MediaCard>> {
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
                    (SELECT COUNT(*) FROM watch_state w WHERE w.anilist_id = m.anilist_id AND w.ep_key NOT LIKE 'S%') AS watched,
                    (SELECT MAX(watched_at) FROM watch_state w WHERE w.anilist_id = m.anilist_id) AS last_watched
             FROM media m
             JOIN local_files f ON f.anilist_id = m.anilist_id
             LEFT JOIN match_groups g ON g.id = f.group_id
             WHERE (?1 IS NULL OR f.library_id = ?1)
             GROUP BY m.anilist_id",
        )
        .map_err(err)?;
    let rows = stmt
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
                library_ids: libs.unwrap_or_default().split(',').filter_map(|s| s.parse().ok()).collect(),
                watched_count: r.get(22)?,
                last_watched_at: r.get(23)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
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
pub fn get_unmatched(st: St) -> CmdResult<Vec<UnmatchedGroup>> {
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
        let mut s = conn.prepare("SELECT path FROM local_files WHERE group_id = ?1 ORDER BY path LIMIT 3").map_err(err)?;
        g.sample_files = s
            .query_map([g.id], |r| r.get::<_, String>(0))
            .map_err(err)?
            .filter_map(|r| r.ok())
            .map(|p| std::path::Path::new(&p).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or(p))
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
    season_year: Option<i64>,
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
    average_score: Option<i64>,
    next_airing_episode: Option<i64>,
    next_airing_at: Option<i64>,
    episodes_source: Option<String>,
    episode_list: Vec<EpisodeRow>,
    relations: Vec<RelationCard>,
    other_files: Vec<FileRef>,
    groups: Vec<GroupRef>,
}

fn file_ref(id: i64, path: String, size: i64) -> FileRef {
    let file_name = std::path::Path::new(&path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
    FileRef { id, path, file_name, size }
}

#[tauri::command]
pub fn get_media_detail(st: St, anilist_id: i64) -> CmdResult<MediaDetail> {
    let conn = st.db();
    let mut d = conn
        .query_row(
            "SELECT anilist_id, id_mal, title_romaji, title_english, title_native, synonyms, format, status, episodes,
                    duration, season, season_year, start_date, end_date, description, genres, tags, studios, cover_url,
                    cover_path, cover_color, banner_url, banner_path, average_score, next_airing_episode, next_airing_at,
                    episodes_source
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
                    episode_list: Vec::new(),
                    relations: Vec::new(),
                    other_files: Vec::new(),
                    groups: Vec::new(),
                })
            },
        )
        .map_err(err)?;

    // Files for this entry
    let mut files_by_key: HashMap<String, Vec<FileRef>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, path, size, ep_key FROM local_files WHERE anilist_id = ?1 ORDER BY path")
            .map_err(err)?;
        let rows = stmt
            .query_map([anilist_id], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?))
            })
            .map_err(err)?;
        for row in rows.flatten() {
            let (id, path, size, key) = row;
            match key {
                Some(k) => files_by_key.entry(k).or_default().push(file_ref(id, path, size)),
                None => d.other_files.push(file_ref(id, path, size)),
            }
        }
    }

    let mut watched: HashMap<String, i64> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT ep_key, watched_at FROM watch_state WHERE anilist_id = ?1").map_err(err)?;
        for row in stmt.query_map([anilist_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).map_err(err)?.flatten() {
            watched.insert(row.0, row.1);
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
                })
            })
            .map_err(err)?;
        for row in rows.flatten() {
            d.episode_list.push(row);
        }
    }

    // Files mapped to keys that have no metadata row (e.g. episode 13 of a "12 episode" show).
    let known: std::collections::HashSet<String> = d.episode_list.iter().map(|e| e.ep_key.clone()).collect();
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
            });
        }
    }
    for e in &mut d.episode_list {
        e.files = files_by_key.remove(&e.ep_key).unwrap_or_default();
        e.watched_at = watched.get(&e.ep_key).copied();
    }
    // Specials are only interesting if owned or described.
    d.episode_list.retain(|e| !e.is_special || !e.files.is_empty() || e.title_en.is_some() || e.title_ja.is_some());
    d.episode_list.sort_by(|a, b| a.is_special.cmp(&b.is_special).then(a.number.cmp(&b.number)));

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
        let mut rels: Vec<RelationCard> = rows.flatten().filter(|r| r.relation_type != "CHARACTER").collect();
        rels.sort_by(|a, b| order(&a.relation_type).cmp(&order(&b.relation_type)).then(a.season_year.cmp(&b.season_year)));
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
    Ok(d)
}

// ---------------------------------------------------------------------------
// Watch state
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn set_watched(app: AppHandle, st: St, anilist_id: i64, ep_keys: Vec<String>, watched: bool) -> CmdResult<()> {
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
            tx.execute("DELETE FROM watch_state WHERE anilist_id = ?1 AND ep_key = ?2", params![anilist_id, key])
                .map_err(err)?;
        }
    }
    tx.commit().map_err(err)?;
    let _ = app.emit("library-changed", ());
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
    let m = st.providers.anilist_media(new_anilist_id).await.map_err(err)?;
    {
        let conn = st.db();
        store::upsert_media(&conn, &m).map_err(err)?;
        let groups: Vec<i64> = if let Some(g) = group_id {
            vec![g]
        } else if let Some(from) = from_anilist_id {
            let mut stmt = conn
                .prepare("SELECT DISTINCT group_id FROM local_files WHERE anilist_id = ?1 AND group_id IS NOT NULL")
                .map_err(err)?;
            let rows = stmt.query_map([from], |r| r.get(0)).map_err(err)?.flatten().collect();
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
            conn.execute("UPDATE local_files SET anilist_id = NULL, ep_key = NULL WHERE group_id = ?1", [g]).map_err(err)?;
        }
    }
    let _ = app.emit("library-changed", ());
    scan::spawn_scan(app, st.inner().clone(), None);
    Ok(())
}

/// Re-fetch metadata + episodes for one entry.
#[tauri::command]
pub async fn refresh_media(app: AppHandle, st: St<'_>, anilist_id: i64) -> CmdResult<()> {
    let m = st.providers.anilist_media(anilist_id).await.map_err(err)?;
    store::upsert_media(&st.db(), &m).map_err(err)?;
    let lite = db::media_lite(&st.db(), anilist_id).map_err(err)?.ok_or("missing media")?;
    crate::service::fetch_episodes(&st, &lite).await.map_err(err)?;
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
        .query_row("SELECT id FROM local_files WHERE path = ?1", [path], |r| r.get(0))
        .optional()
        .map_err(err)?;
    ok.map(|_| ()).ok_or_else(|| "unknown file".to_string())
}

#[tauri::command]
pub fn open_file(st: St, path: String) -> CmdResult<()> {
    known_file(&st, &path)?;
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(err)
}

#[tauri::command]
pub fn reveal_file(st: St, path: String) -> CmdResult<()> {
    known_file(&st, &path)?;
    tauri_plugin_opener::reveal_item_in_dir(&path).map_err(err)
}
