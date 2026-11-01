//! Built-in Anime Torrent Search (AnimeTosho / Nyaa), Pure-Rust BitTorrent Swarm,
//! Sequential Piece Scheduler, and Local HTTP Range Streaming Server for `mpv` & `Memento`.

use crate::db;
use crate::service::AppState;
use anyhow::{bail, Context, Result};
use regex::Regex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::SeekFrom;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::Notify;

// ---------------------------------------------------------------------------
// Public Types (Frontend <-> Backend)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentCandidate {
    pub title: String,
    pub group: Option<String>,
    pub resolution: Option<String>,
    pub size: i64,
    pub seeders: i64,
    pub leechers: i64,
    pub torrent_url: Option<String>,
    pub magnet_uri: Option<String>,
    pub info_hash: Option<String>,
    pub published_at: Option<i64>,
    pub source: String,
    pub is_batch: bool,
    pub score: i64,
    /// Optional pre-extracted reference subtitle URL from AnimeTosho for instant retiming.
    pub ref_subtitle_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentSessionStatus {
    pub session_id: String,
    pub anilist_id: i64,
    pub ep_key: String,
    pub title: String,
    pub file_name: String,
    pub mode: String, // "stream" | "hybrid" | "download"
    pub state: String, // "initializing" | "buffering" | "streaming" | "downloading" | "completed" | "error"
    pub progress_pct: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
    pub peers: usize,
    pub stream_url: Option<String>,
    pub save_path: String,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Pure-Rust SHA-1 (FIPS 180-4) — used for info_hash and piece verification
// ---------------------------------------------------------------------------

pub fn sha1_digest(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = Vec::with_capacity(data.len() + 72);
    msg.extend_from_slice(data);
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn hex_decode_20(hex: &str) -> Option<[u8; 20]> {
    let clean = hex.trim();
    if clean.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    for i in 0..20 {
        out[i] = u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn url_encode_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Minimal Bencode Parser (records exact raw byte range for `info` dict)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum BVal {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<BVal>),
    Dict(Vec<(Vec<u8>, BVal, usize, usize)>), // (key, val, val_start, val_end)
}

impl BVal {
    pub fn as_int(&self) -> Option<i64> {
        if let BVal::Int(n) = self {
            Some(*n)
        } else {
            None
        }
    }
    pub fn as_bytes(&self) -> Option<&[u8]> {
        if let BVal::Bytes(b) = self {
            Some(b)
        } else {
            None
        }
    }
    pub fn as_str(&self) -> Option<String> {
        self.as_bytes().map(|b| String::from_utf8_lossy(b).into_owned())
    }
    pub fn as_list(&self) -> Option<&[BVal]> {
        if let BVal::List(l) = self {
            Some(l)
        } else {
            None
        }
    }
    pub fn dict_get(&self, key: &[u8]) -> Option<&BVal> {
        if let BVal::Dict(entries) = self {
            entries.iter().find(|(k, ..)| k.as_slice() == key).map(|(_, v, ..)| v)
        } else {
            None
        }
    }
    pub fn dict_get_span(&self, key: &[u8]) -> Option<(&BVal, usize, usize)> {
        if let BVal::Dict(entries) = self {
            entries
                .iter()
                .find(|(k, ..)| k.as_slice() == key)
                .map(|(_, v, s, e)| (v, *s, *e))
        } else {
            None
        }
    }
}

pub fn parse_bencode(input: &[u8]) -> Result<(BVal, usize)> {
    parse_bencode_at(input, 0)
}

fn parse_bencode_at(input: &[u8], pos: usize) -> Result<(BVal, usize)> {
    if pos >= input.len() {
        bail!("Unexpected EOF in bencode");
    }
    match input[pos] {
        b'i' => {
            let end = input[pos + 1..]
                .iter()
                .position(|&b| b == b'e')
                .context("Unterminated integer in bencode")?
                + pos
                + 1;
            let s = std::str::from_utf8(&input[pos + 1..end])?;
            let n: i64 = s.parse()?;
            Ok((BVal::Int(n), end + 1))
        }
        b'l' => {
            let mut cur = pos + 1;
            let mut list = Vec::new();
            while cur < input.len() && input[cur] != b'e' {
                let (v, next) = parse_bencode_at(input, cur)?;
                list.push(v);
                cur = next;
            }
            if cur >= input.len() {
                bail!("Unterminated list in bencode");
            }
            Ok((BVal::List(list), cur + 1))
        }
        b'd' => {
            let mut cur = pos + 1;
            let mut entries = Vec::new();
            while cur < input.len() && input[cur] != b'e' {
                let (k_val, val_start) = parse_bencode_at(input, cur)?;
                let key = match k_val {
                    BVal::Bytes(b) => b,
                    _ => bail!("Dictionary key must be byte string"),
                };
                let (val, val_end) = parse_bencode_at(input, val_start)?;
                entries.push((key, val, val_start, val_end));
                cur = val_end;
            }
            if cur >= input.len() {
                bail!("Unterminated dict in bencode");
            }
            Ok((BVal::Dict(entries), cur + 1))
        }
        b'0'..=b'9' => {
            let colon = input[pos..]
                .iter()
                .position(|&b| b == b':')
                .context("Missing colon in byte string")?
                + pos;
            let len_str = std::str::from_utf8(&input[pos..colon])?;
            let len: usize = len_str.parse()?;
            let start = colon + 1;
            let end = start + len;
            if end > input.len() {
                bail!("Byte string exceeds input length");
            }
            Ok((BVal::Bytes(input[start..end].to_vec()), end))
        }
        other => bail!("Invalid bencode prefix byte: {other}"),
    }
}

// ---------------------------------------------------------------------------
// Parsed .torrent Metadata (Supports Single-file and Multi-file Batch Packs)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TorrentMeta {
    pub info_hash: [u8; 20],
    pub name: String,
    pub piece_length: u64,
    pub pieces: Vec<[u8; 20]>,
    pub total_length: u64,
    pub trackers: Vec<String>,
    /// Selected target file inside the torrent (for single-file torrents, offset = 0, len = total_length).
    pub target_file_name: String,
    pub target_file_offset: u64,
    pub target_file_length: u64,
}

pub fn parse_torrent_bytes(raw: &[u8], target_episode: Option<i64>) -> Result<TorrentMeta> {
    let (root, _) = parse_bencode(raw)?;
    let (info, info_start, info_end) = root
        .dict_get_span(b"info")
        .context("Missing `info` dictionary in .torrent")?;
    let info_hash = sha1_digest(&raw[info_start..info_end]);

    let name = info
        .dict_get(b"name")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| "video.mkv".to_string());
    let piece_length = info
        .dict_get(b"piece length")
        .and_then(|v| v.as_int())
        .context("Missing `piece length`")? as u64;
    let pieces_raw = info
        .dict_get(b"pieces")
        .and_then(|v| v.as_bytes())
        .context("Missing `pieces`")?;
    if pieces_raw.len() % 20 != 0 {
        bail!("Invalid `pieces` length in .torrent");
    }
    let mut pieces = Vec::with_capacity(pieces_raw.len() / 20);
    for chunk in pieces_raw.chunks_exact(20) {
        let mut arr = [0u8; 20];
        arr.copy_from_slice(chunk);
        pieces.push(arr);
    }

    let mut trackers = Vec::new();
    if let Some(ann) = root.dict_get(b"announce").and_then(|v| v.as_str()) {
        trackers.push(ann);
    }
    if let Some(list) = root.dict_get(b"announce-list").and_then(|v| v.as_list()) {
        for tier in list {
            if let Some(inner) = tier.as_list() {
                for tr in inner {
                    if let Some(s) = tr.as_str() {
                        if !trackers.contains(&s) {
                            trackers.push(s);
                        }
                    }
                }
            }
        }
    }
    // Always include fast public anime trackers as fallback
    for fallback in [
        "http://nyaa.tracker.wf:7777/announce",
        "udp://open.stealth.si:80/announce",
        "udp://tracker.opentrackr.org:1337/announce",
        "udp://exodus.desync.com:6969/announce",
        "udp://tracker.torrent.eu.org:451/announce",
    ] {
        if !trackers.iter().any(|t| t == fallback) {
            trackers.push(fallback.to_string());
        }
    }

    // Single-file vs Multi-file
    if let Some(len) = info.dict_get(b"length").and_then(|v| v.as_int()) {
        let total = len.max(1) as u64;
        return Ok(TorrentMeta {
            info_hash,
            name: name.clone(),
            piece_length,
            pieces,
            total_length: total,
            trackers,
            target_file_name: sanitize_filename(&name),
            target_file_offset: 0,
            target_file_length: total,
        });
    }

    let files = info
        .dict_get(b"files")
        .and_then(|v| v.as_list())
        .context("Missing `length` or `files` in .torrent")?;

    let mut offset = 0u64;
    let mut candidates: Vec<(String, u64, u64)> = Vec::new(); // (filename, offset, len)
    for f in files {
        let len = f
            .dict_get(b"length")
            .and_then(|v| v.as_int())
            .unwrap_or(0)
            .max(0) as u64;
        let path_parts = f.dict_get(b"path").and_then(|v| v.as_list());
        let file_name = path_parts
            .and_then(|parts| parts.last())
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| "episode.mkv".to_string());
        candidates.push((file_name, offset, len));
        offset += len;
    }

    let total_length = offset.max(1);
    let is_video = |n: &str| {
        let lower = n.to_ascii_lowercase();
        lower.ends_with(".mkv") || lower.ends_with(".mp4") || lower.ends_with(".webm") || lower.ends_with(".avi")
    };

    // Pick best matching video file for `target_episode`
    let chosen = if let Some(ep) = target_episode {
        candidates
            .iter()
            .filter(|(n, _, _)| is_video(n))
            .find(|(n, _, _)| {
                let p = crate::parser::parse_filename(n);
                p.episode == Some(ep as u32)
            })
            .or_else(|| candidates.iter().filter(|(n, _, _)| is_video(n)).max_by_key(|(_, _, l)| *l))
            .or_else(|| candidates.iter().max_by_key(|(_, _, l)| *l))
    } else {
        candidates
            .iter()
            .filter(|(n, _, _)| is_video(n))
            .max_by_key(|(_, _, l)| *l)
            .or_else(|| candidates.iter().max_by_key(|(_, _, l)| *l))
    }
    .cloned()
    .unwrap_or_else(|| (name.clone(), 0, total_length));

    Ok(TorrentMeta {
        info_hash,
        name,
        piece_length,
        pieces,
        total_length,
        trackers,
        target_file_name: sanitize_filename(&chosen.0),
        target_file_offset: chosen.1,
        target_file_length: chosen.2.max(1),
    })
}

fn sanitize_filename(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    if out.trim().is_empty() {
        out = "episode.mkv".to_string();
    }
    out
}

// ---------------------------------------------------------------------------
// Nyaa RSS Search Indexer
// ---------------------------------------------------------------------------

pub fn parse_release_tags(title: &str) -> (Option<String>, Option<String>, bool) {
    static GROUP_RE: OnceLock<Regex> = OnceLock::new();
    static RES_RE: OnceLock<Regex> = OnceLock::new();
    static BATCH_RE: OnceLock<Regex> = OnceLock::new();

    let group_re = GROUP_RE.get_or_init(|| Regex::new(r"^\[([^\]]+)\]").unwrap());
    let res_re = RES_RE.get_or_init(|| Regex::new(r"(?i)\b(2160p|4k|1080p|720p|480p)\b").unwrap());
    let batch_re = BATCH_RE.get_or_init(|| Regex::new(r"(?i)\b(batch|complete|\d+\s*-\s*\d+|season\s*\d+\s*complete)\b").unwrap());

    let group = group_re
        .captures(title)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string());
    let resolution = res_re.captures(title).and_then(|c| c.get(1)).map(|m| {
        let s = m.as_str().to_lowercase();
        if s == "4k" {
            "2160p".to_string()
        } else {
            s
        }
    });
    let is_batch = batch_re.is_match(title);
    (group, resolution, is_batch)
}

pub fn score_candidate(
    title: &str,
    group: Option<&str>,
    resolution: Option<&str>,
    seeders: i64,
    is_batch: bool,
    target_episode: i64,
    pref_res: &str,
) -> i64 {
    let mut score = (seeders.min(500)) * 2;
    let parsed = crate::parser::parse_filename(title);
    let matches_ep = parsed.episode == Some(target_episode as u32);
    if matches_ep {
        score += 1200;
    } else if is_batch {
        score += 350;
    } else if parsed.episode.is_some() {
        // Different episode number!
        score -= 2000;
    }

    if let Some(res) = resolution {
        if res.eq_ignore_ascii_case(pref_res) {
            score += 500;
        } else if res == "1080p" {
            score += 300;
        } else if res == "720p" {
            score += 150;
        }
    }

    if let Some(g) = group {
        let gl = g.to_ascii_lowercase();
        if matches!(
            gl.as_str(),
            "subsplease" | "erai-raws" | "ember" | "judas" | "dkb" | "toffeesubs" | "asw" | "vodes" | " kaleido"
        ) {
            score += 250;
        }
    }

    score
}

pub async fn search_episode_torrents(
    st: &AppState,
    anilist_id: i64,
    episode: i64,
    custom_query: Option<String>,
) -> Result<Vec<TorrentCandidate>> {
    let (titles, pref_res, nyaa_base) = {
        let conn = st.db();
        let pref_res = db::get_setting(&conn, "pref.torrent_quality")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty() && s != "ask")
            .unwrap_or_else(|| "1080p".to_string());
        let nyaa_base = db::get_setting(&conn, "pref.nyaa_url")
            .ok()
            .flatten()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| s.starts_with("http"))
            .unwrap_or_else(|| "https://nyaa.si".to_string());

        let row: Option<(Option<String>, Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT title_romaji, title_english, synonyms FROM media WHERE anilist_id = ?1",
                [anilist_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();

        let mut list = Vec::new();
        if let Some(q) = custom_query.as_ref().filter(|s| !s.trim().is_empty()) {
            list.push(q.trim().to_string());
        } else if let Some((ro, en, syn_raw)) = row {
            if let Some(r) = ro.filter(|s| !s.trim().is_empty()) {
                list.push(format!("{r} {episode:02}"));
                list.push(r);
            }
            if let Some(e) = en.filter(|s| !s.trim().is_empty()) {
                let q = format!("{e} {episode:02}");
                if !list.contains(&q) {
                    list.push(q);
                }
            }
            if let Some(sr) = syn_raw {
                if let Ok(syns) = serde_json::from_str::<Vec<String>>(&sr) {
                    if let Some(first) = syns.into_iter().next() {
                        list.push(format!("{first} {episode:02}"));
                    }
                }
            }
        }
        (list, pref_res, nyaa_base)
    };

    if titles.is_empty() {
        bail!("Anime titles not loaded yet for AniList #{anilist_id}");
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Kura/0.5 (https://github.com/Scanner-darkly/kura)")
        .build()?;

    let mut out: Vec<TorrentCandidate> = Vec::new();
    let mut seen_hashes: HashSet<String> = HashSet::new();

    // Primary indexer: Nyaa RSS (`{nyaa_base}/?page=rss&c=1_0&f=0&q=...`)
    // Category `1_0` covers all Anime on Nyaa (both 1_2 English-translated and 1_4 Raw).
    for q in titles.iter().take(2) {
        let rss_url = format!(
            "{nyaa_base}/?page=rss&c=1_0&f=0&q={}",
            url_encode_bytes(q.as_bytes())
        );
        if let Ok(resp) = client.get(&rss_url).send().await {
            if let Ok(xml) = resp.text().await {
                for item in parse_nyaa_rss(&xml, episode, &pref_res) {
                    let key = item
                        .info_hash
                        .clone()
                        .unwrap_or_else(|| item.title.to_lowercase());
                    if seen_hashes.insert(key) {
                        out.push(item);
                    }
                }
            }
        }
        if out.iter().filter(|c| c.score > 500).count() >= 5 {
            break;
        }
    }

    out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| b.seeders.cmp(&a.seeders)));
    Ok(out)
}

fn extract_xml_tag(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&close)? + start;
    Some(
        block[start..end]
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .trim()
            .to_string(),
    )
}

fn parse_human_size(s: &str) -> i64 {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        return 0;
    }
    let val: f64 = parts[0].parse().unwrap_or(0.0);
    let mult: f64 = match parts[1].to_ascii_lowercase().as_str() {
        "gib" | "gb" => 1024.0 * 1024.0 * 1024.0,
        "mib" | "mb" => 1024.0 * 1024.0,
        "kib" | "kb" => 1024.0,
        _ => 1.0,
    };
    (val * mult) as i64
}

fn parse_nyaa_rss(xml: &str, episode: i64, pref_res: &str) -> Vec<TorrentCandidate> {
    let mut out = Vec::new();
    for raw_item in xml.split("<item>").skip(1) {
        let Some(end) = raw_item.find("</item>") else {
            continue;
        };
        let block = &raw_item[..end];
        let Some(title) = extract_xml_tag(block, "title") else {
            continue;
        };
        let link = extract_xml_tag(block, "link");
        let info_hash = extract_xml_tag(block, "nyaa:infoHash");
        let seeders: i64 = extract_xml_tag(block, "nyaa:seeders")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let leechers: i64 = extract_xml_tag(block, "nyaa:leechers")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let size = extract_xml_tag(block, "nyaa:size")
            .map(|s| parse_human_size(&s))
            .unwrap_or(0);

        let (group, resolution, is_batch) = parse_release_tags(&title);
        let score = score_candidate(
            &title,
            group.as_deref(),
            resolution.as_deref(),
            seeders,
            is_batch,
            episode,
            pref_res,
        );
        if score < -500 {
            continue;
        }
        let magnet_uri = info_hash.as_ref().map(|h| {
            format!(
                "magnet:?xt=urn:btih:{h}&dn={}&tr=http%3A%2F%2Fnyaa.tracker.wf%3A7777%2Fannounce",
                url_encode_bytes(title.as_bytes())
            )
        });
        out.push(TorrentCandidate {
            title,
            group,
            resolution,
            size,
            seeders,
            leechers,
            torrent_url: link,
            magnet_uri,
            info_hash,
            published_at: None,
            source: "Nyaa".into(),
            is_batch,
            score,
            ref_subtitle_url: None,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Tracker Discovery (HTTP/HTTPS + UDP BEP 15)
// ---------------------------------------------------------------------------

async fn fetch_peers_from_trackers(
    meta: &TorrentMeta,
    peer_id: &[u8; 20],
) -> Vec<SocketAddr> {
    let mut set = HashSet::new();
    let mut tasks = Vec::new();

    for tr in meta.trackers.iter().take(8).cloned() {
        let info_hash = meta.info_hash;
        let peer_id = *peer_id;
        let left = meta.total_length;
        tasks.push(tokio::spawn(async move {
            if tr.starts_with("udp://") {
                announce_udp(&tr, &info_hash, &peer_id, left).await.unwrap_or_default()
            } else if tr.starts_with("http://") || tr.starts_with("https://") {
                announce_http(&tr, &info_hash, &peer_id, left).await.unwrap_or_default()
            } else {
                Vec::new()
            }
        }));
    }

    for t in tasks {
        if let Ok(peers) = t.await {
            for p in peers {
                set.insert(p);
            }
        }
    }
    set.into_iter().collect()
}

async fn announce_http(
    tracker_url: &str,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
    left: u64,
) -> Result<Vec<SocketAddr>> {
    let sep = if tracker_url.contains('?') { '&' } else { '?' };
    let url = format!(
        "{tracker_url}{sep}info_hash={}&peer_id={}&port=6881&uploaded=0&downloaded=0&left={left}&compact=1&numwant=80",
        url_encode_bytes(info_hash),
        url_encode_bytes(peer_id),
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent("Kura/0.5")
        .build()?;
    let bytes = client.get(&url).send().await?.bytes().await?;
    let (root, _) = parse_bencode(&bytes)?;
    let mut peers = Vec::new();
    if let Some(compact) = root.dict_get(b"peers").and_then(|v| v.as_bytes()) {
        for chunk in compact.chunks_exact(6) {
            let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
            let port = u16::from_be_bytes([chunk[4], chunk[5]]);
            if port > 0 {
                peers.push(SocketAddr::V4(SocketAddrV4::new(ip, port)));
            }
        }
    }
    Ok(peers)
}

async fn announce_udp(
    tracker_url: &str,
    info_hash: &[u8; 20],
    peer_id: &[u8; 20],
    left: u64,
) -> Result<Vec<SocketAddr>> {
    let host_port = tracker_url
        .strip_prefix("udp://")
        .unwrap_or(tracker_url)
        .split('/')
        .next()
        .context("Invalid UDP tracker URL")?;

    let sock = UdpSocket::bind("0.0.0.0:0").await?;
    tokio::time::timeout(Duration::from_secs(3), sock.connect(host_port)).await??;

    // BEP 15 Step 1: Connect request
    let tx_id: u32 = 0x4B555241; // "KURA"
    let mut req = [0u8; 16];
    req[0..8].copy_from_slice(&0x41727101980u64.to_be_bytes());
    req[8..12].copy_from_slice(&0u32.to_be_bytes()); // action = 0 (connect)
    req[12..16].copy_from_slice(&tx_id.to_be_bytes());
    sock.send(&req).await?;

    let mut resp = [0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(3), sock.recv(&mut resp)).await??;
    if n < 16 {
        bail!("Short UDP connect response");
    }
    let conn_id = u64::from_be_bytes(resp[8..16].try_into()?);

    // BEP 15 Step 2: Announce request (98 bytes)
    let mut ann = [0u8; 98];
    ann[0..8].copy_from_slice(&conn_id.to_be_bytes());
    ann[8..12].copy_from_slice(&1u32.to_be_bytes()); // action = 1 (announce)
    ann[12..16].copy_from_slice(&tx_id.to_be_bytes());
    ann[16..36].copy_from_slice(info_hash);
    ann[36..56].copy_from_slice(peer_id);
    ann[56..64].copy_from_slice(&0u64.to_be_bytes()); // downloaded
    ann[64..72].copy_from_slice(&left.to_be_bytes()); // left
    ann[72..80].copy_from_slice(&0u64.to_be_bytes()); // uploaded
    ann[80..84].copy_from_slice(&2u32.to_be_bytes()); // event = started
    ann[84..88].copy_from_slice(&0u32.to_be_bytes()); // IP = default
    ann[88..92].copy_from_slice(&0x12345678u32.to_be_bytes()); // key
    ann[92..96].copy_from_slice(&80i32.to_be_bytes()); // num_want = 80
    ann[96..98].copy_from_slice(&6881u16.to_be_bytes()); // port
    sock.send(&ann).await?;

    let mut buf = [0u8; 1500];
    let len = tokio::time::timeout(Duration::from_secs(4), sock.recv(&mut buf)).await??;
    if len < 20 {
        bail!("Short UDP announce response");
    }
    let mut peers = Vec::new();
    for chunk in buf[20..len].chunks_exact(6) {
        let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
        let port = u16::from_be_bytes([chunk[4], chunk[5]]);
        if port > 0 {
            peers.push(SocketAddr::V4(SocketAddrV4::new(ip, port)));
        }
    }
    Ok(peers)
}

// ---------------------------------------------------------------------------
// Active Swarm & Sequential Piece Scheduler
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum PieceState {
    Missing,
    InFlight,
    Complete,
}

pub struct SwarmSession {
    pub id: String,
    pub anilist_id: i64,
    pub ep_key: String,
    pub title: String,
    pub mode: String, // "stream" | "hybrid" | "download"
    pub meta: TorrentMeta,
    pub save_path: PathBuf,
    pub first_piece: usize,
    pub last_piece: usize,
    piece_states: Mutex<Vec<PieceState>>,
    read_head_piece: AtomicUsize,
    pub downloaded_bytes: AtomicU64,
    pub speed_bps: AtomicU64,
    pub active_peers: AtomicUsize,
    pub state_str: Mutex<String>,
    pub error_str: Mutex<Option<String>>,
    pub stream_url: Mutex<Option<String>>,
    pub allow_seeding: bool,
    pub cancelled: AtomicBool,
    pub piece_notify: Notify,
    file_write_lock: tokio::sync::Mutex<()>,
}

impl SwarmSession {
    pub fn target_piece_count(&self) -> usize {
        (self.last_piece - self.first_piece) + 1
    }

    pub fn completed_target_pieces(&self) -> usize {
        let states = self.piece_states.lock().unwrap();
        states[self.first_piece..=self.last_piece]
            .iter()
            .filter(|&&s| s == PieceState::Complete)
            .count()
    }

    pub fn is_piece_complete(&self, piece_idx: usize) -> bool {
        let states = self.piece_states.lock().unwrap();
        states.get(piece_idx).copied() == Some(PieceState::Complete)
    }

    /// Ready to launch `mpv` / `Memento` once the first piece (Matroska EBML header + Tracks)
    /// and the second piece are complete.
    pub fn is_ready_to_stream(&self) -> bool {
        let states = self.piece_states.lock().unwrap();
        let p0 = states.get(self.first_piece).copied() == Some(PieceState::Complete);
        let p1_idx = (self.first_piece + 1).min(self.last_piece);
        let p1 = states.get(p1_idx).copied() == Some(PieceState::Complete);
        p0 && p1
    }

    pub fn is_all_complete(&self) -> bool {
        let states = self.piece_states.lock().unwrap();
        states[self.first_piece..=self.last_piece]
            .iter()
            .all(|&s| s == PieceState::Complete)
    }

    /// Pick the highest-priority piece that `peer_has`:
    /// 1. First 2 pieces (EBML header & tracks) + Last piece (Matroska Cues index for seeking)
    /// 2. Pieces starting sequentially from `read_head_piece`
    /// 3. Any remaining missing piece in `[first_piece..=last_piece]`
    fn claim_next_piece(&self, peer_bitfield: &[bool]) -> Option<usize> {
        let mut states = self.piece_states.lock().unwrap();
        let head = self.read_head_piece.load(Ordering::Relaxed).clamp(self.first_piece, self.last_piece);

        let priority_indices = [
            self.first_piece,
            (self.first_piece + 1).min(self.last_piece),
            self.last_piece,
        ];
        for &idx in &priority_indices {
            if states[idx] == PieceState::Missing && peer_bitfield.get(idx).copied().unwrap_or(false) {
                states[idx] = PieceState::InFlight;
                return Some(idx);
            }
        }

        // Sequential from read_head_piece to last_piece
        for idx in head..=self.last_piece {
            if states[idx] == PieceState::Missing && peer_bitfield.get(idx).copied().unwrap_or(false) {
                states[idx] = PieceState::InFlight;
                return Some(idx);
            }
        }

        // Wrap around from first_piece to head
        for idx in self.first_piece..head {
            if states[idx] == PieceState::Missing && peer_bitfield.get(idx).copied().unwrap_or(false) {
                states[idx] = PieceState::InFlight;
                return Some(idx);
            }
        }

        // Endgame: if only a few pieces remain InFlight (e.g. first_piece or head), allow duplicate claim
        if states[head] == PieceState::InFlight && peer_bitfield.get(head).copied().unwrap_or(false) {
            return Some(head);
        }

        None
    }

    fn release_piece(&self, idx: usize) {
        let mut states = self.piece_states.lock().unwrap();
        if states.get(idx) == Some(&PieceState::InFlight) {
            states[idx] = PieceState::Missing;
        }
    }

    fn mark_piece_complete(&self, idx: usize, piece_len: u64) -> bool {
        let mut states = self.piece_states.lock().unwrap();
        if states.get(idx) == Some(&PieceState::Complete) {
            return false;
        }
        if let Some(slot) = states.get_mut(idx) {
            *slot = PieceState::Complete;
        }
        drop(states);
        self.downloaded_bytes.fetch_add(piece_len, Ordering::Relaxed);
        self.piece_notify.notify_waiters();
        true
    }

    pub fn piece_size(&self, idx: usize) -> usize {
        let total = self.meta.total_length;
        let plen = self.meta.piece_length;
        let start = (idx as u64) * plen;
        let rem = total.saturating_sub(start);
        rem.min(plen) as usize
    }

    /// Write verified piece bytes into the target file (cropping to `[target_file_offset, target_file_offset + target_file_length)`).
    async fn write_verified_piece(&self, idx: usize, data: &[u8]) -> Result<()> {
        let piece_global_start = (idx as u64) * self.meta.piece_length;
        let piece_global_end = piece_global_start + (data.len() as u64);

        let file_start = self.meta.target_file_offset;
        let file_end = file_start + self.meta.target_file_length;

        let overlap_start = piece_global_start.max(file_start);
        let overlap_end = piece_global_end.min(file_end);
        if overlap_start >= overlap_end {
            return Ok(());
        }

        let slice_offset = (overlap_start - piece_global_start) as usize;
        let slice_len = (overlap_end - overlap_start) as usize;
        let dest_offset = overlap_start - file_start;

        let _guard = self.file_write_lock.lock().await;
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&self.save_path)
            .await?;
        f.seek(SeekFrom::Start(dest_offset)).await?;
        f.write_all(&data[slice_offset..slice_offset + slice_len]).await?;
        f.flush().await?;
        Ok(())
    }

    /// Read a verified block from disk to upload to a requesting peer when `allow_seeding` is enabled.
    async fn read_verified_block(&self, idx: usize, begin: usize, len: usize) -> Result<Option<Vec<u8>>> {
        if !self.allow_seeding || len == 0 || len > 65_536 || !self.is_piece_complete(idx) {
            return Ok(None);
        }
        let block_global_start = (idx as u64) * self.meta.piece_length + (begin as u64);
        let block_global_end = block_global_start + (len as u64);

        let file_start = self.meta.target_file_offset;
        let file_end = file_start + self.meta.target_file_length;
        if block_global_start < file_start || block_global_end > file_end {
            return Ok(None);
        }

        let file_offset = block_global_start - file_start;
        let mut f = tokio::fs::File::open(&self.save_path).await?;
        f.seek(SeekFrom::Start(file_offset)).await?;
        let mut buf = vec![0u8; len];
        f.read_exact(&mut buf).await?;
        Ok(Some(buf))
    }

    pub fn status(&self) -> TorrentSessionStatus {
        let done = self.completed_target_pieces();
        let total_p = self.target_piece_count().max(1);
        let pct = ((done as f64) / (total_p as f64) * 100.0).min(100.0);
        TorrentSessionStatus {
            session_id: self.id.clone(),
            anilist_id: self.anilist_id,
            ep_key: self.ep_key.clone(),
            title: self.title.clone(),
            file_name: self.meta.target_file_name.clone(),
            mode: self.mode.clone(),
            state: self.state_str.lock().unwrap().clone(),
            progress_pct: pct,
            downloaded_bytes: self.downloaded_bytes.load(Ordering::Relaxed).min(self.meta.target_file_length),
            total_bytes: self.meta.target_file_length,
            speed_bps: self.speed_bps.load(Ordering::Relaxed),
            peers: self.active_peers.load(Ordering::Relaxed),
            stream_url: self.stream_url.lock().unwrap().clone(),
            save_path: self.save_path.to_string_lossy().to_string(),
            error: self.error_str.lock().unwrap().clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// BitTorrent Peer Wire Worker (`BEP 3`)
// ---------------------------------------------------------------------------

async fn run_peer_worker(
    session: Arc<SwarmSession>,
    addr: SocketAddr,
    peer_id: [u8; 20],
) -> Result<()> {
    let mut stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await??;
    let _ = stream.set_nodelay(true);

    // 68-byte Handshake
    let mut hs = [0u8; 68];
    hs[0] = 19;
    hs[1..20].copy_from_slice(b"BitTorrent protocol");
    hs[28..48].copy_from_slice(&session.meta.info_hash);
    hs[48..68].copy_from_slice(&peer_id);
    tokio::time::timeout(Duration::from_secs(5), stream.write_all(&hs)).await??;

    let mut resp_hs = [0u8; 68];
    tokio::time::timeout(Duration::from_secs(6), stream.read_exact(&mut resp_hs)).await??;
    if &resp_hs[1..20] != b"BitTorrent protocol" || resp_hs[28..48] != session.meta.info_hash {
        bail!("Handshake mismatch");
    }

    // Send Interested (len = 1, id = 2)
    stream.write_all(&[0, 0, 0, 1, 2]).await?;
    // If seeding while active is enabled, unchoke the peer immediately for tit-for-tat reciprocity
    if session.allow_seeding {
        let _ = stream.write_all(&[0, 0, 0, 1, 1]).await;
    }

    let num_pieces = session.meta.pieces.len();
    let mut peer_has = vec![false; num_pieces];
    let mut peer_choking = true;
    let mut counted_active = false;

    loop {
        if session.cancelled.load(Ordering::Relaxed) || session.is_all_complete() {
            break;
        }

        // Read messages until unchoked
        while peer_choking {
            if session.cancelled.load(Ordering::Relaxed) || session.is_all_complete() {
                if counted_active {
                    session.active_peers.fetch_sub(1, Ordering::Relaxed);
                }
                return Ok(());
            }
            let msg = tokio::time::timeout(Duration::from_secs(12), read_peer_msg(&mut stream)).await??;
            match msg {
                None => {} // keep-alive
                Some((0, _)) => peer_choking = true,
                Some((1, _)) => {
                    peer_choking = false;
                    if !counted_active {
                        counted_active = true;
                        session.active_peers.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Some((4, payload)) if payload.len() >= 4 => {
                    let idx = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                    if idx < num_pieces {
                        peer_has[idx] = true;
                    }
                }
                Some((5, payload)) => {
                    for (byte_i, &b) in payload.iter().enumerate() {
                        for bit in 0..8 {
                            let idx = byte_i * 8 + bit;
                            if idx < num_pieces && (b & (1 << (7 - bit))) != 0 {
                                peer_has[idx] = true;
                            }
                        }
                    }
                }
                Some((2, _)) if session.allow_seeding => {
                    // Peer is interested -> send Unchoke (id = 1)
                    let _ = stream.write_all(&[0, 0, 0, 1, 1]).await;
                }
                Some((6, payload)) if session.allow_seeding && payload.len() >= 12 => {
                    let req_idx = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                    let req_begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
                    let req_len = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]) as usize;
                    if let Ok(Some(block_data)) = session.read_verified_block(req_idx, req_begin, req_len).await {
                        let msg_len = (9 + block_data.len()) as u32;
                        let mut hdr = [0u8; 13];
                        hdr[0..4].copy_from_slice(&msg_len.to_be_bytes());
                        hdr[4] = 7; // Piece
                        hdr[5..9].copy_from_slice(&(req_idx as u32).to_be_bytes());
                        hdr[9..13].copy_from_slice(&(req_begin as u32).to_be_bytes());
                        let _ = stream.write_all(&hdr).await;
                        let _ = stream.write_all(&block_data).await;
                    }
                }
                _ => {}
            }
        }

        let Some(piece_idx) = session.claim_next_piece(&peer_has) else {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        };

        let piece_len = session.piece_size(piece_idx);
        let mut piece_buf = vec![0u8; piece_len];
        let mut received = 0usize;
        let block_size = 16_384usize;

        // Pipeline block requests in batches of 8 (128 KiB window)
        let mut next_offset = 0usize;
        let mut in_flight = 0usize;
        let mut failed = false;

        while received < piece_len {
            if session.cancelled.load(Ordering::Relaxed) {
                failed = true;
                break;
            }
            while in_flight < 8 && next_offset < piece_len && !peer_choking {
                let req_len = (piece_len - next_offset).min(block_size);
                let mut req = [0u8; 17];
                req[0..4].copy_from_slice(&13u32.to_be_bytes());
                req[4] = 6; // Request
                req[5..9].copy_from_slice(&(piece_idx as u32).to_be_bytes());
                req[9..13].copy_from_slice(&(next_offset as u32).to_be_bytes());
                req[13..17].copy_from_slice(&(req_len as u32).to_be_bytes());
                if stream.write_all(&req).await.is_err() {
                    failed = true;
                    break;
                }
                next_offset += req_len;
                in_flight += 1;
            }
            if failed {
                break;
            }

            match tokio::time::timeout(Duration::from_secs(12), read_peer_msg(&mut stream)).await {
                Ok(Ok(Some((7, payload)))) if payload.len() > 8 => {
                    let idx = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                    let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
                    let block = &payload[8..];
                    if idx == piece_idx && begin + block.len() <= piece_len {
                        piece_buf[begin..begin + block.len()].copy_from_slice(block);
                        received += block.len();
                        in_flight = in_flight.saturating_sub(1);
                    }
                }
                Ok(Ok(Some((0, _)))) => {
                    peer_choking = true;
                    failed = true;
                    break;
                }
                Ok(Ok(Some((4, payload)))) if payload.len() >= 4 => {
                    let idx = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                    if idx < num_pieces {
                        peer_has[idx] = true;
                    }
                }
                Ok(Ok(Some((2, _)))) if session.allow_seeding => {
                    let _ = stream.write_all(&[0, 0, 0, 1, 1]).await;
                }
                Ok(Ok(Some((6, payload)))) if session.allow_seeding && payload.len() >= 12 => {
                    let req_idx = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
                    let req_begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]) as usize;
                    let req_len = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]) as usize;
                    if let Ok(Some(block_data)) = session.read_verified_block(req_idx, req_begin, req_len).await {
                        let msg_len = (9 + block_data.len()) as u32;
                        let mut hdr = [0u8; 13];
                        hdr[0..4].copy_from_slice(&msg_len.to_be_bytes());
                        hdr[4] = 7; // Piece
                        hdr[5..9].copy_from_slice(&(req_idx as u32).to_be_bytes());
                        hdr[9..13].copy_from_slice(&(req_begin as u32).to_be_bytes());
                        let _ = stream.write_all(&hdr).await;
                        let _ = stream.write_all(&block_data).await;
                    }
                }
                Ok(Ok(None)) => {}
                _ => {
                    failed = true;
                    break;
                }
            }
        }

        if failed || received < piece_len {
            session.release_piece(piece_idx);
            if failed && !peer_choking {
                break;
            }
            continue;
        }

        // Verify SHA-1 digest of the piece
        let digest = sha1_digest(&piece_buf);
        if digest != session.meta.pieces[piece_idx] {
            session.release_piece(piece_idx);
            break;
        }

        if session.write_verified_piece(piece_idx, &piece_buf).await.is_ok() {
            session.mark_piece_complete(piece_idx, piece_len as u64);
            if session.allow_seeding {
                let mut have_msg = [0u8; 9];
                have_msg[0..4].copy_from_slice(&5u32.to_be_bytes());
                have_msg[4] = 4; // Have
                have_msg[5..9].copy_from_slice(&(piece_idx as u32).to_be_bytes());
                let _ = stream.write_all(&have_msg).await;
            }
        } else {
            session.release_piece(piece_idx);
        }
    }

    if counted_active {
        session.active_peers.fetch_sub(1, Ordering::Relaxed);
    }
    Ok(())
}

async fn read_peer_msg(stream: &mut TcpStream) -> Result<Option<(u8, Vec<u8>)>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 {
        return Ok(None);
    }
    if len > 2 * 1024 * 1024 {
        bail!("Peer message too large: {len}");
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    let id = buf[0];
    let payload = buf[1..].to_vec();
    Ok(Some((id, payload)))
}

// ---------------------------------------------------------------------------
// Local HTTP Range Server for `mpv` / `Memento`
// ---------------------------------------------------------------------------

async fn spawn_http_range_server(session: Arc<SwarmSession>) -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let url = format!(
        "http://127.0.0.1:{port}/stream/{}/{}",
        session.id,
        url_encode_bytes(session.meta.target_file_name.as_bytes())
    );
    *session.stream_url.lock().unwrap() = Some(url.clone());

    tokio::spawn(async move {
        loop {
            if session.cancelled.load(Ordering::Relaxed) {
                break;
            }
            let accept_res = tokio::select! {
                _ = session.piece_notify.notified() => {
                    if session.cancelled.load(Ordering::Relaxed) {
                        break;
                    }
                    continue;
                }
                r = listener.accept() => r,
            };
            let Ok((socket, _)) = accept_res else {
                break;
            };
            let sess = session.clone();
            tokio::spawn(async move {
                let _ = handle_http_stream_client(socket, sess).await;
            });
        }
    });

    Ok(url)
}

async fn handle_http_stream_client(mut socket: TcpStream, session: Arc<SwarmSession>) -> Result<()> {
    let mut req_buf = [0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut req_buf)).await??;
    if n == 0 {
        return Ok(());
    }
    let req_str = String::from_utf8_lossy(&req_buf[..n]);
    let is_head = req_str.starts_with("HEAD ");
    let total_len = session.meta.target_file_length;

    let mut range_start = 0u64;
    let mut range_end = total_len.saturating_sub(1);
    let mut has_range = false;

    for line in req_str.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("range: bytes=") {
            has_range = true;
            let mut parts = rest.trim().splitn(2, '-');
            if let Some(s) = parts.next().and_then(|s| s.parse::<u64>().ok()) {
                range_start = s.min(total_len.saturating_sub(1));
            }
            if let Some(e) = parts.next().and_then(|e| e.parse::<u64>().ok()) {
                range_end = e.min(total_len.saturating_sub(1));
            }
        }
    }

    let content_len = (range_end - range_start) + 1;
    let header = if has_range {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: video/x-matroska\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {range_start}-{range_end}/{total_len}\r\nContent-Length: {content_len}\r\nConnection: close\r\n\r\n"
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: video/x-matroska\r\nAccept-Ranges: bytes\r\nContent-Length: {total_len}\r\nConnection: close\r\n\r\n"
        )
    };
    socket.write_all(header.as_bytes()).await?;
    if is_head {
        return Ok(());
    }

    let mut cur_offset = range_start;
    let mut chunk_buf = vec![0u8; 65_536];

    while cur_offset <= range_end {
        if session.cancelled.load(Ordering::Relaxed) {
            break;
        }
        let global_offset = session.meta.target_file_offset + cur_offset;
        let needed_piece = (global_offset / session.meta.piece_length) as usize;

        // Point sequential scheduler right at the piece `mpv` / `Memento` is reading
        session.read_head_piece.store(needed_piece, Ordering::Relaxed);

        // Wait until `needed_piece` is verified on disk
        while !session.is_piece_complete(needed_piece) {
            if session.cancelled.load(Ordering::Relaxed) {
                return Ok(());
            }
            session.piece_notify.notified().await;
        }

        // Compute how many bytes we can read within this completed piece
        let piece_end_global = ((needed_piece as u64 + 1) * session.meta.piece_length).min(session.meta.total_length);
        let avail_in_piece = piece_end_global.saturating_sub(global_offset);
        let remaining_in_req = (range_end - cur_offset) + 1;
        let to_read = avail_in_piece.min(remaining_in_req).min(chunk_buf.len() as u64) as usize;
        if to_read == 0 {
            break;
        }

        let mut f = tokio::fs::File::open(&session.save_path).await?;
        f.seek(SeekFrom::Start(cur_offset)).await?;
        f.read_exact(&mut chunk_buf[..to_read]).await?;

        if socket.write_all(&chunk_buf[..to_read]).await.is_err() {
            break;
        }
        cur_offset += to_read as u64;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Global Session Registry & Orchestrator
// ---------------------------------------------------------------------------

static SESSIONS: OnceLock<Mutex<HashMap<String, Arc<SwarmSession>>>> = OnceLock::new();

fn sessions_map() -> &'static Mutex<HashMap<String, Arc<SwarmSession>>> {
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn list_active_sessions() -> Vec<TorrentSessionStatus> {
    let map = sessions_map().lock().unwrap();
    map.values().map(|s| s.status()).collect()
}

pub fn cancel_session(session_id: &str) {
    let sess = {
        let mut map = sessions_map().lock().unwrap();
        map.remove(session_id)
    };
    if let Some(s) = sess {
        s.cancelled.store(true, Ordering::Relaxed);
        s.speed_bps.store(0, Ordering::Relaxed);
        s.active_peers.store(0, Ordering::Relaxed);
        *s.state_str.lock().unwrap() = "cancelled".into();
        s.piece_notify.notify_waiters();
        if s.mode == "stream" {
            let path = s.save_path.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let _ = tokio::fs::remove_file(&path).await;
            });
        }
    }
}

/// Clean up any active streaming session for `(anilist_id, ep_key)` when `mpv` / `Memento` exits.
pub fn on_player_stopped(anilist_id: i64, ep_key: &str) {
    let to_cancel: Vec<String> = {
        let map = sessions_map().lock().unwrap();
        map.values()
            .filter(|s| s.anilist_id == anilist_id && s.ep_key == ep_key && (s.mode == "stream" || s.mode == "hybrid"))
            .map(|s| s.id.clone())
            .collect()
    };
    for id in to_cancel {
        cancel_session(&id);
    }
}

async fn fetch_torrent_bytes(
    nyaa_base: &str,
    torrent_url: Option<&str>,
    magnet_uri: Option<&str>,
    info_hash: Option<&str>,
) -> Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Kura/0.5")
        .build()?;

    // 1. Direct .torrent URL (Nyaa `<link>https://nyaa.si/download/....torrent</link>`)
    if let Some(url) = torrent_url.filter(|u| u.starts_with("http")) {
        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() {
                if let Ok(bytes) = resp.bytes().await {
                    if bytes.starts_with(b"d") {
                        return Ok(bytes.to_vec());
                    }
                }
            }
        }
    }

    // Extract info_hash from magnet if not explicitly passed
    let hash = info_hash
        .map(|s| s.to_string())
        .or_else(|| {
            magnet_uri.and_then(|m| {
                m.split("urn:btih:")
                    .nth(1)
                    .map(|rest| rest.split('&').next().unwrap_or(rest).trim().to_string())
            })
        })
        .context("No .torrent URL or info_hash available")?;

    // 2. Resolve info_hash via Nyaa RSS search (`{nyaa_base}/?page=rss&c=1_0&f=0&q={hash}`)
    let nyaa_lookup = format!("{nyaa_base}/?page=rss&c=1_0&f=0&q={hash}");
    if let Ok(resp) = client.get(&nyaa_lookup).send().await {
        if let Ok(xml) = resp.text().await {
            if let Some(first_item) = xml.split("<item>").nth(1) {
                if let Some(t_url) = extract_xml_tag(first_item, "link") {
                    if let Ok(r2) = client.get(&t_url).send().await {
                        if let Ok(bytes) = r2.bytes().await {
                            if bytes.starts_with(b"d") {
                                return Ok(bytes.to_vec());
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Fallback to iTorrents cache (`https://itorrents.org/torrent/<HASH>.torrent`)
    let itorrents_url = format!("https://itorrents.org/torrent/{}.torrent", hash.to_ascii_uppercase());
    let resp = client.get(&itorrents_url).send().await?;
    let bytes = resp.bytes().await?;
    if !bytes.starts_with(b"d") {
        bail!("Could not retrieve .torrent metadata for info_hash {hash}");
    }
    Ok(bytes.to_vec())
}

fn resolve_destination_path(
    st: &AppState,
    anilist_id: i64,
    mode: &str,
    file_name: &str,
) -> Result<PathBuf> {
    if mode == "stream" {
        let dir = st.data_dir.join("stream_cache");
        std::fs::create_dir_all(&dir)?;
        return Ok(dir.join(file_name));
    }

    // For "hybrid" or "download", place into the show's existing library folder or the first library root
    let conn = st.db();
    let existing_group_folder: Option<String> = conn
        .query_row(
            "SELECT folder_path FROM match_groups WHERE anilist_id = ?1 AND folder_path IS NOT NULL LIMIT 1",
            [anilist_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();

    if let Some(folder) = existing_group_folder {
        let p = PathBuf::from(folder);
        if p.exists() {
            return Ok(p.join(file_name));
        }
    }

    let first_lib_root: Option<String> = conn
        .query_row("SELECT path FROM library_folders ORDER BY id LIMIT 1", [], |r| r.get(0))
        .ok();
    let show_title: String = conn
        .query_row(
            "SELECT COALESCE(title_english, title_romaji, 'Anime') FROM media WHERE anilist_id = ?1",
            [anilist_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| format!("Anime_{anilist_id}"));

    if let Some(root) = first_lib_root {
        let show_dir = PathBuf::from(root).join(sanitize_filename(&show_title));
        std::fs::create_dir_all(&show_dir)?;
        return Ok(show_dir.join(file_name));
    }

    // Fallback if user has no library folders configured yet: create `Downloads` in data_dir
    let dl_dir = st.data_dir.join("downloads").join(sanitize_filename(&show_title));
    std::fs::create_dir_all(&dl_dir)?;
    Ok(dl_dir.join(file_name))
}

pub async fn start_torrent_session(
    app: AppHandle,
    st: Arc<AppState>,
    anilist_id: i64,
    ep_key: String,
    episode_num: Option<i64>,
    candidate: TorrentCandidate,
    mode_override: Option<String>,
) -> Result<TorrentSessionStatus> {
    let (mode, nyaa_base, allow_seeding) = {
        let conn = st.db();
        let m = mode_override.unwrap_or_else(|| {
            db::get_setting(&conn, "pref.torrent_action")
                .ok()
                .flatten()
                .unwrap_or_else(|| "stream".to_string())
        });
        let nb = db::get_setting(&conn, "pref.nyaa_url")
            .ok()
            .flatten()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| s.starts_with("http"))
            .unwrap_or_else(|| "https://nyaa.si".to_string());
        let seed = db::get_setting(&conn, "pref.torrent_seeding")
            .ok()
            .flatten()
            .map(|v| v == "1")
            .unwrap_or(false);
        (m, nb, seed)
    };

    // Cancel any previous session for the same episode
    on_player_stopped(anilist_id, &ep_key);

    let raw_torrent = fetch_torrent_bytes(
        &nyaa_base,
        candidate.torrent_url.as_deref(),
        candidate.magnet_uri.as_deref(),
        candidate.info_hash.as_deref(),
    )
    .await?;

    let meta = parse_torrent_bytes(&raw_torrent, episode_num)?;
    let save_path = resolve_destination_path(&st, anilist_id, &mode, &meta.target_file_name)?;

    // Ensure the show is bookmarked in `stream_library` when in streaming/hybrid mode
    {
        let conn = st.db();
        let _ = conn.execute_batch(db::SCHEMA_V7);
        let _ = conn.execute(
            "INSERT OR IGNORE INTO stream_library (anilist_id, added_at) VALUES (?1, ?2)",
            params![anilist_id, db::now()],
        );
    }

    let first_piece = (meta.target_file_offset / meta.piece_length) as usize;
    let last_byte = (meta.target_file_offset + meta.target_file_length).saturating_sub(1);
    let last_piece = ((last_byte / meta.piece_length) as usize).min(meta.pieces.len().saturating_sub(1));

    let num_pieces = meta.pieces.len();
    let session_id = format!("{:x}-{anilist_id}-{ep_key}", db::now());

    let session = Arc::new(SwarmSession {
        id: session_id.clone(),
        anilist_id,
        ep_key: ep_key.clone(),
        title: candidate.title.clone(),
        mode: mode.clone(),
        meta,
        save_path: save_path.clone(),
        first_piece,
        last_piece,
        piece_states: Mutex::new(vec![PieceState::Missing; num_pieces]),
        read_head_piece: AtomicUsize::new(first_piece),
        downloaded_bytes: AtomicU64::new(0),
        speed_bps: AtomicU64::new(0),
        active_peers: AtomicUsize::new(0),
        state_str: Mutex::new("buffering".into()),
        error_str: Mutex::new(None),
        stream_url: Mutex::new(None),
        allow_seeding,
        cancelled: AtomicBool::new(false),
        piece_notify: Notify::new(),
        file_write_lock: tokio::sync::Mutex::new(()),
    });

    // Pre-allocate target file
    {
        let f = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .open(&save_path)
            .await?;
        f.set_len(session.meta.target_file_length).await?;
    }

    if mode == "stream" || mode == "hybrid" {
        let _ = spawn_http_range_server(session.clone()).await?;
    }

    sessions_map()
        .lock()
        .unwrap()
        .insert(session_id.clone(), session.clone());

    // Spawn the swarm + auto-subtitle + player launch task
    let app_clone = app.clone();
    let st_clone = st.clone();
    let sess_clone = session.clone();
    tokio::spawn(async move {
        run_swarm_orchestrator(app_clone, st_clone, sess_clone, episode_num).await;
    });

    Ok(session.status())
}

async fn run_swarm_orchestrator(
    app: AppHandle,
    st: Arc<AppState>,
    session: Arc<SwarmSession>,
    episode_num: Option<i64>,
) {
    let mut peer_id = [0u8; 20];
    peer_id[0..8].copy_from_slice(b"-KR0050-");
    let now_bytes = (db::now() as u64).to_be_bytes();
    peer_id[8..16].copy_from_slice(&now_bytes);
    peer_id[16..20].copy_from_slice(&std::process::id().to_be_bytes());

    let peers = fetch_peers_from_trackers(&session.meta, &peer_id).await;
    if peers.is_empty() {
        *session.state_str.lock().unwrap() = "error".into();
        *session.error_str.lock().unwrap() = Some("No active peers returned by trackers".into());
        let _ = app.emit("torrent-progress", session.status());
        return;
    }

    // Spawn up to 40 peer workers and keep their handles so we can abort them immediately on stop/cancel
    let mut peer_handles = Vec::new();
    for addr in peers.into_iter().take(40) {
        let s = session.clone();
        peer_handles.push(tokio::spawn(async move {
            let _ = run_peer_worker(s, addr, peer_id).await;
        }));
    }

    // Also auto-fetch Japanese subtitles from Jimaku if enabled and key is configured
    let auto_subs = {
        let conn = st.db();
        let enabled = db::get_setting(&conn, "pref.torrent_auto_subs")
            .ok()
            .flatten()
            .map(|v| v != "0")
            .unwrap_or(true);
        let has_key = db::get_setting(&conn, "pref.jimaku_api_key")
            .ok()
            .flatten()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false);
        enabled && has_key
    };

    if auto_subs {
        let st_sub = st.clone();
        let anilist_id = session.anilist_id;
        let ep_key = session.ep_key.clone();
        tokio::spawn(async move {
            if let Ok(res) = crate::subtitles::jimaku_search_files(&st_sub, anilist_id, episode_num).await {
                if let Some(best) = res.files.into_iter().find(|f| !f.is_rar && f.match_score >= 50) {
                    let _ = crate::subtitles::jimaku_download_and_process(
                        &st_sub,
                        anilist_id,
                        &ep_key,
                        &best.name,
                        &best.url,
                        true,
                    )
                    .await;
                }
            }
        });
    }

    let mut launched_player = false;
    let mut last_bytes = 0u64;

    loop {
        if session.cancelled.load(Ordering::Relaxed) {
            for h in &peer_handles {
                h.abort();
            }
            session.speed_bps.store(0, Ordering::Relaxed);
            session.active_peers.store(0, Ordering::Relaxed);
            *session.state_str.lock().unwrap() = "cancelled".into();
            sessions_map().lock().unwrap().remove(&session.id);
            let _ = app.emit("torrent-progress", session.status());
            break;
        }

        let cur_bytes = session.downloaded_bytes.load(Ordering::Relaxed);
        let speed = cur_bytes.saturating_sub(last_bytes);
        last_bytes = cur_bytes;
        session.speed_bps.store(speed, Ordering::Relaxed);

        // Launch player as soon as initial pieces are buffered in "stream" or "hybrid" mode
        if !launched_player && (session.mode == "stream" || session.mode == "hybrid") && session.is_ready_to_stream() {
            launched_player = true;
            *session.state_str.lock().unwrap() = "streaming".into();
            if let Some(url) = session.stream_url.lock().unwrap().clone() {
                let _ = crate::player::play(
                    app.clone(),
                    st.clone(),
                    crate::player::Ep {
                        anilist_id: session.anilist_id,
                        ep_key: session.ep_key.clone(),
                        path: url,
                    },
                );
            }
        }

        if session.is_all_complete() {
            for h in &peer_handles {
                h.abort();
            }
            session.speed_bps.store(0, Ordering::Relaxed);
            session.active_peers.store(0, Ordering::Relaxed);
            *session.state_str.lock().unwrap() = "completed".into();
            sessions_map().lock().unwrap().remove(&session.id);
            let _ = app.emit("torrent-progress", session.status());

            // If mode is "hybrid" or "download", trigger a rescan so the downloaded file is linked in library
            if session.mode == "hybrid" || session.mode == "download" {
                // Attempt auto-retime on any linked Japanese subtitles now that the full .mkv is on disk
                if let Ok(subs) = crate::subtitles::list_linked_subtitles(&st.db(), session.anilist_id) {
                    for sub in subs.into_iter().filter(|s| s.ep_key == session.ep_key) {
                        let sub_path = Path::new(&sub.path);
                        let _ = crate::subtitles::retime_subtitle_file(
                            sub_path,
                            &session.save_path,
                            sub_path,
                        );
                    }
                }
                crate::scan::spawn_scan(app.clone(), st.clone(), None);
            }
            break;
        }

        let _ = app.emit("torrent-progress", session.status());
        tokio::select! {
            _ = session.piece_notify.notified(), if session.cancelled.load(Ordering::Relaxed) => {}
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_matches_rfc3174_vectors() {
        assert_eq!(
            hex_encode(&sha1_digest(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex_encode(&sha1_digest(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
    }

    #[test]
    fn parses_bencode_and_torrent_metadata() {
        // Minimal synthetic single-file .torrent:
        // d8:announce27:http://tracker.example/ann4:infod6:lengthi1048576e4:name11:episode.mkv12:piece lengthi262144e6:pieces20:01234567890123456789ee
        let raw = b"d8:announce26:http://tracker.example/ann4:infod6:lengthi1048576e4:name11:episode.mkv12:piece lengthi262144e6:pieces20:01234567890123456789ee";
        let meta = parse_torrent_bytes(raw, Some(1)).unwrap();
        assert_eq!(meta.name, "episode.mkv");
        assert_eq!(meta.total_length, 1_048_576);
        assert_eq!(meta.piece_length, 262_144);
        assert_eq!(meta.pieces.len(), 1);
        assert!(meta.trackers.iter().any(|t| t == "http://tracker.example/ann"));
    }

    #[test]
    fn scores_and_parses_release_tags() {
        let title = "[SubsPlease] Kusuriya no Hitorigoto - 25 (1080p) [ABCD1234].mkv";
        let (group, res, batch) = parse_release_tags(title);
        assert_eq!(group.as_deref(), Some("SubsPlease"));
        assert_eq!(res.as_deref(), Some("1080p"));
        assert!(!batch);

        let s_match = score_candidate(title, group.as_deref(), res.as_deref(), 150, batch, 25, "1080p");
        let s_wrong_ep = score_candidate(title, group.as_deref(), res.as_deref(), 150, batch, 12, "1080p");
        assert!(s_match > 1500);
        assert!(s_wrong_ep < 0);
    }
}
