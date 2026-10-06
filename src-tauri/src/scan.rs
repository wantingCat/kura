//! Library scanning pipeline: discover → sync DB → match → resolve → episodes → artwork.

use crate::db::{self, now};
use crate::matcher::{self, MatchHints};
use crate::parser::{self, Parsed, SpecialKind, SubfolderKind};
use crate::service::{self, AppState, FileHints};
use crate::store;
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub phase: String,
    pub current: usize,
    pub total: usize,
    pub message: String,
}

fn progress(app: &AppHandle, phase: &str, current: usize, total: usize, message: impl Into<String>) {
    let _ = app.emit(
        "scan-progress",
        ScanProgress { phase: phase.into(), current, total, message: message.into() },
    );
}

/// Start a scan in the background. If one is running, another pass is queued.
pub fn spawn_scan(app: AppHandle, st: Arc<AppState>, library_id: Option<i64>) {
    if st.scanning.swap(true, Ordering::SeqCst) {
        st.rescan_requested.store(true, Ordering::SeqCst);
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut lib = library_id;
        loop {
            st.rescan_requested.store(false, Ordering::SeqCst);
            if let Err(e) = run_scan(&app, &st, lib).await {
                eprintln!("[kura] scan failed: {e:#}");
                let _ = app.emit("scan-error", e.to_string());
            }
            if !st.rescan_requested.load(Ordering::SeqCst) {
                break;
            }
            lib = None;
        }
        st.scanning.store(false, Ordering::SeqCst);
        let _ = app.emit("scan-finished", ());
        let _ = app.emit("library-changed", ());
    });
}

struct Found {
    library_id: i64,
    root: PathBuf,
    path: PathBuf,
    size: i64,
    mtime: i64,
}

struct GroupInfo {
    key: String,
    folder_path: Option<String>,
    display_name: String,
    title_guess: String,
    title_full: String,
    year: Option<u32>,
    season_hint: Option<u32>,
    special_folder: bool,
}

fn compute_group(root: &Path, path: &Path, parsed: &Parsed) -> Option<GroupInfo> {
    let rel = path.strip_prefix(root).ok()?;
    let dirs: Vec<String> = rel
        .parent()
        .map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect())
        .unwrap_or_default();

    if dirs.iter().any(|d| parser::classify_subfolder(d) == Some(SubfolderKind::Extras)) {
        return None;
    }
    let special_folder = dirs.iter().any(|d| parser::classify_subfolder(d) == Some(SubfolderKind::Specials));
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let root_s = root.to_string_lossy();

    if dirs.is_empty() {
        let title = if parsed.title.is_empty() { stem.clone() } else { parsed.title.clone() };
        return Some(GroupInfo {
            key: format!("{root_s}|file:{}", matcher::normalize(&title)),
            folder_path: None,
            display_name: title.clone(),
            title_full: if parsed.title_full.is_empty() { title.clone() } else { parsed.title_full.clone() },
            title_guess: title,
            year: parsed.year,
            season_hint: parsed.season,
            special_folder,
        });
    }

    let series = &dirs[0];
    let pf = parser::parse_folder(series);
    let folder_title = if pf.title.is_empty() { series.clone() } else { pf.title.clone() };

    // A folder holding many unrelated shows ("Seasonal", "Downloads"...): group by file title instead.
    let use_file_title = dirs.len() == 1
        && parsed.title.chars().count() >= 5
        && !matcher::titles_related(&folder_title, &parsed.title);

    let sub_season = dirs[1..].iter().find_map(|d| parser::parse_folder(d).season);
    let season_hint = parsed.season.or(sub_season).or(pf.season);
    let folder_path = root.join(series).to_string_lossy().to_string();

    if use_file_title {
        Some(GroupInfo {
            key: format!("{root_s}|dir:{series}|{}", matcher::normalize(&parsed.title)),
            folder_path: Some(folder_path),
            display_name: parsed.title.clone(),
            title_guess: parsed.title.clone(),
            title_full: parsed.title_full.clone(),
            year: parsed.year,
            season_hint,
            special_folder,
        })
    } else {
        Some(GroupInfo {
            key: format!("{root_s}|dir:{series}"),
            folder_path: Some(folder_path),
            display_name: series.clone(),
            title_guess: folder_title,
            title_full: if pf.title_full.is_empty() { series.clone() } else { pf.title_full.clone() },
            year: pf.year.or(parsed.year),
            season_hint,
            special_folder,
        })
    }
}

fn discover(st: &AppState, library_id: Option<i64>) -> Result<(Vec<Found>, Vec<i64>)> {
    let folders: Vec<(i64, String)> = {
        let conn = st.db();
        let mut stmt = conn.prepare(
            "SELECT library_id, path FROM library_folders WHERE (?1 IS NULL OR library_id = ?1) ORDER BY id",
        )?;
        let rows = stmt.query_map([library_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let libraries: Vec<i64> = {
        let conn = st.db();
        let mut stmt = conn.prepare("SELECT id FROM libraries WHERE (?1 IS NULL OR id = ?1)")?;
        let rows = stmt.query_map([library_id], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
        rows
    };

    let mut found = Vec::new();
    for (lib, folder) in folders {
        let root = PathBuf::from(&folder);
        if !root.is_dir() {
            eprintln!("[kura] folder not accessible, skipping: {folder}");
            continue;
        }
        for entry in WalkDir::new(&root).follow_links(true).into_iter().filter_entry(|e| {
            !e.file_name().to_string_lossy().starts_with('.')
        }) {
            let Ok(entry) = entry else { continue };
            if !entry.file_type().is_file() || !parser::is_video_file(entry.path()) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            found.push(Found {
                library_id: lib,
                root: root.clone(),
                path: entry.path().to_path_buf(),
                size: meta.len() as i64,
                mtime,
            });
        }
    }
    Ok((found, libraries))
}

/// Insert new / changed files, remove deleted ones. Returns number of new or changed files.
fn sync_files(st: &AppState, found: &[Found], libraries: &[i64]) -> Result<usize> {
    let mut conn = st.db();
    let tx = conn.transaction()?;
    let mut changed = 0;

    let mut existing: HashMap<String, (i64, i64, i64)> = HashMap::new();
    {
        let mut stmt = tx.prepare("SELECT id, path, size, mtime, library_id FROM local_files")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?))
        })?;
        for row in rows {
            let (id, path, size, mtime, lib) = row?;
            if libraries.contains(&lib) {
                existing.insert(path, (id, size, mtime));
            }
        }
    }

    let mut seen: HashSet<String> = HashSet::new();
    for f in found {
        let path_s = f.path.to_string_lossy().to_string();
        if !seen.insert(path_s.clone()) {
            continue;
        }
        if let Some((_, size, mtime)) = existing.get(&path_s) {
            if *size == f.size && *mtime == f.mtime {
                continue;
            }
        }
        let file_name = f.path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let parsed = parser::parse_filename(&file_name);
        if parsed.extra {
            continue;
        }
        let Some(g) = compute_group(&f.root, &f.path, &parsed) else { continue };

        tx.execute(
            "INSERT INTO match_groups (library_id, group_key, folder_path, display_name, title_guess, title_full, year_hint, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(library_id, group_key) DO NOTHING",
            params![f.library_id, g.key, g.folder_path, g.display_name, g.title_guess, g.title_full, g.year, now()],
        )?;
        let group_id: i64 = tx.query_row(
            "SELECT id FROM match_groups WHERE library_id = ?1 AND group_key = ?2",
            params![f.library_id, g.key],
            |r| r.get(0),
        )?;
        let special_hint: Option<&str> = if g.special_folder { Some("special") } else { None };
        tx.execute(
            "INSERT INTO local_files (library_id, group_id, path, size, mtime, parsed, season_hint, special_hint, anilist_id, ep_key, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9)
             ON CONFLICT(path) DO UPDATE SET
                library_id = excluded.library_id, group_id = excluded.group_id, size = excluded.size,
                mtime = excluded.mtime, parsed = excluded.parsed, season_hint = excluded.season_hint,
                special_hint = excluded.special_hint, anilist_id = NULL, ep_key = NULL",
            params![
                f.library_id,
                group_id,
                path_s,
                f.size,
                f.mtime,
                serde_json::to_string(&parsed)?,
                g.season_hint,
                special_hint,
                now()
            ],
        )?;
        changed += 1;
    }

    // Deleted files.
    for (path, (id, ..)) in &existing {
        if !seen.contains(path) {
            tx.execute("DELETE FROM local_files WHERE id = ?1", [id])?;
            changed += 1;
        }
    }
    // Files whose library folder was removed.
    tx.execute(
        "DELETE FROM local_files WHERE NOT EXISTS (
            SELECT 1 FROM library_folders lf WHERE lf.library_id = local_files.library_id
            AND local_files.path LIKE lf.path || '%')",
        [],
    )?;
    tx.execute("DELETE FROM match_groups WHERE id NOT IN (SELECT DISTINCT group_id FROM local_files WHERE group_id IS NOT NULL)", [])?;
    tx.commit()?;
    Ok(changed)
}

struct PendingGroup {
    id: i64,
    title_guess: String,
    title_full: String,
    year: Option<u32>,
    hints: MatchHints,
}

fn pending_groups(st: &AppState, libraries: &[i64]) -> Result<Vec<PendingGroup>> {
    let conn = st.db();
    let mut stmt = conn.prepare(
        "SELECT id, library_id, title_guess, title_full, year_hint FROM match_groups WHERE anilist_id IS NULL AND manual = 0",
    )?;
    let rows: Vec<(i64, i64, String, String, Option<u32>)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut out = Vec::new();
    for (id, lib, title_guess, title_full, year) in rows {
        if !libraries.contains(&lib) {
            continue;
        }
        let mut fstmt = conn.prepare("SELECT parsed, special_hint FROM local_files WHERE group_id = ?1")?;
        let files: Vec<(Parsed, Option<String>)> = fstmt
            .query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?
            .filter_map(|r| r.ok())
            .filter_map(|(p, s)| serde_json::from_str::<Parsed>(&p).ok().map(|p| (p, s)))
            .collect();
        let movie = files.iter().any(|(p, _)| p.special == Some(SpecialKind::Movie))
            || (files.len() == 1 && files[0].0.episode.is_none() && files[0].0.special.is_none());
        let special = !files.is_empty()
            && files.iter().all(|(p, s)| s.is_some() || matches!(p.special, Some(SpecialKind::Ova | SpecialKind::Special | SpecialKind::Ona)));
        out.push(PendingGroup { id, title_guess, title_full, year, hints: MatchHints { year, movie, special } });
    }
    Ok(out)
}

async fn match_group(st: &AppState, g: &PendingGroup) -> Result<()> {
    let mut queries: Vec<String> = vec![g.title_guess.clone()];
    if !g.title_full.is_empty() && matcher::normalize(&g.title_full) != matcher::normalize(&g.title_guess) {
        queries.push(g.title_full.clone());
    }
    for sep in [" - ", ": ", " ~ "] {
        if let Some((head, _)) = g.title_guess.split_once(sep) {
            if head.trim().chars().count() >= 3 {
                queries.push(head.trim().to_string());
            }
        }
    }
    queries.retain(|q| !q.trim().is_empty());
    queries.dedup();

    let mut best: Option<(f64, crate::providers::AlMedia)> = None;
    let _ = g.year;
    for q in &queries {
        let cands = st.providers.anilist_search(q, 8).await?;
        let alt = if q == &g.title_guess { Some(g.title_full.as_str()) } else { Some(g.title_guess.as_str()) };
        if let Some((i, score)) = matcher::best_candidate(q, alt, &cands, g.hints) {
            if best.as_ref().map_or(true, |(b, _)| score > *b) {
                best = Some((score, cands[i].clone()));
            }
        }
        if best.as_ref().map_or(false, |(b, _)| *b >= matcher::REVIEW_THRESHOLD) {
            break;
        }
    }

    let conn = st.db();
    let short = matcher::normalize(&g.title_guess).chars().filter(|c| !c.is_whitespace()).count() <= 4;
    let threshold = if short { 0.97 } else { matcher::ACCEPT_THRESHOLD };
    match best {
        Some((score, m)) if score >= threshold => {
            store::upsert_media(&conn, &m)?;
            conn.execute(
                "UPDATE match_groups SET anilist_id = ?2, confidence = ?3, attempted = 1 WHERE id = ?1",
                params![g.id, m.id, score],
            )?;
        }
        other => {
            conn.execute(
                "UPDATE match_groups SET confidence = ?2, attempted = 1 WHERE id = ?1",
                params![g.id, other.map(|(s, _)| s)],
            )?;
        }
    }
    Ok(())
}

struct PendingFile {
    id: i64,
    base: i64,
    parsed: Parsed,
    season: Option<u32>,
    special_folder: bool,
}

fn pending_files(st: &AppState) -> Result<Vec<PendingFile>> {
    let conn = st.db();
    let mut stmt = conn.prepare(
        "SELECT f.id, g.anilist_id, f.parsed, f.season_hint, f.special_hint
         FROM local_files f JOIN match_groups g ON g.id = f.group_id
         WHERE f.anilist_id IS NULL AND g.anilist_id IS NOT NULL",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<u32>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .filter_map(|(id, base, p, season, sh)| {
            serde_json::from_str::<Parsed>(&p).ok().map(|parsed| PendingFile {
                id,
                base,
                parsed,
                season,
                special_folder: sh.is_some(),
            })
        })
        .collect();
    Ok(rows)
}

pub async fn run_scan(app: &AppHandle, st: &Arc<AppState>, library_id: Option<i64>) -> Result<()> {
    // 1. Discover
    progress(app, "discover", 0, 0, "Looking for video files…");
    let (found, libraries) = discover(st, library_id)?;
    progress(app, "discover", found.len(), found.len(), format!("Found {} video files", found.len()));

    // 2. Sync
    let changed = sync_files(st, &found, &libraries)?;
    if changed > 0 {
        let _ = app.emit("library-changed", ());
    }

    // 3. Match groups
    let groups = pending_groups(st, &libraries)?;
    let total = groups.len();
    for (i, g) in groups.iter().enumerate() {
        progress(app, "match", i, total, format!("Identifying “{}”", g.title_guess));
        if let Err(e) = match_group(st, g).await {
            eprintln!("[kura] matching '{}' failed: {e:#}", g.title_guess);
            progress(app, "match", i, total, format!("Couldn't reach AniList for “{}”", g.title_guess));
        }
    }

    // 4. Resolve files → episodes
    let files = pending_files(st)?;
    let total = files.len();
    for (i, f) in files.iter().enumerate() {
        if i % 5 == 0 {
            progress(app, "resolve", i, total, "Mapping files to episodes…");
        }
        let res = service::resolve_file(
            st,
            f.base,
            FileHints { season: f.season, special_folder: f.special_folder, parsed: &f.parsed },
        )
        .await;
        let conn = st.db();
        match res {
            Ok(Some((id, key))) => {
                conn.execute("UPDATE local_files SET anilist_id = ?2, ep_key = ?3 WHERE id = ?1", params![f.id, id, key])?;
            }
            Ok(None) => {
                conn.execute("UPDATE local_files SET anilist_id = ?2, ep_key = NULL WHERE id = ?1", params![f.id, f.base])?;
            }
            Err(e) => eprintln!("[kura] resolving file {} failed: {e:#}", f.id),
        }
    }
    let _ = app.emit("library-changed", ());

    // 5. Episodes + fresh metadata for every owned entry
    let owned: Vec<i64> = {
        let conn = st.db();
        let mut stmt = conn.prepare("SELECT DISTINCT anilist_id FROM local_files WHERE anilist_id IS NOT NULL")?;
        let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let total = owned.len();
    for (i, id) in owned.iter().enumerate() {
        progress(app, "metadata", i, total, "Fetching episode info…");
        if let Err(e) = service::ensure_media(st, *id).await {
            eprintln!("[kura] media {id}: {e:#}");
        }
        let _ = service::ensure_episodes(st, *id).await;
    }
    let _ = app.emit("library-changed", ());

    // 6. Artwork
    download_artwork(app, st, &owned).await?;
    progress(app, "done", 1, 1, "Library up to date");
    Ok(())
}

struct ImageJob {
    kind: &'static str,
    id: i64,
    key: Option<String>,
    url: String,
    dest: PathBuf,
}

fn ext_from_url(url: &str) -> &str {
    let path = url.split('?').next().unwrap_or(url);
    match path.rsplit('.').next().map(|e| e.to_ascii_lowercase()) {
        Some(e) if e == "png" => "png",
        Some(e) if e == "webp" => "webp",
        Some(e) if e == "gif" => "gif",
        _ => "jpg",
    }
}

async fn download_artwork(app: &AppHandle, st: &Arc<AppState>, owned: &[i64]) -> Result<()> {
    let img_dir = st.data_dir.join("images");
    let mut jobs = Vec::new();
    {
        let conn = st.db();
        for id in owned {
            let row: Option<(Option<String>, Option<String>, Option<String>, Option<String>)> = conn
                .query_row(
                    "SELECT cover_url, cover_path, banner_url, banner_path FROM media WHERE anilist_id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let Some((cu, cp, bu, bp)) = row else { continue };
            let missing = |p: &Option<String>| p.as_ref().map_or(true, |p| !Path::new(p).exists());
            if let (Some(u), true) = (cu, missing(&cp)) {
                let dest = img_dir.join("covers").join(format!("{id}.{}", ext_from_url(&u)));
                jobs.push(ImageJob { kind: "cover", id: *id, key: None, url: u, dest });
            }
            if let (Some(u), true) = (bu, missing(&bp)) {
                let dest = img_dir.join("banners").join(format!("{id}.{}", ext_from_url(&u)));
                jobs.push(ImageJob { kind: "banner", id: *id, key: None, url: u, dest });
            }
            let mut stmt = conn.prepare(
                "SELECT ep_key, thumb_url, thumb_path FROM episodes WHERE anilist_id = ?1 AND thumb_url IS NOT NULL",
            )?;
            let rows = stmt.query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?))
            })?;
            for row in rows.flatten() {
                let (key, url, path) = row;
                if missing(&path) {
                    let dest = img_dir.join("thumbs").join(id.to_string()).join(format!("{key}.{}", ext_from_url(&url)));
                    jobs.push(ImageJob { kind: "thumb", id: *id, key: Some(key), url, dest });
                }
            }
        }
    }

    let total = jobs.len();
    if total == 0 {
        return Ok(());
    }
    let mut set = tokio::task::JoinSet::new();
    let mut iter = jobs.into_iter();
    let mut done = 0usize;
    let mut since_emit = 0usize;
    loop {
        while set.len() < 6 {
            let Some(job) = iter.next() else { break };
            let http = st.providers.http.clone();
            set.spawn(async move {
                let res: Result<()> = async {
                    let bytes = http.get(&job.url).send().await?.error_for_status()?.bytes().await?;
                    if let Some(parent) = job.dest.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&job.dest, &bytes)?;
                    Ok(())
                }
                .await;
                (job, res)
            });
        }
        let Some(joined) = set.join_next().await else { break };
        done += 1;
        since_emit += 1;
        let Ok((job, res)) = joined else { continue };
        match res {
            Ok(()) => {
                let path = job.dest.to_string_lossy().to_string();
                let conn = st.db();
                match job.kind {
                    "cover" => conn.execute("UPDATE media SET cover_path = ?2 WHERE anilist_id = ?1", params![job.id, path])?,
                    "banner" => conn.execute("UPDATE media SET banner_path = ?2 WHERE anilist_id = ?1", params![job.id, path])?,
                    _ => conn.execute(
                        "UPDATE episodes SET thumb_path = ?3 WHERE anilist_id = ?1 AND ep_key = ?2",
                        params![job.id, job.key, path],
                    )?,
                };
            }
            Err(e) => eprintln!("[kura] image {} failed: {e}", job.url),
        }
        if done % 10 == 0 || done == total {
            progress(app, "artwork", done, total, "Downloading artwork…");
        }
        if since_emit >= 40 {
            since_emit = 0;
            let _ = app.emit("library-changed", ());
        }
    }
    let _ = db::now();
    Ok(())
}
