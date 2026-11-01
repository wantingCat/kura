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
use std::io::{Read, Seek, SeekFrom};
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
    let _ = conn.execute_batch(db::SCHEMA_V6);
    let mut stmt = conn.prepare(
        "SELECT id, anilist_id, ep_key, path, lang, cc, source, retimed, offset_ms
         FROM linked_subtitles
         WHERE anilist_id = ?1
         ORDER BY CAST(ep_key AS INTEGER), id",
    )?;
    let rows = stmt
        .query_map([anilist_id], |r| {
            let path: String = r.get(3)?;
            let file_name = Path::new(&path)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| path.clone());
            let exists = Path::new(&path).is_file();
            Ok(LinkedSubtitle {
                id: r.get(0)?,
                anilist_id: r.get(1)?,
                ep_key: r.get(2)?,
                path,
                file_name,
                lang: r.get(4)?,
                cc: r.get::<_, i64>(5)? != 0,
                source: r.get(6)?,
                retimed: r.get::<_, i64>(7)? != 0,
                offset_ms: r.get(8)?,
                exists,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn episode_subtitle_paths(conn: &Connection, anilist_id: i64, ep_key: &str) -> Vec<String> {
    let _ = conn.execute_batch(db::SCHEMA_V6);
    let mut stmt = match conn.prepare(
        "SELECT path FROM linked_subtitles
         WHERE anilist_id = ?1 AND ep_key = ?2
         ORDER BY retimed DESC, id DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = match stmt.query_map(params![anilist_id, ep_key], |r| r.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    rows.flatten().filter(|p| Path::new(p).is_file()).collect()
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
    let _ = conn.execute_batch(db::SCHEMA_V6);
    conn.execute(
        "INSERT INTO linked_subtitles (anilist_id, ep_key, path, lang, cc, source, retimed, offset_ms, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(anilist_id, ep_key, path) DO UPDATE SET
            lang = excluded.lang,
            cc = excluded.cc,
            source = excluded.source,
            retimed = excluded.retimed,
            offset_ms = COALESCE(excluded.offset_ms, linked_subtitles.offset_ms),
            created_at = excluded.created_at",
        params![anilist_id, ep_key, path, lang, cc as i64, source, retimed as i64, offset_ms, now()],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn remove_linked_subtitle(conn: &Connection, id: i64) -> Result<()> {
    let _ = conn.execute_batch(db::SCHEMA_V6);
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
    let re = RE_CC.get_or_init(|| {
        Regex::new(r"(?i)(?:[\[\(\._\-\s](?:cc|sdh)(?:[\]\)\._\-\s]|$))").unwrap()
    });
    re.is_match(stem)
}

/// Detect subtitle language code from filename tags/tokens (`ja`, `en`, `zh`, `ko`, `de`, `fr`, `es`).
pub fn detect_sub_language(stem: &str) -> Option<String> {
    if stem.contains("日本語") || stem.contains("日文") {
        return Some("ja".to_string());
    }
    if stem.contains("简体") || stem.contains("繁體") || stem.contains("繁体") || stem.contains("中字") {
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
        let check = if clean.is_empty() { tok.as_str() } else { clean };
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
        Regex::new(r"(?i)(?:^|[\s._\-\[\(])S(\d{1,2})[\s._\-]*E(\d{1,4})(?:v\d)?(?:\b|[\s._\-\]\)]|$)").unwrap()
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
        Regex::new(r"(?i)(?:^|[\s._\-\[\(])(?:episode|ep|e)\.?\s*0*(\d{1,4})(?:v\d)?(?:\b|[\s._\-\]\)]|$)").unwrap()
    });
    if let Some(c) = re_ep.captures(stem) {
        if let Ok(ep) = c[1].parse::<i64>() {
            return (None, Some(ep));
        }
    }

    // 4. #08 (e.g. Azumanga Daioh #08)
    let re_hash = RE_HASH.get_or_init(|| Regex::new(r"#\s*0*(\d{1,4})(?:v\d)?(?:\b|[\s._\-\]\)]|$)").unwrap());
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
        Regex::new(r"[\s._][\-–—][\s._]*0*(\d{1,4})(?:v\d)?(?:\s*(?:END|FIN))?(?:[\s._\-\[\(]|$)").unwrap()
    });
    for c in re_dash.captures_iter(stem) {
        let m = c.get(1).unwrap();
        let after = &stem[m.end()..];
        let next_char = after.chars().next().map(|ch| ch.to_ascii_lowercase());
        if matches!(next_char, Some('p' | 'i' | 'k' | 'b' | 'f')) {
            continue;
        }
        if let Ok(ep) = m.as_str().parse::<i64>() {
            if !matches!(ep, 264 | 265 | 480 | 576 | 720 | 1080 | 1440 | 2160) && !(1970..=2099).contains(&ep) {
                return (None, Some(ep));
            }
        }
    }

    // 7. Bracketed number: [08]
    let re_bracket = RE_BRACKET.get_or_init(|| Regex::new(r"\[\s*0*(\d{1,4})(?:v\d)?\s*\]").unwrap());
    for c in re_bracket.captures_iter(stem) {
        if let Ok(ep) = c[1].parse::<i64>() {
            if !matches!(ep, 264 | 265 | 480 | 576 | 720 | 1080 | 1440 | 2160) && !(1970..=2099).contains(&ep) {
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
    Some(ParsedSubtitleFile { season, episode, lang, cc, ext })
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
        let direct: Vec<&KnownEpisodeTarget> = targets.iter().filter(|t| t.number == parsed_ep).collect();
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
            return Ok((t, Some(format!("Absolute #{parsed_ep} (−{prequel_offset}) → Ep {}", t.number))));
        }
    }

    if prequel_offset > 0 && parsed_ep <= prequel_offset {
        return Err(format!(
            "Absolute episode #{parsed_ep} belongs to an earlier season (starts after #{prequel_offset})"
        ));
    }

    Err(format!("No owned episode matches #{parsed_ep}"))
}

fn load_series_targets(conn: &Connection, anilist_id: i64) -> Result<(Vec<KnownEpisodeTarget>, i64, Vec<PathBuf>)> {
    let mut files_by_ep: BTreeMap<String, String> = BTreeMap::new();
    let mut folders: HashSet<PathBuf> = HashSet::new();
    {
        let mut stmt = conn.prepare(
            "SELECT ep_key, path FROM local_files
             WHERE anilist_id = ?1 AND ep_key IS NOT NULL AND ep_key NOT LIKE 'S%'
             ORDER BY path",
        )?;
        let rows = stmt.query_map([anilist_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
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
    let stem = video_path.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let cc_part = if cc { ".cc" } else { "" };
    let lang_part = if lang.trim().is_empty() { "ja" } else { lang.trim() };
    parent.join(format!("{stem}.{lang_part}{cc_part}.{ext}"))
}

pub fn preview_subtitle_matches(
    conn: &Connection,
    anilist_id: i64,
    extra_folder: Option<&Path>,
    default_lang: &str,
) -> Result<SubtitleMatchPreview> {
    let _ = conn.execute_batch(db::SCHEMA_V6);
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
                let already_linked = existing_links.contains(&source_path.replace('\\', "/").to_lowercase())
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

    matches.sort_by(|a, b| a.episode_number.cmp(&b.episode_number).then(a.source_name.cmp(&b.source_name)));

    Ok(SubtitleMatchPreview {
        anilist_id,
        scanned_folders: folders.into_iter().map(|f| f.to_string_lossy().to_string()).collect(),
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

    for item in items {
        let src = Path::new(&item.source_path);
        if !src.is_file() {
            report.errors.push(format!("Missing file: {}", item.source_name));
            continue;
        }

        let mut active_sub_path = if rename_mode {
            let dst = Path::new(&item.target_path);
            if src != dst {
                if dst.exists() {
                    // Never overwrite an existing subtitle file; link it instead.
                    dst.to_path_buf()
                } else {
                    match std::fs::rename(src, dst).or_else(|_| std::fs::copy(src, dst).map(|_| ())) {
                        Ok(()) => {
                            report.renamed += 1;
                            dst.to_path_buf()
                        }
                        Err(e) => {
                            report.errors.push(format!("Couldn't rename {}: {e}", item.source_name));
                            continue;
                        }
                    }
                }
            } else {
                dst.to_path_buf()
            }
        } else {
            src.to_path_buf()
        };

        let mut was_retimed = false;
        let mut offset_ms = None;

        if auto_retime && Path::new(&item.video_path).is_file() {
            let out_dest = if rename_mode {
                active_sub_path.clone()
            } else {
                let fname = active_sub_path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("ep_{}.srt", item.ep_key));
                cache_dir.join(format!("ep{}_{fname}", item.ep_key))
            };
            if let Ok(rt) = retime_subtitle_file(&active_sub_path, Path::new(&item.video_path), &out_dest) {
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
        3 => (parts[0].parse::<i64>().ok()?, parts[1].parse::<i64>().ok()?, parts[2]),
        2 => (0, parts[0].parse::<i64>().ok()?, parts[1]),
        _ => return None,
    };
    let (sec_str, ms_str) = sec_ms.split_once([',', '.'])?;
    let sec = sec_str.parse::<i64>().ok()?;
    let ms_norm = match ms_str.len() {
        1 => ms_str.parse::<i64>().ok()? * 100,
        2 => ms_str.parse::<i64>().ok()? * 10,
        _ => ms_str[..3.min(ms_str.len())].parse::<i64>().ok()?,
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
    let cs_ms = match cs_str.len() {
        1 => cs_str.parse::<i64>().ok()? * 100,
        2 => cs_str.parse::<i64>().ok()? * 10,
        _ => cs_str[..3.min(cs_str.len())].parse::<i64>().ok()?,
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

/// Extract timing spans from `.srt`, `.vtt`, or `.ass`/`.ssa` content.
pub fn parse_subtitle_spans(content: &str, ext: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let is_ass = matches!(ext.to_ascii_lowercase().as_str(), "ass" | "ssa");

    if is_ass {
        for line in content.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("Dialogue:") {
                let fields: Vec<&str> = rest.splitn(10, ',').collect();
                if fields.len() >= 10 {
                    let style = fields[3].trim().to_ascii_lowercase();
                    let text = fields[9].trim();
                    // Ignore pure vector drawing commands (\p1) or empty lines when building alignment cues
                    if text.is_empty() || text.contains("\\p1") || style.contains("sign") || style.contains("op") || style.contains("ed") {
                        continue;
                    }
                    if let (Some(s), Some(e)) = (parse_ass_time(fields[1]), parse_ass_time(fields[2])) {
                        if e > s && (e - s) <= 15_000 {
                            spans.push(Span { start_ms: s, end_ms: e });
                        }
                    }
                }
            }
        }
        // Fallback if all lines had OP/Sign styles
        if spans.is_empty() {
            for line in content.lines() {
                if let Some(rest) = line.trim_start().strip_prefix("Dialogue:") {
                    let fields: Vec<&str> = rest.splitn(10, ',').collect();
                    if fields.len() >= 3 {
                        if let (Some(s), Some(e)) = (parse_ass_time(fields[1]), parse_ass_time(fields[2])) {
                            if e > s && (e - s) <= 15_000 {
                                spans.push(Span { start_ms: s, end_ms: e });
                            }
                        }
                    }
                }
            }
        }
    } else {
        for line in content.lines() {
            if let Some((left, right)) = line.split_once("-->") {
                let right_time = right.split_whitespace().next().unwrap_or(right);
                if let (Some(s), Some(e)) = (parse_srt_vtt_time(left), parse_srt_vtt_time(right_time)) {
                    if e > s && (e - s) <= 15_000 {
                        spans.push(Span { start_ms: s, end_ms: e });
                    }
                }
            }
        }
    }

    spans.sort_by_key(|s| s.start_ms);
    // Deduplicate identical/simultaneous typesetting spans
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
    let line_ending = if content.contains("\r\n") { "\r\n" } else { "\n" };

    let mut out_lines = Vec::new();
    for line in content.lines() {
        if is_ass {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("Dialogue:") {
                let fields: Vec<&str> = rest.splitn(10, ',').collect();
                if fields.len() == 10 {
                    if let (Some(s), Some(e)) = (parse_ass_time(fields[1]), parse_ass_time(fields[2])) {
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
/// 1. Pairwise start-time difference histogram to discover candidate offsets (supporting shifts up to ±120s).
/// 2. Alass-style Dynamic Programming over `(cue_index, candidate_offset)` with split penalty
///    to handle both constant lead-in offsets AND mid-episode commercial break / eyecatch / OP shifts.
/// 3. Fine 10ms local refinement per segment.
pub fn align_subtitle_spans(target: &[Span], reference: &[Span]) -> Option<AlignmentModel> {
    if target.is_empty() || reference.is_empty() {
        return None;
    }

    // Step 1: Build a histogram of start-time differences `r.start_ms - t.start_ms` in 50ms buckets
    // within [-120_000ms, +120_000ms].
    const MAX_SHIFT_MS: i64 = 120_000;
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
            // Similar durations are more likely to be the same line of dialogue
            let dur_diff = (t.len() - r.len()).abs();
            if dur_diff > 3_500 {
                continue;
            }
            let diff = r.start_ms - t.start_ms;
            let b = (diff + if diff >= 0 { BUCKET_MS / 2 } else { -BUCKET_MS / 2 }) / BUCKET_MS;
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

    // Pick top 12 distinct candidate offsets (at least 350ms apart)
    let mut ranked: Vec<(i64, i64)> = hist.into_iter().map(|(b, w)| (b * BUCKET_MS, w)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    let mut candidates: Vec<i64> = Vec::new();
    for (off, _) in ranked {
        if candidates.iter().all(|&c| (c - off).abs() >= 350) {
            candidates.push(off);
            if candidates.len() >= 12 {
                break;
            }
        }
    }
    if !candidates.iter().any(|&c| c.abs() < 200) {
        candidates.push(0);
    }

    // Refine each candidate offset to 10ms precision globally
    for cand in &mut candidates {
        let base = *cand;
        let mut best_off = base;
        let mut best_score = -1i64;
        for delta in (-150..=150).step_by(10) {
            let off = base + delta;
            let sc: i64 = target.iter().map(|&sp| span_overlap_score(sp, off, reference)).sum();
            if sc > best_score {
                best_score = sc;
                best_off = off;
            }
        }
        *cand = best_off;
    }
    candidates.sort();
    candidates.dedup_by(|a, b| (*a - *b).abs() < 40);

    // Step 2: Dynamic Programming over `n` target cues and `k` candidate offsets.
    // Penalty for switching offset mid-episode prevents noisy per-line jumps while allowing real eyecatch/OP cuts.
    let n = target.len();
    let k = candidates.len();
    let total_single_best: i64 = candidates
        .iter()
        .map(|&off| target.iter().map(|&sp| span_overlap_score(sp, off, reference)).sum::<i64>())
        .max()
        .unwrap_or(0);
    let split_penalty = ((total_single_best / 12).max(4_500)).min(18_000);

    let mut dp = vec![vec![i64::MIN / 4; k]; n];
    let mut back = vec![vec![0usize; k]; n];

    for j in 0..k {
        dp[0][j] = span_overlap_score(target[0], candidates[j], reference);
    }

    for i in 1..n {
        // Larger gap between target cues (e.g. >= 5s commercial break / OP) reduces split penalty slightly
        let gap_ms = (target[i].start_ms - target[i - 1].end_ms).max(0);
        let penalty = if gap_ms >= 5_000 {
            split_penalty * 2 / 3
        } else {
            split_penalty
        };

        for j in 0..k {
            let cue_sc = span_overlap_score(target[i], candidates[j], reference);
            let mut best_prev = dp[i - 1][j];
            let mut best_idx = j;
            for p in 0..k {
                if p == j {
                    continue;
                }
                let cand_val = dp[i - 1][p] - penalty;
                if cand_val > best_prev {
                    best_prev = cand_val;
                    best_idx = p;
                }
            }
            dp[i][j] = best_prev + cue_sc;
            back[i][j] = best_idx;
        }
    }

    // Backtrack
    let mut cur_j = (0..k).max_by_key(|&j| dp[n - 1][j]).unwrap_or(0);
    let mut path = vec![0usize; n];
    path[n - 1] = cur_j;
    for i in (1..n).rev() {
        cur_j = back[i][cur_j];
        path[i - 1] = cur_j;
    }

    // Build segments and count matched cues
    let mut segments: Vec<(i64, i64)> = Vec::new();
    let mut off_counts: HashMap<i64, usize> = HashMap::new();
    let mut matched_cues = 0usize;

    for (i, &j) in path.iter().enumerate() {
        let off = candidates[j];
        *off_counts.entry(off).or_insert(0) += 1;
        if span_overlap_score(target[i], off, reference) > 150 {
            matched_cues += 1;
        }
        if segments.last().map(|&(_, last_off)| last_off) != Some(off) {
            let seg_start = if i == 0 { 0 } else { target[i].start_ms };
            segments.push((seg_start, off));
        }
    }

    let primary_offset_ms = off_counts
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .map(|(off, _)| off)
        .unwrap_or(0);

    Some(AlignmentModel {
        segments,
        primary_offset_ms,
        matched_cues,
    })
}

// ---------------------------------------------------------------------------
// Pure-Rust Matroska (.mkv) EBML Subtitle Cue Extractor
// ---------------------------------------------------------------------------

/// Read an EBML variable-length integer (VINT).
/// If `keep_marker` is true, returns the raw Element ID (with the leading VINT marker bit intact).
/// If `keep_marker` is false, masks out the VINT marker bit to return the data size / value.
fn read_ebml_vint<R: Read>(r: &mut R, keep_marker: bool) -> std::io::Result<(u64, usize)> {
    let mut first = [0u8; 1];
    r.read_exact(&mut first)?;
    let b = first[0];
    if b == 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "zero VINT prefix"));
    }
    let len = b.leading_zeros() as usize + 1;
    if len > 8 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "VINT > 8 bytes"));
    }
    let mask = if keep_marker { 0xFFu8 } else { (1u8 << (8 - len)) - 1 };
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
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "uint > 8 bytes"));
    }
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf[..size])?;
    let mut val = 0u64;
    for &b in &buf[..size] {
        val = (val << 8) | (b as u64);
    }
    Ok(val)
}

/// Extract subtitle timing spans `(start_ms, end_ms)` directly from a Matroska (`.mkv`) file
/// in pure Rust without needing `ffmpeg` or external binaries.
pub fn extract_mkv_subtitle_spans(mkv_path: &Path) -> Result<(Vec<Span>, String)> {
    let mut file = File::open(mkv_path)?;
    let file_len = file.metadata()?.len();

    // Verify EBML magic `0x1A45DFA3`
    let (ebml_id, _) = read_ebml_vint(&mut file, true)?;
    if ebml_id != 0x1A45DFA3 {
        bail!("Not an EBML/Matroska file");
    }
    let (ebml_size, _) = read_ebml_vint(&mut file, false)?;
    file.seek(SeekFrom::Current(ebml_size as i64))?;

    // Find Segment `0x18538067`
    let (seg_id, _) = read_ebml_vint(&mut file, true)?;
    if seg_id != 0x18538067 {
        bail!("Matroska Segment not found");
    }
    let (_seg_size, _) = read_ebml_vint(&mut file, false)?;

    let mut timestamp_scale: u64 = 1_000_000; // default 1ms in nanoseconds
    let mut sub_tracks: Vec<(u64, String)> = Vec::new();
    let mut spans_by_track: HashMap<u64, Vec<Span>> = HashMap::new();

    while file.stream_position()? < file_len {
        let elem_pos = file.stream_position()?;
        let Ok((id, _)) = read_ebml_vint(&mut file, true) else { break };
        let Ok((size, _)) = read_ebml_vint(&mut file, false) else { break };
        let data_start = file.stream_position()?;
        let is_unknown_size = size == 0x00FF_FFFF_FFFF_FFFF || size == 0x7FFF_FFFF_FFFF_FFFF;
        let elem_end = if is_unknown_size { file_len } else { (data_start + size).min(file_len) };

        match id {
            // Segment Info (0x1549A966)
            0x1549A966 => {
                while file.stream_position()? < elem_end {
                    let Ok((sub_id, _)) = read_ebml_vint(&mut file, true) else { break };
                    let Ok((sub_sz, _)) = read_ebml_vint(&mut file, false) else { break };
                    let sub_start = file.stream_position()?;
                    if sub_id == 0x2AD7B1 && sub_sz <= 8 {
                        if let Ok(scale) = read_ebml_uint(&mut file, sub_sz as usize) {
                            if scale > 0 {
                                timestamp_scale = scale;
                            }
                        }
                    }
                    let _ = file.seek(SeekFrom::Start(sub_start + sub_sz));
                }
                let _ = file.seek(SeekFrom::Start(elem_end));
            }
            // Tracks (0x1654AE6B)
            0x1654AE6B => {
                while file.stream_position()? < elem_end {
                    let Ok((entry_id, _)) = read_ebml_vint(&mut file, true) else { break };
                    let Ok((entry_sz, _)) = read_ebml_vint(&mut file, false) else { break };
                    let entry_start = file.stream_position()?;
                    let entry_end = (entry_start + entry_sz).min(elem_end);
                    if entry_id == 0xAE {
                        // TrackEntry
                        let mut track_num = 0u64;
                        let mut track_type = 0u64;
                        let mut lang = String::from("und");
                        while file.stream_position()? < entry_end {
                            let Ok((tid, _)) = read_ebml_vint(&mut file, true) else { break };
                            let Ok((tsz, _)) = read_ebml_vint(&mut file, false) else { break };
                            let tstart = file.stream_position()?;
                            match tid {
                                0xD7 if tsz <= 8 => track_num = read_ebml_uint(&mut file, tsz as usize).unwrap_or(0),
                                0x83 if tsz <= 8 => track_type = read_ebml_uint(&mut file, tsz as usize).unwrap_or(0),
                                0x22B59C | 0x22B59D if tsz <= 32 => {
                                    let mut b = vec![0u8; tsz as usize];
                                    if file.read_exact(&mut b).is_ok() {
                                        let s = String::from_utf8_lossy(&b).trim_matches('\0').trim().to_string();
                                        if !s.is_empty() {
                                            lang = s;
                                        }
                                    }
                                }
                                _ => {}
                            }
                            let _ = file.seek(SeekFrom::Start(tstart + tsz));
                        }
                        // 0x11 == Subtitle track in Matroska spec
                        if track_type == 0x11 && track_num > 0 {
                            sub_tracks.push((track_num, lang));
                        }
                    }
                    let _ = file.seek(SeekFrom::Start(entry_end));
                }
                let _ = file.seek(SeekFrom::Start(elem_end));
                if sub_tracks.is_empty() {
                    bail!("No embedded subtitle tracks in MKV");
                }
            }
            // Cluster (0x1F43B675)
            0x1F43B675 => {
                if sub_tracks.is_empty() {
                    // Haven't seen Tracks yet or none exist
                    let _ = file.seek(SeekFrom::Start(elem_end));
                    continue;
                }
                let mut cluster_ts: i64 = 0;
                while file.stream_position()? < elem_end {
                    let Ok((cid, _)) = read_ebml_vint(&mut file, true) else { break };
                    let Ok((csz, _)) = read_ebml_vint(&mut file, false) else { break };
                    let cstart = file.stream_position()?;
                    let cend = (cstart + csz).min(elem_end);
                    match cid {
                        // Cluster Timestamp (0xE7)
                        0xE7 if csz <= 8 => {
                            cluster_ts = read_ebml_uint(&mut file, csz as usize).unwrap_or(0) as i64;
                        }
                        // SimpleBlock (0xA3)
                        0xA3 if csz >= 4 => {
                            if let Ok((tnum, vlen)) = read_ebml_vint(&mut file, false) {
                                if sub_tracks.iter().any(|(n, _)| *n == tnum) {
                                    let mut hdr = [0u8; 3];
                                    if file.read_exact(&mut hdr).is_ok() {
                                        let rel_ts = i16::from_be_bytes([hdr[0], hdr[1]]) as i64;
                                        let abs_ticks = (cluster_ts + rel_ts).max(0) as u64;
                                        let start_ms = ((abs_ticks * timestamp_scale) / 1_000_000) as i64;
                                        let end_ms = start_ms + 2_200;
                                        let _ = vlen;
                                        spans_by_track.entry(tnum).or_default().push(Span { start_ms, end_ms });
                                    }
                                }
                            }
                        }
                        // BlockGroup (0xA0) — standard container for subtitle blocks with BlockDuration (0x9B)
                        0xA0 => {
                            let mut blk_track = 0u64;
                            let mut blk_rel_ts: i64 = 0;
                            let mut blk_dur_ticks: Option<u64> = None;
                            while file.stream_position()? < cend {
                                let Ok((bid, _)) = read_ebml_vint(&mut file, true) else { break };
                                let Ok((bsz, _)) = read_ebml_vint(&mut file, false) else { break };
                                let bstart = file.stream_position()?;
                                match bid {
                                    // Block (0xA1)
                                    0xA1 if bsz >= 4 => {
                                        if let Ok((tnum, _)) = read_ebml_vint(&mut file, false) {
                                            blk_track = tnum;
                                            if sub_tracks.iter().any(|(n, _)| *n == tnum) {
                                                let mut hdr = [0u8; 2];
                                                if file.read_exact(&mut hdr).is_ok() {
                                                    blk_rel_ts = i16::from_be_bytes(hdr) as i64;
                                                }
                                            }
                                        }
                                    }
                                    // BlockDuration (0x9B)
                                    0x9B if bsz <= 8 => {
                                        if sub_tracks.iter().any(|(n, _)| *n == blk_track) {
                                            blk_dur_ticks = read_ebml_uint(&mut file, bsz as usize).ok();
                                        }
                                    }
                                    _ => {}
                                }
                                let _ = file.seek(SeekFrom::Start(bstart + bsz));
                            }
                            if sub_tracks.iter().any(|(n, _)| *n == blk_track) {
                                let abs_ticks = (cluster_ts + blk_rel_ts).max(0) as u64;
                                let start_ms = ((abs_ticks * timestamp_scale) / 1_000_000) as i64;
                                let dur_ms = blk_dur_ticks
                                    .map(|d| ((d * timestamp_scale) / 1_000_000) as i64)
                                    .unwrap_or(2_200)
                                    .clamp(100, 15_000);
                                spans_by_track.entry(blk_track).or_default().push(Span {
                                    start_ms,
                                    end_ms: start_ms + dur_ms,
                                });
                            }
                        }
                        _ => {}
                    }
                    let _ = file.seek(SeekFrom::Start(cend));
                }
                let _ = file.seek(SeekFrom::Start(elem_end));
            }
            _ => {
                if elem_end <= elem_pos {
                    break;
                }
                let _ = file.seek(SeekFrom::Start(elem_end));
            }
        }
    }

    // Pick the subtitle track with the most dialogue cues (dialogue tracks have 150-500 cues; sign tracks have <50)
    let mut best_track: Option<(u64, String, Vec<Span>)> = None;
    for (tnum, lang) in sub_tracks {
        if let Some(mut sp) = spans_by_track.remove(&tnum) {
            sp.sort_by_key(|s| s.start_ms);
            sp.dedup_by(|a, b| (a.start_ms - b.start_ms).abs() < 40);
            if sp.len() >= 15 {
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
        Some((tnum, lang, spans)) => Ok((spans, format!("Embedded MKV subtitle track #{tnum} ({lang})"))),
        None => bail!("No populated subtitle cues found inside MKV container"),
    }
}

/// Extract reference subtitle spans from:
/// 1. Embedded Matroska `.mkv` subtitle track (pure Rust, instant).
/// 2. Sibling external subtitle file in the same directory (e.g. `.en.srt` / `.en.ass`).
/// 3. `ffmpeg` CLI fallback (`ffmpeg -v quiet -i <video> -map 0:s:0 -f srt -`).
pub fn extract_reference_spans(video_path: &Path, target_sub_path: &Path) -> Result<(Vec<Span>, String)> {
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
pub fn retime_subtitle_file(sub_path: &Path, video_path: &Path, out_path: &Path) -> Result<RetimeReport> {
    let raw_bytes = std::fs::read(sub_path).with_context(|| format!("Reading {}", sub_path.display()))?;
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
        bail!("Subtitle file has too few dialogue lines ({}) to auto-retime.", target_spans.len());
    }

    let (ref_spans, reference_source) = extract_reference_spans(video_path, sub_path)?;
    let model = align_subtitle_spans(&target_spans, &ref_spans)
        .ok_or_else(|| anyhow!("Could not compute a reliable alignment between target and reference subtitles."))?;

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
// Pure-Rust ZIP Extractor (Stored & Deflate) for Jimaku Season Packs
// ---------------------------------------------------------------------------

/// Extract all subtitle files (`.srt`, `.ass`, `.ssa`, `.vtt`) from a `.zip` archive into `dest_dir`.
pub fn extract_subtitles_from_zip(zip_bytes: &[u8], dest_dir: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dest_dir)?;
    let mut extracted = Vec::new();

    // Locate End of Central Directory (EOCD) signature `0x06054b50` scanning backwards from end
    if zip_bytes.len() < 22 {
        bail!("ZIP archive is too small");
    }
    let min_pos = zip_bytes.len().saturating_sub(65_557);
    let mut eocd_pos = None;
    for i in (min_pos..=zip_bytes.len() - 22).rev() {
        if &zip_bytes[i..i + 4] == b"PK\x05\x06" {
            eocd_pos = Some(i);
            break;
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

    let mut pos = cd_offset;
    for _ in 0..total_entries {
        if pos + 46 > zip_bytes.len() || &zip_bytes[pos..pos + 4] != b"PK\x01\x02" {
            break;
        }
        let method = u16::from_le_bytes([zip_bytes[pos + 10], zip_bytes[pos + 11]]);
        let comp_size = u32::from_le_bytes([
            zip_bytes[pos + 20],
            zip_bytes[pos + 21],
            zip_bytes[pos + 22],
            zip_bytes[pos + 23],
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

        let name_end = (pos + 46 + name_len).min(zip_bytes.len());
        let raw_name = String::from_utf8_lossy(&zip_bytes[pos + 46..name_end]).to_string();
        pos = pos + 46 + name_len + extra_len + comment_len;

        // Sanitize file name (strip subdirectories inside archive)
        let Some(base_name) = Path::new(&raw_name).file_name().map(|f| f.to_string_lossy().to_string()) else {
            continue;
        };
        if base_name.starts_with('.') || !is_subtitle_file(Path::new(&base_name)) {
            continue;
        }

        if lfh_offset + 30 > zip_bytes.len() || &zip_bytes[lfh_offset..lfh_offset + 4] != b"PK\x03\x04" {
            continue;
        }
        let lfh_name_len = u16::from_le_bytes([zip_bytes[lfh_offset + 26], zip_bytes[lfh_offset + 27]]) as usize;
        let lfh_extra_len = u16::from_le_bytes([zip_bytes[lfh_offset + 28], zip_bytes[lfh_offset + 29]]) as usize;
        let data_start = lfh_offset + 30 + lfh_name_len + lfh_extra_len;
        let data_end = (data_start + comp_size).min(zip_bytes.len());
        let slice = &zip_bytes[data_start..data_end];

        let out_bytes = match method {
            0 => slice.to_vec(),
            8 => {
                let mut dec = DeflateDecoder::new(slice);
                let mut buf = Vec::new();
                if dec.read_to_end(&mut buf).is_err() {
                    continue;
                }
                buf
            }
            _ => continue,
        };

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
        let conn = st.db();
        let _ = conn.execute_batch(db::SCHEMA_V6);
        get_jimaku_api_key(&conn)
    };
    let Some(key) = api_key else {
        return Ok(JimakuSearchResponse {
            configured: false,
            entry: None,
            files: Vec::new(),
        });
    };

    // 1. Resolve Jimaku entry for `anilist_id` (cached for 6 hours)
    let entry_cache_key = format!("entry:{anilist_id}");
    let cached_entry: Option<JimakuEntry> = {
        let conn = st.db();
        conn.query_row(
            "SELECT payload, fetched_at FROM jimaku_cache WHERE cache_key = ?1",
            [&entry_cache_key],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()
        .ok()
        .flatten()
        .filter(|(_, ts)| now() - *ts < 6 * 3600)
        .and_then(|(payload, _)| serde_json::from_str(&payload).ok())
    };

    let entry = match cached_entry {
        Some(e) => Some(e),
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
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED || resp.status() == reqwest::StatusCode::FORBIDDEN {
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
            if let Some(ref e) = first {
                if let Ok(json_str) = serde_json::to_string(e) {
                    let _ = st.db().execute(
                        "INSERT OR REPLACE INTO jimaku_cache (cache_key, payload, fetched_at) VALUES (?1, ?2, ?3)",
                        params![entry_cache_key, json_str, now()],
                    );
                }
            }
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
            .filter(|(_, ts)| now() - *ts < 1800)
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
        let name = f.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let url = f.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if name.is_empty() || url.is_empty() {
            continue;
        }
        let size = f.get("size").and_then(|x| x.as_i64()).unwrap_or(0);
        let last_modified = f.get("last_modified").and_then(|x| x.as_str()).map(|s| s.to_string());
        let lower = name.to_ascii_lowercase();
        let is_zip = lower.ends_with(".zip");
        let is_rar = lower.ends_with(".rar") || lower.ends_with(".7z");
        let is_archive = is_zip || is_rar;

        let stem = Path::new(&name).file_stem().and_then(|s| s.to_str()).unwrap_or(&name);
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

pub async fn jimaku_download_and_process(
    st: &AppState,
    anilist_id: i64,
    ep_key: &str,
    file_name: &str,
    file_url: &str,
    auto_retime: bool,
) -> Result<JimakuDownloadResult> {
    let api_key = get_jimaku_api_key(&st.db());
    let mut req = st.providers.http.get(file_url).timeout(Duration::from_secs(30));
    if let Some(ref k) = api_key {
        if file_url.contains("jimaku.cc") {
            req = req.header("Authorization", k);
        }
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        bail!("Failed to download subtitle ({})", resp.status());
    }
    let bytes = resp.bytes().await?;

    let video_path: Option<PathBuf> = {
        let conn = st.db();
        conn.query_row(
            "SELECT path FROM local_files WHERE anilist_id = ?1 AND ep_key = ?2 ORDER BY id LIMIT 1",
            params![anilist_id, ep_key],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    };

    let cache_dir = st.data_dir.join("subtitles").join(anilist_id.to_string());
    std::fs::create_dir_all(&cache_dir)?;

    let lower_name = file_name.to_ascii_lowercase();
    let mut saved_paths = Vec::new();
    let mut linked_count = 0usize;
    let mut retime_report: Option<RetimeReport> = None;

    if lower_name.ends_with(".zip") {
        let pack_dir = cache_dir.join("pack");
        let extracted = extract_subtitles_from_zip(&bytes, &pack_dir)?;
        if extracted.is_empty() {
            bail!("No .srt/.ass/.vtt subtitle files were found inside the .zip archive.");
        }
        // Run Smart Subtitle Matcher on the extracted pack directory
        let preview = preview_subtitle_matches(&st.db(), anilist_id, Some(&pack_dir), "ja")?;
        if !preview.matches.is_empty() {
            for m in preview.matches {
                let mut final_path = PathBuf::from(&m.source_path);
                let mut was_retimed = false;
                let mut offset_ms = None;

                // Auto-retime the requested episode immediately (and others on play or if few)
                if auto_retime && m.ep_key == ep_key && Path::new(&m.video_path).is_file() {
                    let retimed_dest = cache_dir.join(format!("ep{}_retimed_{}", m.ep_key, m.source_name));
                    if let Ok(rt) = retime_subtitle_file(&final_path, Path::new(&m.video_path), &retimed_dest) {
                        final_path = PathBuf::from(&rt.output_path);
                        was_retimed = true;
                        offset_ms = Some(rt.offset_ms);
                        retime_report = Some(rt);
                    }
                }

                let p_str = final_path.to_string_lossy().to_string();
                let _ = upsert_linked_subtitle(
                    &st.db(),
                    anilist_id,
                    &m.ep_key,
                    &p_str,
                    &m.lang,
                    m.cc,
                    "jimaku",
                    was_retimed,
                    offset_ms,
                );
                saved_paths.push(p_str);
                linked_count += 1;
            }
        } else {
            // Link the first extracted file to the current episode
            let first = &extracted[0];
            let p_str = first.to_string_lossy().to_string();
            let _ = upsert_linked_subtitle(&st.db(), anilist_id, ep_key, &p_str, "ja", false, "jimaku", false, None);
            saved_paths.push(p_str);
            linked_count += 1;
        }
    } else {
        let ext = Path::new(file_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("ass")
            .to_ascii_lowercase();
        let cc = detect_cc_tag(file_name);

        let dest_path = if let Some(ref vpath) = video_path {
            build_target_sub_path(vpath, "ja", cc, &ext)
        } else {
            let safe_name: String = file_name
                .chars()
                .map(|c| if "<>:\"/\\|?*".contains(c) { '_' } else { c })
                .collect();
            cache_dir.join(format!("ep{ep_key}_{safe_name}"))
        };

        std::fs::write(&dest_path, &bytes)?;
        let mut active_path = dest_path.clone();
        let mut was_retimed = false;
        let mut offset_ms = None;

        if auto_retime {
            if let Some(ref vpath) = video_path {
                // Save original backup in cache_dir before overwriting dest_path with retimed version
                let orig_backup = cache_dir.join(format!("ep{ep_key}.orig.{ext}"));
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
        upsert_linked_subtitle(
            &st.db(),
            anilist_id,
            ep_key,
            &p_str,
            "ja",
            cc,
            "jimaku",
            was_retimed,
            offset_ms,
        )?;
        saved_paths.push(p_str);
        linked_count = 1;
    }

    // If mpv is currently playing this episode, hot-load the subtitle over IPC!
    let ep_sub = episode_subtitle_paths(&st.db(), anilist_id, ep_key)
        .into_iter()
        .next()
        .or_else(|| saved_paths.first().cloned());

    let mut loaded_into_mpv = false;
    if let Some(ref sub_to_load) = ep_sub {
        loaded_into_mpv = crate::player::try_hotload_subtitle_in_mpv(st, anilist_id, ep_key, sub_to_load).await;
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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_subtitle_filenames_and_languages() {
        // 1. Apothecary Diaries Netflix absolute S02E25 with [cc]
        let p1 = parse_subtitle_filename("薬屋のひとりごと.S02E25.猫猫と毛毛.WEBRip.Netflix.ja[cc].srt").unwrap();
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
        let p4 = parse_subtitle_filename("[SubsPlease] Frieren - 14 (1080p) [ABCD1234].jpn.ass").unwrap();
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
            reference.push(Span { start_ms: s, end_ms: s + 2_200 });
        }
        for i in 0..20 {
            let s = 200_000 + i * 5_000;
            reference.push(Span { start_ms: s, end_ms: s + 2_200 });
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
        assert!((model_a.primary_offset_ms - 2_450).abs() <= 20, "got {}", model_a.primary_offset_ms);
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
        assert!(model_b.segments.len() >= 2, "Expected piecewise split, got {:?}", model_b.segments);
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
        assert!(out_ass.contains("Dialogue: 0,0:00:08.75,0:00:11.25,Default,,0,0,0,,{\\blur2}猫猫（マオマオ）"));
    }
}

