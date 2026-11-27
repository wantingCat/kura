//! Subtitle management for Kura v0.5:
//! 1. **Smart Subtitle Matcher** (Rust port of Jellyfin Anime Subtitle Renamer v3 + Kura `ani.zip` absolute mapping).
//! 2. **Automatic Subtitle Retiming** (Alass-style dynamic programming alignment for `.srt`, `.ass`/`.ssa`, `.vtt`
//!    against embedded `.mkv` Matroska EBML subtitle tracks, sibling reference subtitles, or `ffmpeg`).
//! 3. **Jimaku Subtitle Fetcher** (`https://jimaku.cc/api` lookup by `anilist_id`, `.zip` extraction, auto-retime, and live `mpv` IPC injection).

use crate::db::{self, now};
use crate::service::AppState;
use anyhow::{anyhow, bail, Context, Result};
use flate2::read::DeflateDecoder;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

pub const SUB_EXTENSIONS: &[&str] = &["ass", "ssa", "srt", "vtt", "sub"];

// ---------------------------------------------------------------------------
// Data models exposed to Tauri commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedSubtitle {
    pub id: i64,
    pub anilist_id: i64,
    pub ep_key: String,
    pub path: String,
    pub file_name: String,
    pub lang: String,
    pub cc: bool,
    pub source: String,
    pub retimed: bool,
    pub offset_ms: Option<i64>,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleMatchItem {
    pub source_path: String,
    pub source_name: String,
    pub ep_key: String,
    pub episode_number: i64,
    pub video_path: String,
    pub video_name: String,
    pub target_path: String,
    pub target_name: String,
    pub lang: String,
    pub cc: bool,
    pub resolution_note: Option<String>,
    pub already_linked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleSkipItem {
    pub source_path: String,
    pub source_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleMatchPreview {
    pub anilist_id: i64,
    pub scanned_folders: Vec<String>,
    pub matches: Vec<SubtitleMatchItem>,
    pub skipped: Vec<SubtitleSkipItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleApplyReport {
    pub linked: usize,
    pub renamed: usize,
    pub retimed: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetimeReport {
    pub subtitle_path: String,
    pub output_path: String,
    pub offset_ms: i64,
    pub segments: usize,
    pub matched_cues: usize,
    pub total_cues: usize,
    pub reference_source: String,
    pub loaded_into_mpv: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JimakuEntry {
    pub id: i64,
    pub name: String,
    pub english_name: Option<String>,
    pub japanese_name: Option<String>,
    pub anilist_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JimakuFileItem {
    pub name: String,
    pub url: String,
    pub size: i64,
    pub last_modified: Option<String>,
    pub is_archive: bool,
    pub is_rar: bool,
    pub detected_episode: Option<i64>,
    pub match_score: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JimakuSearchResponse {
    pub configured: bool,
    pub entry: Option<JimakuEntry>,
    pub files: Vec<JimakuFileItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JimakuDownloadResult {
    pub saved_paths: Vec<String>,
    pub linked_count: usize,
    pub retime_report: Option<RetimeReport>,
    pub loaded_into_mpv: bool,
}

// ---------------------------------------------------------------------------
// Database helpers for linked_subtitles
// ---------------------------------------------------------------------------

pub fn list_linked_subtitles(conn: &Connection, anilist_id: i64) -> Result<Vec<LinkedSubtitle>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, anilist_id, ep_key, path, lang, cc, source, retimed, offset_ms
         FROM linked_subtitles
         WHERE anilist_id = ?1
         ORDER BY CAST(ep_key AS INTEGER), id",
    )?;
    let raw_rows: Vec<(i64, i64, String, String, String, bool, String, bool, Option<i64>)> = stmt
        .query_map([anilist_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)? != 0,
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)? != 0,
                r.get::<_, Option<i64>>(8)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut out = Vec::with_capacity(raw_rows.len());
    for (id, anilist_id, ep_key, path, lang, cc, source, retimed, offset_ms) in raw_rows {
        let p = Path::new(&path);
        let file_name = p
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        let exists = p.is_file();
        out.push(LinkedSubtitle {
            id,
            anilist_id,
            ep_key,
            path,
            file_name,
            lang,
            cc,
            source,
            retimed,
            offset_ms,
            exists,
        });
    }
    Ok(out)
}

pub fn episode_subtitle_paths(conn: &Connection, anilist_id: i64, ep_key: &str) -> Vec<String> {
    episode_subtitle_paths_for_pref(conn, anilist_id, ep_key, None)
}

pub fn episode_subtitle_paths_for_pref(
    conn: &Connection,
    anilist_id: i64,
    ep_key: &str,
    sub_pref: Option<&str>,
) -> Vec<String> {
    if let Some(pref) = sub_pref.map(|s| s.trim().to_ascii_lowercase()) {
        if pref == "off" || pref == "none" {
            return Vec::new();
        }
    }
    let mut stmt = match conn.prepare_cached(
        "SELECT path, lang FROM linked_subtitles
         WHERE anilist_id = ?1 AND ep_key = ?2
         ORDER BY retimed DESC, id DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows: Vec<(String, String)> = match stmt.query_map(params![anilist_id, ep_key], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    }) {
        Ok(r) => r.flatten().collect(),
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for (path, lang) in rows {
        if let Some(pref) = sub_pref {
            let p_trim = pref.trim();
            if !p_trim.is_empty()
                && !p_trim.eq_ignore_ascii_case("any")
                && !p_trim.eq_ignore_ascii_case("default")
                && !crate::player::lang_matches_code(&lang, Some(&path), p_trim)
            {
                continue;
            }
        }
        if Path::new(&path).is_file() {
            out.push(path);
        }
    }
    out
}

pub fn has_saved_subtitle(conn: &Connection, anilist_id: i64, ep_key: &str) -> bool {
    !episode_subtitle_paths(conn, anilist_id, ep_key).is_empty()
}

/// Delete subtitle files and DB rows for episodes that have been marked as watched (`watch_state.watched_at IS NOT NULL`).
/// Called when the application closes (and on startup in case of a prior force-close) so unwatched episodes never
/// re-download subtitles while watched episodes don't leave subtitle files behind.
pub fn cleanup_watched_subtitles(conn: &Connection, data_dir: &Path) -> usize {
    let rows: Vec<(i64, i64, String, String, String)> = {
        let Ok(mut stmt) = conn.prepare(
            "SELECT ls.id, ls.anilist_id, ls.ep_key, ls.path, ls.source
             FROM linked_subtitles ls
             INNER JOIN watch_state ws
               ON ws.anilist_id = ls.anilist_id AND ws.ep_key = ls.ep_key
             WHERE ws.watched_at IS NOT NULL",
        ) else {
            return 0;
        };
        let Ok(mapped) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        }) else {
            return 0;
        };
        mapped.flatten().collect()
    };

    if rows.is_empty() {
        return 0;
    }

    let subs_root = data_dir.join("subtitles");
    let stream_cache_root = data_dir.join("stream_cache");
    let mut cleaned = 0usize;
    let mut touched_shows = HashSet::new();

    for (id, anilist_id, ep_key, path, source) in rows {
        let p = Path::new(&path);
        // Remove file from disk if it lives in Kura's data_dir (`subtitles/` or `stream_cache/`) or was downloaded from Jimaku
        if p.starts_with(&subs_root) || p.starts_with(&stream_cache_root) || source == "jimaku" {
            let _ = std::fs::remove_file(p);
            let show_sub_dir = subs_root.join(anilist_id.to_string());
            for ext in SUB_EXTENSIONS {
                let orig = show_sub_dir.join(format!("ep{ep_key}.orig.{ext}"));
                let _ = std::fs::remove_file(orig);
            }
            touched_shows.insert(anilist_id);
        }
        if conn
            .execute("DELETE FROM linked_subtitles WHERE id = ?1", [id])
            .is_ok()
        {
            cleaned += 1;
        }
    }

    // Remove empty per-show subtitle cache directories
    for anilist_id in touched_shows {
        let show_sub_dir = subs_root.join(anilist_id.to_string());
        let _ = std::fs::remove_dir(&show_sub_dir);
    }

    cleaned
}

pub fn upsert_linked_subtitle(
    conn: &Connection,
    anilist_id: i64,
    ep_key: &str,
    path: &str,
    lang: &str,
    cc: bool,
    source: &str,
    retimed: bool,
    offset_ms: Option<i64>,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO linked_subtitles (anilist_id, ep_key, path, lang, cc, source, retimed, offset_ms, content, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9)
         ON CONFLICT(anilist_id, ep_key, path) DO UPDATE SET
            lang = excluded.lang,
            cc = excluded.cc,
            source = excluded.source,
            retimed = excluded.retimed,
            offset_ms = COALESCE(excluded.offset_ms, linked_subtitles.offset_ms),
            content = NULL,
            created_at = excluded.created_at",
        params![
            anilist_id,
            ep_key,
            path,
            lang,
            cc as i64,
            source,
            retimed as i64,
            offset_ms,
            now()
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn remove_linked_subtitle(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM linked_subtitles WHERE id = ?1", [id])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 1. Smart Subtitle Matcher (Port of Jellyfin Anime Subtitle Renamer v3)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSubtitleFile {
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub lang: Option<String>,
    pub cc: bool,
    pub ext: String,
}

pub fn is_subtitle_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUB_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Detect CC / SDH strictly in delimiter/tag positions (`[cc]`, `(cc)`, `.cc.`, `_cc`, `[sdh]`, `.sdh.`),
/// never inside normal words like "success" or "acclaim".
pub fn detect_cc_tag(stem: &str) -> bool {
    static RE_CC: OnceLock<Regex> = OnceLock::new();
    let re = RE_CC
        .get_or_init(|| Regex::new(r"(?i)(?:[\[\(\._\-\s](?:cc|sdh)(?:[\]\)\._\-\s]|$))").unwrap());
    re.is_match(stem)
}

/// Detect subtitle language code from filename tags/tokens (`ja`, `en`, `zh`, `ko`, `de`, `fr`, `es`).
pub fn detect_sub_language(stem: &str) -> Option<String> {
    if stem.contains("日本語") || stem.contains("日文") {
        return Some("ja".to_string());
    }
    if stem.contains("简体")
        || stem.contains("繁體")
        || stem.contains("繁体")
        || stem.contains("中字")
    {
        return Some("zh".to_string());
    }
    static RE_TOKEN: OnceLock<Regex> = OnceLock::new();
    let re = RE_TOKEN.get_or_init(|| Regex::new(r"[\[\(\]\)\._\-\s]+").unwrap());

    // Inspect tokens from right to left (language tags almost always sit near the end of the stem).
    let tokens: Vec<String> = re
        .split(stem)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect();

    for tok in tokens.iter().rev() {
        let clean = tok.trim_end_matches("cc").trim_end_matches("sdh");
        let check = if clean.is_empty() {
            tok.as_str()
        } else {
            clean
        };
        match check {
            "ja" | "jpn" | "jp" | "jap" | "japanese" => return Some("ja".to_string()),
            "en" | "eng" | "english" => return Some("en".to_string()),
            "zh" | "chs" | "cht" | "chi" | "zho" | "sc" | "tc" | "zh-hans" | "zh-hant" => {
                return Some("zh".to_string())
            }
            "ko" | "kor" | "korean" => return Some("ko".to_string()),
            "de" | "ger" | "deu" | "german" => return Some("de".to_string()),
            "fr" | "fre" | "fra" | "french" => return Some("fr".to_string()),
            "es" | "spa" | "spanish" => return Some("es".to_string()),
            _ => {}
        }
    }
    None
}

/// Extract `(season, episode)` from any anime subtitle filename:
/// - `S01E01`, `S02E25` (including absolute `S02E25` which resolves via Kura's episode map)
/// - `1x08`
/// - `EP08`, `E08`, `Ep. 08`
/// - `#08` (e.g. `Azumanga Daioh #08`)
/// - `第08話`, `08話`
/// - `Show - 08`, `Show – 12 END` (guarded so `- 1080p` is never parsed as ep 1080)
/// - `[08]`
/// - bare `25.srt` / `25.ja.srt` / `01.ja[cc].ass`
pub fn extract_sub_episode(stem: &str) -> (Option<i64>, Option<i64>) {
    static RE_SXXEXX: OnceLock<Regex> = OnceLock::new();
    static RE_NXN: OnceLock<Regex> = OnceLock::new();
    static RE_EP: OnceLock<Regex> = OnceLock::new();
    static RE_HASH: OnceLock<Regex> = OnceLock::new();
    static RE_KANJI: OnceLock<Regex> = OnceLock::new();
    static RE_DASH: OnceLock<Regex> = OnceLock::new();
    static RE_BRACKET: OnceLock<Regex> = OnceLock::new();
    static RE_BARE: OnceLock<Regex> = OnceLock::new();

    // 1. S01E01 / S02E25
    let re_sxxexx = RE_SXXEXX.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[\s._\-\[\(])S(\d{1,2})[\s._\-]*E(\d{1,4})(?:v\d)?(?:\b|[\s._\-\]\)]|$)",
        )
        .unwrap()
    });
    if let Some(c) = re_sxxexx.captures(stem) {
        let s = c[1].parse::<i64>().ok();
        let e = c[2].parse::<i64>().ok();
        return (s, e);
    }

    // 2. 1x08
    let re_nxn = RE_NXN.get_or_init(|| Regex::new(r"(?i)\b(\d{1,2})x(\d{1,3})(?:v\d)?\b").unwrap());
    if let Some(c) = re_nxn.captures(stem) {
        let s = c[1].parse::<i64>().ok();
        let e = c[2].parse::<i64>().ok();
        return (s, e);
    }

    // 3. E08 / EP08 / Episode 08
    let re_ep = RE_EP.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[\s._\-\[\(])(?:episode|ep|e)\.?\s*0*(\d{1,4})(?:v\d)?(?:\b|[\s._\-\]\)]|$)",
        )
        .unwrap()
    });
    if let Some(c) = re_ep.captures(stem) {
        if let Ok(ep) = c[1].parse::<i64>() {
            return (None, Some(ep));
        }
    }

    // 4. #08 (e.g. Azumanga Daioh #08)
    let re_hash = RE_HASH
        .get_or_init(|| Regex::new(r"#\s*0*(\d{1,4})(?:v\d)?(?:\b|[\s._\-\]\)]|$)").unwrap());
    if let Some(c) = re_hash.captures(stem) {
        if let Ok(ep) = c[1].parse::<i64>() {
            return (None, Some(ep));
        }
    }

    // 5. 第08話 / 08話 / 第08集
    let re_kanji = RE_KANJI.get_or_init(|| Regex::new(r"第?\s*0*(\d{1,4})\s*[話话集화]").unwrap());
    if let Some(c) = re_kanji.captures(stem) {
        if let Ok(ep) = c[1].parse::<i64>() {
            return (None, Some(ep));
        }
    }

    // 6. Dash pattern: "Show - 01", "Show - 12 END"
    // Must not read "- 1080p" or "- 720p" or "- 264" as episode numbers.
    let re_dash = RE_DASH.get_or_init(|| {
        Regex::new(r"[\s._][\-–—][\s._]*0*(\d{1,4})(?:v\d)?(?:\s*(?:END|FIN))?(?:[\s._\-\[\(]|$)")
            .unwrap()
    });
    for c in re_dash.captures_iter(stem) {
        let m = c.get(1).unwrap();
        let after = &stem[m.end()..];
        let next_char = after.chars().next().map(|ch| ch.to_ascii_lowercase());
        if matches!(next_char, Some('p' | 'i' | 'k' | 'b' | 'f')) {
            continue;
        }
        if let Ok(ep) = m.as_str().parse::<i64>() {
            if !matches!(ep, 264 | 265 | 480 | 576 | 720 | 1080 | 1440 | 2160)
                && !(1970..=2099).contains(&ep)
            {
                return (None, Some(ep));
            }
        }
    }

    // 7. Bracketed number: [08]
    let re_bracket =
        RE_BRACKET.get_or_init(|| Regex::new(r"\[\s*0*(\d{1,4})(?:v\d)?\s*\]").unwrap());
    for c in re_bracket.captures_iter(stem) {
        if let Ok(ep) = c[1].parse::<i64>() {
            if !matches!(ep, 264 | 265 | 480 | 576 | 720 | 1080 | 1440 | 2160)
                && !(1970..=2099).contains(&ep)
            {
                return (None, Some(ep));
            }
        }
    }

    // 8. Bare number filename: "25", "25.ja", "01.ja[cc]"
    let re_bare = RE_BARE.get_or_init(|| {
        Regex::new(r"^\s*0*(\d{1,4})(?:v\d)?(?:[\.\s_\-\[\(][a-zA-Z\[\]\(\)\._\-]*)?$").unwrap()
    });
    if let Some(c) = re_bare.captures(stem) {
        if let Ok(ep) = c[1].parse::<i64>() {
            return (None, Some(ep));
        }
    }

    (None, None)
}

pub fn parse_subtitle_filename(file_name: &str) -> Option<ParsedSubtitleFile> {
    let p = Path::new(file_name);
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if !SUB_EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }
    let stem = p.file_stem()?.to_str()?;
    let (season, episode) = extract_sub_episode(stem);
    let cc = detect_cc_tag(stem);
    let lang = detect_sub_language(stem);
    Some(ParsedSubtitleFile {
        season,
        episode,
        lang,
        cc,
        ext,
    })
}

#[derive(Debug, Clone)]
pub struct KnownEpisodeTarget {
    pub ep_key: String,
    pub number: i64,
    pub tvdb_season: Option<i64>,
    pub tvdb_episode: Option<i64>,
    pub absolute_number: Option<i64>,
    pub video_path: String,
}

/// Resolve a parsed subtitle `(season, episode)` against the series' known episodes.
/// Handles:
/// 1. Direct match (`number == ep`)
/// 2. `ani.zip` `absolute_number == ep` (e.g. `S02E25` or `25.ja.srt` where Ep 1 has `absolute_number = 25`)
/// 3. `tvdb_season == season && tvdb_episode == ep`
/// 4. Franchise prequel offset fallback (`ep - prequel_total_episodes`)
pub fn resolve_episode_target<'a>(
    parsed_season: Option<i64>,
    parsed_ep: i64,
    targets: &'a [KnownEpisodeTarget],
    prequel_offset: i64,
) -> Result<(&'a KnownEpisodeTarget, Option<String>), String> {
    // 1. Check exact match on `absolute_number` first if `parsed_ep` is beyond max regular episode number
    let max_reg = targets.iter().map(|t| t.number).max().unwrap_or(0);

    if parsed_ep <= max_reg {
        let direct: Vec<&KnownEpisodeTarget> =
            targets.iter().filter(|t| t.number == parsed_ep).collect();
        if direct.len() == 1 {
            return Ok((direct[0], None));
        } else if direct.len() > 1 {
            return Err(format!("Ambiguous episode {parsed_ep}"));
        }
    }

    // 2. Match via ani.zip `absolute_number`
    let by_abs: Vec<&KnownEpisodeTarget> = targets
        .iter()
        .filter(|t| t.absolute_number == Some(parsed_ep))
        .collect();
    if by_abs.len() == 1 {
        let t = by_abs[0];
        let note = if t.number != parsed_ep {
            Some(format!("Absolute #{parsed_ep} → Ep {}", t.number))
        } else {
            None
        };
        return Ok((t, note));
    }

    // 3. Match via TVDB season/episode
    if let Some(s) = parsed_season {
        let by_tvdb: Vec<&KnownEpisodeTarget> = targets
            .iter()
            .filter(|t| t.tvdb_season == Some(s) && t.tvdb_episode == Some(parsed_ep))
            .collect();
        if by_tvdb.len() == 1 {
            let t = by_tvdb[0];
            let note = Some(format!("S{s:02}E{parsed_ep:02} → Ep {}", t.number));
            return Ok((t, note));
        }
    }

    // 4. Match via prequel episode offset (e.g. Season 1 had 24 episodes, so S02E25 -> Ep 1)
    if prequel_offset > 0 && parsed_ep > prequel_offset {
        let rel = parsed_ep - prequel_offset;
        let by_rel: Vec<&KnownEpisodeTarget> = targets.iter().filter(|t| t.number == rel).collect();
        if by_rel.len() == 1 {
            let t = by_rel[0];
            return Ok((
                t,
                Some(format!(
                    "Absolute #{parsed_ep} (−{prequel_offset}) → Ep {}",
                    t.number
                )),
            ));
        }
    }

    if prequel_offset > 0 && parsed_ep <= prequel_offset {
        return Err(format!(
            "Absolute episode #{parsed_ep} belongs to an earlier season (starts after #{prequel_offset})"
        ));
    }

    Err(format!("No owned episode matches #{parsed_ep}"))
}

fn load_series_targets(
    conn: &Connection,
    anilist_id: i64,
) -> Result<(Vec<KnownEpisodeTarget>, i64, Vec<PathBuf>)> {
    let mut files_by_ep: BTreeMap<String, String> = BTreeMap::new();
    let mut folders: HashSet<PathBuf> = HashSet::new();
    {
        let mut stmt = conn.prepare(
            "SELECT ep_key, path FROM local_files
             WHERE anilist_id = ?1 AND ep_key IS NOT NULL AND ep_key NOT LIKE 'S%'
             ORDER BY path",
        )?;
        let rows = stmt.query_map([anilist_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows.flatten() {
            let (ep_key, path) = row;
            if let Some(parent) = Path::new(&path).parent() {
                folders.insert(parent.to_path_buf());
                // Also check Common "Subs" / "Subtitles" child directories
                for sub_dir in ["Subs", "Subtitles", "subs", "subtitles"] {
                    let cand = parent.join(sub_dir);
                    if cand.is_dir() {
                        folders.insert(cand);
                    }
                }
            }
            files_by_ep.entry(ep_key).or_insert(path);
        }
    }

    let mut ep_meta: HashMap<String, (i64, Option<i64>, Option<i64>, Option<i64>)> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT ep_key, number, tvdb_season, tvdb_episode, absolute_number
             FROM episodes WHERE anilist_id = ?1 AND is_special = 0",
        )?;
        let rows = stmt.query_map([anilist_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
            ))
        })?;
        for row in rows.flatten() {
            ep_meta.insert(row.0, (row.1, row.2, row.3, row.4));
        }
    }

    let mut targets = Vec::new();
    for (ep_key, video_path) in files_by_ep {
        let number = ep_key.parse::<i64>().unwrap_or(0);
        if number <= 0 {
            continue;
        }
        let meta = ep_meta.get(&ep_key).copied();
        targets.push(KnownEpisodeTarget {
            ep_key,
            number,
            tvdb_season: meta.and_then(|m| m.1),
            tvdb_episode: meta.and_then(|m| m.2),
            absolute_number: meta.and_then(|m| m.3),
            video_path,
        });
    }
    targets.sort_by_key(|t| t.number);

    // Compute prequel offset from either minimum absolute_number or PREQUEL relation episode count
    let mut prequel_offset = targets
        .iter()
        .filter_map(|t| t.absolute_number.map(|abs| abs - t.number))
        .find(|&off| off > 0)
        .unwrap_or(0);

    if prequel_offset == 0 {
        // Walk PREQUEL chain in media_relations / media to sum prior seasons' episode counts
        let mut cur = anilist_id;
        let mut visited = HashSet::new();
        visited.insert(cur);
        while let Some((pre_id, pre_eps)) = conn
            .query_row(
                "SELECT r.related_id, COALESCE(m.episodes, r.episodes, 0)
                 FROM media_relations r
                 LEFT JOIN media m ON m.anilist_id = r.related_id
                 WHERE r.anilist_id = ?1 AND r.relation_type = 'PREQUEL'
                   AND COALESCE(r.format, '') IN ('TV', 'TV_SHORT', 'ONA')
                 LIMIT 1",
                [cur],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
        {
            if !visited.insert(pre_id) {
                break;
            }
            if pre_eps > 0 {
                prequel_offset += pre_eps;
            }
            cur = pre_id;
        }
    }

    let mut folder_list: Vec<PathBuf> = folders.into_iter().collect();
    folder_list.sort();
    Ok((targets, prequel_offset, folder_list))
}

pub fn build_target_sub_path(video_path: &Path, lang: &str, cc: bool, ext: &str) -> PathBuf {
    let parent = video_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video");
    let cc_part = if cc { ".cc" } else { "" };
    let lang_part = if lang.trim().is_empty() {
        "ja"
    } else {
        lang.trim()
    };
    parent.join(format!("{stem}.{lang_part}{cc_part}.{ext}"))
}

pub fn preview_subtitle_matches(
    conn: &Connection,
    anilist_id: i64,
    extra_folder: Option<&Path>,
    default_lang: &str,
) -> Result<SubtitleMatchPreview> {
    let (targets, prequel_offset, mut folders) = load_series_targets(conn, anilist_id)?;
    if let Some(extra) = extra_folder {
        if extra.is_dir() && !folders.iter().any(|f| f == extra) {
            folders.insert(0, extra.to_path_buf());
        }
    }

    let existing_links: HashSet<String> = list_linked_subtitles(conn, anilist_id)?
        .into_iter()
        .map(|l| l.path.replace('\\', "/").to_lowercase())
        .collect();

    let mut sub_files: Vec<PathBuf> = Vec::new();
    for dir in &folders {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() && is_subtitle_file(&p) {
                    sub_files.push(p);
                }
            }
        }
    }
    sub_files.sort();
    sub_files.dedup();

    let fallback_lang = if default_lang.trim().is_empty() {
        "ja"
    } else {
        default_lang.trim()
    };

    let mut matches = Vec::new();
    let mut skipped = Vec::new();

    for sub_path in sub_files {
        let source_path = sub_path.to_string_lossy().to_string();
        let source_name = sub_path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| source_path.clone());

        let Some(parsed) = parse_subtitle_filename(&source_name) else {
            skipped.push(SubtitleSkipItem {
                source_path,
                source_name,
                reason: "Unsupported subtitle format".into(),
            });
            continue;
        };

        let Some(ep_num) = parsed.episode else {
            skipped.push(SubtitleSkipItem {
                source_path,
                source_name,
                reason: "No episode number found in filename".into(),
            });
            continue;
        };

        let lang = parsed.lang.unwrap_or_else(|| fallback_lang.to_string());
        match resolve_episode_target(parsed.season, ep_num, &targets, prequel_offset) {
            Ok((target, resolution_note)) => {
                let vpath = Path::new(&target.video_path);
                let target_pb = build_target_sub_path(vpath, &lang, parsed.cc, &parsed.ext);
                let target_path = target_pb.to_string_lossy().to_string();
                let target_name = target_pb
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| target_path.clone());
                let video_name = vpath
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| target.video_path.clone());
                let already_linked = existing_links
                    .contains(&source_path.replace('\\', "/").to_lowercase())
                    || existing_links.contains(&target_path.replace('\\', "/").to_lowercase());

                matches.push(SubtitleMatchItem {
                    source_path,
                    source_name,
                    ep_key: target.ep_key.clone(),
                    episode_number: target.number,
                    video_path: target.video_path.clone(),
                    video_name,
                    target_path,
                    target_name,
                    lang,
                    cc: parsed.cc,
                    resolution_note,
                    already_linked,
                });
            }
            Err(reason) => {
                skipped.push(SubtitleSkipItem {
                    source_path,
                    source_name,
                    reason,
                });
            }
        }
    }

    matches.sort_by(|a, b| {
        a.episode_number
            .cmp(&b.episode_number)
            .then(a.source_name.cmp(&b.source_name))
    });

    Ok(SubtitleMatchPreview {
        anilist_id,
        scanned_folders: folders
            .into_iter()
            .map(|f| f.to_string_lossy().to_string())
            .collect(),
        matches,
        skipped,
    })
}

pub fn apply_subtitle_matches(
    st: &AppState,
    anilist_id: i64,
    mode: &str,
    items: Vec<SubtitleMatchItem>,
    auto_retime: bool,
) -> Result<SubtitleApplyReport> {
    let mut report = SubtitleApplyReport {
        linked: 0,
        renamed: 0,
        retimed: 0,
        errors: Vec::new(),
    };

    let rename_mode = mode.eq_ignore_ascii_case("rename");
    let cache_dir = st.data_dir.join("subtitles").join(anilist_id.to_string());
    let _ = std::fs::create_dir_all(&cache_dir);

    // Build canonical allowlist of destination directories belonging to `anilist_id` (`V-H03`).
    let mut allowed_dst_dirs: Vec<PathBuf> = Vec::new();
    if let Ok(canon_cache) = std::fs::canonicalize(&cache_dir) {
        allowed_dst_dirs.push(canon_cache);
    }
    {
        let conn = st.db_read();
        if let Ok(mut stmt) = conn.prepare("SELECT path FROM local_files WHERE anilist_id = ?1") {
            if let Ok(rows) = stmt.query_map([anilist_id], |r| r.get::<_, String>(0)) {
                for p_str in rows.flatten() {
                    if let Some(parent) = Path::new(&p_str).parent() {
                        if let Ok(canon) = std::fs::canonicalize(parent) {
                            if !allowed_dst_dirs.contains(&canon) {
                                allowed_dst_dirs.push(canon);
                            }
                        }
                    }
                }
            }
        }
        if let Ok(mut gstmt) = conn.prepare(
            "SELECT folder_path FROM match_groups WHERE anilist_id = ?1 AND folder_path IS NOT NULL",
        ) {
            if let Ok(rows) = gstmt.query_map([anilist_id], |r| r.get::<_, String>(0)) {
                for folder in rows.flatten() {
                    if let Ok(canon) = std::fs::canonicalize(&folder) {
                        if !allowed_dst_dirs.contains(&canon) {
                            allowed_dst_dirs.push(canon);
                        }
                    }
                }
            }
        };
    }

    for item in items {
        let src = Path::new(&item.source_path);
        let dst = Path::new(&item.target_path);

        if !is_subtitle_file(src) || !is_subtitle_file(dst) {
            report.errors.push(format!(
                "Rejected non-subtitle path for {}",
                item.source_name
            ));
            continue;
        }

        // Verify `src` exists as a regular subtitle file (never a symlink to a non-subtitle file).
        let Ok(canon_src) = std::fs::canonicalize(src) else {
            report
                .errors
                .push(format!("Missing file: {}", item.source_name));
            continue;
        };
        let Ok(src_meta) = std::fs::metadata(&canon_src) else {
            report
                .errors
                .push(format!("Missing file: {}", item.source_name));
            continue;
        };
        if !src_meta.is_file() || !is_subtitle_file(&canon_src) {
            report.errors.push(format!(
                "Invalid subtitle source file: {}",
                item.source_name
            ));
            continue;
        }

        // Verify `dst` has a normal filename and its canonical parent belongs to `anilist_id`.
        let has_valid_dst_name = dst
            .components()
            .next_back()
            .map_or(false, |c| matches!(c, std::path::Component::Normal(_)));
        let dst_parent_allowed = dst
            .parent()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .map_or(false, |canon_parent| {
                allowed_dst_dirs
                    .iter()
                    .any(|allowed| canon_parent.starts_with(allowed))
            });
        if !has_valid_dst_name || !dst_parent_allowed {
            report.errors.push(format!(
                "Rejected destination path outside series folder for {}",
                item.source_name
            ));
            continue;
        }

        let mut did_copy_or_rename_to_dst = false;
        let mut active_sub_path = if rename_mode {
            let same_file = std::fs::canonicalize(dst)
                .map(|cd| cd == canon_src)
                .unwrap_or(src == dst);
            if !same_file {
                if dst.exists() || std::fs::symlink_metadata(dst).is_ok() {
                    // Never overwrite an existing subtitle file; link it instead.
                    dst.to_path_buf()
                } else {
                    match std::fs::rename(&canon_src, dst)
                        .or_else(|_| std::fs::copy(&canon_src, dst).map(|_| ()))
                    {
                        Ok(()) => {
                            did_copy_or_rename_to_dst = true;
                            report.renamed += 1;
                            dst.to_path_buf()
                        }
                        Err(e) => {
                            report
                                .errors
                                .push(format!("Couldn't rename {}: {e}", item.source_name));
                            continue;
                        }
                    }
                }
            } else {
                dst.to_path_buf()
            }
        } else {
            canon_src
        };

        let mut was_retimed = false;
        let mut offset_ms = None;

        let vpath = Path::new(&item.video_path);
        let video_allowed = crate::parser::is_video_file(vpath)
            && vpath.is_file()
            && vpath
                .parent()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .map_or(false, |cp| {
                    allowed_dst_dirs.iter().any(|allowed| cp.starts_with(allowed))
                });

        if auto_retime && video_allowed {
            let safe_ep_key: String = item
                .ep_key
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
                .collect();
            let safe_ep_key = if safe_ep_key.is_empty() {
                "0".to_string()
            } else {
                safe_ep_key
            };
            // Only overwrite `dst` in-place when this call actually created `dst`; otherwise write to `cache_dir`.
            let out_dest = if rename_mode && did_copy_or_rename_to_dst {
                active_sub_path.clone()
            } else {
                let fname = active_sub_path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("ep_{safe_ep_key}.srt"));
                cache_dir.join(format!("ep{safe_ep_key}_{fname}"))
            };
            if let Ok(rt) = retime_subtitle_file(&active_sub_path, vpath, &out_dest) {
                active_sub_path = PathBuf::from(&rt.output_path);
                was_retimed = true;
                offset_ms = Some(rt.offset_ms);
                report.retimed += 1;
            }
        }

        let conn = st.db();
        if let Err(e) = upsert_linked_subtitle(
            &conn,
            anilist_id,
            &item.ep_key,
            &active_sub_path.to_string_lossy(),
            &item.lang,
            item.cc,
            "matcher",
            was_retimed,
            offset_ms,
        ) {
            report.errors.push(e.to_string());
        } else {
            report.linked += 1;
        }
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// 2. Automatic Subtitle Retiming (Alass-style Dynamic Programming Alignment)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl Span {
    pub fn len(&self) -> i64 {
        (self.end_ms - self.start_ms).max(0)
    }
}

/// Parse SRT/VTT timestamp `HH:MM:SS,mmm` or `HH:MM:SS.mmm` or `MM:SS.mmm` into milliseconds.
pub fn parse_srt_vtt_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    let (h, m, sec_ms) = match parts.len() {
        3 => (
            parts[0].parse::<i64>().ok()?,
            parts[1].parse::<i64>().ok()?,
            parts[2],
        ),
        2 => (0, parts[0].parse::<i64>().ok()?, parts[1]),
        _ => return None,
    };
    let (sec_str, ms_str) = sec_ms.split_once([',', '.'])?;
    let sec = sec_str.parse::<i64>().ok()?;
    let digit_len = ms_str.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digit_len == 0 {
        return None;
    }
    let digits = &ms_str[..digit_len.min(3)];
    let ms_norm = match digits.len() {
        1 => digits.parse::<i64>().ok()? * 100,
        2 => digits.parse::<i64>().ok()? * 10,
        _ => digits.parse::<i64>().ok()?,
    };
    Some(h * 3_600_000 + m * 60_000 + sec * 1_000 + ms_norm)
}

pub fn format_srt_time(ms: i64) -> String {
    let ms = ms.max(0);
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1_000;
    let rem = ms % 1_000;
    format!("{h:02}:{m:02}:{s:02},{rem:03}")
}

pub fn format_vtt_time(ms: i64) -> String {
    let ms = ms.max(0);
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1_000;
    let rem = ms % 1_000;
    format!("{h:02}:{m:02}:{s:02}.{rem:03}")
}

/// Parse ASS/SSA timestamp `H:MM:SS.cc` into milliseconds.
pub fn parse_ass_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h = parts[0].parse::<i64>().ok()?;
    let m = parts[1].parse::<i64>().ok()?;
    let (sec_str, cs_str) = parts[2].split_once('.')?;
    let sec = sec_str.parse::<i64>().ok()?;
    let digit_len = cs_str.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digit_len == 0 {
        return None;
    }
    let digits = &cs_str[..digit_len.min(3)];
    let cs_ms = match digits.len() {
        1 => digits.parse::<i64>().ok()? * 100,
        2 => digits.parse::<i64>().ok()? * 10,
        _ => digits.parse::<i64>().ok()?,
    };
    Some(h * 3_600_000 + m * 60_000 + sec * 1_000 + cs_ms)
}

pub fn format_ass_time(ms: i64) -> String {
    let ms = ms.max(0);
    let h = ms / 3_600_000;
    let m = (ms % 3_600_000) / 60_000;
    let s = (ms % 60_000) / 1_000;
    let cs = (ms % 1_000 + 5) / 10;
    let (s, m, h, cs) = if cs >= 100 {
        let total_s = ms / 1_000 + 1;
        (total_s % 60, (total_s / 60) % 60, total_s / 3600, 0)
    } else {
        (s, m, h, cs)
    };
    format!("{h}:{m:02}:{s:02}.{cs:02}")
}

fn is_distributor_bumper_cue(start_ms: i64, text: &str) -> bool {
    if start_ms > 20_000 {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    const BUMPER_TAGS: &[&str] = &[
        "viz media",
        "crunchyroll",
        "funimation",
        "hidive",
        "sentai",
        "netflix",
        "aniplex",
        "bilibili",
        "muse ",
        "ani-one",
        "presented by",
        "subtitles by",
        "translated by",
        "encoded by",
        "synced by",
        "rips by",
    ];
    BUMPER_TAGS.iter().any(|tag| lower.contains(tag))
}

/// Extract timing spans from `.srt`, `.vtt`, or `.ass`/`.ssa` content via the shared cue parser.
pub fn parse_subtitle_spans(content: &str, ext: &str) -> Vec<Span> {
    let is_ass = matches!(ext.to_ascii_lowercase().as_str(), "ass" | "ssa");
    let mut cues = parse_subtitle_cues_inner(content, ext, true);
    if cues.is_empty() && is_ass {
        cues = parse_subtitle_cues_inner(content, ext, false);
    }

    let mut spans: Vec<Span> = cues
        .into_iter()
        .filter(|c| {
            c.end_ms > c.start_ms
                && (c.end_ms - c.start_ms) <= 15_000
                && !is_distributor_bumper_cue(c.start_ms, &c.text)
        })
        .map(|c| Span {
            start_ms: c.start_ms,
            end_ms: c.end_ms,
        })
        .collect();

    spans.sort_by_key(|s| s.start_ms);
    let mut filtered: Vec<Span> = Vec::with_capacity(spans.len());
    for sp in spans {
        if let Some(last) = filtered.last() {
            if (sp.start_ms - last.start_ms).abs() < 40 && (sp.end_ms - last.end_ms).abs() < 40 {
                continue;
            }
        }
        filtered.push(sp);
    }
    filtered
}

/// Apply a piecewise time mapping `map_fn(ms) -> new_ms` to `.srt`, `.vtt`, or `.ass`/`.ssa` content.
pub fn rewrite_subtitle_timestamps<F>(content: &str, ext: &str, map_fn: F) -> String
where
    F: Fn(i64) -> i64,
{
    let ext_lower = ext.to_ascii_lowercase();
    let is_ass = matches!(ext_lower.as_str(), "ass" | "ssa");
    let is_vtt = ext_lower == "vtt";
    let line_ending = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };

    let mut out_lines = Vec::new();
    for line in content.lines() {
        if is_ass {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("Dialogue:") {
                let fields: Vec<&str> = rest.splitn(10, ',').collect();
                if fields.len() == 10 {
                    if let (Some(s), Some(e)) =
                        (parse_ass_time(fields[1]), parse_ass_time(fields[2]))
                    {
                        let dur = (e - s).max(0);
                        let new_s = map_fn(s).max(0);
                        let new_e = map_fn(e).max(new_s + dur.min(100));
                        let rebuilt = format!(
                            "Dialogue:{},{},{},{},{},{},{},{},{},{}",
                            fields[0],
                            format_ass_time(new_s),
                            format_ass_time(new_e),
                            fields[3],
                            fields[4],
                            fields[5],
                            fields[6],
                            fields[7],
                            fields[8],
                            fields[9]
                        );
                        out_lines.push(rebuilt);
                        continue;
                    }
                }
            }
            out_lines.push(line.to_string());
        } else if let Some((left, right)) = line.split_once("-->") {
            let mut right_parts = right.trim().splitn(2, char::is_whitespace);
            let right_time = right_parts.next().unwrap_or("");
            let right_extra = right_parts.next().unwrap_or("");
            if let (Some(s), Some(e)) = (parse_srt_vtt_time(left), parse_srt_vtt_time(right_time)) {
                let dur = (e - s).max(0);
                let new_s = map_fn(s).max(0);
                let new_e = map_fn(e).max(new_s + dur.min(100));
                let (fs, fe) = if is_vtt {
                    (format_vtt_time(new_s), format_vtt_time(new_e))
                } else {
                    (format_srt_time(new_s), format_srt_time(new_e))
                };
                if right_extra.is_empty() {
                    out_lines.push(format!("{fs} --> {fe}"));
                } else {
                    out_lines.push(format!("{fs} --> {fe} {right_extra}"));
                }
                continue;
            }
            out_lines.push(line.to_string());
        } else {
            out_lines.push(line.to_string());
        }
    }
    out_lines.join(line_ending)
}

/// Piecewise alignment result from Alass-style dynamic programming.
#[derive(Debug, Clone)]
pub struct AlignmentModel {
    /// Sorted list of `(split_start_target_ms, offset_ms)` segments.
    /// `new_time = old_time + offset_ms` for the active segment.
    pub segments: Vec<(i64, i64)>,
    pub primary_offset_ms: i64,
    pub matched_cues: usize,
}

impl AlignmentModel {
    pub fn apply(&self, t_ms: i64) -> i64 {
        let mut offset = self.primary_offset_ms;
        for &(seg_start, seg_off) in &self.segments {
            if t_ms >= seg_start {
                offset = seg_off;
            } else {
                break;
            }
        }
        (t_ms + offset).max(0)
    }
}

/// Compute overlap score between `Span(s + offset, e + offset)` and sorted reference spans `ref_spans`.
fn span_overlap_score(sp: Span, offset_ms: i64, ref_spans: &[Span]) -> i64 {
    let s = sp.start_ms + offset_ms;
    let e = sp.end_ms + offset_ms;
    if e < 0 {
        // Penalize offsets that push leading dialogue lines into negative timestamps (< 0:00.000)
        return -400;
    }
    if let Some(first_ref) = ref_spans.first() {
        if e < first_ref.start_ms - 1_000 {
            // Penalize offsets that strand leading dialogue lines inside a silent pre-dialogue intro bumper
            return -300;
        }
    }
    // Binary search for first reference span whose end_ms > s - 500
    let idx = ref_spans.partition_point(|r| r.end_ms < s - 500);
    let mut best = 0;
    for r in &ref_spans[idx..] {
        if r.start_ms > e + 500 {
            break;
        }
        let ov_s = s.max(r.start_ms);
        let ov_e = e.min(r.end_ms);
        let overlap = (ov_e - ov_s).max(0);
        // Also reward close start boundary alignment (within 250ms)
        let start_diff = (s - r.start_ms).abs();
        let start_bonus = (250 - start_diff).max(0) * 2;
        let score = overlap + start_bonus;
        if score > best {
            best = score;
        }
    }
    best
}

/// Align target subtitle spans against reference subtitle spans using:
/// 1. Coverage validation so partial/sparse torrent files or short sign tracks are rejected.
/// 2. Pairwise start-time difference histogram within ±30s to discover candidate constant offsets.
/// 3. Global 10ms refinement with strict confidence validation against `0ms` so clean subtitles are never warped.
pub fn align_subtitle_spans(target: &[Span], reference: &[Span]) -> Option<AlignmentModel> {
    if target.len() < 2 || reference.len() < 2 {
        return None;
    }

    let target_span_ms = target
        .last()
        .map(|s| s.end_ms)
        .unwrap_or(0)
        .saturating_sub(target.first().map(|s| s.start_ms).unwrap_or(0));
    let ref_span_ms = reference
        .last()
        .map(|s| s.end_ms)
        .unwrap_or(0)
        .saturating_sub(reference.first().map(|s| s.start_ms).unwrap_or(0));

    // Reject incomplete/sparse reference tracks when aligning a full-episode target (25+ lines)
    if target.len() >= 25 && (target_span_ms <= 0 || ref_span_ms < (target_span_ms * 55) / 100) {
        return None;
    }

    // Step 1: Build a histogram of start-time differences `r.start_ms - t.start_ms` in 50ms buckets
    // within [-30_000ms, +30_000ms] (covers distributor intro bumpers, sponsor cards, and prerolls).
    const MAX_SHIFT_MS: i64 = 30_000;
    const BUCKET_MS: i64 = 50;
    let mut hist: HashMap<i64, i64> = HashMap::new();

    for t in target {
        let lo = t.start_ms - MAX_SHIFT_MS;
        let hi = t.start_ms + MAX_SHIFT_MS;
        let start_idx = reference.partition_point(|r| r.start_ms < lo);
        for r in &reference[start_idx..] {
            if r.start_ms > hi {
                break;
            }
            let dur_diff = (t.len() - r.len()).abs();
            if dur_diff > 2_500 {
                continue;
            }
            let diff = r.start_ms - t.start_ms;
            let b = (diff
                + if diff >= 0 {
                    BUCKET_MS / 2
                } else {
                    -BUCKET_MS / 2
                })
                / BUCKET_MS;
            let weight = (1_500 - dur_diff / 2).max(200);
            for neighbor in [b - 1, b, b + 1] {
                let factor = if neighbor == b { 2 } else { 1 };
                *hist.entry(neighbor).or_insert(0) += weight * factor;
            }
        }
    }

    if hist.is_empty() {
        return None;
    }

    // Pick top 8 distinct candidate offsets (at least 350ms apart)
    let mut ranked: Vec<(i64, i64)> = hist.into_iter().map(|(b, w)| (b * BUCKET_MS, w)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    let mut candidates: Vec<i64> = Vec::new();
    for (off, _) in ranked {
        if candidates.iter().all(|&c| (c - off).abs() >= 350) {
            candidates.push(off);
            if candidates.len() >= 8 {
                break;
            }
        }
    }
    if !candidates.iter().any(|&c| c.abs() < 200) {
        candidates.push(0);
    }

    // Refine each candidate offset to 10ms precision globally
    let mut best_global_off = 0i64;
    let mut best_global_score = -1i64;
    for cand in &mut candidates {
        let base = *cand;
        let mut best_off = base;
        let mut best_score = -1i64;
        for delta in (-150..=150).step_by(10) {
            let off = base + delta;
            let sc: i64 = target
                .iter()
                .map(|&sp| span_overlap_score(sp, off, reference))
                .sum();
            if sc > best_score {
                best_score = sc;
                best_off = off;
            }
        }
        *cand = best_off;
        if best_score > best_global_score
            || (best_score == best_global_score && best_off.abs() < best_global_off.abs())
        {
            best_global_score = best_score;
            best_global_off = best_off;
        }
    }

    let zero_score: i64 = target
        .iter()
        .map(|&sp| span_overlap_score(sp, 0, reference))
        .sum();

    // Require non-zero offset to decisively beat 0ms by at least 18% (on full-length tracks)
    if target.len() >= 20
        && best_global_off.abs() >= 250
        && zero_score > 0
        && best_global_score < (zero_score * 118) / 100
    {
        best_global_off = 0;
    }

    // Step 2: Alass-style Viterbi / dynamic programming piecewise commercial-break segmentation
    // across the refined candidate offsets (`SPLIT_PENALTY` prevents false splits on jitter while
    // cleanly splitting at mid-episode sponsor / eyecatch breaks!).
    let segments: Vec<(i64, i64)> = if target.len() >= 20 && candidates.len() >= 2 {
        const SPLIT_PENALTY: i64 = 8_500;
        let n = target.len();
        let k_len = candidates.len();
        let mut dp = vec![i64::MIN / 4; k_len];
        let mut back = vec![vec![0usize; k_len]; n];

        for (k, &off) in candidates.iter().enumerate() {
            let prior = if off == best_global_off { 0 } else { -200 };
            dp[k] = span_overlap_score(target[0], off, reference) + prior;
        }

        for i in 1..n {
            let mut next_dp = vec![i64::MIN / 4; k_len];
            for (k, &off) in candidates.iter().enumerate() {
                let mut best_prev = k;
                let mut best_val = dp[k];
                for prev_k in 0..k_len {
                    if prev_k == k {
                        continue;
                    }
                    let switched = dp[prev_k] - SPLIT_PENALTY;
                    if switched > best_val {
                        best_val = switched;
                        best_prev = prev_k;
                    }
                }
                back[i][k] = best_prev;
                next_dp[k] = best_val + span_overlap_score(target[i], off, reference);
            }
            dp = next_dp;
        }

        let mut cur_k = (0..k_len).max_by_key(|&k| dp[k]).unwrap_or(0);
        let mut chosen_k = vec![0usize; n];
        for i in (0..n).rev() {
            chosen_k[i] = cur_k;
            if i > 0 {
                cur_k = back[i][cur_k];
            }
        }

        let mut segs: Vec<(i64, i64)> = Vec::new();
        for (i, &k) in chosen_k.iter().enumerate() {
            let off = candidates[k];
            if segs.last().map(|&(_, prev_off)| prev_off != off).unwrap_or(true) {
                let seg_start = if i == 0 { 0 } else { target[i].start_ms };
                segs.push((seg_start, off));
            }
        }
        segs
    } else {
        vec![(0, best_global_off)]
    };

    let primary_offset_ms = segments
        .first()
        .map(|&(_, off)| off)
        .unwrap_or(best_global_off);
    let model = AlignmentModel {
        segments,
        primary_offset_ms,
        matched_cues: 0,
    };

    let matched_cues = target
        .iter()
        .filter(|&&sp| {
            let applied_off = model.apply(sp.start_ms) - sp.start_ms;
            span_overlap_score(sp, applied_off, reference) > 250
        })
        .count();

    if matched_cues < ((target.len() * 30) / 100).max(2) {
        return None;
    }

    Some(AlignmentModel {
        matched_cues,
        ..model
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LiveMkvAlignOutcome {
    Aligned(f64),
    NoEmbeddedSubTracks,
    NotReady,
}

#[derive(Debug)]
enum MkvExtractResult {
    Spans(Vec<Span>, String),
    NoSubTracks,
    InsufficientCues,
}

/// Non-destructively estimate a constant subtitle offset (in seconds) between external `target_cues`
/// and the embedded subtitle cues present in `video_path`, distinguishing 0-subtitle-track MKVs from
/// partially downloaded MKVs (`C-08`).
pub fn compute_live_mkv_cue_offset_outcome(
    target_cues: &[SubtitleCue],
    video_path: &Path,
) -> LiveMkvAlignOutcome {
    if target_cues.len() < 15 || !video_path.is_file() {
        return LiveMkvAlignOutcome::NotReady;
    }
    let ref_spans = match extract_mkv_subtitle_spans_classified(video_path) {
        Ok(MkvExtractResult::NoSubTracks) => return LiveMkvAlignOutcome::NoEmbeddedSubTracks,
        Ok(MkvExtractResult::InsufficientCues) | Err(_) => return LiveMkvAlignOutcome::NotReady,
        Ok(MkvExtractResult::Spans(spans, _)) => spans,
    };
    if ref_spans.len() < 12 {
        return LiveMkvAlignOutcome::NotReady;
    }
    let max_ref_ms = ref_spans.last().map(|s| s.end_ms).unwrap_or(0);
    if max_ref_ms < 45_000 {
        return LiveMkvAlignOutcome::NotReady;
    }

    let window_target: Vec<Span> = target_cues
        .iter()
        .filter(|c| {
            c.start_ms <= max_ref_ms + 25_000
                && c.end_ms > c.start_ms
                && !is_distributor_bumper_cue(c.start_ms, &c.text)
        })
        .map(|c| Span {
            start_ms: c.start_ms,
            end_ms: c.end_ms,
        })
        .collect();
    if window_target.len() < 12 {
        return LiveMkvAlignOutcome::NotReady;
    }

    const MAX_SHIFT_MS: i64 = 25_000;
    const BUCKET_MS: i64 = 50;
    let mut hist: HashMap<i64, i64> = HashMap::new();

    for t in &window_target {
        let lo = t.start_ms - MAX_SHIFT_MS;
        let hi = t.start_ms + MAX_SHIFT_MS;
        let start_idx = ref_spans.partition_point(|r| r.start_ms < lo);
        for r in &ref_spans[start_idx..] {
            if r.start_ms > hi {
                break;
            }
            let dur_diff = (t.len() - r.len()).abs();
            if dur_diff > 2_200 {
                continue;
            }
            let diff = r.start_ms - t.start_ms;
            let b = (diff
                + if diff >= 0 {
                    BUCKET_MS / 2
                } else {
                    -BUCKET_MS / 2
                })
                / BUCKET_MS;
            let weight = (1_500 - dur_diff / 2).max(200);
            for neighbor in [b - 1, b, b + 1] {
                let factor = if neighbor == b { 2 } else { 1 };
                *hist.entry(neighbor).or_insert(0) += weight * factor;
            }
        }
    }

    let mut ranked: Vec<(i64, i64)> = hist.into_iter().map(|(b, w)| (b * BUCKET_MS, w)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    let mut candidates: Vec<i64> = ranked.into_iter().take(6).map(|(off, _)| off).collect();
    candidates.push(0);

    let mut best_off = 0i64;
    let mut best_score = -1i64;
    for base in candidates {
        for delta in (-150..=150).step_by(10) {
            let off = base + delta;
            let sc: i64 = window_target
                .iter()
                .map(|&sp| span_overlap_score(sp, off, &ref_spans))
                .sum();
            if sc > best_score || (sc == best_score && off.abs() < best_off.abs()) {
                best_score = sc;
                best_off = off;
            }
        }
    }

    if best_off.abs() < 300 {
        return LiveMkvAlignOutcome::Aligned(0.0);
    }

    let zero_score: i64 = window_target
        .iter()
        .map(|&sp| span_overlap_score(sp, 0, &ref_spans))
        .sum();
    if best_score < (zero_score * 125) / 100 {
        return LiveMkvAlignOutcome::NotReady;
    }

    let matched = window_target
        .iter()
        .filter(|&&sp| span_overlap_score(sp, best_off, &ref_spans) > 300)
        .count();
    if matched < 8 || matched < (window_target.len() * 35) / 100 {
        return LiveMkvAlignOutcome::NotReady;
    }

    LiveMkvAlignOutcome::Aligned(((best_off as f64 / 1000.0) * 10.0).round() / 10.0)
}

pub fn compute_live_mkv_cue_offset_sec(target_cues: &[SubtitleCue], video_path: &Path) -> Option<f64> {
    match compute_live_mkv_cue_offset_outcome(target_cues, video_path) {
        LiveMkvAlignOutcome::Aligned(off) => Some(off),
        LiveMkvAlignOutcome::NoEmbeddedSubTracks | LiveMkvAlignOutcome::NotReady => None,
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust Matroska (.mkv) EBML Subtitle Cue Extractor
// ---------------------------------------------------------------------------

struct TrackedBufReader {
    inner: BufReader<File>,
    pos: u64,
}

impl TrackedBufReader {
    fn new(file: File) -> Self {
        Self {
            inner: BufReader::with_capacity(256 * 1024, file),
            pos: 0,
        }
    }

    #[inline]
    fn position(&self) -> u64 {
        self.pos
    }

    fn skip_to(&mut self, target: u64) -> std::io::Result<()> {
        if target <= self.pos {
            return Ok(());
        }
        let delta = target - self.pos;
        if let Ok(rel) = i64::try_from(delta) {
            self.inner.seek_relative(rel)?;
        } else {
            self.inner.seek(SeekFrom::Start(target))?;
        }
        self.pos = target;
        Ok(())
    }
}

impl Read for TrackedBufReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

/// Read an EBML variable-length integer (VINT).
/// If `keep_marker` is true, returns the raw Element ID (with the leading VINT marker bit intact).
/// If `keep_marker` is false, masks out the VINT marker bit to return the data size / value.
fn read_ebml_vint<R: Read>(r: &mut R, keep_marker: bool) -> std::io::Result<(u64, usize)> {
    let mut first = [0u8; 1];
    r.read_exact(&mut first)?;
    let b = first[0];
    if b == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "zero VINT prefix",
        ));
    }
    let len = b.leading_zeros() as usize + 1;
    if len > 8 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "VINT > 8 bytes",
        ));
    }
    let mask = if keep_marker {
        0xFFu8
    } else {
        (1u8 << (8 - len)) - 1
    };
    let mut val = (b & mask) as u64;
    if len > 1 {
        let mut buf = [0u8; 7];
        r.read_exact(&mut buf[..len - 1])?;
        for &byte in &buf[..len - 1] {
            val = (val << 8) | (byte as u64);
        }
    }
    Ok((val, len))
}

fn read_ebml_uint<R: Read>(r: &mut R, size: usize) -> std::io::Result<u64> {
    if size > 8 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "uint > 8 bytes",
        ));
    }
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf[..size])?;
    let mut val = 0u64;
    for &b in &buf[..size] {
        val = (val << 8) | (b as u64);
    }
    Ok(val)
}

fn decode_mkv_block_bytes(raw_bytes: &[u8]) -> Option<String> {
    if raw_bytes.is_empty() {
        return None;
    }

    if raw_bytes.len() >= 2 {
        let cmf = raw_bytes[0];
        let flg = raw_bytes[1];
        let is_zlib_header =
            (cmf & 0x0F) == 8 && (cmf >> 4) <= 7 && (u16::from_be_bytes([cmf, flg]) % 31 == 0);
        if is_zlib_header {
            let mut decoded = Vec::new();
            if flate2::read::ZlibDecoder::new(raw_bytes)
                .take(65_536)
                .read_to_end(&mut decoded)
                .is_ok()
                && !decoded.is_empty()
            {
                if let Ok(s) = String::from_utf8(decoded) {
                    return Some(s);
                }
            }
        }
    }

    if let Ok(s) = std::str::from_utf8(raw_bytes) {
        let has_binary_ctrl = s
            .bytes()
            .any(|b| b < 0x20 && b != b'\n' && b != b'\r' && b != b'\t' && b != 0);
        if !has_binary_ctrl {
            return Some(s.to_string());
        }
    }

    let mut decoded = Vec::new();
    if flate2::read::ZlibDecoder::new(raw_bytes)
        .take(65_536)
        .read_to_end(&mut decoded)
        .is_ok()
        && !decoded.is_empty()
    {
        return Some(String::from_utf8_lossy(&decoded).into_owned());
    }
    decoded.clear();
    if DeflateDecoder::new(raw_bytes)
        .take(65_536)
        .read_to_end(&mut decoded)
        .is_ok()
        && !decoded.is_empty()
    {
        return Some(String::from_utf8_lossy(&decoded).into_owned());
    }

    None
}

fn clean_mkv_block_text(raw_bytes: &[u8], is_ass: bool) -> String {
    let Some(decoded) = decode_mkv_block_bytes(raw_bytes) else {
        return String::new();
    };
    let raw = decoded.trim_matches('\0').trim();
    if raw.is_empty() {
        return String::new();
    }
    if is_ass {
        let parts: Vec<&str> = raw.splitn(9, ',').collect();
        if parts.len() == 9 {
            let style = parts[2].trim().to_ascii_lowercase();
            let text_part = parts[8].trim();
            if text_part.contains("\\p1") || style.contains("sign") || style.contains("logo") {
                return String::new();
            }
            return strip_sub_markup(text_part, true);
        }
        strip_sub_markup(raw, true)
    } else {
        strip_sub_markup(raw, false)
    }
}

const MAX_MKV_TIME_MS: i64 = i64::MAX / 4;

fn open_shared_read(path: &Path) -> std::io::Result<File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ (0x01) | FILE_SHARE_WRITE (0x02) | FILE_SHARE_DELETE (0x04)
        opts.share_mode(0x07);
    }
    opts.open(path)
}

fn read_bytes_shared(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = open_shared_read(path)?;
    let mut out = Vec::new();
    file.read_to_end(&mut out)?;
    Ok(out)
}

#[inline]
fn scale_mkv_ticks_to_ms(cluster_ts: u64, rel_ts: i16, timestamp_scale: u64) -> i64 {
    let base = (cluster_ts as i128) + (rel_ts as i128);
    if base <= 0 {
        return 0;
    }
    let ns = (base as u128).saturating_mul(timestamp_scale as u128);
    let ms = ns / 1_000_000;
    ms.min(MAX_MKV_TIME_MS as u128) as i64
}

#[inline]
fn scale_mkv_duration_to_ms(dur_ticks: u64, timestamp_scale: u64) -> i64 {
    let ns = (dur_ticks as u128).saturating_mul(timestamp_scale as u128);
    let ms = ns / 1_000_000;
    ms.min(MAX_MKV_TIME_MS as u128) as i64
}

fn extract_mkv_subtitle_spans_classified(mkv_path: &Path) -> Result<MkvExtractResult> {
    let file = open_shared_read(mkv_path)?;
    let file_len = file.metadata()?.len();
    let mut reader = TrackedBufReader::new(file);

    // Verify EBML magic `0x1A45DFA3`
    let (ebml_id, _) = read_ebml_vint(&mut reader, true)?;
    if ebml_id != 0x1A45DFA3 {
        bail!("Not an EBML/Matroska file");
    }
    let (ebml_size, _) = read_ebml_vint(&mut reader, false)?;
    let ebml_end = reader.position().saturating_add(ebml_size).min(file_len);
    reader.skip_to(ebml_end)?;

    // Find Segment `0x18538067`
    let (seg_id, _) = read_ebml_vint(&mut reader, true)?;
    if seg_id != 0x18538067 {
        bail!("Matroska Segment not found");
    }
    let (_seg_size, _) = read_ebml_vint(&mut reader, false)?;

    let mut timestamp_scale: u64 = 1_000_000; // default 1ms in nanoseconds
    // (track_num, lang, is_ass)
    let mut sub_tracks: Vec<(u64, String, bool)> = Vec::new();
    let mut spans_by_track: HashMap<u64, Vec<Span>> = HashMap::new();
    let mut tracks_seen = false;
    let mut clusters_seen = 0usize;

    while reader.position() < file_len {
        let elem_pos = reader.position();
        let Ok((id, _)) = read_ebml_vint(&mut reader, true) else {
            break;
        };
        let Ok((size, _)) = read_ebml_vint(&mut reader, false) else {
            break;
        };
        let data_start = reader.position();
        let is_unknown_size = size == 0x00FF_FFFF_FFFF_FFFF || size == 0x7FFF_FFFF_FFFF_FFFF;
        let elem_end = if is_unknown_size {
            file_len
        } else {
            data_start.saturating_add(size).min(file_len)
        };

        match id {
            // Segment Info (0x1549A966)
            0x1549A966 => {
                while reader.position() < elem_end {
                    let sub_elem_pos = reader.position();
                    let Ok((sub_id, _)) = read_ebml_vint(&mut reader, true) else {
                        break;
                    };
                    let Ok((sub_sz, _)) = read_ebml_vint(&mut reader, false) else {
                        break;
                    };
                    let sub_start = reader.position();
                    let sub_end = sub_start.saturating_add(sub_sz).min(elem_end);
                    if sub_id == 0x2AD7B1 && sub_sz <= 8 && sub_end <= elem_end {
                        if let Ok(scale) = read_ebml_uint(&mut reader, sub_sz as usize) {
                            if scale > 0 {
                                timestamp_scale = scale;
                            }
                        }
                    }
                    if sub_end <= sub_elem_pos {
                        break;
                    }
                    let _ = reader.skip_to(sub_end);
                }
                let _ = reader.skip_to(elem_end);
            }
            // Tracks (0x1654AE6B)
            0x1654AE6B => {
                tracks_seen = true;
                while reader.position() < elem_end {
                    let entry_elem_pos = reader.position();
                    let Ok((entry_id, _)) = read_ebml_vint(&mut reader, true) else {
                        break;
                    };
                    let Ok((entry_sz, _)) = read_ebml_vint(&mut reader, false) else {
                        break;
                    };
                    let entry_start = reader.position();
                    let entry_end = entry_start.saturating_add(entry_sz).min(elem_end);
                    if entry_id == 0xAE {
                        let mut track_num = 0u64;
                        let mut track_type = 0u64;
                        let mut flag_forced = 0u64;
                        let mut lang = String::from("und");
                        let mut track_name: Option<String> = None;
                        let mut codec = String::new();
                        while reader.position() < entry_end {
                            let t_elem_pos = reader.position();
                            let Ok((tid, _)) = read_ebml_vint(&mut reader, true) else {
                                break;
                            };
                            let Ok((tsz, _)) = read_ebml_vint(&mut reader, false) else {
                                break;
                            };
                            let tstart = reader.position();
                            let tend = tstart.saturating_add(tsz).min(entry_end);
                            let avail = tend.saturating_sub(tstart);
                            match tid {
                                0xD7 if avail == tsz && tsz <= 8 => {
                                    track_num =
                                        read_ebml_uint(&mut reader, tsz as usize).unwrap_or(0)
                                }
                                0x83 if avail == tsz && tsz <= 8 => {
                                    track_type =
                                        read_ebml_uint(&mut reader, tsz as usize).unwrap_or(0)
                                }
                                0x55AA if avail == tsz && tsz <= 8 => {
                                    flag_forced =
                                        read_ebml_uint(&mut reader, tsz as usize).unwrap_or(0)
                                }
                                0x86 if avail == tsz && tsz <= 64 => {
                                    let mut b = vec![0u8; tsz as usize];
                                    if reader.read_exact(&mut b).is_ok() {
                                        codec = String::from_utf8_lossy(&b)
                                            .trim_matches('\0')
                                            .trim()
                                            .to_string();
                                    }
                                }
                                0x536E if avail == tsz && tsz <= 256 => {
                                    let mut b = vec![0u8; tsz as usize];
                                    if reader.read_exact(&mut b).is_ok() {
                                        let s = String::from_utf8_lossy(&b)
                                            .trim_matches('\0')
                                            .trim()
                                            .to_string();
                                        if !s.is_empty() {
                                            track_name = Some(s);
                                        }
                                    }
                                }
                                0x22B59C | 0x22B59D if avail == tsz && tsz <= 32 => {
                                    let mut b = vec![0u8; tsz as usize];
                                    if reader.read_exact(&mut b).is_ok() {
                                        let s = String::from_utf8_lossy(&b)
                                            .trim_matches('\0')
                                            .trim()
                                            .to_string();
                                        if !s.is_empty() {
                                            lang = s;
                                        }
                                    }
                                }
                                _ => {}
                            }
                            if tend <= t_elem_pos {
                                break;
                            }
                            let _ = reader.skip_to(tend);
                        }
                        let is_signs =
                            crate::player::is_signs_or_songs_title(track_name.as_deref());
                        if track_type == 0x11 && track_num > 0 && flag_forced == 0 && !is_signs {
                            let is_ass = codec.contains("ASS") || codec.contains("SSA");
                            sub_tracks.push((track_num, lang, is_ass));
                        }
                    }
                    if entry_end <= entry_elem_pos {
                        break;
                    }
                    let _ = reader.skip_to(entry_end);
                }
                let _ = reader.skip_to(elem_end);
                if sub_tracks.is_empty() {
                    return Ok(MkvExtractResult::NoSubTracks);
                }
            }
            // Cluster (0x1F43B675)
            0x1F43B675 => {
                clusters_seen += 1;
                if sub_tracks.is_empty() {
                    if !tracks_seen && clusters_seen > 4 && file_len > 2 * 1024 * 1024 {
                        return Ok(MkvExtractResult::NoSubTracks);
                    }
                    if elem_end <= elem_pos {
                        break;
                    }
                    let _ = reader.skip_to(elem_end);
                    continue;
                }
                let mut cluster_ts: u64 = 0;
                while reader.position() < elem_end {
                    let c_elem_pos = reader.position();
                    let Ok((cid, _)) = read_ebml_vint(&mut reader, true) else {
                        break;
                    };
                    let Ok((csz, _)) = read_ebml_vint(&mut reader, false) else {
                        break;
                    };
                    let cstart = reader.position();
                    let cend = cstart.saturating_add(csz).min(elem_end);
                    let cavail = cend.saturating_sub(cstart);
                    match cid {
                        // Cluster Timestamp (0xE7)
                        0xE7 if cavail == csz && csz <= 8 => {
                            cluster_ts =
                                read_ebml_uint(&mut reader, csz as usize).unwrap_or(0);
                        }
                        // SimpleBlock (0xA3)
                        0xA3 if cavail >= 4 => {
                            if let Ok((tnum, vlen)) = read_ebml_vint(&mut reader, false) {
                                if let Some(&(_, _, is_ass)) =
                                    sub_tracks.iter().find(|(n, _, _)| *n == tnum)
                                {
                                    let mut hdr = [0u8; 3];
                                    if reader.read_exact(&mut hdr).is_ok() {
                                        let rel_ts = i16::from_be_bytes([hdr[0], hdr[1]]);
                                        let start_ms =
                                            scale_mkv_ticks_to_ms(cluster_ts, rel_ts, timestamp_scale);
                                        let mut skip_bumper = false;
                                        if start_ms <= 20_000 {
                                            let payload_len = (cavail as usize)
                                                .saturating_sub(vlen.saturating_add(3));
                                            if payload_len > 0 && payload_len <= 4_096 {
                                                let mut payload = vec![0u8; payload_len];
                                                if reader.read_exact(&mut payload).is_ok() {
                                                    let txt =
                                                        clean_mkv_block_text(&payload, is_ass);
                                                    if is_distributor_bumper_cue(start_ms, &txt) {
                                                        skip_bumper = true;
                                                    }
                                                }
                                            }
                                        }
                                        if !skip_bumper {
                                            let end_ms =
                                                start_ms.saturating_add(2_200).min(MAX_MKV_TIME_MS);
                                            spans_by_track
                                                .entry(tnum)
                                                .or_default()
                                                .push(Span { start_ms, end_ms });
                                        }
                                    }
                                }
                            }
                        }
                        // BlockGroup (0xA0) — standard container for subtitle blocks with BlockDuration (0x9B)
                        0xA0 => {
                            let mut blk_track = 0u64;
                            let mut blk_rel_ts: i16 = 0;
                            let mut blk_dur_ticks: Option<u64> = None;
                            let mut blk_early_payload: Option<(Vec<u8>, bool)> = None;
                            while reader.position() < cend {
                                let b_elem_pos = reader.position();
                                let Ok((bid, _)) = read_ebml_vint(&mut reader, true) else {
                                    break;
                                };
                                let Ok((bsz, _)) = read_ebml_vint(&mut reader, false) else {
                                    break;
                                };
                                let bstart = reader.position();
                                let bend = bstart.saturating_add(bsz).min(cend);
                                let bavail = bend.saturating_sub(bstart);
                                match bid {
                                    // Block (0xA1)
                                    0xA1 if bavail >= 4 => {
                                        if let Ok((tnum, vlen)) = read_ebml_vint(&mut reader, false)
                                        {
                                            blk_track = tnum;
                                            if let Some(&(_, _, is_ass)) =
                                                sub_tracks.iter().find(|(n, _, _)| *n == tnum)
                                            {
                                                let mut hdr = [0u8; 3];
                                                if reader.read_exact(&mut hdr).is_ok() {
                                                    blk_rel_ts =
                                                        i16::from_be_bytes([hdr[0], hdr[1]]);
                                                    let est_start_ms = scale_mkv_ticks_to_ms(
                                                        cluster_ts,
                                                        blk_rel_ts,
                                                        timestamp_scale,
                                                    );
                                                    if est_start_ms <= 20_000 {
                                                        let payload_len = (bavail as usize)
                                                            .saturating_sub(vlen.saturating_add(3));
                                                        if payload_len > 0 && payload_len <= 4_096 {
                                                            let mut payload =
                                                                vec![0u8; payload_len];
                                                            if reader
                                                                .read_exact(&mut payload)
                                                                .is_ok()
                                                            {
                                                                blk_early_payload =
                                                                    Some((payload, is_ass));
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    // BlockDuration (0x9B)
                                    0x9B if bavail == bsz && bsz <= 8 => {
                                        blk_dur_ticks =
                                            read_ebml_uint(&mut reader, bsz as usize).ok();
                                    }
                                    _ => {}
                                }
                                if bend <= b_elem_pos {
                                    break;
                                }
                                let _ = reader.skip_to(bend);
                            }
                            if sub_tracks.iter().any(|(n, _, _)| *n == blk_track) {
                                let start_ms =
                                    scale_mkv_ticks_to_ms(cluster_ts, blk_rel_ts, timestamp_scale);
                                let skip_bumper = if let Some((payload, is_ass)) = blk_early_payload
                                {
                                    let txt = clean_mkv_block_text(&payload, is_ass);
                                    is_distributor_bumper_cue(start_ms, &txt)
                                } else {
                                    false
                                };
                                if !skip_bumper {
                                    let dur_ms = blk_dur_ticks
                                        .map(|d| scale_mkv_duration_to_ms(d, timestamp_scale))
                                        .unwrap_or(2_200)
                                        .clamp(100, 15_000);
                                    spans_by_track.entry(blk_track).or_default().push(Span {
                                        start_ms,
                                        end_ms: start_ms
                                            .saturating_add(dur_ms)
                                            .min(MAX_MKV_TIME_MS),
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                    if cend <= c_elem_pos {
                        break;
                    }
                    let _ = reader.skip_to(cend);
                }
                let _ = reader.skip_to(elem_end);
            }
            _ => {
                if elem_end <= elem_pos {
                    break;
                }
                let _ = reader.skip_to(elem_end);
            }
        }
    }

    if tracks_seen && sub_tracks.is_empty() {
        return Ok(MkvExtractResult::NoSubTracks);
    }

    // Pick the subtitle track with the most dialogue cues (dialogue tracks have 150-500 cues; sign tracks have <50)
    let mut best_track: Option<(u64, String, Vec<Span>)> = None;
    for (tnum, lang, _) in sub_tracks {
        if let Some(mut sp) = spans_by_track.remove(&tnum) {
            sp.sort_by_key(|s| s.start_ms);
            sp.dedup_by(|a, b| (a.start_ms - b.start_ms).abs() < 40);
            if sp.len() >= 12 {
                let is_better = match &best_track {
                    None => true,
                    Some((_, _, prev)) => sp.len() > prev.len(),
                };
                if is_better {
                    best_track = Some((tnum, lang, sp));
                }
            }
        }
    }

    match best_track {
        Some((tnum, lang, spans)) => Ok(MkvExtractResult::Spans(
            spans,
            format!("Embedded MKV subtitle track #{tnum} ({lang})"),
        )),
        None => Ok(MkvExtractResult::InsufficientCues),
    }
}

/// Extract subtitle timing spans `(start_ms, end_ms)` directly from a Matroska (`.mkv`) file
/// in pure Rust without needing `ffmpeg` or external binaries.
pub fn extract_mkv_subtitle_spans(mkv_path: &Path) -> Result<(Vec<Span>, String)> {
    match extract_mkv_subtitle_spans_classified(mkv_path)? {
        MkvExtractResult::Spans(spans, label) => Ok((spans, label)),
        MkvExtractResult::NoSubTracks => bail!("No embedded subtitle tracks in MKV"),
        MkvExtractResult::InsufficientCues => {
            bail!("No populated subtitle cues found inside MKV container")
        }
    }
}

/// Extract reference subtitle spans from:
/// 1. Embedded Matroska `.mkv` subtitle track (pure Rust, instant).
/// 2. Sibling external subtitle file in the same directory (e.g. `.en.srt` / `.en.ass`).
/// 3. `ffmpeg` CLI fallback (`ffmpeg -v quiet -i <video> -map 0:s:0 -f srt -`).
pub fn extract_reference_spans(
    video_path: &Path,
    target_sub_path: &Path,
) -> Result<(Vec<Span>, String)> {
    // 1. Pure-Rust MKV EBML parser
    if video_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("mkv"))
        .unwrap_or(false)
    {
        if let Ok(res) = extract_mkv_subtitle_spans(video_path) {
            return Ok(res);
        }
    }

    // 2. Sibling external subtitle next to the video
    if let Some(parent) = video_path.parent() {
        let v_stem = video_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p == target_sub_path || !p.is_file() || !is_subtitle_file(&p) {
                    continue;
                }
                let fname = p
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !v_stem.is_empty() && fname.to_lowercase().starts_with(&v_stem) {
                    if let Ok(content) = std::fs::read_to_string(&p) {
                        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("srt");
                        let spans = parse_subtitle_spans(&content, ext);
                        if spans.len() >= 15 {
                            return Ok((spans, format!("Sibling subtitle ({fname})")));
                        }
                    }
                }
            }
        }
    }

    // 3. ffmpeg fallback (for MP4 / M4V / external containers)
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-v", "quiet", "-i"])
        .arg(video_path)
        .args(["-map", "0:s:0", "-f", "srt", "-"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    if let Ok(out) = cmd.output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            let spans = parse_subtitle_spans(&text, "srt");
            if spans.len() >= 10 {
                return Ok((spans, "Embedded subtitle stream (via ffmpeg)".to_string()));
            }
        }
    }

    bail!("Could not find reference subtitles in the video container or folder to align against.")
}

/// Retime `sub_path` against `video_path` and write the synchronized subtitle to `out_path`.
pub fn retime_subtitle_file(
    sub_path: &Path,
    video_path: &Path,
    out_path: &Path,
) -> Result<RetimeReport> {
    let raw_bytes =
        read_bytes_shared(sub_path).with_context(|| format!("Reading {}", sub_path.display()))?;
    // Strip UTF-8 BOM if present
    let content = String::from_utf8_lossy(&raw_bytes);
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let ext = sub_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("srt")
        .to_ascii_lowercase();

    let target_spans = parse_subtitle_spans(content, &ext);
    if target_spans.len() < 5 {
        bail!(
            "Subtitle file has too few dialogue lines ({}) to auto-retime.",
            target_spans.len()
        );
    }

    let (ref_spans, reference_source) = extract_reference_spans(video_path, sub_path)?;
    let model = align_subtitle_spans(&target_spans, &ref_spans).ok_or_else(|| {
        anyhow!("Could not compute a reliable alignment between target and reference subtitles.")
    })?;

    let retimed_text = rewrite_subtitle_timestamps(content, &ext, |ms| model.apply(ms));
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out_path, retimed_text.as_bytes())
        .with_context(|| format!("Writing retimed subtitle to {}", out_path.display()))?;

    Ok(RetimeReport {
        subtitle_path: sub_path.to_string_lossy().to_string(),
        output_path: out_path.to_string_lossy().to_string(),
        offset_ms: model.primary_offset_ms,
        segments: model.segments.len(),
        matched_cues: model.matched_cues,
        total_cues: target_spans.len(),
        reference_source,
        loaded_into_mpv: false,
    })
}

// ---------------------------------------------------------------------------
// Shared Bounds-Checked Pure-Rust ZIP Central Directory Reader (`H-06`)
// ---------------------------------------------------------------------------

pub(crate) struct ZipCentralEntry<'a> {
    pub name: String,
    pub method: u16,
    pub uncomp_size: usize,
    pub comp_slice: &'a [u8],
}

impl<'a> ZipCentralEntry<'a> {
    pub fn decompress(&self, max_bytes: u64) -> Result<Vec<u8>> {
        match self.method {
            0 => {
                if (self.comp_slice.len() as u64) > max_bytes {
                    bail!(
                        "ZIP entry '{}' exceeds maximum allowed size ({max_bytes} bytes)",
                        self.name
                    );
                }
                Ok(self.comp_slice.to_vec())
            }
            8 => {
                let cap = self.uncomp_size.min(max_bytes as usize).min(8 * 1024 * 1024);
                let mut buf = Vec::with_capacity(cap);
                let mut limited =
                    DeflateDecoder::new(self.comp_slice).take(max_bytes.saturating_add(1));
                limited.read_to_end(&mut buf)?;
                if (buf.len() as u64) > max_bytes {
                    bail!(
                        "ZIP entry '{}' decompressed size exceeds limit ({max_bytes} bytes)",
                        self.name
                    );
                }
                Ok(buf)
            }
            other => bail!(
                "Unsupported ZIP compression method {other} for '{}'",
                self.name
            ),
        }
    }
}

pub(crate) fn iter_zip_entries(zip_bytes: &[u8]) -> Result<Vec<ZipCentralEntry<'_>>> {
    if zip_bytes.len() < 22 {
        bail!("ZIP archive is too small");
    }
    let min_pos = zip_bytes.len().saturating_sub(65_557);
    let mut eocd_pos = None;
    for i in (min_pos..=zip_bytes.len() - 22).rev() {
        if &zip_bytes[i..i + 4] == b"PK\x05\x06" {
            let comment_len = u16::from_le_bytes([zip_bytes[i + 20], zip_bytes[i + 21]]) as usize;
            if i.checked_add(22).and_then(|p| p.checked_add(comment_len)) <= Some(zip_bytes.len()) {
                eocd_pos = Some(i);
                break;
            }
        }
    }
    let eocd = eocd_pos.ok_or_else(|| anyhow!("Invalid ZIP archive (EOCD not found)"))?;
    let total_entries = u16::from_le_bytes([zip_bytes[eocd + 10], zip_bytes[eocd + 11]]) as usize;
    let cd_offset = u32::from_le_bytes([
        zip_bytes[eocd + 16],
        zip_bytes[eocd + 17],
        zip_bytes[eocd + 18],
        zip_bytes[eocd + 19],
    ]) as usize;
    if cd_offset > eocd {
        bail!("Invalid ZIP central directory offset");
    }

    let max_possible = (eocd - cd_offset) / 46;
    let mut entries = Vec::with_capacity(total_entries.min(max_possible).min(16_384));
    let mut pos = cd_offset;

    for _ in 0..total_entries {
        let Some(hdr_end) = pos.checked_add(46) else {
            break;
        };
        if hdr_end > zip_bytes.len() || &zip_bytes[pos..pos + 4] != b"PK\x01\x02" {
            break;
        }
        let method = u16::from_le_bytes([zip_bytes[pos + 10], zip_bytes[pos + 11]]);
        let comp_size = u32::from_le_bytes([
            zip_bytes[pos + 20],
            zip_bytes[pos + 21],
            zip_bytes[pos + 22],
            zip_bytes[pos + 23],
        ]) as usize;
        let uncomp_size = u32::from_le_bytes([
            zip_bytes[pos + 24],
            zip_bytes[pos + 25],
            zip_bytes[pos + 26],
            zip_bytes[pos + 27],
        ]) as usize;
        let name_len = u16::from_le_bytes([zip_bytes[pos + 28], zip_bytes[pos + 29]]) as usize;
        let extra_len = u16::from_le_bytes([zip_bytes[pos + 30], zip_bytes[pos + 31]]) as usize;
        let comment_len = u16::from_le_bytes([zip_bytes[pos + 32], zip_bytes[pos + 33]]) as usize;
        let lfh_offset = u32::from_le_bytes([
            zip_bytes[pos + 42],
            zip_bytes[pos + 43],
            zip_bytes[pos + 44],
            zip_bytes[pos + 45],
        ]) as usize;

        let Some(name_end) = hdr_end.checked_add(name_len) else {
            bail!("Overflow in ZIP central directory entry name");
        };
        let Some(next_pos) = name_end
            .checked_add(extra_len)
            .and_then(|p| p.checked_add(comment_len))
        else {
            bail!("Overflow in ZIP central directory entry");
        };
        if name_end > zip_bytes.len() || next_pos > zip_bytes.len() {
            bail!("Truncated ZIP central directory entry");
        }

        let name = String::from_utf8_lossy(&zip_bytes[hdr_end..name_end]).into_owned();
        pos = next_pos;

        let Some(lfh_end) = lfh_offset.checked_add(30) else {
            continue;
        };
        if lfh_end > zip_bytes.len() || &zip_bytes[lfh_offset..lfh_offset + 4] != b"PK\x03\x04" {
            continue;
        }
        let lfh_name_len =
            u16::from_le_bytes([zip_bytes[lfh_offset + 26], zip_bytes[lfh_offset + 27]]) as usize;
        let lfh_extra_len =
            u16::from_le_bytes([zip_bytes[lfh_offset + 28], zip_bytes[lfh_offset + 29]]) as usize;
        let Some(data_start) = lfh_end
            .checked_add(lfh_name_len)
            .and_then(|p| p.checked_add(lfh_extra_len))
        else {
            continue;
        };
        let Some(data_end) = data_start.checked_add(comp_size) else {
            continue;
        };
        if data_end > zip_bytes.len() {
            continue;
        }

        entries.push(ZipCentralEntry {
            name,
            method,
            uncomp_size,
            comp_slice: &zip_bytes[data_start..data_end],
        });
    }

    Ok(entries)
}

/// Extract all subtitle files (`.srt`, `.ass`, `.ssa`, `.vtt`) from a `.zip` archive into `dest_dir`.
pub fn extract_subtitles_from_zip(zip_bytes: &[u8], dest_dir: &Path) -> Result<Vec<PathBuf>> {
    const MAX_SUB_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
    const MAX_TOTAL_UNCOMP_BYTES: u64 = 64 * 1024 * 1024;

    std::fs::create_dir_all(dest_dir)?;
    let mut extracted = Vec::new();
    let mut total_written = 0u64;

    for entry in iter_zip_entries(zip_bytes)? {
        if entry.name.ends_with('/') || entry.name.ends_with('\\') {
            continue;
        }
        let Some(base_name) = Path::new(&entry.name)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
        else {
            continue;
        };
        if base_name.starts_with('.') || !is_subtitle_file(Path::new(&base_name)) {
            continue;
        }
        let Ok(out_bytes) = entry.decompress(MAX_SUB_ENTRY_BYTES) else {
            continue;
        };
        total_written = total_written.saturating_add(out_bytes.len() as u64);
        if total_written > MAX_TOTAL_UNCOMP_BYTES {
            bail!("Subtitle ZIP archive exceeds maximum total extracted size (64 MiB)");
        }
        let out_path = dest_dir.join(&base_name);
        if std::fs::write(&out_path, &out_bytes).is_ok() {
            extracted.push(out_path);
        }
    }

    Ok(extracted)
}

// ---------------------------------------------------------------------------
// 3. Jimaku Subtitle Fetcher API Client
// ---------------------------------------------------------------------------

pub fn get_jimaku_api_key(conn: &Connection) -> Option<String> {
    db::get_setting(conn, "pref.jimaku_api_key")
        .ok()
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub async fn jimaku_search_files(
    st: &AppState,
    anilist_id: i64,
    episode: Option<i64>,
) -> Result<JimakuSearchResponse> {
    let api_key = {
        let conn = st.db_read();
        get_jimaku_api_key(&conn)
    };
    let Some(key) = api_key else {
        return Ok(JimakuSearchResponse {
            configured: false,
            entry: None,
            files: Vec::new(),
        });
    };

    // 1. Resolve Jimaku entry for `anilist_id` (cached for 6 hours; negative lookup cached for 1 hour)
    let entry_cache_key = format!("entry:{anilist_id}");
    let cached_lookup: Option<Option<JimakuEntry>> = {
        let conn = st.db();
        conn.query_row(
            "SELECT payload, fetched_at FROM jimaku_cache WHERE cache_key = ?1",
            [&entry_cache_key],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()
        .ok()
        .flatten()
        .and_then(|(payload, ts)| {
            let age = now() - ts;
            if payload == "null" {
                if age < 3600 {
                    Some(None)
                } else {
                    None
                }
            } else if age < 6 * 3600 {
                serde_json::from_str::<JimakuEntry>(&payload)
                    .ok()
                    .map(Some)
            } else {
                None
            }
        })
    };

    let entry = match cached_lookup {
        Some(opt) => opt,
        None => {
            let url = format!("https://jimaku.cc/api/entries/search?anilist_id={anilist_id}");
            let resp = st
                .providers
                .http
                .get(&url)
                .header("Authorization", &key)
                .timeout(Duration::from_secs(10))
                .send()
                .await?;
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED
                || resp.status() == reqwest::StatusCode::FORBIDDEN
            {
                bail!("Invalid Jimaku API key — check Settings → Integrations.");
            }
            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                bail!("Jimaku rate limit reached. Please wait a few seconds and try again.");
            }
            if !resp.status().is_success() {
                bail!("Jimaku API returned {}", resp.status());
            }
            let entries: Vec<JimakuEntry> = resp.json().await.unwrap_or_default();
            let first = entries.into_iter().next();
            let payload_str = match first {
                Some(ref e) => serde_json::to_string(e).unwrap_or_else(|_| "null".to_string()),
                None => "null".to_string(),
            };
            let _ = st.db().execute(
                "INSERT OR REPLACE INTO jimaku_cache (cache_key, payload, fetched_at) VALUES (?1, ?2, ?3)",
                params![entry_cache_key, payload_str, now()],
            );
            first
        }
    };

    let Some(entry) = entry else {
        return Ok(JimakuSearchResponse {
            configured: true,
            entry: None,
            files: Vec::new(),
        });
    };

    // 2. Fetch all files for this Jimaku entry (cached for 30 minutes) so we can show both episode-specific files and season packs
    let files_cache_key = format!("files:{}", entry.id);
    let raw_files: Vec<serde_json::Value> = {
        let cached: Option<Vec<serde_json::Value>> = {
            let conn = st.db();
            conn.query_row(
                "SELECT payload, fetched_at FROM jimaku_cache WHERE cache_key = ?1",
                [&files_cache_key],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()
            .ok()
            .flatten()
            .filter(|(_, ts)| now() - *ts < 600)
            .and_then(|(p, _)| serde_json::from_str(&p).ok())
        };
        match cached {
            Some(v) => v,
            None => {
                let url = format!("https://jimaku.cc/api/entries/{}/files", entry.id);
                let resp = st
                    .providers
                    .http
                    .get(&url)
                    .header("Authorization", &key)
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await?;
                if !resp.status().is_success() {
                    bail!("Jimaku files endpoint returned {}", resp.status());
                }
                let list: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
                if let Ok(json_str) = serde_json::to_string(&list) {
                    let _ = st.db().execute(
                        "INSERT OR REPLACE INTO jimaku_cache (cache_key, payload, fetched_at) VALUES (?1, ?2, ?3)",
                        params![files_cache_key, json_str, now()],
                    );
                }
                list
            }
        }
    };

    let (_, prequel_offset, _) = load_series_targets(&st.db(), anilist_id).unwrap_or_default();

    let mut items: Vec<JimakuFileItem> = Vec::new();
    for f in raw_files {
        let name = f
            .get("name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let url = f
            .get("url")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() || url.is_empty() {
            continue;
        }
        let size = f.get("size").and_then(|x| x.as_i64()).unwrap_or(0);
        let last_modified = f
            .get("last_modified")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let lower = name.to_ascii_lowercase();
        let is_zip = lower.ends_with(".zip");
        let is_rar = lower.ends_with(".rar") || lower.ends_with(".7z");
        let is_archive = is_zip || is_rar;

        let stem = Path::new(&name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&name);
        let (_, raw_ep) = extract_sub_episode(stem);
        let detected_episode = raw_ep.map(|e| {
            if prequel_offset > 0 && e > prequel_offset {
                e - prequel_offset
            } else {
                e
            }
        });

        let mut match_score: i64 = 0;
        if let Some(target_ep) = episode {
            if detected_episode == Some(target_ep) || raw_ep == Some(target_ep) {
                match_score += 100;
            } else if is_zip && detected_episode.is_none() {
                // Season pack archive
                match_score += 40;
            } else if detected_episode.is_some() {
                match_score -= 50;
            }
        }
        if lower.ends_with(".ass") || lower.ends_with(".ssa") {
            match_score += 15;
        } else if lower.ends_with(".srt") {
            match_score += 10;
        } else if is_zip {
            match_score += 5;
        } else if is_rar {
            match_score -= 20;
        }
        if lower.contains("webfs") || lower.contains("webrip") || lower.contains("netflix") {
            match_score += 3;
        }

        items.push(JimakuFileItem {
            name,
            url,
            size,
            last_modified,
            is_archive,
            is_rar,
            detected_episode,
            match_score,
        });
    }

    items.sort_by(|a, b| b.match_score.cmp(&a.match_score).then(a.name.cmp(&b.name)));

    Ok(JimakuSearchResponse {
        configured: true,
        entry: Some(entry),
        files: items,
    })
}

const MAX_SUBTITLE_DOWNLOAD_BYTES: usize = 32 * 1024 * 1024;

pub async fn jimaku_download_and_process(
    st: &AppState,
    anilist_id: i64,
    ep_key: &str,
    file_name: &str,
    file_url: &str,
    auto_retime: bool,
) -> Result<JimakuDownloadResult> {
    let lower_name = file_name.to_ascii_lowercase();
    let single_ext = if !lower_name.ends_with(".zip") {
        let ext = Path::new(file_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "srt" | "ass" | "ssa" | "vtt") {
            bail!("Unsupported subtitle file extension: .{ext}");
        }
        Some(ext)
    } else {
        None
    };

    let parsed_url =
        reqwest::Url::parse(file_url).map_err(|_| anyhow!("Invalid Jimaku download URL"))?;
    if !matches!(parsed_url.scheme(), "https" | "http") {
        bail!("Unsupported subtitle URL scheme: {}", parsed_url.scheme());
    }

    let api_key = get_jimaku_api_key(&st.db_read());
    let mut header_pairs: Vec<(String, String)> = Vec::new();
    if parsed_url.scheme() == "https" && parsed_url.host_str() == Some("jimaku.cc") {
        if let Some(k) = api_key {
            header_pairs.push(("Authorization".to_string(), k));
        }
    }

    let bytes = crate::extensions::fetch_public_bytes_limited(
        file_url,
        &header_pairs,
        MAX_SUBTITLE_DOWNLOAD_BYTES,
        30,
    )
    .await
    .map_err(|e| anyhow!(e))?;

    let local_video_path: Option<PathBuf> = {
        let conn = st.db_read();
        conn.query_row(
            "SELECT path FROM local_files WHERE anilist_id = ?1 AND ep_key = ?2 ORDER BY id LIMIT 1",
            params![anilist_id, ep_key],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    };
    let data_dir = st.data_dir.clone();
    let ep_key_owned = ep_key.to_string();
    let file_name_owned = file_name.to_string();

    struct PendingUpsert {
        ep_key: String,
        path: String,
        lang: String,
        cc: bool,
        was_retimed: bool,
        offset_ms: Option<i64>,
    }

    let (mut saved_paths, mut linked_count, mut retime_report, mut pending_upserts, extracted_pack) =
        tokio::task::spawn_blocking(
            move || -> Result<(
                Vec<String>,
                usize,
                Option<RetimeReport>,
                Vec<PendingUpsert>,
                Option<(PathBuf, Vec<PathBuf>)>,
            )> {
                let video_path: Option<PathBuf> = local_video_path.clone();
                let cache_dir = data_dir.join("subtitles").join(anilist_id.to_string());
                std::fs::create_dir_all(&cache_dir)?;

                let mut saved_paths = Vec::new();
                let mut linked_count = 0usize;
                let mut retime_report: Option<RetimeReport> = None;
                let mut upserts = Vec::new();

                if let Some(ext) = single_ext {
                    let cc = detect_cc_tag(&file_name_owned);
                    let dest_path = if let Some(ref vpath) = local_video_path {
                        build_target_sub_path(vpath, "ja", cc, &ext)
                    } else {
                        let safe_name: String = file_name_owned
                            .chars()
                            .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
                            .collect();
                        let safe_ep: String = ep_key_owned
                            .chars()
                            .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
                            .collect();
                        cache_dir.join(format!("ep{safe_ep}_{safe_name}"))
                    };

                    std::fs::write(&dest_path, &bytes)?;
                    let mut active_path = dest_path.clone();
                    let mut was_retimed = false;
                    let mut offset_ms = None;

                    if auto_retime {
                        if let Some(ref vpath) = video_path {
                            let safe_ep: String = ep_key_owned
                                .chars()
                                .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
                                .collect();
                            let orig_backup = cache_dir.join(format!("ep{safe_ep}.orig.{ext}"));
                            let _ = std::fs::copy(&dest_path, &orig_backup);
                            if let Ok(rt) = retime_subtitle_file(&orig_backup, vpath, &dest_path) {
                                active_path = PathBuf::from(&rt.output_path);
                                was_retimed = true;
                                offset_ms = Some(rt.offset_ms);
                                retime_report = Some(rt);
                            }
                        }
                    }

                    let p_str = active_path.to_string_lossy().to_string();
                    upserts.push(PendingUpsert {
                        ep_key: ep_key_owned,
                        path: p_str.clone(),
                        lang: "ja".to_string(),
                        cc,
                        was_retimed,
                        offset_ms,
                    });
                    saved_paths.push(p_str);
                    linked_count = 1;
                    Ok((saved_paths, linked_count, retime_report, upserts, None))
                } else {
                    let pack_dir = cache_dir.join("pack");
                    let extracted = extract_subtitles_from_zip(&bytes, &pack_dir)?;
                    if extracted.is_empty() {
                        bail!("No .srt/.ass/.vtt subtitle files were found inside the .zip archive.");
                    }
                    Ok((
                        saved_paths,
                        linked_count,
                        retime_report,
                        upserts,
                        Some((pack_dir, extracted)),
                    ))
                }
            },
        )
        .await??;

    if let Some((pack_dir, extracted)) = extracted_pack {
        let preview = preview_subtitle_matches(&st.db_read(), anilist_id, Some(&pack_dir), "ja")?;
        let cache_dir = st.data_dir.join("subtitles").join(anilist_id.to_string());
        let ep_key_owned = ep_key.to_string();
        let video_path: Option<PathBuf> = {
            let conn = st.db_read();
            conn.query_row(
                "SELECT path FROM local_files WHERE anilist_id = ?1 AND ep_key = ?2 ORDER BY id LIMIT 1",
                params![anilist_id, ep_key],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .map(PathBuf::from)
            .filter(|p| p.is_file())
        };

        let (pack_paths, pack_count, pack_rt, pack_upserts) = tokio::task::spawn_blocking(
            move || -> Result<(Vec<String>, usize, Option<RetimeReport>, Vec<PendingUpsert>)> {
                let mut sp = Vec::new();
                let mut lc = 0usize;
                let mut rt_rep = None;
                let mut ups = Vec::new();

                if !preview.matches.is_empty() {
                    for m in preview.matches {
                        let mut final_path = PathBuf::from(&m.source_path);
                        let mut was_retimed = false;
                        let mut offset_ms = None;

                        if auto_retime && m.ep_key == ep_key_owned && Path::new(&m.video_path).is_file()
                        {
                            let retimed_dest =
                                cache_dir.join(format!("ep{}_retimed_{}", m.ep_key, m.source_name));
                            if let Ok(rt) =
                                retime_subtitle_file(&final_path, Path::new(&m.video_path), &retimed_dest)
                            {
                                final_path = PathBuf::from(&rt.output_path);
                                was_retimed = true;
                                offset_ms = Some(rt.offset_ms);
                                rt_rep = Some(rt);
                            }
                        }

                        let p_str = final_path.to_string_lossy().to_string();
                        ups.push(PendingUpsert {
                            ep_key: m.ep_key,
                            path: p_str.clone(),
                            lang: m.lang,
                            cc: m.cc,
                            was_retimed,
                            offset_ms,
                        });
                        sp.push(p_str);
                        lc += 1;
                    }
                } else {
                    let mut matched_current = false;
                    for ext_path in &extracted {
                        let fname = ext_path
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let target_ep_key = parse_subtitle_filename(&fname)
                            .and_then(|p| p.episode)
                            .map(|ep| ep.to_string())
                            .or_else(|| {
                                if extracted.len() == 1 {
                                    Some(ep_key_owned.clone())
                                } else {
                                    None
                                }
                            });
                        if let Some(k) = target_ep_key {
                            let mut final_path = ext_path.clone();
                            let mut was_retimed = false;
                            let mut offset_ms = None;
                            if auto_retime && k == ep_key_owned {
                                matched_current = true;
                                if let Some(ref vpath) = video_path {
                                    let retimed_dest = cache_dir.join(format!("ep{k}_retimed_{fname}"));
                                    if let Ok(rt) = retime_subtitle_file(&final_path, vpath, &retimed_dest)
                                    {
                                        final_path = PathBuf::from(&rt.output_path);
                                        was_retimed = true;
                                        offset_ms = Some(rt.offset_ms);
                                        rt_rep = Some(rt);
                                    }
                                }
                            } else if k == ep_key_owned {
                                matched_current = true;
                            }
                            let cc = detect_cc_tag(&fname);
                            let p_str = final_path.to_string_lossy().to_string();
                            ups.push(PendingUpsert {
                                ep_key: k,
                                path: p_str.clone(),
                                lang: "ja".to_string(),
                                cc,
                                was_retimed,
                                offset_ms,
                            });
                            sp.push(p_str);
                            lc += 1;
                        }
                    }
                    if !matched_current && extracted.len() == 1 {
                        let first = &extracted[0];
                        let fname = first
                            .file_name()
                            .map(|f| f.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let detected_ep = parse_subtitle_filename(&fname).and_then(|p| p.episode);
                        if detected_ep.is_none()
                            || detected_ep.map(|e| e.to_string()).as_deref()
                                == Some(ep_key_owned.as_str())
                        {
                            let p_str = first.to_string_lossy().to_string();
                            ups.push(PendingUpsert {
                                ep_key: ep_key_owned,
                                path: p_str.clone(),
                                lang: "ja".to_string(),
                                cc: false,
                                was_retimed: false,
                                offset_ms: None,
                            });
                            sp.push(p_str);
                            lc += 1;
                        }
                    }
                }
                Ok((sp, lc, rt_rep, ups))
            },
        )
        .await??;

        saved_paths.extend(pack_paths);
        linked_count += pack_count;
        if pack_rt.is_some() {
            retime_report = pack_rt;
        }
        pending_upserts.extend(pack_upserts);
    }

    if !pending_upserts.is_empty() {
        let conn = st.db();
        for u in pending_upserts {
            let _ = upsert_linked_subtitle(
                &conn,
                anilist_id,
                &u.ep_key,
                &u.path,
                &u.lang,
                u.cc,
                "jimaku",
                u.was_retimed,
                u.offset_ms,
            );
        }
    }

    // If mpv is currently playing this episode, hot-load the subtitle over IPC!
    let ep_sub = episode_subtitle_paths(&st.db_read(), anilist_id, ep_key)
        .into_iter()
        .next()
        .or_else(|| saved_paths.first().cloned());

    let mut loaded_into_mpv = false;
    if let Some(ref sub_to_load) = ep_sub {
        loaded_into_mpv =
            crate::player::try_hotload_subtitle_in_mpv(st, anilist_id, ep_key, sub_to_load).await;
        if let Some(ref mut rt) = retime_report {
            rt.loaded_into_mpv = loaded_into_mpv;
        }
    }

    Ok(JimakuDownloadResult {
        saved_paths,
        linked_count,
        retime_report,
        loaded_into_mpv,
    })
}

/// Check if Japanese subtitles for `(anilist_id, episode)` are available on Jimaku,
/// and if so, prefetch & link them into `linked_subtitles` so they are ready immediately.
pub async fn check_and_prefetch_jimaku_episode(
    st: &AppState,
    anilist_id: i64,
    episode: i64,
) -> bool {
    let ep_key = episode.to_string();
    if has_saved_subtitle(&st.db(), anilist_id, &ep_key) {
        return true;
    }
    let has_key = {
        let conn = st.db();
        get_jimaku_api_key(&conn).is_some()
    };
    if !has_key {
        return false;
    }

    let Ok(resp) = jimaku_search_files(st, anilist_id, Some(episode)).await else {
        return false;
    };
    let Some(best) = resp
        .files
        .iter()
        .find(|f| !f.is_rar && f.detected_episode == Some(episode) && f.match_score >= 100)
        .or_else(|| {
            resp.files
                .iter()
                .find(|f| !f.is_rar && f.is_archive && f.detected_episode.is_none() && f.match_score >= 40)
        })
    else {
        return false;
    };

    jimaku_download_and_process(st, anilist_id, &ep_key, &best.name, &best.url, false)
        .await
        .map(|r| r.linked_count > 0 || !r.saved_paths.is_empty())
        .unwrap_or(false)
}

/// Retime `sub_path` (e.g. Jimaku Japanese `.ass`/`.srt`) against already-parsed reference cues
/// (such as a clean soft-sub `.vtt` stream track or AnimeTosho pre-extracted subtitle) using
/// piecewise commercial-break-aware alignment, writing the synchronized subtitle to `out_path`.
pub fn retime_subtitle_against_reference_cues(
    sub_path: &Path,
    ref_cues: &[SubtitleCue],
    out_path: &Path,
    reference_label: &str,
) -> Result<RetimeReport> {
    let raw_bytes =
        std::fs::read(sub_path).with_context(|| format!("Reading {}", sub_path.display()))?;
    let content = String::from_utf8_lossy(&raw_bytes);
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let ext = sub_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("srt")
        .to_ascii_lowercase();

    let target_spans = parse_subtitle_spans(content, &ext);
    if target_spans.len() < 5 {
        bail!(
            "Subtitle file has too few dialogue lines ({}) to auto-retime.",
            target_spans.len()
        );
    }

    let mut ref_spans: Vec<Span> = ref_cues
        .iter()
        .filter(|c| c.end_ms > c.start_ms && (c.end_ms - c.start_ms) <= 15_000)
        .map(|c| Span {
            start_ms: c.start_ms,
            end_ms: c.end_ms,
        })
        .collect();
    ref_spans.sort_by_key(|s| s.start_ms);
    ref_spans.dedup_by(|a, b| (a.start_ms - b.start_ms).abs() < 40);
    if ref_spans.len() < 8 {
        bail!(
            "Reference subtitle track has too few dialogue cues ({}) to align against.",
            ref_spans.len()
        );
    }

    let model = align_subtitle_spans(&target_spans, &ref_spans).ok_or_else(|| {
        anyhow!("Could not compute a reliable alignment between Jimaku and stream subtitles.")
    })?;

    let retimed_text = rewrite_subtitle_timestamps(content, &ext, |ms| model.apply(ms));
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out_path, retimed_text.as_bytes())
        .with_context(|| format!("Writing retimed subtitle to {}", out_path.display()))?;

    Ok(RetimeReport {
        subtitle_path: sub_path.to_string_lossy().to_string(),
        output_path: out_path.to_string_lossy().to_string(),
        offset_ms: model.primary_offset_ms,
        segments: model.segments.len(),
        matched_cues: model.matched_cues,
        total_cues: target_spans.len(),
        reference_source: reference_label.to_string(),
        loaded_into_mpv: false,
    })
}

/// Download a remote `.vtt` / `.srt` / `.ass` subtitle URL (supporting optional `.gz` compression
/// and custom stream headers) via the SSRF-hardened public HTTP fetcher and parse its cues.
pub async fn fetch_remote_subtitle_cues(
    _http: &reqwest::Client,
    url: &str,
    headers: &HashMap<String, String>,
) -> Result<Vec<SubtitleCue>> {
    const MAX_REMOTE_SUB_BYTES: usize = 10 * 1024 * 1024;

    let mut header_pairs: Vec<(String, String)> = Vec::with_capacity(headers.len() + 1);
    let mut has_ua = false;
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("user-agent") {
            has_ua = true;
        }
        header_pairs.push((k.clone(), v.clone()));
    }
    if !has_ua {
        header_pairs.push((
            "User-Agent".to_string(),
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/131.0.0.0 Safari/537.36".to_string(),
        ));
    }

    let raw_bytes = crate::extensions::fetch_public_bytes_limited(
        url,
        &header_pairs,
        MAX_REMOTE_SUB_BYTES,
        20,
    )
    .await
    .map_err(|e| anyhow!(e))?;

    let decoded_bytes = if raw_bytes.starts_with(&[0x1F, 0x8B]) {
        let mut gz =
            flate2::read::GzDecoder::new(&raw_bytes[..]).take((MAX_REMOTE_SUB_BYTES as u64) + 1);
        let mut out = Vec::new();
        gz.read_to_end(&mut out)?;
        if out.len() > MAX_REMOTE_SUB_BYTES {
            bail!("Decompressed remote subtitle exceeds maximum allowed size (10 MiB)");
        }
        out
    } else {
        raw_bytes
    };

    let content = String::from_utf8_lossy(&decoded_bytes);
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);

    let clean_path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    let clean_path = clean_path.strip_suffix(".gz").unwrap_or(&clean_path);
    let ext = if clean_path.ends_with(".ass") || clean_path.ends_with(".ssa")
        || content.contains("[Script Info]")
        || content.contains("Dialogue:")
    {
        "ass"
    } else if clean_path.ends_with(".vtt") || content.trim_start().starts_with("WEBVTT") {
        "vtt"
    } else {
        "srt"
    };

    Ok(parse_subtitle_cues(content, ext))
}

/// Automatically fetch Jimaku Japanese subtitles for a web stream or torrent stream episode,
/// align them piecewise against reference `.vtt`/AnimeTosho cues when available, persist in
/// `linked_subtitles`, and hot-load into the active `embedded_player` session.
pub async fn auto_prepare_stream_subtitles(
    app: &tauri::AppHandle,
    st: &AppState,
    anilist_id: i64,
    ep_key: &str,
    episode_num: i64,
    ref_cues: &[SubtitleCue],
    reference_label: &str,
) -> Option<String> {
    let (auto_subs_enabled, auto_retime_enabled) = {
        let conn = st.db();
        let auto_subs = db::get_setting(&conn, "pref.torrent_auto_subs")
            .ok()
            .flatten()
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        let auto_retime = db::get_setting(&conn, "pref.jimaku_auto_retime")
            .ok()
            .flatten()
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true);
        (auto_subs, auto_retime)
    };

    // 1. Ensure Japanese subtitle exists in `linked_subtitles` (or auto-download from Jimaku)
    let mut sub_path_opt = candidate_subtitle_paths_for_episode(&st.db(), anilist_id, ep_key)
        .into_iter()
        .find(|p| Path::new(p).is_file());

    if sub_path_opt.is_none() && auto_subs_enabled && episode_num > 0 {
        if check_and_prefetch_jimaku_episode(st, anilist_id, episode_num).await {
            sub_path_opt = candidate_subtitle_paths_for_episode(&st.db(), anilist_id, ep_key)
                .into_iter()
                .find(|p| Path::new(p).is_file());
        }
    }

    let sub_path_str = sub_path_opt?;
    let sub_path = PathBuf::from(&sub_path_str);

    // 2. If we have reference cues (e.g. clean soft-sub `.vtt` or AnimeTosho subtitle) and auto-retime is on,
    //    write a piecewise-synchronized copy (`ep{ep_key}_streamsync_{fname}`) and link it!
    let mut final_path_str = sub_path_str.clone();
    if auto_retime_enabled && ref_cues.len() >= 8 {
        let fname = sub_path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "ja.srt".to_string());
        let clean_fname = fname
            .strip_prefix(&format!("ep{ep_key}_streamsync_"))
            .unwrap_or(&fname);
        let cache_dir = st.data_dir.join("subtitles").join(anilist_id.to_string());
        let synced_dest = cache_dir.join(format!("ep{ep_key}_streamsync_{clean_fname}"));
        let source_for_align = if sub_path == synced_dest {
            // Re-align from clean original if present, else current file
            sub_path.clone()
        } else {
            sub_path.clone()
        };

        if let Ok(rt) = retime_subtitle_against_reference_cues(
            &source_for_align,
            ref_cues,
            &synced_dest,
            reference_label,
        ) {
            final_path_str = rt.output_path.clone();
            let cc = detect_cc_tag(clean_fname);
            let _ = upsert_linked_subtitle(
                &st.db(),
                anilist_id,
                ep_key,
                &final_path_str,
                "ja",
                cc,
                "jimaku",
                true,
                Some(rt.offset_ms),
            );
            crate::klog!(
                "[subtitles] Auto-retimed JP subtitle for anilist={} ep={} against {} (offset={}ms, segments={}, matched={}/{})",
                anilist_id,
                ep_key,
                reference_label,
                rt.offset_ms,
                rt.segments,
                rt.matched_cues,
                rt.total_cues
            );
        }
    }

    // 3. Hot-load into built-in player (and emit state) if this episode is active
    if crate::embedded_player::reload_stream_cues_if_active(app, anilist_id, ep_key, &final_path_str) {
        let _ = tauri::Emitter::emit(app, "library-changed", ());
    }

    Some(final_path_str)
}

// ---------------------------------------------------------------------------
// 4. Subtitle Cue Text Parser for Built-in Player
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleCue {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

fn strip_sub_markup(raw: &str, is_ass: bool) -> String {
    let mut s = raw
        .replace("\\N", "\n")
        .replace("\\n", "\n")
        .replace("\\h", " ");
    // Strip { ... } override tags
    let mut out = String::with_capacity(s.len());
    let mut in_brace = false;
    let mut in_angle = false;
    for ch in s.chars() {
        if ch == '{' {
            in_brace = true;
            continue;
        }
        if ch == '}' && in_brace {
            in_brace = false;
            continue;
        }
        if !is_ass && ch == '<' {
            in_angle = true;
            continue;
        }
        if !is_ass && ch == '>' && in_angle {
            in_angle = false;
            continue;
        }
        if !in_brace && !in_angle {
            out.push(ch);
        }
    }
    s = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    s
}

fn parse_subtitle_cues_inner(
    content: &str,
    ext: &str,
    skip_non_dialogue_ass_styles: bool,
) -> Vec<SubtitleCue> {
    let mut cues = Vec::new();
    let is_ass = matches!(ext.to_ascii_lowercase().as_str(), "ass" | "ssa");

    if is_ass {
        for line in content.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("Dialogue:") {
                let fields: Vec<&str> = rest.splitn(10, ',').collect();
                if fields.len() >= 10 {
                    if skip_non_dialogue_ass_styles {
                        let style = fields[3].trim().to_ascii_lowercase();
                        if style.contains("sign") || style.contains("op") || style.contains("ed") {
                            continue;
                        }
                    }
                    let raw_text = fields[9].trim();
                    if raw_text.is_empty() || raw_text.contains("\\p1") {
                        continue;
                    }
                    if let (Some(s), Some(e)) =
                        (parse_ass_time(fields[1]), parse_ass_time(fields[2]))
                    {
                        if e > s {
                            let clean = strip_sub_markup(raw_text, true);
                            if !clean.is_empty() {
                                cues.push(SubtitleCue {
                                    start_ms: s,
                                    end_ms: e,
                                    text: clean,
                                });
                            }
                        }
                    }
                }
            }
        }
    } else {
        let mut lines = content.lines().peekable();
        while let Some(line) = lines.next() {
            if let Some((left, right)) = line.split_once("-->") {
                let right_time = right.split_whitespace().next().unwrap_or(right);
                if let (Some(s), Some(e)) =
                    (parse_srt_vtt_time(left), parse_srt_vtt_time(right_time))
                {
                    let mut text_lines = Vec::new();
                    while let Some(&next_line) = lines.peek() {
                        if next_line.trim().is_empty() || next_line.contains("-->") {
                            break;
                        }
                        text_lines.push(lines.next().unwrap_or(""));
                    }
                    let raw = text_lines.join("\n");
                    let clean = strip_sub_markup(&raw, false);
                    if e > s && !clean.is_empty() {
                        cues.push(SubtitleCue {
                            start_ms: s,
                            end_ms: e,
                            text: clean,
                        });
                    }
                }
            }
        }
    }

    cues.sort_by_key(|c| c.start_ms);
    cues
}

pub fn parse_subtitle_cues(content: &str, ext: &str) -> Vec<SubtitleCue> {
    parse_subtitle_cues_inner(content, ext, false)
}

pub fn parse_subtitle_file_cues(path: &Path) -> Vec<SubtitleCue> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("srt")
        .to_ascii_lowercase();
    let Ok(bytes) = read_bytes_shared(path) else {
        return Vec::new();
    };
    let content = String::from_utf8_lossy(&bytes);
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
    parse_subtitle_cues(content, &ext)
}

fn compute_prequel_offset(conn: &Connection, anilist_id: i64) -> i64 {
    let from_abs: Option<i64> = conn
        .query_row(
            "SELECT MIN(absolute_number - number)
             FROM episodes
             WHERE anilist_id = ?1 AND is_special = 0 AND absolute_number IS NOT NULL AND absolute_number > number",
            [anilist_id],
            |r| r.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten();
    if let Some(off) = from_abs.filter(|&o| o > 0) {
        return off;
    }

    let mut prequel_offset = 0i64;
    let mut cur = anilist_id;
    let mut visited = HashSet::new();
    visited.insert(cur);
    while let Ok(Some((pre_id, pre_eps))) = conn
        .query_row(
            "SELECT r.related_id, COALESCE(m.episodes, r.episodes, 0)
             FROM media_relations r
             LEFT JOIN media m ON m.anilist_id = r.related_id
             WHERE r.anilist_id = ?1 AND r.relation_type = 'PREQUEL'
               AND COALESCE(r.format, '') IN ('TV', 'TV_SHORT', 'ONA')
             LIMIT 1",
            [cur],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()
    {
        if !visited.insert(pre_id) {
            break;
        }
        if pre_eps > 0 {
            prequel_offset += pre_eps;
        }
        cur = pre_id;
    }
    prequel_offset
}

/// Validate `linked_subtitles` rows for `(anilist_id, ep_key)` without rolling back valid retimed files (`H-12`).
/// Only recovers missing retimed files if deleted from disk and prunes clearly mismatched `pack/` rows after
/// checking `absolute_number`, TVDB season/episode, and franchise prequel offsets.
fn heal_linked_subtitles_for_episode(conn: &Connection, anilist_id: i64, ep_key: &str) {
    let Ok(mut stmt) = conn.prepare_cached(
        "SELECT id, path FROM linked_subtitles WHERE anilist_id = ?1 AND ep_key = ?2",
    ) else {
        return;
    };
    let rows: Vec<(i64, String)> = stmt
        .query_map(params![anilist_id, ep_key], |r| Ok((r.get(0)?, r.get(1)?)))
        .ok()
        .map(|it| it.flatten().collect())
        .unwrap_or_default();

    if rows.is_empty() {
        return;
    }

    let ep_meta: Option<(i64, Option<i64>, Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT number, tvdb_season, tvdb_episode, absolute_number
             FROM episodes WHERE anilist_id = ?1 AND ep_key = ?2 LIMIT 1",
            params![anilist_id, ep_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .ok()
        .flatten();
    let prequel_offset = compute_prequel_offset(conn, anilist_id);

    for (row_id, path_str) in rows {
        let p = Path::new(&path_str);
        let Some(parent) = p.parent() else { continue };
        let fname = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();

        // Only if a retimed cache file was deleted from disk while `pack/{orig_fname}` still exists, point back to it.
        let retimed_prefix = format!("ep{ep_key}_retimed_");
        if !p.is_file() {
            if let Some(orig_fname) = fname.strip_prefix(&retimed_prefix) {
                let pack_cand = parent.join("pack").join(orig_fname);
                if pack_cand.is_file() {
                    let clean_str = pack_cand.to_string_lossy().to_string();
                    let _ = conn.execute(
                        "UPDATE linked_subtitles SET path = ?1, retimed = 0, offset_ms = NULL WHERE id = ?2",
                        params![clean_str, row_id],
                    );
                }
            }
            continue;
        }

        // If this file came from `.../pack/{fname}`, verify its detected episode matches `ep_key`, `absolute_number`, TVDB, or prequel offset.
        if parent.file_name().and_then(|n| n.to_str()) == Some("pack") {
            if let Some(parsed) = parse_subtitle_filename(fname) {
                if let Some(det_ep) = parsed.episode {
                    let mut matches_ep = det_ep.to_string() == ep_key;
                    if !matches_ep {
                        if let Some((num, tvdb_s, tvdb_e, abs_num)) = ep_meta {
                            if det_ep == num || abs_num == Some(det_ep) {
                                matches_ep = true;
                            } else if let Some(s) = parsed.season {
                                if tvdb_s == Some(s) && tvdb_e == Some(det_ep) {
                                    matches_ep = true;
                                }
                            }
                            if !matches_ep
                                && prequel_offset > 0
                                && det_ep == num + prequel_offset
                            {
                                matches_ep = true;
                            }
                        } else if let Ok(ep_num) = ep_key.parse::<i64>() {
                            if prequel_offset > 0 && det_ep == ep_num + prequel_offset {
                                matches_ep = true;
                            }
                        }
                    }
                    if !matches_ep {
                        let _ = conn.execute("DELETE FROM linked_subtitles WHERE id = ?1", [row_id]);
                    }
                }
            }
        }
    }
}

/// Load external subtitle cues for an episode (either from linked subtitles in DB or sibling `.srt`/`.ass` files).
pub fn candidate_subtitle_paths_for_episode(
    conn: &Connection,
    anilist_id: i64,
    ep_key: &str,
) -> Vec<String> {
    heal_linked_subtitles_for_episode(conn, anilist_id, ep_key);
    episode_subtitle_paths(conn, anilist_id, ep_key)
}

pub fn load_cues_from_candidates(
    mut candidate_paths: Vec<String>,
    video_path: &str,
) -> (Option<String>, Vec<SubtitleCue>) {
    let vpath = Path::new(video_path);
    if let (Some(parent), Some(stem)) = (vpath.parent(), vpath.file_stem().and_then(|s| s.to_str()))
    {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let p = entry.path();
                if !p.is_file() {
                    continue;
                }
                let Some(ext) = p.extension().and_then(|e| e.to_str()) else {
                    continue;
                };
                if !SUB_EXTENSIONS.iter().any(|s| s.eq_ignore_ascii_case(ext)) {
                    continue;
                }
                let Some(fname) = p.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if fname.starts_with(stem) {
                    let p_str = p.to_string_lossy().to_string();
                    if !candidate_paths.contains(&p_str) {
                        candidate_paths.push(p_str);
                    }
                }
            }
        }
    }

    for sub_path in candidate_paths {
        let p = Path::new(&sub_path);
        if p.is_file() {
            let cues = parse_subtitle_file_cues(p);
            if !cues.is_empty() {
                return (Some(sub_path), cues);
            }
        }
    }
    (None, Vec::new())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_subtitle_filenames_and_languages() {
        // 1. Apothecary Diaries Netflix absolute S02E25 with [cc]
        let p1 =
            parse_subtitle_filename("薬屋のひとりごと.S02E25.猫猫と毛毛.WEBRip.Netflix.ja[cc].srt")
                .unwrap();
        assert_eq!(p1.season, Some(2));
        assert_eq!(p1.episode, Some(25));
        assert_eq!(p1.lang.as_deref(), Some("ja"));
        assert!(p1.cc);
        assert_eq!(p1.ext, "srt");

        // 2. Azumanga Daioh #08
        let p2 = parse_subtitle_filename("Azumanga Daioh #08 [DVD].ja.ass").unwrap();
        assert_eq!(p2.episode, Some(8));
        assert_eq!(p2.lang.as_deref(), Some("ja"));
        assert!(!p2.cc);

        // 3. Bare 25.ja.srt
        let p3 = parse_subtitle_filename("25.ja.srt").unwrap();
        assert_eq!(p3.episode, Some(25));
        assert_eq!(p3.lang.as_deref(), Some("ja"));

        // 4. Dash pattern must NOT read "- 1080p" as episode 1080
        let p4 = parse_subtitle_filename("[SubsPlease] Frieren - 14 (1080p) [ABCD1234].jpn.ass")
            .unwrap();
        assert_eq!(p4.episode, Some(14));
        assert_eq!(p4.lang.as_deref(), Some("ja"));

        // 5. Movie or title with "- 1080p" and no episode number
        let p5 = parse_subtitle_filename("Look Back (2024) - 1080p.ja.srt").unwrap();
        assert_eq!(p5.episode, None);

        // 6. Strict CC detection does not false-trigger on words containing "cc"
        let p6 = parse_subtitle_filename("Eccentric Family - 03.ja.srt").unwrap();
        assert!(!p6.cc);
        assert_eq!(p6.episode, Some(3));
    }

    #[test]
    fn resolves_absolute_episode_numbers_via_anizip_and_offset() {
        let targets = vec![
            KnownEpisodeTarget {
                ep_key: "1".into(),
                number: 1,
                tvdb_season: Some(2),
                tvdb_episode: Some(1),
                absolute_number: Some(25),
                video_path: r"C:\Anime\Kusuriya S2\01.mkv".into(),
            },
            KnownEpisodeTarget {
                ep_key: "2".into(),
                number: 2,
                tvdb_season: Some(2),
                tvdb_episode: Some(2),
                absolute_number: Some(26),
                video_path: r"C:\Anime\Kusuriya S2\02.mkv".into(),
            },
        ];

        // Absolute S02E25 -> resolves to Ep 1
        let (t, note) = resolve_episode_target(Some(2), 25, &targets, 24).unwrap();
        assert_eq!(t.ep_key, "1");
        assert!(note.unwrap().contains("Absolute #25"));

        // Bare 26.ja.srt -> resolves to Ep 2
        let (t2, _) = resolve_episode_target(None, 26, &targets, 24).unwrap();
        assert_eq!(t2.ep_key, "2");

        // Absolute #12 belongs to Season 1 -> rejected with clear reason
        let err = resolve_episode_target(None, 12, &targets, 24).unwrap_err();
        assert!(err.contains("earlier season"));
    }

    #[test]
    fn aligns_constant_and_piecewise_eyecatch_subtitle_shifts() {
        // Reference spans: 20 cues before commercial break (t = 10s..110s) and 20 cues after (t = 200s..300s)
        let mut reference = Vec::new();
        for i in 0..20 {
            let s = 10_000 + i * 5_000;
            reference.push(Span {
                start_ms: s,
                end_ms: s + 2_200,
            });
        }
        for i in 0..20 {
            let s = 200_000 + i * 5_000;
            reference.push(Span {
                start_ms: s,
                end_ms: s + 2_200,
            });
        }

        // Case A: Target subtitle is shifted by -2450ms (needs +2450ms to match reference)
        let target_const: Vec<Span> = reference
            .iter()
            .map(|r| Span {
                start_ms: r.start_ms - 2_450,
                end_ms: r.end_ms - 2_450,
            })
            .collect();
        let model_a = align_subtitle_spans(&target_const, &reference).unwrap();
        assert!(
            (model_a.primary_offset_ms - 2_450).abs() <= 20,
            "got {}",
            model_a.primary_offset_ms
        );
        assert!(model_a.matched_cues >= 35);

        // Case B: Piecewise commercial-break shift!
        // First half is off by +1500ms, second half has an extra 6000ms sponsor/eyecatch break (+7500ms total)
        let mut target_split = Vec::new();
        for r in &reference[..20] {
            target_split.push(Span {
                start_ms: r.start_ms - 1_500,
                end_ms: r.end_ms - 1_500,
            });
        }
        for r in &reference[20..] {
            target_split.push(Span {
                start_ms: r.start_ms - 7_500,
                end_ms: r.end_ms - 7_500,
            });
        }
        let model_b = align_subtitle_spans(&target_split, &reference).unwrap();
        assert!(
            model_b.segments.len() >= 2,
            "Expected piecewise split, got {:?}",
            model_b.segments
        );
        let mapped_first = model_b.apply(target_split[0].start_ms);
        let mapped_last = model_b.apply(target_split[39].start_ms);
        assert!((mapped_first - reference[0].start_ms).abs() <= 30);
        assert!((mapped_last - reference[39].start_ms).abs() <= 30);
    }

    #[test]
    fn rewrites_srt_and_ass_preserving_formatting() {
        let srt = "1\r\n00:00:05,000 --> 00:00:07,500\r\nこんにちは\r\n";
        let out_srt = rewrite_subtitle_timestamps(srt, "srt", |ms| ms + 2_150);
        assert!(out_srt.contains("00:00:07,150 --> 00:00:09,650"));
        assert!(out_srt.contains("こんにちは"));

        let ass = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:10.00,0:00:12.50,Default,,0,0,0,,{\\blur2}猫猫（マオマオ）";
        let out_ass = rewrite_subtitle_timestamps(ass, "ass", |ms| ms - 1_250);
        assert!(out_ass.contains(
            "Dialogue: 0,0:00:08.75,0:00:11.25,Default,,0,0,0,,{\\blur2}猫猫（マオマオ）"
        ));
    }

    #[test]
    fn decompresses_zlib_mkv_ass_and_utf8_blocks() {
        use std::io::Write;
        let raw_ass = "0,0,Default,,0,0,0,,{\\blur1.5}魔法使いフリーレン\\N新しい旅";
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(raw_ass.as_bytes()).unwrap();
        let compressed = enc.finish().unwrap();

        // Starts with `0x78 0x9C` (`x...`)
        assert_eq!(compressed[0], 0x78);

        let cleaned = clean_mkv_block_text(&compressed, true);
        assert_eq!(cleaned, "魔法使いフリーレン\n新しい旅");
    }

    #[test]
    fn parses_timestamps_with_trailing_non_ascii_and_rejects_malformed_zips() {
        assert_eq!(parse_srt_vtt_time("00:01:02,5（注）"), Some(62_500));
        assert_eq!(parse_srt_vtt_time("01:02.34日本語"), Some(62_340));
        assert_eq!(parse_ass_time("0:01:02.5（注）"), Some(62_500));
        assert_eq!(parse_ass_time("0:01:02.75テスト"), Some(62_750));

        // Malformed EOCD / central directory offsets must never panic
        let mut fake_zip = vec![0u8; 30];
        fake_zip[8..12].copy_from_slice(b"PK\x05\x06");
        fake_zip[18..20].copy_from_slice(&5u16.to_le_bytes());
        fake_zip[24..28].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        assert!(iter_zip_entries(&fake_zip).is_err());
    }

    #[test]
    fn persists_subtitle_in_db_and_cleans_up_after_watched() {
        let conn = Connection::open_in_memory().unwrap();
        db::migrate_for_tests(&conn).unwrap();

        let tmp_dir = std::env::temp_dir().join(format!("kura_sub_test_{}", std::process::id()));
        let sub_dir = tmp_dir.join("subtitles").join("154587");
        let pack_dir = sub_dir.join("pack");
        std::fs::create_dir_all(&pack_dir).unwrap();
        let sub_file = sub_dir.join("ep1_retimed_25.ja.srt");
        let pack_file = pack_dir.join("25.ja.srt");
        std::fs::write(&sub_file, "1\n00:00:01,000 --> 00:00:03,000\nフリーレン\n").unwrap();
        std::fs::write(&pack_file, "1\n00:00:00,500 --> 00:00:02,500\nフリーレン\n").unwrap();

        conn.execute(
            "INSERT INTO media (anilist_id, title_romaji, fetched_at) VALUES (154587, 'Sousou no Frieren', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO episodes (anilist_id, ep_key, number, absolute_number, is_special) VALUES (154587, '1', 1, 25, 0)",
            [],
        )
        .unwrap();

        let sub_path_str = sub_file.to_string_lossy().to_string();
        upsert_linked_subtitle(
            &conn,
            154587,
            "1",
            &sub_path_str,
            "ja",
            false,
            "jimaku",
            true,
            Some(500),
        )
        .unwrap();

        // Retimed file is preserved by `candidate_subtitle_paths_for_episode` (not rolled back!)
        let cands = candidate_subtitle_paths_for_episode(&conn, 154587, "1");
        assert_eq!(cands, vec![sub_path_str.clone()]);
        assert!(sub_file.is_file());
        assert!(has_saved_subtitle(&conn, 154587, "1"));

        // While unwatched (`watched_at IS NULL`), `cleanup_watched_subtitles` keeps it
        assert_eq!(cleanup_watched_subtitles(&conn, &tmp_dir), 0);
        assert!(has_saved_subtitle(&conn, 154587, "1"));

        // Mark episode as watched -> `cleanup_watched_subtitles` deletes the file and DB row
        conn.execute(
            "INSERT INTO watch_state (anilist_id, ep_key, watched_at) VALUES (154587, '1', 1700000000)",
            [],
        )
        .unwrap();
        assert_eq!(cleanup_watched_subtitles(&conn, &tmp_dir), 1);
        assert!(!sub_file.is_file());
        assert!(!has_saved_subtitle(&conn, 154587, "1"));

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn vtt_reference_retiming_handles_short_timestamps_and_commercial_breaks() {
        let mut vtt = String::from("WEBVTT\n\n");
        let mut jp_ass = String::from(
            "[Script Info]\nTitle: JP Broadcast\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        );

        // Stream WebVTT uses short `MM:SS.mmm` timestamps with `<c.yellow>` markup and cue settings
        // JP broadcast `.ass` starts 3.20s earlier in Part A and has a +10.00s sponsor/commercial break at line 20
        for i in 0..40 {
            let ref_start_ms = 5_000 + i * 4_100 + ((i * 733) % 1_700);
            let ref_end_ms = ref_start_ms + 1_400 + ((i * 317) % 1_500);
            let m1 = ref_start_ms / 60_000;
            let s1 = (ref_start_ms % 60_000) / 1_000;
            let ms1 = ref_start_ms % 1_000;
            let m2 = ref_end_ms / 60_000;
            let s2 = (ref_end_ms % 60_000) / 1_000;
            let ms2 = ref_end_ms % 1_000;
            vtt.push_str(&format!(
                "{m1:02}:{s1:02}.{ms1:03} --> {m2:02}:{s2:02}.{ms2:03} line:85%\n<c.white>English cue {i}</c>\n\n"
            ));

            let jp_shift = if i < 20 { -3_200 } else { 6_800 };
            let jp_s = ref_start_ms + jp_shift;
            let jp_e = ref_end_ms + jp_shift;
            jp_ass.push_str(&format!(
                "Dialogue: 0,{},{},Default,,0,0,0,,日本語のセリフ {i}\n",
                format_ass_time(jp_s),
                format_ass_time(jp_e)
            ));
        }

        let vtt_cues = parse_subtitle_cues(&vtt, "vtt");
        assert_eq!(vtt_cues.len(), 40);
        assert_eq!(vtt_cues[0].text, "English cue 0");

        let tmp_dir = std::env::temp_dir().join(format!("kura_vtt_retime_{}", now()));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let jp_in = tmp_dir.join("ep1_orig.ass");
        let jp_out = tmp_dir.join("ep1_streamsync.ass");
        std::fs::write(&jp_in, jp_ass.as_bytes()).unwrap();

        let report =
            retime_subtitle_against_reference_cues(&jp_in, &vtt_cues, &jp_out, "Stream VTT")
                .unwrap();
        assert!(report.segments >= 2);
        assert!(report.matched_cues >= 35);

        let aligned_cues = parse_subtitle_file_cues(&jp_out);
        assert_eq!(aligned_cues.len(), 40);
        assert!((aligned_cues[0].start_ms - vtt_cues[0].start_ms).abs() <= 30);
        assert!((aligned_cues[39].start_ms - vtt_cues[39].start_ms).abs() <= 30);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn saturating_mkv_ebml_math_and_shared_read_allow_concurrent_rename() {
        // Huge EBML timestamps / scales saturate cleanly without panic or negative wrap
        assert_eq!(scale_mkv_ticks_to_ms(0, -500, 1_000_000), 0);
        assert_eq!(scale_mkv_ticks_to_ms(1_500, -500, 1_000_000), 1_000);
        let huge = scale_mkv_ticks_to_ms(u64::MAX, i16::MAX, u64::MAX);
        assert_eq!(huge, MAX_MKV_TIME_MS);
        let huge_dur = scale_mkv_duration_to_ms(u64::MAX, u64::MAX);
        assert_eq!(huge_dur, MAX_MKV_TIME_MS);

        // `open_shared_read` permits concurrent `std::fs::rename` even while handle is open on Windows
        let tmp_dir = std::env::temp_dir().join(format!(
            "kura_share_del_{}",
            crate::db::random_hex_token(6)
        ));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let part_path = tmp_dir.join("ep01.mkv.part");
        let final_path = tmp_dir.join("ep01.mkv");
        std::fs::write(&part_path, b"matroska test payload").unwrap();

        let _open_reader = open_shared_read(&part_path).unwrap();
        std::fs::rename(&part_path, &final_path).unwrap();
        assert!(final_path.is_file());
        drop(_open_reader);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
