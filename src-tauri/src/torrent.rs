//! Built-in Modular Torrent Extension Engine, Pure-Rust BitTorrent Swarm (BEP 3 / BEP 10 / BEP 11 PEX / BEP 15),
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
pub struct TorrentExtension {
    pub id: String,
    pub name: String,
    /// `"rss"` | `"subsplease-json"` | `"seadex-api"`
    pub kind: String,
    pub url: String,
    pub enabled: bool,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub accuracy: Option<String>,
}

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
    /// Optional pre-extracted reference subtitle URL for instant retiming.
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
    pub mode: String,  // "stream" | "hybrid" | "download"
    pub state: String, // "initializing" | "buffering" | "streaming" | "downloading" | "completed" | "error"
    pub progress_pct: f64,
    pub downloaded_bytes: u64,
    pub uploaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
    pub upload_speed_bps: u64,
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
    parse_btih_20(hex)
}

/// Decode a 32-character RFC 4648 Base32 `btih` string (used by SubsPlease & Anidex magnets) into 20 bytes.
pub fn base32_decode_20(b32: &str) -> Option<[u8; 20]> {
    let clean = b32.trim();
    if clean.len() != 32 {
        return None;
    }
    let mut out = [0u8; 20];
    let mut buffer: u64 = 0;
    let mut bits_left: u32 = 0;
    let mut out_idx = 0;
    for ch in clean.bytes() {
        let val: u64 = match ch {
            b'A'..=b'Z' => (ch - b'A') as u64,
            b'a'..=b'z' => (ch - b'a') as u64,
            b'2'..=b'7' => (ch - b'2' + 26) as u64,
            _ => return None,
        };
        buffer = (buffer << 5) | val;
        bits_left += 5;
        if bits_left >= 8 {
            bits_left -= 8;
            if out_idx >= 20 {
                return None;
            }
            out[out_idx] = ((buffer >> bits_left) & 0xFF) as u8;
            out_idx += 1;
        }
    }
    if out_idx == 20 {
        Some(out)
    } else {
        None
    }
}

/// Parse either a 40-char hex or 32-char RFC 4648 Base32 `btih` info_hash into 20 raw bytes.
pub fn parse_btih_20(raw: &str) -> Option<[u8; 20]> {
    let clean = raw.trim();
    if clean.len() == 40 {
        let mut out = [0u8; 20];
        for i in 0..20 {
            out[i] = u8::from_str_radix(&clean[i * 2..i * 2 + 2], 16).ok()?;
        }
        return Some(out);
    }
    if clean.len() == 32 {
        return base32_decode_20(clean);
    }
    None
}

/// Normalize any 40-char hex or 32-char Base32 `btih` info_hash into a 40-char lowercase hex string.
pub fn normalize_btih_hex(raw: &str) -> Option<String> {
    parse_btih_20(raw).map(|b| hex_encode(&b))
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
        self.as_bytes()
            .map(|b| String::from_utf8_lossy(b).into_owned())
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
            entries
                .iter()
                .find(|(k, ..)| k.as_slice() == key)
                .map(|(_, v, ..)| v)
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
    // Always include standard public BitTorrent UDP trackers as fallback
    for fallback in [
        "udp://open.stealth.si:80/announce",
        "udp://tracker.opentrackr.org:1337/announce",
        "udp://exodus.desync.com:6969/announce",
        "udp://tracker.torrent.eu.org:451/announce",
        "udp://opentracker.io:6969/announce",
        "udp://tracker.cyberia.is:6969/announce",
        "udp://explodie.org:6969/announce",
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
        lower.ends_with(".mkv")
            || lower.ends_with(".mp4")
            || lower.ends_with(".webm")
            || lower.ends_with(".avi")
    };

    // Pick best matching video file for `target_episode` (preferring non-extra/non-NCOP/NCED episodes)
    let chosen = if let Some(ep) = target_episode {
        let ep_u32 = ep as u32;
        candidates
            .iter()
            .filter(|(n, _, _)| is_video(n))
            .find(|(n, _, _)| {
                let p = crate::parser::parse_filename(n);
                !p.extra && p.special.is_none() && p.episode == Some(ep_u32)
            })
            .or_else(|| {
                candidates
                    .iter()
                    .filter(|(n, _, _)| is_video(n))
                    .find(|(n, _, _)| {
                        let p = crate::parser::parse_filename(n);
                        p.episode == Some(ep_u32)
                    })
            })
            .or_else(|| {
                candidates
                    .iter()
                    .filter(|(n, _, _)| is_video(n))
                    .max_by_key(|(_, _, l)| *l)
            })
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
    let batch_re = BATCH_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(batch|complete|\d+\s*-\s*\d+|season\s*\d+\s*complete)\b").unwrap()
    });

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
    score_candidate_with_context(
        title,
        group,
        resolution,
        seeders,
        0,
        is_batch,
        target_episode,
        None,
        false,
        pref_res,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
/// Seeder-health component of the candidate score.
pub fn seeder_score(seeders: i64) -> i64 {
    if seeders <= 0 {
        -5000
    } else if seeders == 1 {
        -900
    } else if seeders <= 3 {
        -450 + seeders * 30
    } else if seeders <= 7 {
        -100 + seeders * 30
    } else if seeders <= 20 {
        250 + seeders * 15
    } else if seeders <= 80 {
        550 + seeders * 8
    } else {
        1190 + (seeders.min(500) - 80) * 2
    }
}

/// Whether a candidate is relevant to the requested episode, ignoring swarm health.
/// Indexer seeder counts are often stale (fresh uploads show 0 for minutes, old releases
/// show 0 while DHT still has peers), so low seeders must only lower the rank, never hide a release.
pub fn is_relevant_candidate(score: i64, seeders: i64) -> bool {
    score - seeder_score(seeders) >= -500
}

pub fn score_candidate_with_context(
    title: &str,
    group: Option<&str>,
    resolution: Option<&str>,
    seeders: i64,
    size_bytes: i64,
    is_batch: bool,
    target_episode: i64,
    abs_episode: Option<i64>,
    is_movie: bool,
    pref_res: &str,
    known_titles: &[String],
) -> i64 {
    // 1. Seeder health is the #1 predictor of whether a torrent will connect & stream without stalling.
    // Torrents with only 1–3 seeders are frequently NAT-locked home peers that reject incoming TCP connections.
    let mut score = seeder_score(seeders);

    let parsed = crate::parser::parse_filename(title);

    let ep_u32 = target_episode.max(0) as u32;
    let abs_u32 = abs_episode.filter(|&a| a > 0).map(|a| a as u32);
    let in_range = match (parsed.episode, parsed.episode_end) {
        (Some(start), Some(end)) if end > start => {
            (ep_u32 >= start && ep_u32 <= end)
                || abs_u32.map(|a| a >= start && a <= end).unwrap_or(false)
        }
        _ => false,
    };

    let matches_ep = parsed.episode == Some(ep_u32)
        || (abs_u32.is_some() && parsed.episode == abs_u32)
        || (is_movie && target_episode <= 1 && parsed.episode.is_none());

    if matches_ep && !is_batch && !in_range {
        score += 1400;
    } else if is_batch || in_range {
        score += 350;
    } else if parsed.episode.is_some() && !is_movie {
        // Different episode number!
        score -= 2500;
    }

    // 2. Smart file-size scoring for fast startup & smooth streaming
    if size_bytes > 0 {
        let mb = size_bytes / (1024 * 1024);
        if is_batch || in_range {
            // Prefer smaller/medium batches over massive 60+ GB BD remux batches
            if mb > 35_000 {
                score -= 700;
            } else if mb > 18_000 {
                score -= 300;
            }
        } else if !is_movie {
            // Single 24-min TV episode:
            // Sweet spot is 180 MB – 1,550 MB (fast piece size, quick header buffer, low bandwidth demand)
            if (180..=1550).contains(&mb) {
                score += 350;
            } else if (1551..=2200).contains(&mb) {
                score += 100;
            } else if mb > 4000 {
                // Uncompressed Blu-ray Remux (> 4 GB for 1 episode)
                score -= 950;
            } else if mb > 2500 {
                score -= 400;
            } else if mb < 60 {
                // Suspiciously tiny / broken / trailer
                score -= 600;
            }
        }
    }

    // Penalize raw BD-Remux tags for streaming
    let lower_title = title.to_ascii_lowercase();
    if lower_title.contains("remux")
        || lower_title.contains("bdmv")
        || lower_title.contains("bdremux")
    {
        score -= 600;
    }

    // Filter out Dub-only (English/Russian/French/German/Spanish/Hindi/etc. audio only without Japanese audio).
    // Dual-Audio / Multi-Audio releases that include Japanese audio are kept and given a small bonus.
    static DUAL_JP_RE: OnceLock<Regex> = OnceLock::new();
    static DUB_ONLY_RE: OnceLock<Regex> = OnceLock::new();
    let dual_jp_re = DUAL_JP_RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:dual[\s_\-]*audio|multi[\s_\-]*audio|jpn|jap|japanese|ja[\s_\-\+/]+en|en[\s_\-\+/]+ja)\b")
            .unwrap()
    });
    let dub_only_re = DUB_ONLY_RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:\b(?:eng(?:lish)?[\s_\-]*dub(?:bed)?|dubbed|dub[\s_\-]*only|english[\s_\-]*audio|multi[\s_\-]*dub|(?:rus(?:sian)?|ger(?:man)?|fre?(?:nch)?|spa(?:nish)?|ita(?:lian)?|hin(?:di)?|por(?:tuguese)?|tam(?:il)?|tel(?:ugu)?)[\s_\-]*(?:dub(?:bed)?|audio)|vostfr)\b|[\[\(]\s*(?:eng?(?:lish)?\s*)?dub(?:bed|s)?\s*[\]\)])",
        )
        .unwrap()
    });

    let has_dual_or_jp = dual_jp_re.is_match(title);
    let is_dub_group = group
        .map(|g| {
            matches!(
                g.trim().to_ascii_lowercase().as_str(),
                "yameii" | "anime-dub" | "dubbedanime"
            )
        })
        .unwrap_or(false);
    if has_dual_or_jp {
        score += 100;
    } else if dub_only_re.is_match(title) || is_dub_group {
        score -= 6500;
    }

    if let Some(res) = resolution {
        if res.eq_ignore_ascii_case(pref_res) {
            score += 450;
        } else if res == "1080p" {
            score += 300;
        } else if res == "720p" {
            score += 180;
        }
    }

    if let Some(g) = group {
        let gl = g.trim().to_ascii_lowercase();
        if matches!(
            gl.as_str(),
            "subsplease"
                | "erai-raws"
                | "ember"
                | "judas"
                | "dkb"
                | "toffeesubs"
                | "asw"
                | "vodes"
                | "kaleido"
                | "nyaa"
                | "varyg"
                | "tsundere-raws"
                | "toonshub"
                | "guzzi"
        ) {
            score += 250;
        }
    }

    // Verify title token overlap when known titles are provided so broad/batch queries
    // never rank an unrelated show above the target anime.
    if !known_titles.is_empty() {
        let cand_clean = clean_search_title(&parsed.title).to_ascii_lowercase();
        let cand_full = clean_search_title(title).to_ascii_lowercase();
        let mut best_overlap = 0usize;
        let mut any_match = false;
        for kt in known_titles {
            let kt_clean = clean_search_title(kt).to_ascii_lowercase();
            let words: Vec<&str> = kt_clean
                .split_whitespace()
                .filter(|w| {
                    w.len() >= 3
                        && !matches!(*w, "the" | "and" | "for" | "season" | "part" | "cour")
                })
                .collect();
            if words.is_empty() {
                continue;
            }
            let matched = words
                .iter()
                .filter(|&&w| cand_clean.contains(w) || cand_full.contains(w))
                .count();
            if matched * 2 >= words.len() {
                any_match = true;
                best_overlap = best_overlap.max(matched);
            }
        }
        if any_match {
            score += (best_overlap.min(5) as i64) * 70;
        } else {
            score -= 900;
        }
    }

    score
}

/// Strip punctuation and noise tokens that cause Nyaa's strict keyword search to miss releases.
pub fn clean_search_title(raw: &str) -> String {
    static NOISE_RE: OnceLock<Regex> = OnceLock::new();
    static POSSESSIVE_RE: OnceLock<Regex> = OnceLock::new();
    static SPACE_RE: OnceLock<Regex> = OnceLock::new();
    let noise_re = NOISE_RE.get_or_init(|| {
        Regex::new(r"(?i)\((?:tv|ona|ova|movie|special|20\d{2}|19\d{2})\)").unwrap()
    });
    let possessive_re = POSSESSIVE_RE.get_or_init(|| Regex::new(r"(?i)['’]s\b").unwrap());
    let space_re = SPACE_RE.get_or_init(|| Regex::new(r"\s+").unwrap());

    let without_tags = noise_re.replace_all(raw, " ");
    let without_possessive = possessive_re.replace_all(&without_tags, "");
    let mapped: String = without_possessive
        .chars()
        .map(|c| match c {
            ':' | ';' | '!' | '?' | '~' | '〜' | '—' | '–' | '・' | '"' | '“' | '”' | '’'
            | '\'' | '/' | '\\' | '*' | '|' | ',' | '☆' | '★' | '♡' | '♪' => ' ',
            _ => c,
        })
        .collect();
    space_re.replace_all(&mapped, " ").trim().to_string()
}

/// Extract `(optional_season_number, base_title_without_season_suffix)` from an AniList title.
fn split_season_from_title(title: &str) -> (Option<u32>, String) {
    static S_ORD_RE: OnceLock<Regex> = OnceLock::new();
    static S_WORD_RE: OnceLock<Regex> = OnceLock::new();
    static S_PART_RE: OnceLock<Regex> = OnceLock::new();
    static S_ROMAN_RE: OnceLock<Regex> = OnceLock::new();

    let ord_re =
        S_ORD_RE.get_or_init(|| Regex::new(r"(?i)\b(\d{1,2})(?:st|nd|rd|th)\s+season\b").unwrap());
    let word_re = S_WORD_RE.get_or_init(|| Regex::new(r"(?i)\bseason\s*(\d{1,2})\b").unwrap());
    let part_re =
        S_PART_RE.get_or_init(|| Regex::new(r"(?i)\b(?:part|cour)\s*(\d{1,2})\b").unwrap());
    let roman_re = S_ROMAN_RE.get_or_init(|| Regex::new(r"\s+(II|III|IV|V)\s*$").unwrap());

    let mut season: Option<u32> = None;
    let mut base = title.to_string();

    if let Some(c) = ord_re.captures(&base) {
        season = c.get(1).and_then(|m| m.as_str().parse().ok());
        base = ord_re.replace_all(&base, " ").to_string();
    } else if let Some(c) = word_re.captures(&base) {
        season = c.get(1).and_then(|m| m.as_str().parse().ok());
        base = word_re.replace_all(&base, " ").to_string();
    } else if let Some(c) = roman_re.captures(&base) {
        season = match c.get(1).map(|m| m.as_str()).unwrap_or("") {
            "II" => Some(2),
            "III" => Some(3),
            "IV" => Some(4),
            "V" => Some(5),
            _ => None,
        };
        base = roman_re.replace_all(&base, " ").to_string();
    }

    if part_re.is_match(&base) {
        base = part_re.replace_all(&base, " ").to_string();
    }

    (season, clean_search_title(&base))
}

fn push_unique_query(list: &mut Vec<String>, q: String) {
    let trimmed = q.trim();
    if trimmed.len() < 2 {
        return;
    }
    if !list
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(trimmed))
    {
        list.push(trimmed.to_string());
    }
}

// ---------------------------------------------------------------------------
// Modular Bring-Your-Own Torrent Extensions (Hayase-style Repository / Source System)
// ---------------------------------------------------------------------------

pub fn list_torrent_extensions(conn: &rusqlite::Connection) -> Vec<TorrentExtension> {
    let mut exts: Vec<TorrentExtension> = db::get_setting(conn, "pref.torrent_extensions")
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    // Migrate legacy `pref.nyaa_url` if user had a custom URL saved previously
    if exts.is_empty() {
        if let Some(legacy_url) = db::get_setting(conn, "pref.nyaa_url")
            .ok()
            .flatten()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| s.starts_with("http"))
        {
            let ext = build_extension_from_url(&legacy_url, None, None);
            exts.push(ext);
            let _ = save_torrent_extensions(conn, &exts);
        }
    }

    for ext in &mut exts {
        enrich_extension_metadata(ext);
    }

    exts
}

fn enrich_extension_metadata(ext: &mut TorrentExtension) {
    let host = ext
        .url
        .split("://")
        .nth(1)
        .unwrap_or(&ext.url)
        .split('/')
        .next()
        .unwrap_or("source");
    let lower = format!("{} {} {}", ext.name, ext.kind, host).to_ascii_lowercase();

    if ext.icon_url.as_deref().unwrap_or("").is_empty() && host.contains('.') {
        ext.icon_url = Some(format!(
            "https://www.google.com/s2/favicons?domain={host}&sz=128"
        ));
    }
    if ext.version.as_deref().unwrap_or("").is_empty() {
        ext.version = Some("1.2.0".to_string());
    }
    if ext.accuracy.as_deref().unwrap_or("").is_empty() {
        ext.accuracy = Some(
            if ext.kind == "seadex-api" || lower.contains("seadex") {
                "Curated · Best".to_string()
            } else if ext.kind == "subsplease-json" || lower.contains("subsplease") {
                "Simulcast · Fast".to_string()
            } else {
                "High Accuracy".to_string()
            },
        );
    }
    if ext.description.as_deref().unwrap_or("").is_empty() {
        ext.description = Some(if ext.kind == "seadex-api" || lower.contains("seadex") {
            "Curated best-release anime index mapped directly by AniList ID.".to_string()
        } else if ext.kind == "subsplease-json" || lower.contains("subsplease") {
            "Official simulcast JSON release feed for 1080p & 720p web-rips.".to_string()
        } else if lower.contains("tosho") {
            "High-speed anime mirror with pre-extracted subtitle attachments.".to_string()
        } else if lower.contains("nyaa") {
            "Primary public anime BitTorrent RSS indexer with full swarm metrics.".to_string()
        } else {
            format!("BitTorrent RSS release indexer ({host}).")
        });
    }
}

pub fn save_torrent_extensions(
    conn: &rusqlite::Connection,
    exts: &[TorrentExtension],
) -> Result<()> {
    let json = serde_json::to_string(exts)?;
    db::set_setting(conn, "pref.torrent_extensions", &json)?;
    clear_torrent_search_cache();
    Ok(())
}

fn torrent_http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(1600))
            .timeout(Duration::from_millis(3200))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
            .tcp_keepalive(Duration::from_secs(30))
            .user_agent("Kura/0.6.1 (https://github.com/wantingCat/kura)")
            .build()
            .expect("torrent http client")
    })
}

type SearchCacheKey = (i64, i64, String, String);

/// HTTP client for indexer searches & `.torrent` downloads. Indexers (especially Nyaa RSS) regularly take
/// 3–8 s to answer from some regions, so this uses far more generous timeouts than the peer-wire helpers.
fn torrent_search_http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(6))
            .timeout(Duration::from_secs(14))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(16)
            .tcp_keepalive(Duration::from_secs(30))
            .user_agent("Kura/0.6 (https://github.com/wantingCat/kura)")
            .build()
            .expect("torrent search http client")
    })
}

const SCRAPE_UDP_TRACKERS: [&str; 3] = [
    "tracker.opentrackr.org:1337",
    "open.stealth.si:80",
    "exodus.desync.com:6969",
];
const SCRAPE_HTTP_TRACKERS: [&str; 1] = ["http://nyaa.tracker.wf:7777/scrape"];

/// BEP 15 UDP scrape: returns `info_hash -> (seeders, leechers)`.
async fn scrape_udp(host_port: &str, hashes: &[[u8; 20]]) -> Result<HashMap<[u8; 20], (i64, i64)>> {
    let sock = UdpSocket::bind("0.0.0.0:0").await?;
    tokio::time::timeout(Duration::from_secs(3), sock.connect(host_port)).await??;
    let tx_id: u32 = 0x4B52_5343; // "KRSC"
    let mut req = [0u8; 16];
    req[0..8].copy_from_slice(&0x41727101980u64.to_be_bytes());
    req[8..12].copy_from_slice(&0u32.to_be_bytes());
    req[12..16].copy_from_slice(&tx_id.to_be_bytes());
    sock.send(&req).await?;
    let mut resp = [0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(3), sock.recv(&mut resp)).await??;
    if n < 16 || u32::from_be_bytes(resp[0..4].try_into()?) != 0 {
        bail!("bad UDP connect response");
    }
    let conn_id = u64::from_be_bytes(resp[8..16].try_into()?);

    let mut out = HashMap::new();
    for chunk in hashes.chunks(70) {
        let mut pkt = Vec::with_capacity(16 + chunk.len() * 20);
        pkt.extend_from_slice(&conn_id.to_be_bytes());
        pkt.extend_from_slice(&2u32.to_be_bytes()); // action = scrape
        pkt.extend_from_slice(&tx_id.to_be_bytes());
        for h in chunk {
            pkt.extend_from_slice(h);
        }
        sock.send(&pkt).await?;
        let mut buf = vec![0u8; 8 + chunk.len() * 12];
        let len = tokio::time::timeout(Duration::from_secs(3), sock.recv(&mut buf)).await??;
        if len < 8 || u32::from_be_bytes(buf[0..4].try_into()?) != 2 {
            bail!("bad UDP scrape response");
        }
        for (i, rec) in buf[8..len].chunks_exact(12).enumerate() {
            if let Some(h) = chunk.get(i) {
                let seeders = u32::from_be_bytes(rec[0..4].try_into()?) as i64;
                let leechers = u32::from_be_bytes(rec[8..12].try_into()?) as i64;
                out.insert(*h, (seeders, leechers));
            }
        }
    }
    Ok(out)
}

/// HTTP tracker scrape (`/scrape?info_hash=…`): returns `info_hash -> (seeders, leechers)`.
async fn scrape_http(scrape_url: &str, hashes: &[[u8; 20]]) -> Result<HashMap<[u8; 20], (i64, i64)>> {
    let mut url = String::from(scrape_url);
    for (i, h) in hashes.iter().take(60).enumerate() {
        url.push(if i == 0 { '?' } else { '&' });
        url.push_str("info_hash=");
        url.push_str(&url_encode_bytes(h));
    }
    let bytes = torrent_http().get(&url).send().await?.bytes().await?;
    let (root, _) = parse_bencode(&bytes)?;
    let mut out = HashMap::new();
    if let Some(BVal::Dict(files)) = root.dict_get(b"files") {
        for (k, v, ..) in files {
            if k.len() == 20 {
                let mut h = [0u8; 20];
                h.copy_from_slice(k);
                let seeders = v.dict_get(b"complete").and_then(|x| x.as_int()).unwrap_or(0);
                let leechers = v.dict_get(b"incomplete").and_then(|x| x.as_int()).unwrap_or(0);
                out.insert(h, (seeders, leechers));
            }
        }
    }
    Ok(out)
}

/// Replace placeholder swarm stats on magnet-only candidates (SubsPlease, SeaDex) with real tracker scrape data.
async fn refresh_swarm_health(cands: &mut [TorrentCandidate]) {
    let targets: Vec<(usize, [u8; 20])> = cands
        .iter()
        .enumerate()
        .filter(|(_, c)| c.torrent_url.is_none())
        .filter_map(|(i, c)| c.info_hash.as_deref().and_then(parse_btih_20).map(|h| (i, h)))
        .take(60)
        .collect();
    if targets.is_empty() {
        return;
    }
    let hashes: Vec<[u8; 20]> = targets.iter().map(|(_, h)| *h).collect();

    let mut tasks = Vec::new();
    for t in SCRAPE_UDP_TRACKERS {
        let hs = hashes.clone();
        tasks.push(tokio::spawn(async move { scrape_udp(t, &hs).await.unwrap_or_default() }));
    }
    for t in SCRAPE_HTTP_TRACKERS {
        let hs = hashes.clone();
        tasks.push(tokio::spawn(async move { scrape_http(t, &hs).await.unwrap_or_default() }));
    }

    let mut best: HashMap<[u8; 20], (i64, i64)> = HashMap::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(3500);
    for t in tasks {
        if let Ok(Ok(map)) = tokio::time::timeout_at(deadline, t).await {
            for (h, (s, l)) in map {
                let e = best.entry(h).or_insert((0, 0));
                e.0 = e.0.max(s);
                e.1 = e.1.max(l);
            }
        }
    }
    for (i, h) in targets {
        if let Some(&(s, l)) = best.get(&h) {
            cands[i].seeders = s;
            cands[i].leechers = l;
        }
    }
}

fn search_cache(
) -> &'static Mutex<HashMap<SearchCacheKey, (std::time::Instant, Vec<TorrentCandidate>)>> {
    static CACHE: OnceLock<
        Mutex<HashMap<SearchCacheKey, (std::time::Instant, Vec<TorrentCandidate>)>>,
    > = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn in_flight_searches() -> &'static Mutex<HashMap<SearchCacheKey, Arc<Notify>>> {
    static IN_FLIGHT: OnceLock<Mutex<HashMap<SearchCacheKey, Arc<Notify>>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn clear_torrent_search_cache() {
    if let Ok(mut guard) = search_cache().lock() {
        guard.clear();
    }
}

fn build_extension_from_url(
    url: &str,
    custom_name: Option<String>,
    custom_kind: Option<String>,
) -> TorrentExtension {
    let clean_url = url.trim().trim_end_matches('/').to_string();
    let lower = clean_url.to_ascii_lowercase();
    let kind = custom_kind.unwrap_or_else(|| {
        if lower.contains("collections/entries/records") || lower.contains("releases.moe") {
            "seadex-api".to_string()
        } else if lower.contains("f=search") || lower.contains("subsplease") {
            "subsplease-json".to_string()
        } else {
            "rss".to_string()
        }
    });

    let host = clean_url
        .split("://")
        .nth(1)
        .unwrap_or(&clean_url)
        .split('/')
        .next()
        .unwrap_or("Custom Source");

    let name = custom_name.unwrap_or_else(|| match kind.as_str() {
        "seadex-api" => format!("SeaDex ({host})"),
        "subsplease-json" => format!("SubsPlease ({host})"),
        _ => format!("{host} (RSS)"),
    });

    let id = hex_encode(&sha1_digest(format!("{kind}:{clean_url}").as_bytes()))[..12].to_string();
    let mut ext = TorrentExtension {
        id,
        name,
        kind,
        url: clean_url,
        enabled: true,
        icon_url: None,
        description: None,
        version: None,
        accuracy: None,
    };
    enrich_extension_metadata(&mut ext);
    ext
}

pub fn toggle_torrent_extension(
    st: &AppState,
    id: &str,
    enabled: bool,
) -> Result<Vec<TorrentExtension>> {
    let conn = st.db();
    let mut exts = list_torrent_extensions(&conn);
    for ext in &mut exts {
        if ext.id == id {
            ext.enabled = enabled;
        }
    }
    save_torrent_extensions(&conn, &exts)?;
    Ok(exts)
}

pub fn remove_torrent_extension(st: &AppState, id: &str) -> Result<Vec<TorrentExtension>> {
    let conn = st.db();
    let mut exts = list_torrent_extensions(&conn);
    exts.retain(|e| e.id != id);
    save_torrent_extensions(&conn, &exts)?;
    Ok(exts)
}

/// Import a Torrent Extension from:
/// - A direct RSS / SubsPlease JSON / SeaDex API URL (`https://...`)
/// - A JSON manifest URL (including Hayase `index.json` manifests or Kura JSON manifests)
/// - Raw JSON pasted directly by the user
pub async fn import_torrent_extension(
    st: &AppState,
    raw_input: &str,
) -> Result<Vec<TorrentExtension>> {
    let trimmed = raw_input.trim();
    if trimmed.is_empty() {
        bail!("Extension URL or manifest cannot be empty");
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Kura/0.5")
        .build()?;

    let mut discovered: Vec<TorrentExtension> = Vec::new();

    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        discovered.extend(parse_manifest_json_or_script(&client, trimmed, None).await);
    } else if let Some(gh_repo) = trimmed
        .strip_prefix("gh:")
        .or_else(|| trimmed.strip_prefix("github:"))
        .or_else(|| {
            trimmed
                .strip_prefix("https://github.com/")
                .filter(|r| !r.ends_with(".json") && !r.ends_with(".js"))
        })
    {
        let clean_repo = gh_repo.trim_matches('/');
        for candidate_url in [
            format!("https://raw.githubusercontent.com/{clean_repo}/main/index.json"),
            format!("https://raw.githubusercontent.com/{clean_repo}/main/hayase/index.json"),
            format!("https://raw.githubusercontent.com/{clean_repo}/master/index.json"),
        ] {
            if let Ok(resp) = client.get(&candidate_url).send().await {
                if resp.status().is_success() {
                    if let Ok(text) = resp.text().await {
                        let items =
                            parse_manifest_json_or_script(&client, &text, Some(&candidate_url))
                                .await;
                        if !items.is_empty() {
                            discovered.extend(items);
                            break;
                        }
                    }
                }
            }
        }
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let lower = trimmed.to_ascii_lowercase();
        let looks_like_manifest = lower.ends_with(".json")
            || lower.contains("raw.githubusercontent.com")
            || lower.contains("gist.githubusercontent.com")
            || lower.ends_with(".js");

        if looks_like_manifest {
            let text = client
                .get(trimmed)
                .send()
                .await
                .context("Failed to fetch extension manifest URL")?
                .text()
                .await?;
            discovered.extend(parse_manifest_json_or_script(&client, &text, Some(trimmed)).await);
        } else {
            discovered.push(build_extension_from_url(trimmed, None, None));
        }
    } else if trimmed.contains('.') && !trimmed.contains(' ') {
        let with_https = format!("https://{trimmed}");
        discovered.push(build_extension_from_url(&with_https, None, None));
    } else {
        bail!("Unrecognized extension format. Paste an `https://` source or manifest URL.");
    }

    if discovered.is_empty() {
        bail!("No compatible torrent sources found in the provided manifest/URL.");
    }

    let conn = st.db();
    let mut existing = list_torrent_extensions(&conn);
    for ext in discovered {
        if let Some(pos) = existing
            .iter()
            .position(|e| e.url.eq_ignore_ascii_case(&ext.url) && e.kind == ext.kind)
        {
            existing[pos].enabled = true;
            existing[pos].name = ext.name;
            if ext.icon_url.is_some() {
                existing[pos].icon_url = ext.icon_url;
            }
            if ext.description.is_some() {
                existing[pos].description = ext.description;
            }
            if ext.version.is_some() {
                existing[pos].version = ext.version;
            }
            if ext.accuracy.is_some() {
                existing[pos].accuracy = ext.accuracy;
            }
        } else {
            existing.push(ext);
        }
    }
    save_torrent_extensions(&conn, &existing)?;
    Ok(existing)
}

async fn parse_manifest_json_or_script(
    client: &reqwest::Client,
    text: &str,
    base_manifest_url: Option<&str>,
) -> Vec<TorrentExtension> {
    let mut out = Vec::new();
    let resolve_rel = |raw_ref: &str| -> Option<String> {
        if raw_ref.starts_with("http://") || raw_ref.starts_with("https://") {
            Some(raw_ref.to_string())
        } else if let Some(base) = base_manifest_url {
            base.rfind('/').map(|idx| {
                format!("{}/{}", &base[..idx], raw_ref.trim_start_matches("./"))
            })
        } else {
            None
        }
    };

    if let Ok(val) = serde_json::from_str::<serde_json::Value>(text) {
        let arr_opt = val
            .as_array()
            .cloned()
            .or_else(|| val.get("sources").and_then(|s| s.as_array()).cloned())
            .or_else(|| val.get("extensions").and_then(|s| s.as_array()).cloned());

        if let Some(arr) = arr_opt {
            for item in arr {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let kind = item
                    .get("kind")
                    .or_else(|| item.get("type"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let icon_url = item
                    .get("icon")
                    .or_else(|| item.get("logo"))
                    .and_then(|v| v.as_str())
                    .and_then(&resolve_rel);
                let description = item
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let version = item
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let accuracy = item
                    .get("accuracy")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let apply_meta = |mut ext: TorrentExtension| -> TorrentExtension {
                    if let Some(ref ic) = icon_url {
                        ext.icon_url = Some(ic.clone());
                    }
                    if let Some(ref d) = description {
                        ext.description = Some(d.clone());
                    }
                    if let Some(ref v) = version {
                        ext.version = Some(v.clone());
                    }
                    if let Some(ref a) = accuracy {
                        ext.accuracy = Some(a.clone());
                    }
                    enrich_extension_metadata(&mut ext);
                    ext
                };

                if let Some(url) = item
                    .get("url")
                    .or_else(|| item.get("api"))
                    .and_then(|v| v.as_str())
                {
                    if url.starts_with("http") {
                        out.push(apply_meta(build_extension_from_url(url, name.clone(), kind.clone())));
                        continue;
                    }
                }
                if let Some(code_ref) = item
                    .get("code")
                    .or_else(|| item.get("main"))
                    .and_then(|v| v.as_str())
                {
                    if let Some(js_url) = resolve_rel(code_ref) {
                        if let Ok(resp) = client.get(&js_url).send().await {
                            if let Ok(js_code) = resp.text().await {
                                for ext in extract_sources_from_script(&js_code, name.as_deref()) {
                                    out.push(apply_meta(ext));
                                }
                            }
                        }
                    }
                }
            }
        } else if let Some(url) = val.get("url").and_then(|v| v.as_str()) {
            let name = val
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let kind = val
                .get("kind")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            out.push(build_extension_from_url(url, name, kind));
        }
    }

    if out.is_empty() {
        out.extend(extract_sources_from_script(text, None));
    }

    out
}

fn extract_sources_from_script(script: &str, hint_name: Option<&str>) -> Vec<TorrentExtension> {
    static URL_RE: OnceLock<Regex> = OnceLock::new();
    let url_re = URL_RE
        .get_or_init(|| Regex::new(r#"https?://[a-zA-Z0-9.\-_]+(?:/[a-zA-Z0-9.\-_/]*)?"#).unwrap());
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for m in url_re.find_iter(script) {
        let raw = m.as_str().trim_end_matches('/');
        let lower = raw.to_ascii_lowercase();
        // Skip non-indexer, dead, or non-standard URLs commonly found in extension metadata
        if lower.contains("github.com")
            || lower.contains("githubusercontent.com")
            || lower.contains("anilist.co")
            || lower.contains("w3.org")
            || lower.contains("schema.org")
            || lower.contains("animetosho.org")
            || lower.contains("sukebei.")
            || lower.contains("openbittorrent")
            || lower.contains("opentrackr")
            || lower.contains("tracker.")
        {
            continue;
        }
        let ext = if lower.contains("collections/entries/records") || lower.contains("releases.moe")
        {
            let base = if lower.contains("collections/entries/records") {
                raw.to_string()
            } else {
                format!("{raw}/api/collections/entries/records")
            };
            build_extension_from_url(
                &base,
                hint_name.map(|s| s.to_string()),
                Some("seadex-api".into()),
            )
        } else if lower.contains("subsplease") {
            let host_root = raw.split("/api").next().unwrap_or(raw);
            build_extension_from_url(
                &format!("{host_root}/api/"),
                hint_name.map(|s| s.to_string()),
                Some("subsplease-json".into()),
            )
        } else {
            let host_root = raw
                .split("://")
                .nth(1)
                .and_then(|rest| rest.split('/').next())
                .map(|h| format!("{}://{h}", raw.split("://").next().unwrap_or("https")))
                .unwrap_or_else(|| raw.to_string());
            build_extension_from_url(
                &host_root,
                hint_name.map(|s| s.to_string()),
                Some("rss".into()),
            )
        };
        if seen.insert((ext.kind.clone(), ext.url.clone())) {
            out.push(ext);
        }
    }
    out
}

pub async fn search_episode_torrents(
    st: &AppState,
    anilist_id: i64,
    episode: i64,
    custom_query: Option<String>,
) -> Result<Vec<TorrentCandidate>> {
    // Ensure metadata is present in SQLite even if the user clicked Stream immediately
    let has_media: bool = {
        let conn = st.db();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM media WHERE anilist_id = ?1 AND (title_romaji IS NOT NULL OR title_english IS NOT NULL))",
            [anilist_id],
            |r| r.get(0),
        )
        .unwrap_or(false)
    };
    if !has_media
        && custom_query
            .as_ref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    {
        let _ = crate::service::ensure_media(st, anilist_id).await;
    }

    let is_custom = custom_query
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    let (wave1, wave2, known_titles, abs_episode, is_movie, pref_res, enabled_exts) = {
        let conn = st.db();
        let pref_res = db::get_setting(&conn, "pref.torrent_quality")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty() && s != "ask")
            .unwrap_or_else(|| "1080p".to_string());
        let enabled_exts: Vec<TorrentExtension> = list_torrent_extensions(&conn)
            .into_iter()
            .filter(|e| {
                let u = e.url.to_ascii_lowercase();
                e.enabled
                    && u.starts_with("http")
                    && !u.contains("animetosho.org")
                    && !u.contains("sukebei.")
            })
            .collect();

        let row: Option<(
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        )> = conn
            .query_row(
                "SELECT title_romaji, title_english, synonyms, format, episodes FROM media WHERE anilist_id = ?1",
                [anilist_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .ok();

        let (ro, en, syn_raw, format_opt, total_eps) =
            row.unwrap_or((None, None, None, None, None));
        let is_movie = format_opt.as_deref() == Some("MOVIE")
            || (format_opt.as_deref() == Some("SPECIAL") && total_eps == Some(1));

        // Check for prequel episode count or TVDB absolute_number for continuous-numbered releases
        let db_abs_ep: Option<i64> = conn
            .query_row(
                "SELECT absolute_number FROM episodes WHERE anilist_id = ?1 AND number = ?2 AND absolute_number IS NOT NULL",
                params![anilist_id, episode],
                |r| r.get(0),
            )
            .ok();
        let prequel_info: Option<(Option<String>, Option<i64>)> = conn
            .query_row(
                "SELECT title, episodes FROM media_relations WHERE anilist_id = ?1 AND relation_type = 'PREQUEL' AND format IN ('TV', 'TV_SHORT', 'ONA') LIMIT 1",
                [anilist_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();

        let abs_episode = db_abs_ep.or_else(|| {
            prequel_info
                .as_ref()
                .and_then(|(_, peps)| peps.filter(|&p| p > 0).map(|p| p + episode))
        });

        let mut known_titles: Vec<String> = Vec::new();
        // Primary titles (Romaji + English) drive the episode queries; synonyms and the prequel
        // title are only used for broader fallbacks so they can't crowd out the main queries.
        let mut raw_titles: Vec<String> = Vec::new();
        let mut extra_titles: Vec<String> = Vec::new();
        if let Some(r) = ro.filter(|s| !s.trim().is_empty()) {
            known_titles.push(r.clone());
            raw_titles.push(r);
        }
        if let Some(e) = en.filter(|s| !s.trim().is_empty()) {
            known_titles.push(e.clone());
            raw_titles.push(e);
        }
        let prequel_title: Option<String> = prequel_info
            .as_ref()
            .and_then(|(t, _)| t.clone())
            .filter(|t| !t.trim().is_empty());
        if let Some(pt) = &prequel_title {
            known_titles.push(pt.clone());
        }
        if let Some(sr) = syn_raw {
            if let Ok(syns) = serde_json::from_str::<Vec<String>>(&sr) {
                for syn in syns {
                    // Only include Latin/ASCII synonyms for torrent search
                    let trimmed = syn.trim();
                    if !trimmed.is_empty()
                        && trimmed.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 3
                    {
                        known_titles.push(trimmed.to_string());
                        if extra_titles.len() < 3 {
                            extra_titles.push(trimmed.to_string());
                        }
                    }
                }
            }
        }
        if raw_titles.is_empty() {
            raw_titles = extra_titles.drain(..).collect();
        }

        let mut wave1 = Vec::new();
        let mut wave2 = Vec::new();

        if let Some(q) = custom_query.as_ref().filter(|s| !s.trim().is_empty()) {
            let trimmed = q.trim().to_string();
            let parsed_q = crate::parser::parse_filename(&trimmed);
            if parsed_q.episode.is_none() && !is_movie && episode > 0 {
                push_unique_query(
                    &mut wave1,
                    format!("{} {:02}", clean_search_title(&trimmed), episode),
                );
                push_unique_query(&mut wave1, trimmed.clone());
            } else {
                push_unique_query(&mut wave1, trimmed.clone());
                push_unique_query(&mut wave1, clean_search_title(&trimmed));
            }
        } else {
            // Pass 1: Primary episode query for EVERY title (Romaji + English) first so neither is crowded out
            for t in &raw_titles {
                let clean = clean_search_title(t);
                if is_movie {
                    push_unique_query(&mut wave1, clean);
                } else {
                    push_unique_query(&mut wave1, format!("{clean} {episode:02}"));
                }
            }
            // Pass 2: Season-shortened, absolute episode, and colon-prefix queries
            for t in &raw_titles {
                let clean = clean_search_title(t);
                let (season_opt, base_clean) = split_season_from_title(t);

                let colon_prefix = t
                    .split(':')
                    .next()
                    .map(clean_search_title)
                    .filter(|p| p.len() >= 4 && p.len() < clean.len());

                if is_movie {
                    if !base_clean.is_empty() {
                        push_unique_query(&mut wave1, base_clean);
                    }
                    if let Some(cp) = colon_prefix {
                        push_unique_query(&mut wave2, cp);
                    }
                } else {
                    if let Some(s_num) = season_opt.filter(|&s| s > 1) {
                        if !base_clean.is_empty() {
                            push_unique_query(
                                &mut wave1,
                                format!("{base_clean} S{s_num} {episode:02}"),
                            );
                            push_unique_query(
                                &mut wave1,
                                format!("{base_clean} S{s_num:02}E{episode:02}"),
                            );
                            push_unique_query(&mut wave2, format!("{base_clean} {episode:02}"));
                        }
                    }
                    if let Some(abs_ep) = abs_episode.filter(|&a| a != episode) {
                        if !base_clean.is_empty() {
                            push_unique_query(&mut wave1, format!("{base_clean} {abs_ep:02}"));
                        }
                    }
                    if let Some(cp) = colon_prefix {
                        push_unique_query(&mut wave1, format!("{cp} {episode:02}"));
                        push_unique_query(&mut wave2, cp);
                    }
                    push_unique_query(&mut wave2, clean);
                    if !base_clean.is_empty() {
                        push_unique_query(&mut wave2, base_clean);
                    }
                }
            }
            // Continuous-numbered releases are usually named after the first season.
            if let (Some(pt), Some(abs_ep)) = (&prequel_title, abs_episode.filter(|&a| a != episode)) {
                if !is_movie {
                    let (_, base) = split_season_from_title(pt);
                    if !base.is_empty() {
                        push_unique_query(&mut wave1, format!("{base} {abs_ep:02}"));
                    }
                }
            }
            for t in &extra_titles {
                let clean = clean_search_title(t);
                if is_movie {
                    push_unique_query(&mut wave2, clean);
                } else {
                    push_unique_query(&mut wave2, format!("{clean} {episode:02}"));
                }
            }
        }

        (
            wave1,
            wave2,
            known_titles,
            abs_episode,
            is_movie,
            pref_res,
            enabled_exts,
        )
    };

    if enabled_exts.is_empty() {
        bail!("NO_TORRENT_EXTENSIONS: No torrent extensions configured. Add an extension or RSS source in Settings → Extensions.");
    }

    if wave1.is_empty() && wave2.is_empty() {
        bail!("Anime titles not loaded yet for AniList #{anilist_id}");
    }

    let q_key = custom_query
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let cache_key: SearchCacheKey = (anilist_id, episode, q_key, pref_res.clone());

    // 1. Return immediately in 0ms if a fresh cached result exists (< 5 min old)
    if let Ok(guard) = search_cache().lock() {
        if let Some((ts, cached)) = guard.get(&cache_key) {
            if ts.elapsed() < Duration::from_secs(300) && !cached.is_empty() {
                return Ok(cached.clone());
            }
        }
    }

    // 2. If a background prefetch for this exact episode is already in flight, join it instead of re-querying
    let existing_notify = {
        let mut guard = in_flight_searches().lock().unwrap();
        if let Some(n) = guard.get(&cache_key) {
            Some(n.clone())
        } else {
            let n = Arc::new(Notify::new());
            guard.insert(cache_key.clone(), n);
            None
        }
    };

    if let Some(notify) = existing_notify {
        let _ = tokio::time::timeout(Duration::from_secs(15), notify.notified()).await;
        if let Ok(guard) = search_cache().lock() {
            if let Some((_, cached)) = guard.get(&cache_key) {
                if !cached.is_empty() {
                    return Ok(cached.clone());
                }
            }
        }
        // The other search came back empty or failed — register ourselves and search again.
        if let Ok(mut guard) = in_flight_searches().lock() {
            guard
                .entry(cache_key.clone())
                .or_insert_with(|| Arc::new(Notify::new()));
        }
    }

    // Always release the in-flight slot (even on early return / error) so later searches never wait on a dead entry.
    struct InFlightGuard(SearchCacheKey);
    impl Drop for InFlightGuard {
        fn drop(&mut self) {
            if let Ok(mut guard) = in_flight_searches().lock() {
                if let Some(n) = guard.remove(&self.0) {
                    n.notify_waiters();
                }
            }
        }
    }
    let _in_flight = InFlightGuard(cache_key.clone());

    let client = torrent_search_http().clone();

    let mut out: Vec<TorrentCandidate> = Vec::new();
    let mut bases: Vec<i64> = Vec::new();
    let mut seen_hashes: HashSet<String> = HashSet::new();
    let mut source_errors: Vec<String> = Vec::new();

    type WaveBatch = std::result::Result<Vec<TorrentCandidate>, String>;

    // Execute SeaDex + SubsPlease + RSS queries concurrently, with a short grace window once
    // good seeded torrents have arrived so slow/dead mirrors never stall search.
    let run_wave = |queries: Vec<String>, include_seadex: bool| {
        let client = client.clone();
        let exts = enabled_exts.clone();
        let pref_res = pref_res.clone();
        let known_titles = known_titles.clone();
        async move {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WaveBatch>();
            let mut handles = Vec::new();

            if include_seadex {
                for ext in exts.iter().filter(|e| e.kind == "seadex-api") {
                    let c = client.clone();
                    let ext_clone = ext.clone();
                    let pr = pref_res.clone();
                    let tx_c = tx.clone();
                    handles.push(tokio::spawn(async move {
                        let res = query_seadex_extension(
                            &c,
                            &ext_clone,
                            anilist_id,
                            episode,
                            abs_episode,
                            is_movie,
                            &pr,
                        )
                        .await
                        .map_err(|e| format!("{}: {}", ext_clone.name, short_err(&e)));
                        let _ = tx_c.send(res);
                    }));
                }
            }

            // SubsPlease searches by show name only — an episode number in the query returns nothing.
            let mut subsplease_queries: Vec<String> = Vec::new();
            for q in queries.iter().take(6) {
                push_unique_query(&mut subsplease_queries, strip_episode_from_query(q));
            }

            for ext in exts.iter().filter(|e| e.kind != "seadex-api") {
                let ext_queries: Vec<String> = if ext.kind == "subsplease-json" {
                    subsplease_queries.iter().take(3).cloned().collect()
                } else {
                    queries.iter().take(6).cloned().collect()
                };
                for q in ext_queries {
                    let c = client.clone();
                    let ext_clone = ext.clone();
                    let pr = pref_res.clone();
                    let kt = known_titles.clone();
                    let tx_c = tx.clone();
                    handles.push(tokio::spawn(async move {
                        let encoded = url_encode_bytes(q.as_bytes());
                        let res: WaveBatch = if ext_clone.kind == "subsplease-json" {
                            query_subsplease_extension(
                                &c,
                                &ext_clone,
                                &encoded,
                                episode,
                                abs_episode,
                                is_movie,
                                is_custom,
                                &pr,
                                &kt,
                            )
                            .await
                            .map_err(|e| format!("{}: {}", ext_clone.name, short_err(&e)))
                        } else {
                            let rss_url = if ext_clone.url.contains("{query}") {
                                ext_clone.url.replace("{query}", &encoded)
                            } else {
                                format!(
                                    "{}/?page=rss&c=1_0&f=0&q={encoded}",
                                    ext_clone.url.trim_end_matches('/')
                                )
                            };
                            fetch_text_with_retry(&c, &rss_url)
                                .await
                                .map(|xml| {
                                    parse_rss_with_context(
                                        &xml,
                                        &ext_clone.name,
                                        episode,
                                        abs_episode,
                                        is_movie,
                                        is_custom,
                                        &pr,
                                        &kt,
                                    )
                                })
                                .map_err(|e| format!("{}: {e}", ext_clone.name))
                        };
                        let _ = tx_c.send(res);
                    }));
                }
            }
            drop(tx);

            let mut wave_items: Vec<TorrentCandidate> = Vec::new();
            let mut wave_errors: Vec<String> = Vec::new();
            let mut direct_seeded_hits = 0usize;

            loop {
                let next_batch = if direct_seeded_hits >= 3 {
                    match tokio::time::timeout(Duration::from_millis(850), rx.recv()).await {
                        Ok(b) => b,
                        Err(_) => break,
                    }
                } else {
                    rx.recv().await
                };

                let Some(batch) = next_batch else {
                    break;
                };
                match batch {
                    Ok(items) => {
                        // Only count candidates with a direct `.torrent` URL and real seeders toward early timeout
                        // so fast magnet-only indexers (like SeaDex) never abort in-flight RSS responses.
                        direct_seeded_hits += items
                            .iter()
                            .filter(|c| c.torrent_url.is_some() && c.seeders >= 5 && c.score > 0)
                            .count();
                        wave_items.extend(items);
                    }
                    Err(e) => wave_errors.push(e),
                }
            }

            for h in handles {
                h.abort();
            }
            (wave_items, wave_errors)
        }
    };

    let merge_or_push = |item: TorrentCandidate,
                         out: &mut Vec<TorrentCandidate>,
                         bases: &mut Vec<i64>,
                         seen_hashes: &mut HashSet<String>| {
        let key_of = |c: &TorrentCandidate| {
            c.info_hash
                .as_deref()
                .and_then(normalize_btih_hex)
                .or_else(|| c.info_hash.as_ref().map(|h| h.to_ascii_lowercase()))
                .unwrap_or_else(|| c.title.to_ascii_lowercase())
        };
        let key = key_of(&item);
        if seen_hashes.insert(key.clone()) {
            bases.push(item.score - seeder_score(item.seeders));
            out.push(item);
        } else if let Some(existing) = out.iter_mut().find(|c| key_of(c) == key) {
            // A real indexer entry (with a `.torrent` link) carries real swarm stats —
            // prefer them over magnet-only placeholder counts.
            if existing.torrent_url.is_none() && item.torrent_url.is_some() {
                existing.torrent_url = item.torrent_url;
                existing.seeders = item.seeders;
                existing.leechers = item.leechers;
            } else if item.torrent_url.is_some() && item.seeders > existing.seeders {
                existing.seeders = item.seeders;
            }
            if existing.ref_subtitle_url.is_none() && item.ref_subtitle_url.is_some() {
                existing.ref_subtitle_url = item.ref_subtitle_url;
            }
            if existing.size == 0 && item.size > 0 {
                existing.size = item.size;
            }
        }
    };

    let (w1_items, w1_errors) = run_wave(wave1, true).await;
    source_errors.extend(w1_errors);
    for item in w1_items {
        merge_or_push(item, &mut out, &mut bases, &mut seen_hashes);
    }

    let has_seeded_match = out
        .iter()
        .any(|c| c.torrent_url.is_some() && c.seeders > 0 && c.score > 0);

    if !has_seeded_match && !wave2.is_empty() {
        let (w2_items, w2_errors) = run_wave(wave2, false).await;
        source_errors.extend(w2_errors);
        for item in w2_items {
            merge_or_push(item, &mut out, &mut bases, &mut seen_hashes);
        }
    }

    // Magnet-only candidates (SubsPlease, SeaDex) have no real seeder counts — ask the trackers.
    refresh_swarm_health(&mut out).await;

    // Re-apply the seeder component now that swarm stats are final.
    for (c, base) in out.iter_mut().zip(bases.iter()) {
        c.score = base + seeder_score(c.seeders);
    }

    out.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.seeders.cmp(&a.seeders))
    });

    if out.is_empty() {
        source_errors.sort();
        source_errors.dedup();
        if !source_errors.is_empty() {
            let shown: Vec<String> = source_errors.into_iter().take(3).collect();
            bail!("Couldn't reach your torrent sources — {}", shown.join("; "));
        }
        return Ok(out);
    }

    if let Ok(mut guard) = search_cache().lock() {
        if guard.len() > 80 {
            guard.clear();
        }
        guard.insert(cache_key.clone(), (std::time::Instant::now(), out.clone()));
    }

    Ok(out)
}

/// Remove a trailing episode / season-episode token from a search query (`"Title 05"` → `"Title"`).
fn strip_episode_from_query(q: &str) -> String {
    static EP_TAIL_RE: OnceLock<Regex> = OnceLock::new();
    let re = EP_TAIL_RE.get_or_init(|| {
        Regex::new(r"(?i)(?:\s+S\d{1,2}E\d{1,4}|\s+S\d{1,2}\s+\d{1,4}|\s+\d{1,4})\s*$").unwrap()
    });
    re.replace(q.trim(), "").trim().to_string()
}

fn short_err(e: &anyhow::Error) -> String {
    describe_reqwest_error(e.downcast_ref::<reqwest::Error>()).unwrap_or_else(|| {
        let s = e.to_string();
        s.chars().take(80).collect()
    })
}

fn describe_reqwest_error(e: Option<&reqwest::Error>) -> Option<String> {
    let e = e?;
    Some(if e.is_timeout() {
        "timed out".to_string()
    } else if e.is_connect() {
        "couldn't connect".to_string()
    } else if let Some(status) = e.status() {
        format!("HTTP {}", status.as_u16())
    } else if e.is_decode() {
        "unexpected response".to_string()
    } else {
        "request failed".to_string()
    })
}

/// GET a URL as text, retrying once on timeouts, connection errors, HTTP 429 and 5xx.
async fn fetch_text_with_retry(client: &reqwest::Client, url: &str) -> std::result::Result<String, String> {
    let mut last_err = String::from("request failed");
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(700)).await;
        }
        match client.get(url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    match resp.text().await {
                        Ok(t) => return Ok(t),
                        Err(e) => last_err = describe_reqwest_error(Some(&e)).unwrap_or_default(),
                    }
                } else {
                    last_err = format!("HTTP {}", status.as_u16());
                    if !(status.as_u16() == 429 || status.is_server_error()) {
                        return Err(last_err);
                    }
                }
            }
            Err(e) => {
                last_err = describe_reqwest_error(Some(&e)).unwrap_or_default();
                if !(e.is_timeout() || e.is_connect() || e.is_request()) {
                    return Err(last_err);
                }
            }
        }
    }
    Err(last_err)
}

async fn query_subsplease_extension(
    client: &reqwest::Client,
    ext: &TorrentExtension,
    encoded_query: &str,
    episode: i64,
    abs_episode: Option<i64>,
    is_movie: bool,
    is_custom: bool,
    pref_res: &str,
    known_titles: &[String],
) -> Result<Vec<TorrentCandidate>> {
    let base = ext.url.trim_end_matches('/');
    let url = if base.ends_with("/api") {
        format!("{base}/?f=search&tz=UTC&s={encoded_query}")
    } else if base.contains("f=search") {
        format!("{base}&s={encoded_query}")
    } else {
        format!("{base}/api/?f=search&tz=UTC&s={encoded_query}")
    };
    let val: serde_json::Value = client.get(&url).send().await?.json().await?;
    let Some(obj) = val.as_object() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (_key, entry) in obj {
        let show = entry.get("show").and_then(|v| v.as_str()).unwrap_or("");
        let ep_str = entry.get("episode").and_then(|v| v.as_str()).unwrap_or("");
        let Some(downloads) = entry.get("downloads").and_then(|v| v.as_array()) else {
            continue;
        };
        for dl in downloads {
            let res_raw = dl.get("res").and_then(|v| v.as_str()).unwrap_or("1080");
            let res = if res_raw.ends_with('p') {
                res_raw.to_string()
            } else {
                format!("{res_raw}p")
            };
            let Some(magnet) = dl.get("magnet").and_then(|v| v.as_str()) else {
                continue;
            };
            let info_hash = magnet
                .split("urn:btih:")
                .nth(1)
                .map(|r| r.split('&').next().unwrap_or(r).trim())
                .and_then(|raw_h| {
                    normalize_btih_hex(raw_h).or_else(|| Some(raw_h.to_ascii_lowercase()))
                });
            let title = format!("[SubsPlease] {show} - {ep_str} ({res}).mkv");
            let score = score_candidate_with_context(
                &title,
                Some("SubsPlease"),
                Some(&res),
                85, // SubsPlease official XDCC/WebSeed swarm health baseline
                1_100 * 1024 * 1024,
                false,
                episode,
                abs_episode,
                is_movie,
                pref_res,
                if is_custom { &[] } else { known_titles },
            );
            if !is_custom && !is_relevant_candidate(score, 85) {
                continue;
            }
            out.push(TorrentCandidate {
                title,
                group: Some("SubsPlease".into()),
                resolution: Some(res),
                size: 0,
                seeders: 85,
                leechers: 5,
                torrent_url: None,
                magnet_uri: Some(magnet.to_string()),
                info_hash,
                published_at: None,
                source: ext.name.clone(),
                is_batch: false,
                score,
                ref_subtitle_url: None,
            });
        }
    }
    Ok(out)
}

async fn query_seadex_extension(
    client: &reqwest::Client,
    ext: &TorrentExtension,
    anilist_id: i64,
    episode: i64,
    abs_episode: Option<i64>,
    is_movie: bool,
    pref_res: &str,
) -> Result<Vec<TorrentCandidate>> {
    let base = ext.url.trim_end_matches('/');
    let url = format!("{base}?page=1&perPage=1&filter=(alID={anilist_id})&expand=trs");
    let val: serde_json::Value = client.get(&url).send().await?.json().await?;
    let Some(trs) = val
        .get("items")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|item| item.get("expand"))
        .and_then(|exp| exp.get("trs"))
        .and_then(|t| t.as_array())
    else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for tr in trs {
        let Some(info_hash) = tr
            .get("infoHash")
            .and_then(|v| v.as_str())
            .filter(|h| !h.is_empty() && !h.contains('<'))
        else {
            continue;
        };
        let group = tr
            .get("releaseGroup")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let is_best = tr.get("isBest").and_then(|v| v.as_bool()).unwrap_or(false);
        let files = tr.get("files").and_then(|v| v.as_array());
        let total_size: i64 = files
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| f.get("length").and_then(|l| l.as_i64()))
                    .sum()
            })
            .unwrap_or(0);
        let file_count = files.map(|a| a.len()).unwrap_or(1);
        let first_name = files
            .and_then(|a| a.first())
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("Release.mkv");
        let title = if file_count > 1 {
            format!(
                "[{}] {} (Batch {} files)",
                group.as_deref().unwrap_or("SeaDex"),
                first_name,
                file_count
            )
        } else {
            first_name.to_string()
        };
        let (_, resolution, is_batch) = parse_release_tags(&title);
        let mut score = score_candidate_with_context(
            &title,
            group.as_deref(),
            resolution.as_deref(),
            50,
            total_size,
            is_batch || file_count > 1,
            episode,
            abs_episode,
            is_movie,
            pref_res,
            &[],
        );
        score += if is_best { 350 } else { 200 };
        let magnet_uri = Some(format!(
            "magnet:?xt=urn:btih:{info_hash}&dn={}",
            url_encode_bytes(title.as_bytes())
        ));
        out.push(TorrentCandidate {
            title,
            group,
            resolution,
            size: total_size,
            seeders: 50,
            leechers: 2,
            torrent_url: tr
                .get("url")
                .and_then(|v| v.as_str())
                .filter(|u| u.ends_with(".torrent"))
                .map(|s| s.to_string()),
            magnet_uri,
            info_hash: Some(info_hash.to_ascii_lowercase()),
            published_at: None,
            source: if is_best {
                format!("{} ★ Best", ext.name)
            } else {
                ext.name.clone()
            },
            is_batch: is_batch || file_count > 1,
            score,
            ref_subtitle_url: None,
        });
    }
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
            .replace("&#39;", "'")
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

#[allow(dead_code)]
fn parse_nyaa_rss(xml: &str, episode: i64, pref_res: &str) -> Vec<TorrentCandidate> {
    parse_rss_with_context(xml, "RSS", episode, None, false, false, pref_res, &[])
}

fn parse_rss_with_context(
    xml: &str,
    source_name: &str,
    episode: i64,
    abs_episode: Option<i64>,
    is_movie: bool,
    is_custom: bool,
    pref_res: &str,
    known_titles: &[String],
) -> Vec<TorrentCandidate> {
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
        let info_hash =
            extract_xml_tag(block, "nyaa:infoHash").or_else(|| extract_xml_tag(block, "infoHash"));
        let seeders: i64 = extract_xml_tag(block, "nyaa:seeders")
            .or_else(|| extract_xml_tag(block, "seeders"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let leechers: i64 = extract_xml_tag(block, "nyaa:leechers")
            .or_else(|| extract_xml_tag(block, "leechers"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let size = extract_xml_tag(block, "nyaa:size")
            .or_else(|| extract_xml_tag(block, "size"))
            .map(|s| parse_human_size(&s))
            .unwrap_or(0);

        let (group, resolution, is_batch) = parse_release_tags(&title);
        let score = score_candidate_with_context(
            &title,
            group.as_deref(),
            resolution.as_deref(),
            seeders,
            size,
            is_batch,
            episode,
            abs_episode,
            is_movie,
            pref_res,
            if is_custom { &[] } else { known_titles },
        );
        if !is_custom && !is_relevant_candidate(score, seeders) {
            continue;
        }
        let magnet_uri = info_hash.as_ref().map(|h| {
            format!(
                "magnet:?xt=urn:btih:{h}&dn={}",
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
            source: source_name.to_string(),
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

fn spawn_tracker_peer_discovery(
    meta: &TorrentMeta,
    peer_id: &[u8; 20],
    peer_tx: tokio::sync::mpsc::UnboundedSender<SocketAddr>,
) {
    for tr in meta.trackers.iter().take(12).cloned() {
        let info_hash = meta.info_hash;
        let peer_id = *peer_id;
        let left = meta.total_length;
        let tx = peer_tx.clone();
        tokio::spawn(async move {
            let peers = if tr.starts_with("udp://") {
                announce_udp(&tr, &info_hash, &peer_id, left)
                    .await
                    .unwrap_or_default()
            } else if tr.starts_with("http://") || tr.starts_with("https://") {
                announce_http(&tr, &info_hash, &peer_id, left)
                    .await
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            for p in peers {
                if tx.send(p).is_err() {
                    break;
                }
            }
        });
    }
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
    let bytes = torrent_http().get(&url).send().await?.bytes().await?;
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
    piece_claimed_at: Mutex<Vec<Option<std::time::Instant>>>,
    read_head_piece: AtomicUsize,
    pub downloaded_bytes: AtomicU64,
    pub wire_rx_bytes: AtomicU64,
    pub uploaded_bytes: AtomicU64,
    pub speed_bps: AtomicU64,
    pub upload_speed_bps: AtomicU64,
    pub active_peers: AtomicUsize,
    pub state_str: Mutex<String>,
    pub error_str: Mutex<Option<String>>,
    pub stream_url: Mutex<Option<String>>,
    pub allow_seeding: bool,
    pub cancelled: AtomicBool,
    pub piece_notify: Notify,
    file_write_lock: tokio::sync::Mutex<()>,
    hot_pieces: Mutex<HashMap<usize, Arc<Vec<u8>>>>,
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

    /// Ready to launch `mpv` / `Memento` once both the Matroska EBML header (`first_piece`, plus `first_piece + 1`
    /// when pieces are < 1 MiB) AND the Matroska EOF Cues seek index (`last_piece`) are verified on disk.
    pub fn is_ready_to_stream(&self) -> bool {
        let states = self.piece_states.lock().unwrap();
        let p0 = states.get(self.first_piece).copied() == Some(PieceState::Complete);
        let plast = states.get(self.last_piece).copied() == Some(PieceState::Complete);
        if self.meta.piece_length >= 1_048_576 {
            return p0 && plast;
        }
        let p1 = states
            .get((self.first_piece + 1).min(self.last_piece))
            .copied()
            == Some(PieceState::Complete);
        p0 && p1 && plast
    }

    pub fn is_all_complete(&self) -> bool {
        let states = self.piece_states.lock().unwrap();
        states[self.first_piece..=self.last_piece]
            .iter()
            .all(|&s| s == PieceState::Complete)
    }

    /// Pick the highest-priority piece that `peer_has`:
    /// 1. Header pieces (`first_piece..=first_piece+2`) + EOF Cues piece (`last_piece`)
    /// 2. Pieces starting sequentially from `read_head_piece`
    /// 3. Critical-window multi-peer racing: if any critical piece (`first_piece..=first_piece+2`,
    ///    `last_piece`, or `head..=head+4`) has been `InFlight` for > 650ms on a slow peer, allow a fast
    ///    peer to race it in parallel so slow peers can never stall startup or video playback!
    fn claim_next_piece(&self, peer_bitfield: &[bool]) -> Option<usize> {
        let mut states = self.piece_states.lock().unwrap();
        let mut claimed_at = self.piece_claimed_at.lock().unwrap();
        let now = std::time::Instant::now();
        let head = self
            .read_head_piece
            .load(Ordering::Relaxed)
            .clamp(self.first_piece, self.last_piece);

        let priority_indices = [
            self.first_piece,
            (self.first_piece + 1).min(self.last_piece),
            (self.first_piece + 2).min(self.last_piece),
            self.last_piece,
            self.last_piece.saturating_sub(1).max(self.first_piece),
        ];

        // 1. Unclaimed priority header/EOF pieces
        for &idx in &priority_indices {
            if states[idx] == PieceState::Missing
                && peer_bitfield.get(idx).copied().unwrap_or(false)
            {
                states[idx] = PieceState::InFlight;
                claimed_at[idx] = Some(now);
                return Some(idx);
            }
        }

        // 2. If a critical startup piece (`priority_indices`) is stuck > 900ms, or a playback head piece
        // (`head..=head+2`) is stuck > 2500ms on a slow peer, allow a fast peer to race it.
        // Keeping normal sequential pieces un-duplicated ensures 20+ seeders download 20 distinct pieces in parallel!
        let startup_incomplete = states.get(self.first_piece) != Some(&PieceState::Complete)
            || states.get(self.last_piece) != Some(&PieceState::Complete);
        if startup_incomplete {
            for &idx in &priority_indices {
                if states[idx] == PieceState::InFlight
                    && peer_bitfield.get(idx).copied().unwrap_or(false)
                {
                    let elapsed = claimed_at[idx]
                        .map(|t| now.duration_since(t))
                        .unwrap_or(Duration::from_secs(10));
                    if elapsed >= Duration::from_millis(900) {
                        claimed_at[idx] = Some(now);
                        return Some(idx);
                    }
                }
            }
        }

        // 3. Unclaimed sequential pieces from `read_head_piece` to `last_piece`
        for idx in head..=self.last_piece {
            if states[idx] == PieceState::Missing
                && peer_bitfield.get(idx).copied().unwrap_or(false)
            {
                states[idx] = PieceState::InFlight;
                claimed_at[idx] = Some(now);
                return Some(idx);
            }
        }

        // 4. Preempt slow InFlight pieces near the playback head (`head..=head+3` stuck > 2.2s, or `head..=head+20` stuck > 4.0s)
        let lookahead_end = (head + 20).min(self.last_piece);
        for idx in head..=lookahead_end {
            if states[idx] == PieceState::InFlight
                && peer_bitfield.get(idx).copied().unwrap_or(false)
            {
                let elapsed = claimed_at[idx]
                    .map(|t| now.duration_since(t))
                    .unwrap_or(Duration::from_secs(10));
                let limit = if idx <= head + 3 {
                    Duration::from_millis(2200)
                } else {
                    Duration::from_millis(4000)
                };
                if elapsed >= limit {
                    claimed_at[idx] = Some(now);
                    return Some(idx);
                }
            }
        }

        // 5. Wrap around from `first_piece` to `head`
        for idx in self.first_piece..head {
            if states[idx] == PieceState::Missing
                && peer_bitfield.get(idx).copied().unwrap_or(false)
            {
                states[idx] = PieceState::InFlight;
                claimed_at[idx] = Some(now);
                return Some(idx);
            }
        }

        // 6. Endgame: race any remaining InFlight piece
        for idx in head..=self.last_piece {
            if states[idx] == PieceState::InFlight
                && peer_bitfield.get(idx).copied().unwrap_or(false)
            {
                return Some(idx);
            }
        }

        None
    }

    fn release_piece(&self, idx: usize) {
        let mut states = self.piece_states.lock().unwrap();
        let mut claimed_at = self.piece_claimed_at.lock().unwrap();
        if states.get(idx) == Some(&PieceState::InFlight) {
            states[idx] = PieceState::Missing;
            claimed_at[idx] = None;
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
        self.downloaded_bytes
            .fetch_add(piece_len, Ordering::Relaxed);
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

    fn cache_hot_piece(&self, idx: usize, data: Arc<Vec<u8>>) {
        let head = self.read_head_piece.load(Ordering::Relaxed);
        let is_pinned = idx <= self.first_piece + 2 || idx >= self.last_piece.saturating_sub(1);
        if !is_pinned && idx.abs_diff(head) > 16 {
            return;
        }
        if let Ok(mut map) = self.hot_pieces.lock() {
            if map.len() >= 48 && !map.contains_key(&idx) {
                let evict_cand = map.keys().copied().find(|&k| {
                    k > self.first_piece + 2
                        && k < self.last_piece.saturating_sub(1)
                        && (k < head.saturating_sub(4) || k > head + 16)
                });
                if let Some(k) = evict_cand {
                    map.remove(&k);
                }
            }
            map.insert(idx, data);
        }
    }

    fn get_hot_piece(&self, idx: usize) -> Option<Arc<Vec<u8>>> {
        self.hot_pieces
            .lock()
            .ok()
            .and_then(|m| m.get(&idx).cloned())
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
        f.write_all(&data[slice_offset..slice_offset + slice_len])
            .await?;
        Ok(())
    }

    /// Read a verified block from disk to upload to a requesting peer when `allow_seeding` is enabled.
    async fn read_verified_block(
        &self,
        idx: usize,
        begin: usize,
        len: usize,
    ) -> Result<Option<Vec<u8>>> {
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
        let verified_dl = self.downloaded_bytes.load(Ordering::Relaxed);
        let live_dl = if verified_dl == 0 {
            self.wire_rx_bytes
                .load(Ordering::Relaxed)
                .min(self.meta.piece_length)
        } else {
            verified_dl
        };
        TorrentSessionStatus {
            session_id: self.id.clone(),
            anilist_id: self.anilist_id,
            ep_key: self.ep_key.clone(),
            title: self.title.clone(),
            file_name: self.meta.target_file_name.clone(),
            mode: self.mode.clone(),
            state: self.state_str.lock().unwrap().clone(),
            progress_pct: pct,
            downloaded_bytes: live_dl.min(self.meta.target_file_length),
            uploaded_bytes: self.uploaded_bytes.load(Ordering::Relaxed),
            total_bytes: self.meta.target_file_length,
            speed_bps: self.speed_bps.load(Ordering::Relaxed),
            upload_speed_bps: self.upload_speed_bps.load(Ordering::Relaxed),
            peers: self.active_peers.load(Ordering::Relaxed),
            stream_url: self.stream_url.lock().unwrap().clone(),
            save_path: self.save_path.to_string_lossy().to_string(),
            error: self.error_str.lock().unwrap().clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// BitTorrent Peer Wire Worker (`BEP 3` + `BEP 10` + `BEP 11` Peer Exchange)
// ---------------------------------------------------------------------------

fn handle_extended_pex_msg(
    payload: &[u8],
    peer_tx: &tokio::sync::mpsc::UnboundedSender<SocketAddr>,
) {
    if payload.len() < 2 {
        return;
    }
    if let Ok((dict, _)) = parse_bencode(&payload[1..]) {
        if let Some(compact) = dict.dict_get(b"added").and_then(|v| v.as_bytes()) {
            for chunk in compact.chunks_exact(6).take(60) {
                let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]);
                let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                if port > 0 && !ip.is_unspecified() && !ip.is_loopback() {
                    let _ = peer_tx.send(SocketAddr::V4(SocketAddrV4::new(ip, port)));
                }
            }
        }
    }
}

async fn run_peer_worker(
    session: Arc<SwarmSession>,
    addr: SocketAddr,
    peer_id: [u8; 20],
    peer_tx: tokio::sync::mpsc::UnboundedSender<SocketAddr>,
) -> Result<()> {
    let mut stream =
        tokio::time::timeout(Duration::from_millis(3500), TcpStream::connect(addr)).await??;
    let _ = stream.set_nodelay(true);

    // 68-byte Handshake with BEP 10 Extension Protocol bit (`hs[25] |= 0x10`)
    let mut hs = [0u8; 68];
    hs[0] = 19;
    hs[1..20].copy_from_slice(b"BitTorrent protocol");
    hs[25] |= 0x10;
    hs[28..48].copy_from_slice(&session.meta.info_hash);
    hs[48..68].copy_from_slice(&peer_id);
    tokio::time::timeout(Duration::from_secs(5), stream.write_all(&hs)).await??;

    let mut resp_hs = [0u8; 68];
    tokio::time::timeout(Duration::from_secs(6), stream.read_exact(&mut resp_hs)).await??;
    if &resp_hs[1..20] != b"BitTorrent protocol" || resp_hs[28..48] != session.meta.info_hash {
        bail!("Handshake mismatch");
    }

    // If peer supports BEP 10 Extension Protocol, advertise `ut_pex = 1`
    if (resp_hs[25] & 0x10) != 0 {
        let ext_body = b"d1:md6:ut_pexi1eee";
        let msg_len = (2 + ext_body.len()) as u32;
        let mut ext_hs = Vec::with_capacity(6 + ext_body.len());
        ext_hs.extend_from_slice(&msg_len.to_be_bytes());
        ext_hs.push(20); // BEP 10 Extended message
        ext_hs.push(0); // Extended handshake ID = 0
        ext_hs.extend_from_slice(ext_body);
        let _ = stream.write_all(&ext_hs).await;
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
            let Ok(Ok(msg)) =
                tokio::time::timeout(Duration::from_secs(12), read_peer_msg(&mut stream)).await
            else {
                if counted_active {
                    session.active_peers.fetch_sub(1, Ordering::Relaxed);
                }
                return Ok(());
            };
            match msg {
                None => {} // keep-alive
                Some((0, _)) => {
                    peer_choking = true;
                    if counted_active {
                        counted_active = false;
                        session.active_peers.fetch_sub(1, Ordering::Relaxed);
                    }
                }
                Some((1, _)) => {
                    peer_choking = false;
                    if !counted_active {
                        counted_active = true;
                        session.active_peers.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Some((4, payload)) if payload.len() >= 4 => {
                    let idx = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
                        as usize;
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
                Some((20, payload)) => {
                    handle_extended_pex_msg(&payload, &peer_tx);
                }
                Some((2, _)) if session.allow_seeding => {
                    // Peer is interested -> send Unchoke (id = 1)
                    let _ = stream.write_all(&[0, 0, 0, 1, 1]).await;
                }
                Some((6, payload)) if session.allow_seeding && payload.len() >= 12 => {
                    let req_idx =
                        u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
                            as usize;
                    let req_begin =
                        u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]])
                            as usize;
                    let req_len =
                        u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]])
                            as usize;
                    if let Ok(Some(block_data)) = session
                        .read_verified_block(req_idx, req_begin, req_len)
                        .await
                    {
                        let msg_len = (9 + block_data.len()) as u32;
                        let mut hdr = [0u8; 13];
                        hdr[0..4].copy_from_slice(&msg_len.to_be_bytes());
                        hdr[4] = 7; // Piece
                        hdr[5..9].copy_from_slice(&(req_idx as u32).to_be_bytes());
                        hdr[9..13].copy_from_slice(&(req_begin as u32).to_be_bytes());
                        if stream.write_all(&hdr).await.is_ok()
                            && stream.write_all(&block_data).await.is_ok()
                        {
                            session
                                .uploaded_bytes
                                .fetch_add(block_data.len() as u64, Ordering::Relaxed);
                        }
                    }
                }
                _ => {}
            }
        }

        let Some(piece_idx) = session.claim_next_piece(&peer_has) else {
            tokio::time::sleep(Duration::from_millis(150)).await;
            continue;
        };

        let piece_len = session.piece_size(piece_idx);
        let mut piece_buf = vec![0u8; piece_len];
        let mut received = 0usize;
        let block_size = 16_384usize;

        // Pipeline up to 48 block requests (768 KiB window per peer) with batched TCP writes
        const PIPELINE_DEPTH: usize = 48;
        let mut next_offset = 0usize;
        let mut in_flight = 0usize;
        let mut io_failed = false;
        let mut preempted = false;
        let mut req_batch = Vec::with_capacity(PIPELINE_DEPTH * 17);

        while received < piece_len {
            if session.cancelled.load(Ordering::Relaxed) {
                io_failed = true;
                break;
            }
            if session.is_piece_complete(piece_idx) {
                // Another peer completed this piece first — keep this TCP connection alive and move to next piece!
                preempted = true;
                break;
            }
            req_batch.clear();
            while in_flight < PIPELINE_DEPTH && next_offset < piece_len && !peer_choking {
                let req_len = (piece_len - next_offset).min(block_size);
                let mut req = [0u8; 17];
                req[0..4].copy_from_slice(&13u32.to_be_bytes());
                req[4] = 6; // Request
                req[5..9].copy_from_slice(&(piece_idx as u32).to_be_bytes());
                req[9..13].copy_from_slice(&(next_offset as u32).to_be_bytes());
                req[13..17].copy_from_slice(&(req_len as u32).to_be_bytes());
                req_batch.extend_from_slice(&req);
                next_offset += req_len;
                in_flight += 1;
            }
            if !req_batch.is_empty() {
                if stream.write_all(&req_batch).await.is_err() {
                    io_failed = true;
                    break;
                }
            }

            match tokio::time::timeout(Duration::from_secs(8), read_peer_msg(&mut stream)).await {
                Ok(Ok(Some((7, payload)))) if payload.len() > 8 => {
                    let idx = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
                        as usize;
                    let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]])
                        as usize;
                    let block = &payload[8..];
                    if idx == piece_idx && begin + block.len() <= piece_len {
                        piece_buf[begin..begin + block.len()].copy_from_slice(block);
                        received += block.len();
                        in_flight = in_flight.saturating_sub(1);
                        session
                            .wire_rx_bytes
                            .fetch_add(block.len() as u64, Ordering::Relaxed);
                    }
                }
                Ok(Ok(Some((0, _)))) => {
                    peer_choking = true;
                    if counted_active {
                        counted_active = false;
                        session.active_peers.fetch_sub(1, Ordering::Relaxed);
                    }
                    break;
                }
                Ok(Ok(Some((4, payload)))) if payload.len() >= 4 => {
                    let idx = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
                        as usize;
                    if idx < num_pieces {
                        peer_has[idx] = true;
                    }
                }
                Ok(Ok(Some((20, payload)))) => {
                    handle_extended_pex_msg(&payload, &peer_tx);
                }
                Ok(Ok(Some((2, _)))) if session.allow_seeding => {
                    let _ = stream.write_all(&[0, 0, 0, 1, 1]).await;
                }
                Ok(Ok(Some((6, payload)))) if session.allow_seeding && payload.len() >= 12 => {
                    let req_idx =
                        u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
                            as usize;
                    let req_begin =
                        u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]])
                            as usize;
                    let req_len =
                        u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]])
                            as usize;
                    if let Ok(Some(block_data)) = session
                        .read_verified_block(req_idx, req_begin, req_len)
                        .await
                    {
                        let msg_len = (9 + block_data.len()) as u32;
                        let mut hdr = [0u8; 13];
                        hdr[0..4].copy_from_slice(&msg_len.to_be_bytes());
                        hdr[4] = 7; // Piece
                        hdr[5..9].copy_from_slice(&(req_idx as u32).to_be_bytes());
                        hdr[9..13].copy_from_slice(&(req_begin as u32).to_be_bytes());
                        if stream.write_all(&hdr).await.is_ok()
                            && stream.write_all(&block_data).await.is_ok()
                        {
                            session
                                .uploaded_bytes
                                .fetch_add(block_data.len() as u64, Ordering::Relaxed);
                        }
                    }
                }
                Ok(Ok(None)) => {}
                _ => {
                    io_failed = true;
                    break;
                }
            }
        }

        if preempted {
            continue;
        }
        if io_failed || received < piece_len {
            session.release_piece(piece_idx);
            if io_failed {
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

        let piece_arc = Arc::new(piece_buf);
        session.cache_hot_piece(piece_idx, piece_arc.clone());

        if session
            .write_verified_piece(piece_idx, &piece_arc)
            .await
            .is_ok()
        {
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
// Local HTTP Range Server for Built-in Player & `mpv`
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

async fn handle_http_stream_client(
    mut socket: TcpStream,
    session: Arc<SwarmSession>,
) -> Result<()> {
    let mut req_buf = [0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut req_buf)).await??;
    if n == 0 {
        return Ok(());
    }
    let req_str = String::from_utf8_lossy(&req_buf[..n]);
    let first_line = req_str.lines().next().unwrap_or("");
    let is_head = first_line.starts_with("HEAD ");
    let req_target = first_line.split_whitespace().nth(1).unwrap_or("/");
    let selected_aid: i64 = req_target
        .split_once('?')
        .and_then(|(_, q)| {
            q.split('&').find_map(|pair| {
                let (k, v) = pair.split_once('=')?;
                if k == "aid" {
                    v.parse::<i64>().ok()
                } else {
                    None
                }
            })
        })
        .unwrap_or(0);
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
            "HTTP/1.1 206 Partial Content\r\nContent-Type: video/x-matroska\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nContent-Range: bytes {range_start}-{range_end}/{total_len}\r\nContent-Length: {content_len}\r\nConnection: close\r\n\r\n"
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: video/x-matroska\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {total_len}\r\nConnection: close\r\n\r\n"
        )
    };
    socket.write_all(header.as_bytes()).await?;
    if is_head {
        return Ok(());
    }

    let mut cur_offset = range_start;
    let mut chunk_buf = vec![0u8; 262_144];
    let mut f = tokio::fs::File::open(&session.save_path).await?;
    let mut cached_audio_offsets: Option<Vec<usize>> = None;

    while cur_offset <= range_end {
        if session.cancelled.load(Ordering::Relaxed) {
            break;
        }
        let global_offset = session.meta.target_file_offset + cur_offset;
        let needed_piece = (global_offset / session.meta.piece_length) as usize;

        // Point sequential scheduler at `needed_piece`, UNLESS this is a short metadata/Cues probe
        // at the very end of the file (`last_piece` / `last_piece - 1`), which is already in `priority_indices`
        // and must not hijack sequential buffering at the playback head!
        if needed_piece < session.last_piece.saturating_sub(1) {
            session
                .read_head_piece
                .store(needed_piece, Ordering::Relaxed);
        }

        // Wait until `needed_piece` is verified on disk
        while !session.is_piece_complete(needed_piece) {
            if session.cancelled.load(Ordering::Relaxed) {
                return Ok(());
            }
            session.piece_notify.notified().await;
        }

        // Compute how many bytes we can read within this completed piece
        let piece_start_global = (needed_piece as u64) * session.meta.piece_length;
        let piece_end_global =
            ((needed_piece as u64 + 1) * session.meta.piece_length).min(session.meta.total_length);
        let avail_in_piece = piece_end_global.saturating_sub(global_offset);
        let remaining_in_req = (range_end - cur_offset) + 1;
        let to_read = avail_in_piece
            .min(remaining_in_req)
            .min(chunk_buf.len() as u64) as usize;
        if to_read == 0 {
            break;
        }

        let needs_audio_mask = selected_aid > 0 && cur_offset < 65_536;
        if needs_audio_mask && cached_audio_offsets.is_none() {
            let hdr_len = total_len.min(65_536) as usize;
            let mut hdr_buf = vec![0u8; hdr_len];
            if f.seek(SeekFrom::Start(0)).await.is_ok() {
                if let Ok(read_hdr) = f.read(&mut hdr_buf).await {
                    cached_audio_offsets = Some(
                        crate::subtitles::find_mkv_audio_track_type_offsets(&hdr_buf[..read_hdr]),
                    );
                }
            }
        }

        // Fast path: serve directly from RAM `hot_pieces` cache without disk seek/read!
        if let Some(mem_piece) = session.get_hot_piece(needed_piece) {
            let in_piece_offset = (global_offset - piece_start_global) as usize;
            if in_piece_offset + to_read <= mem_piece.len() {
                if needs_audio_mask {
                    chunk_buf[..to_read]
                        .copy_from_slice(&mem_piece[in_piece_offset..in_piece_offset + to_read]);
                    if let Some(ref a_offs) = cached_audio_offsets {
                        crate::subtitles::apply_mkv_audio_track_mask(
                            &mut chunk_buf[..to_read],
                            cur_offset,
                            a_offs,
                            selected_aid,
                        );
                    }
                    if socket.write_all(&chunk_buf[..to_read]).await.is_err() {
                        break;
                    }
                } else if socket
                    .write_all(&mem_piece[in_piece_offset..in_piece_offset + to_read])
                    .await
                    .is_err()
                {
                    break;
                }
                cur_offset += to_read as u64;
                continue;
            }
        }

        f.seek(SeekFrom::Start(cur_offset)).await?;
        f.read_exact(&mut chunk_buf[..to_read]).await?;
        if needs_audio_mask {
            if let Some(ref a_offs) = cached_audio_offsets {
                crate::subtitles::apply_mkv_audio_track_mask(
                    &mut chunk_buf[..to_read],
                    cur_offset,
                    a_offs,
                    selected_aid,
                );
            }
        }

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

pub fn save_path_for_stream_url(url: &str) -> Option<PathBuf> {
    let base_url = url.split('?').next().unwrap_or(url);
    let map = sessions_map().lock().unwrap();
    for s in map.values() {
        if s.stream_url.lock().unwrap().as_deref() == Some(base_url) {
            return Some(s.save_path.clone());
        }
    }
    None
}

pub fn active_save_path_for_episode(anilist_id: i64, ep_key: &str) -> Option<PathBuf> {
    let map = sessions_map().lock().unwrap();
    for s in map.values() {
        if s.anilist_id == anilist_id && s.ep_key == ep_key && s.save_path.is_file() {
            return Some(s.save_path.clone());
        }
    }
    None
}

pub fn cancel_session(session_id: &str) {
    let sess = {
        let mut map = sessions_map().lock().unwrap();
        map.remove(session_id)
    };
    if let Some(s) = sess {
        s.cancelled.store(true, Ordering::Relaxed);
        s.speed_bps.store(0, Ordering::Relaxed);
        s.upload_speed_bps.store(0, Ordering::Relaxed);
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
            .filter(|s| {
                s.anilist_id == anilist_id
                    && s.ep_key == ep_key
                    && (s.mode == "stream" || s.mode == "hybrid")
            })
            .map(|s| s.id.clone())
            .collect()
    };
    for id in to_cancel {
        cancel_session(&id);
    }
}

fn torrent_meta_cache() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    static META_CACHE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    META_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn percent_decode_str(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn extract_magnet_trackers(magnet_uri: Option<&str>) -> Vec<String> {
    let mut trackers = Vec::new();
    if let Some(m) = magnet_uri {
        for part in m.split('&') {
            if let Some(raw_tr) = part.strip_prefix("tr=") {
                let decoded = percent_decode_str(raw_tr).trim().to_string();
                if (decoded.starts_with("udp://")
                    || decoded.starts_with("http://")
                    || decoded.starts_with("https://"))
                    && !trackers.contains(&decoded)
                {
                    trackers.push(decoded);
                }
            }
        }
    }
    for fallback in [
        "http://nyaa.tracker.wf:7777/announce",
        "udp://open.stealth.si:80/announce",
        "udp://tracker.opentrackr.org:1337/announce",
        "udp://exodus.desync.com:6969/announce",
        "udp://tracker.torrent.eu.org:451/announce",
        "udp://opentracker.io:6969/announce",
        "udp://tracker.cyberia.is:6969/announce",
        "udp://explodie.org:6969/announce",
    ] {
        if !trackers.iter().any(|t| t == fallback) {
            trackers.push(fallback.to_string());
        }
    }
    trackers
}

fn parse_hex_hash_20(hex: &str) -> Option<[u8; 20]> {
    parse_btih_20(hex)
}

/// Fallback `BEP 9` (`ut_metadata`) peer-wire metadata exchange when magnet-only releases are not cached on HTTP indexers.
async fn fetch_metadata_via_bep9(info_hash: [u8; 20], trackers: &[String]) -> Result<Vec<u8>> {
    let mut peer_id = [0u8; 20];
    peer_id[0..8].copy_from_slice(b"-KR0061-");
    let now_bytes = (db::now() as u64).to_be_bytes();
    peer_id[8..16].copy_from_slice(&now_bytes);
    peer_id[16..20].copy_from_slice(&std::process::id().to_be_bytes());

    let (peer_tx, mut peer_rx) = tokio::sync::mpsc::unbounded_channel::<SocketAddr>();
    for tr in trackers.iter().take(12).cloned() {
        let tx = peer_tx.clone();
        tokio::spawn(async move {
            let peers = if tr.starts_with("udp://") {
                announce_udp(&tr, &info_hash, &peer_id, 100 * 1024 * 1024)
                    .await
                    .unwrap_or_default()
            } else if tr.starts_with("http://") || tr.starts_with("https://") {
                announce_http(&tr, &info_hash, &peer_id, 100 * 1024 * 1024)
                    .await
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            for p in peers {
                if tx.send(p).is_err() {
                    break;
                }
            }
        });
    }
    // Trackers often don't know magnet-only swarms (or are down) — walk the DHT in parallel.
    crate::dht::spawn_get_peers(info_hash, peer_tx.clone(), Duration::from_secs(14), None);
    drop(peer_tx);

    let (win_tx, mut win_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let mut peer_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let mut seen_peers = HashSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);

    loop {
        tokio::select! {
            Some(torrent_bytes) = win_rx.recv() => {
                for h in peer_handles {
                    h.abort();
                }
                return Ok(torrent_bytes);
            }
            maybe_addr = tokio::time::timeout_at(deadline, peer_rx.recv()) => {
                match maybe_addr {
                    Ok(Some(addr)) => {
                        if seen_peers.len() < 80 && seen_peers.insert(addr) {
                            let wtx = win_tx.clone();
                            peer_handles.push(tokio::spawn(async move {
                                if let Ok(bytes) = fetch_bep9_from_peer(addr, info_hash, peer_id).await {
                                    let _ = wtx.send(bytes);
                                }
                            }));
                        }
                    }
                    Ok(None) => {
                        // All tracker tasks finished; wait up to remaining deadline for active peer workers
                        drop(win_tx);
                        if let Ok(Some(bytes)) = tokio::time::timeout_at(deadline, win_rx.recv()).await {
                            for h in peer_handles {
                                h.abort();
                            }
                            return Ok(bytes);
                        }
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
    }

    for h in peer_handles {
        h.abort();
    }
    bail!("Timed out fetching BEP 9 metadata from swarm")
}

async fn fetch_bep9_from_peer(
    addr: SocketAddr,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
) -> Result<Vec<u8>> {
    let mut stream =
        tokio::time::timeout(Duration::from_millis(2500), TcpStream::connect(addr)).await??;
    let _ = stream.set_nodelay(true);

    let mut hs = [0u8; 68];
    hs[0] = 19;
    hs[1..20].copy_from_slice(b"BitTorrent protocol");
    hs[25] |= 0x10; // BEP 10 Extension Protocol
    hs[28..48].copy_from_slice(&info_hash);
    hs[48..68].copy_from_slice(&peer_id);
    tokio::time::timeout(Duration::from_secs(3), stream.write_all(&hs)).await??;

    let mut resp_hs = [0u8; 68];
    tokio::time::timeout(Duration::from_secs(3), stream.read_exact(&mut resp_hs)).await??;
    if &resp_hs[1..20] != b"BitTorrent protocol"
        || resp_hs[28..48] != info_hash
        || (resp_hs[25] & 0x10) == 0
    {
        bail!("Peer does not support BEP 10");
    }

    let ext_body = b"d1:md11:ut_metadatai1eee";
    let msg_len = (2 + ext_body.len()) as u32;
    let mut ext_hs = Vec::with_capacity(6 + ext_body.len());
    ext_hs.extend_from_slice(&msg_len.to_be_bytes());
    ext_hs.push(20);
    ext_hs.push(0);
    ext_hs.extend_from_slice(ext_body);
    stream.write_all(&ext_hs).await?;

    let mut meta_buf: Vec<u8> = Vec::new();
    let mut num_chunks = 0usize;
    let mut received_chunks = 0usize;
    let mut chunk_done: Vec<bool> = Vec::new();

    for _ in 0..48 {
        let Some((msg_id, payload)) =
            tokio::time::timeout(Duration::from_secs(4), read_peer_msg(&mut stream)).await??
        else {
            continue;
        };
        if msg_id != 20 || payload.is_empty() {
            continue;
        }
        let ext_id = payload[0];
        if ext_id == 0 {
            let (dict, _) = parse_bencode(&payload[1..])?;
            let ut_meta_id = dict
                .dict_get(b"m")
                .and_then(|m| m.dict_get(b"ut_metadata"))
                .and_then(|v| v.as_int())
                .unwrap_or(0);
            let meta_size = dict
                .dict_get(b"metadata_size")
                .and_then(|v| v.as_int())
                .unwrap_or(0);
            if ut_meta_id <= 0 || meta_size <= 0 || meta_size > 8 * 1024 * 1024 {
                bail!("Peer has no ut_metadata");
            }
            let size = meta_size as usize;
            meta_buf = vec![0u8; size];
            num_chunks = (size + 16_383) / 16_384;
            chunk_done = vec![false; num_chunks];

            for p in 0..num_chunks {
                let req_dict = format!("d8:msg_typei0e5:piecei{p}ee");
                let rlen = (2 + req_dict.len()) as u32;
                let mut pkt = Vec::with_capacity(6 + req_dict.len());
                pkt.extend_from_slice(&rlen.to_be_bytes());
                pkt.push(20);
                pkt.push(ut_meta_id as u8);
                pkt.extend_from_slice(req_dict.as_bytes());
                stream.write_all(&pkt).await?;
            }
        } else if ext_id == 1 && !meta_buf.is_empty() {
            let (dict, consumed) = parse_bencode(&payload[1..])?;
            let raw_chunk = payload.get(1 + consumed..).unwrap_or(&[]);
            let msg_type = dict
                .dict_get(b"msg_type")
                .and_then(|v| v.as_int())
                .unwrap_or(-1);
            let piece = dict
                .dict_get(b"piece")
                .and_then(|v| v.as_int())
                .unwrap_or(-1);
            if msg_type == 1 && piece >= 0 && (piece as usize) < num_chunks {
                let p_idx = piece as usize;
                let start = p_idx * 16_384;
                let expected = (meta_buf.len() - start).min(16_384);
                if raw_chunk.len() >= expected && !chunk_done[p_idx] {
                    meta_buf[start..start + expected].copy_from_slice(&raw_chunk[..expected]);
                    chunk_done[p_idx] = true;
                    received_chunks += 1;
                    if received_chunks == num_chunks {
                        if sha1_digest(&meta_buf) != info_hash {
                            bail!("BEP 9 metadata SHA-1 mismatch");
                        }
                        let mut wrapped = Vec::with_capacity(7 + meta_buf.len());
                        wrapped.extend_from_slice(b"d4:info");
                        wrapped.extend_from_slice(&meta_buf);
                        wrapped.push(b'e');
                        return Ok(wrapped);
                    }
                }
            }
        }
    }
    bail!("Incomplete BEP 9 metadata exchange")
}

async fn fetch_torrent_bytes(
    rss_bases: &[String],
    candidate_title: &str,
    torrent_url: Option<&str>,
    magnet_uri: Option<&str>,
    info_hash: Option<&str>,
) -> Result<Vec<u8>> {
    let client = torrent_search_http();

    let hash_opt = info_hash
        .map(|s| s.trim())
        .and_then(|s| normalize_btih_hex(s).or_else(|| Some(s.to_ascii_lowercase())))
        .or_else(|| {
            magnet_uri.and_then(|m| {
                m.split("urn:btih:")
                    .nth(1)
                    .map(|rest| rest.split('&').next().unwrap_or(rest).trim())
                    .and_then(|raw_h| {
                        normalize_btih_hex(raw_h).or_else(|| Some(raw_h.to_ascii_lowercase()))
                    })
            })
        });

    if let Some(ref h) = hash_opt {
        if let Ok(guard) = torrent_meta_cache().lock() {
            if let Some(cached) = guard.get(h) {
                return Ok(cached.clone());
            }
        }
    }

    // 1. Direct .torrent URL
    if let Some(url) = torrent_url.filter(|u| u.starts_with("http")) {
        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() {
                if let Ok(bytes) = resp.bytes().await {
                    if bytes.starts_with(b"d") {
                        let vec = bytes.to_vec();
                        if let Some(h) = hash_opt {
                            if let Ok(mut guard) = torrent_meta_cache().lock() {
                                guard.insert(h, vec.clone());
                            }
                        }
                        return Ok(vec);
                    }
                }
            }
        }
    }

    let hash = hash_opt.context("No .torrent URL or info_hash available")?;
    let upper_hash = hash.to_ascii_uppercase();

    // 2. Race public info_hash caches (`itorrents.org`, `btcache.me`, `torrage.info`),
    // hash RSS lookup, AND title RSS lookup concurrently!
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let mut handles = Vec::new();

    for cache_url in [
        format!("https://itorrents.org/torrent/{upper_hash}.torrent"),
        format!("https://btcache.me/torrent/{upper_hash}"),
        format!("https://torrage.info/torrent.php?h={upper_hash}"),
    ] {
        let c = client.clone();
        let tx_c = tx.clone();
        handles.push(tokio::spawn(async move {
            if let Ok(resp) = c.get(&cache_url).send().await {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes().await {
                        if bytes.starts_with(b"d") {
                            let _ = tx_c.send(bytes.to_vec());
                        }
                    }
                }
            }
        }));
    }

    // Clean release title for RSS title-to-infoHash lookup (strip file extension and CRC hex tags)
    let title_query = {
        static CRC_RE: OnceLock<Regex> = OnceLock::new();
        let crc_re =
            CRC_RE.get_or_init(|| Regex::new(r"\[[0-9A-Fa-f]{8}\]|\.[a-zA-Z0-9]{2,4}$").unwrap());
        let stripped = crc_re.replace_all(candidate_title, " ");
        clean_search_title(&stripped)
    };

    let all_rss_bases: Vec<String> = rss_bases.to_vec();

    for base in &all_rss_bases {
        let base_trimmed = base.trim_end_matches('/').to_string();
        let mut lookups = vec![format!("{base_trimmed}/?page=rss&c=1_0&f=0&q={hash}")];
        if !title_query.is_empty() {
            let encoded_title = url_encode_bytes(title_query.as_bytes());
            lookups.push(format!(
                "{base_trimmed}/?page=rss&c=1_0&f=0&q={encoded_title}"
            ));
        }
        for lookup in lookups {
            let c = client.clone();
            let tx_c = tx.clone();
            let target_hash = hash.clone();
            handles.push(tokio::spawn(async move {
                if let Ok(resp) = c.get(&lookup).send().await {
                    if let Ok(xml) = resp.text().await {
                        for raw_item in xml.split("<item>").skip(1) {
                            let Some(end) = raw_item.find("</item>") else {
                                continue;
                            };
                            let block = &raw_item[..end];
                            let item_hash = extract_xml_tag(block, "nyaa:infoHash")
                                .or_else(|| extract_xml_tag(block, "infoHash"))
                                .and_then(|raw_h| {
                                    normalize_btih_hex(&raw_h)
                                        .or_else(|| Some(raw_h.to_ascii_lowercase()))
                                })
                                .unwrap_or_default();
                            if item_hash == target_hash {
                                if let Some(t_url) = extract_xml_tag(block, "link") {
                                    if let Ok(r2) = c.get(&t_url).send().await {
                                        if let Ok(bytes) = r2.bytes().await {
                                            if bytes.starts_with(b"d") {
                                                let _ = tx_c.send(bytes.to_vec());
                                                return;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }));
        }
    }
    // 3. Race the swarm itself (BEP 9 `ut_metadata` via tracker + DHT peers) at the same time,
    // so a dead/blocked HTTP cache never delays magnet-only releases.
    let target_bytes = parse_hex_hash_20(&hash);
    if let Some(hash_bytes) = target_bytes {
        let trackers = extract_magnet_trackers(magnet_uri);
        let tx_c = tx.clone();
        handles.push(tokio::spawn(async move {
            if let Ok(bytes) = fetch_metadata_via_bep9(hash_bytes, &trackers).await {
                let _ = tx_c.send(bytes);
            }
        }));
    }
    drop(tx);

    // Accept the first response whose `info` dictionary actually hashes to the requested info_hash.
    let overall = tokio::time::Instant::now() + Duration::from_secs(25);
    while let Ok(Some(candidate)) = tokio::time::timeout_at(overall, rx.recv()).await {
        let valid = match target_bytes {
            Some(expected) => parse_bencode(&candidate)
                .ok()
                .and_then(|(root, _)| root.dict_get_span(b"info").map(|(_, s, e)| (s, e)))
                .map(|(s, e)| sha1_digest(&candidate[s..e]) == expected)
                .unwrap_or(false),
            None => true,
        };
        if !valid {
            continue;
        }
        for h in &handles {
            h.abort();
        }
        if let Ok(mut guard) = torrent_meta_cache().lock() {
            guard.insert(hash, candidate.clone());
        }
        return Ok(candidate);
    }
    for h in &handles {
        h.abort();
    }

    bail!("Couldn't get this release's metadata — no cache, indexer or peer had it. Try another release.")
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
        .query_row(
            "SELECT path FROM library_folders ORDER BY id LIMIT 1",
            [],
            |r| r.get(0),
        )
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
    let dl_dir = st
        .data_dir
        .join("downloads")
        .join(sanitize_filename(&show_title));
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
    let (mode, rss_bases, allow_seeding) = {
        let conn = st.db();
        let m = mode_override.unwrap_or_else(|| {
            db::get_setting(&conn, "pref.torrent_action")
                .ok()
                .flatten()
                .unwrap_or_else(|| "stream".to_string())
        });
        let rss_bases: Vec<String> = list_torrent_extensions(&conn)
            .into_iter()
            .filter(|e| e.enabled && e.kind == "rss")
            .map(|e| e.url)
            .collect();
        let seed = db::get_setting(&conn, "pref.torrent_seeding")
            .ok()
            .flatten()
            .map(|v| v != "0")
            .unwrap_or(true);
        (m, rss_bases, seed)
    };

    // Cancel any previous session for the same episode
    on_player_stopped(anilist_id, &ep_key);

    let raw_torrent = fetch_torrent_bytes(
        &rss_bases,
        &candidate.title,
        candidate.torrent_url.as_deref(),
        candidate.magnet_uri.as_deref(),
        candidate.info_hash.as_deref(),
    )
    .await?;

    let mut meta = parse_torrent_bytes(&raw_torrent, episode_num)?;
    for tr in extract_magnet_trackers(candidate.magnet_uri.as_deref()) {
        if !meta.trackers.contains(&tr) {
            meta.trackers.push(tr);
        }
    }
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
    let last_piece =
        ((last_byte / meta.piece_length) as usize).min(meta.pieces.len().saturating_sub(1));

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
        piece_claimed_at: Mutex::new(vec![None; num_pieces]),
        read_head_piece: AtomicUsize::new(first_piece),
        downloaded_bytes: AtomicU64::new(0),
        wire_rx_bytes: AtomicU64::new(0),
        uploaded_bytes: AtomicU64::new(0),
        speed_bps: AtomicU64::new(0),
        upload_speed_bps: AtomicU64::new(0),
        active_peers: AtomicUsize::new(0),
        state_str: Mutex::new("buffering".into()),
        error_str: Mutex::new(None),
        stream_url: Mutex::new(None),
        allow_seeding,
        cancelled: AtomicBool::new(false),
        piece_notify: Notify::new(),
        file_write_lock: tokio::sync::Mutex::new(()),
        hot_pieces: Mutex::new(HashMap::new()),
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

    // Stream discovered peers from all trackers in parallel so fast UDP/HTTP trackers
    // begin connecting to seeders within ~100ms instead of waiting for slow trackers.
    let (peer_tx, mut peer_rx) = tokio::sync::mpsc::unbounded_channel::<SocketAddr>();
    spawn_tracker_peer_discovery(&session.meta, &peer_id, peer_tx.clone());
    // DHT runs alongside trackers; it stops by itself once this orchestrator drops `peer_rx`.
    crate::dht::spawn_get_peers(session.meta.info_hash, peer_tx.clone(), Duration::from_secs(30), None);
    let mut last_discovery_t = std::time::Instant::now();

    let mut seen_peers: HashSet<SocketAddr> = HashSet::new();
    let mut peer_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // Also auto-fetch Japanese subtitles from Jimaku if enabled, key is configured,
    // preferred subtitle language is Japanese, and this episode does not already have
    // a saved subtitle in `linked_subtitles`.
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
        let wants_jp = crate::player::wants_japanese_subs(&crate::player::effective_track_prefs(
            &conn,
            session.anilist_id,
        ));
        let already_saved =
            crate::subtitles::has_saved_subtitle(&conn, session.anilist_id, &session.ep_key);
        enabled && has_key && wants_jp && !already_saved
    };

    if auto_subs {
        let st_sub = st.clone();
        let anilist_id = session.anilist_id;
        let ep_key = session.ep_key.clone();
        tokio::spawn(async move {
            if let Ok(res) =
                crate::subtitles::jimaku_search_files(&st_sub, anilist_id, episode_num).await
            {
                if let Some(best) = res
                    .files
                    .into_iter()
                    .find(|f| !f.is_rar && f.match_score >= 50)
                {
                    let _ = crate::subtitles::jimaku_download_and_process(
                        &st_sub, anilist_id, &ep_key, &best.name, &best.url, true,
                    )
                    .await;
                }
            }
        });
    }

    let mut launched_player = false;
    let mut last_bytes = 0u64;
    let mut last_up_bytes = 0u64;
    let mut last_speed_t = std::time::Instant::now();
    let mut last_emit_t = std::time::Instant::now();
    let start_t = std::time::Instant::now();
    let _ = app.emit("torrent-progress", session.status());

    loop {
        if session.cancelled.load(Ordering::Relaxed) {
            for h in &peer_handles {
                h.abort();
            }
            session.speed_bps.store(0, Ordering::Relaxed);
            session.upload_speed_bps.store(0, Ordering::Relaxed);
            session.active_peers.store(0, Ordering::Relaxed);
            *session.state_str.lock().unwrap() = "cancelled".into();
            sessions_map().lock().unwrap().remove(&session.id);
            let _ = app.emit("torrent-progress", session.status());
            break;
        }

        // Drain any newly discovered peer addresses and spawn up to 120 concurrent peer workers
        while let Ok(addr) = peer_rx.try_recv() {
            if seen_peers.len() < 120 && seen_peers.insert(addr) {
                let s = session.clone();
                let tx = peer_tx.clone();
                peer_handles.push(tokio::spawn(async move {
                    let _ = run_peer_worker(s, addr, peer_id, tx).await;
                }));
            }
        }

        if start_t.elapsed() >= Duration::from_secs(15) && seen_peers.is_empty() {
            for h in &peer_handles {
                h.abort();
            }
            *session.state_str.lock().unwrap() = "error".into();
            *session.error_str.lock().unwrap() =
                Some("No peers found for this release (trackers and DHT) — trying next source…".into());
            sessions_map().lock().unwrap().remove(&session.id);
            let _ = app.emit("torrent-progress", session.status());
            return;
        }

        // Fast dead-swarm failover: if no peer has transferred a single byte after 13 seconds
        // (enough time for DHT peers to arrive), fail so the UI can auto-try the next candidate.
        if !launched_player
            && start_t.elapsed() >= Duration::from_secs(13)
            && session.wire_rx_bytes.load(Ordering::Relaxed) == 0
            && session.downloaded_bytes.load(Ordering::Relaxed) == 0
        {
            for h in &peer_handles {
                h.abort();
            }
            session.speed_bps.store(0, Ordering::Relaxed);
            session.upload_speed_bps.store(0, Ordering::Relaxed);
            session.active_peers.store(0, Ordering::Relaxed);
            *session.state_str.lock().unwrap() = "error".into();
            *session.error_str.lock().unwrap() =
                Some("No responsive seeders on this release — trying next source…".into());
            sessions_map().lock().unwrap().remove(&session.id);
            let _ = app.emit("torrent-progress", session.status());
            return;
        }

        let elapsed_sec = last_speed_t.elapsed().as_secs_f64();
        if elapsed_sec >= 0.5 {
            let cur_bytes = session.wire_rx_bytes.load(Ordering::Relaxed);
            let delta = cur_bytes.saturating_sub(last_bytes);
            let inst_speed = ((delta as f64) / elapsed_sec) as u64;
            last_bytes = cur_bytes;
            let prev_speed = session.speed_bps.load(Ordering::Relaxed);
            let speed = if prev_speed == 0 {
                inst_speed
            } else if inst_speed == 0 {
                if session.is_all_complete() {
                    0
                } else {
                    prev_speed / 2
                }
            } else {
                (prev_speed * 2 + inst_speed * 3) / 5
            };

            let cur_up = session.uploaded_bytes.load(Ordering::Relaxed);
            let up_delta = cur_up.saturating_sub(last_up_bytes);
            let up_speed = ((up_delta as f64) / elapsed_sec) as u64;
            last_up_bytes = cur_up;

            last_speed_t = std::time::Instant::now();
            session.speed_bps.store(speed, Ordering::Relaxed);
            session.upload_speed_bps.store(up_speed, Ordering::Relaxed);
            // Keep the peer supply topped up: re-announce to trackers and walk the DHT again
            // whenever the number of working peers drops low (at most every 20 s).
            if session.active_peers.load(Ordering::Relaxed) < 12
                && last_discovery_t.elapsed() >= Duration::from_secs(20)
            {
                last_discovery_t = std::time::Instant::now();
                seen_peers.clear();
                spawn_tracker_peer_discovery(&session.meta, &peer_id, peer_tx.clone());
                crate::dht::spawn_get_peers(
                    session.meta.info_hash,
                    peer_tx.clone(),
                    Duration::from_secs(20),
                    None,
                );
            }
        }

        // Launch player as soon as initial pieces are buffered in "stream" or "hybrid" mode
        if !launched_player
            && (session.mode == "stream" || session.mode == "hybrid")
            && session.is_ready_to_stream()
        {
            launched_player = true;
            *session.state_str.lock().unwrap() = "streaming".into();
            let _ = app.emit("torrent-progress", session.status());
            last_emit_t = std::time::Instant::now();
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
            session.upload_speed_bps.store(0, Ordering::Relaxed);
            session.active_peers.store(0, Ordering::Relaxed);
            *session.state_str.lock().unwrap() = "completed".into();
            sessions_map().lock().unwrap().remove(&session.id);
            let _ = app.emit("torrent-progress", session.status());

            // If mode is "hybrid" or "download", trigger a rescan so the downloaded file is linked in library
            if session.mode == "hybrid" || session.mode == "download" {
                // Attempt auto-retime on any linked Japanese subtitles now that the full .mkv is on disk
                if let Ok(subs) =
                    crate::subtitles::list_linked_subtitles(&st.db(), session.anilist_id)
                {
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

        if last_emit_t.elapsed() >= Duration::from_millis(380) {
            let _ = app.emit("torrent-progress", session.status());
            last_emit_t = std::time::Instant::now();
        }
        tokio::select! {
            Some(addr) = peer_rx.recv() => {
                if seen_peers.len() < 120 && seen_peers.insert(addr) {
                    let s = session.clone();
                    let tx = peer_tx.clone();
                    peer_handles.push(tokio::spawn(async move {
                        let _ = run_peer_worker(s, addr, peer_id, tx).await;
                    }));
                }
            }
            _ = session.piece_notify.notified(), if !launched_player || session.cancelled.load(Ordering::Relaxed) => {}
            _ = tokio::time::sleep(Duration::from_millis(800)) => {}
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
    fn search_helpers_keep_relevant_low_seeder_releases() {
        // SubsPlease searches by show name — episode tokens must be stripped.
        assert_eq!(strip_episode_from_query("Sousou no Frieren 05"), "Sousou no Frieren");
        assert_eq!(strip_episode_from_query("Kusuriya no Hitorigoto S2 05"), "Kusuriya no Hitorigoto");
        assert_eq!(strip_episode_from_query("Dandadan S01E05"), "Dandadan");
        assert_eq!(strip_episode_from_query("Frieren"), "Frieren");

        // A correct-episode release with stale "0 seeders" must stay visible (just ranked low)...
        let known = vec!["Sousou no Frieren".to_string()];
        let title = "[SubsPlease] Sousou no Frieren - 05 (1080p) [ABCD1234].mkv";
        let s0 = score_candidate_with_context(
            title, Some("SubsPlease"), Some("1080p"), 0, 1_400 * 1024 * 1024, false, 5, None, false, "1080p", &known,
        );
        assert!(is_relevant_candidate(s0, 0));
        // ...while the wrong episode is still filtered out.
        let wrong = score_candidate_with_context(
            title, Some("SubsPlease"), Some("1080p"), 200, 1_400 * 1024 * 1024, false, 9, None, false, "1080p", &known,
        );
        assert!(!is_relevant_candidate(wrong, 200));
    }

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
        assert!(meta
            .trackers
            .iter()
            .any(|t| t == "http://tracker.example/ann"));
    }

    #[test]
    fn scores_and_parses_release_tags() {
        let title = "[SubsPlease] Kusuriya no Hitorigoto - 25 (1080p) [ABCD1234].mkv";
        let (group, res, batch) = parse_release_tags(title);
        assert_eq!(group.as_deref(), Some("SubsPlease"));
        assert_eq!(res.as_deref(), Some("1080p"));
        assert!(!batch);

        let s_match = score_candidate(
            title,
            group.as_deref(),
            res.as_deref(),
            150,
            batch,
            25,
            "1080p",
        );
        let s_wrong_ep = score_candidate(
            title,
            group.as_deref(),
            res.as_deref(),
            150,
            batch,
            12,
            "1080p",
        );
        assert!(s_match > 1500);
        assert!(s_wrong_ep < 0);

        // Season 2 episode 1 matching continuous episode 25 via `abs_episode`
        let known = vec![
            "Kusuriya no Hitorigoto 2nd Season".to_string(),
            "The Apothecary Diaries Season 2".to_string(),
        ];
        let s_abs = score_candidate_with_context(
            title,
            group.as_deref(),
            res.as_deref(),
            150,
            950 * 1024 * 1024,
            batch,
            1,
            Some(25),
            false,
            "1080p",
            &known,
        );
        assert!(s_abs > 1500);

        // Ensure a well-seeded release (80 seeders, 720p) beats a dead/NAT-prone 1-seeder 1080p release
        let s_1_seeder = score_candidate_with_context(
            "[Ember] Kusuriya no Hitorigoto - 25 (1080p).mkv",
            Some("Ember"),
            Some("1080p"),
            1,
            600 * 1024 * 1024,
            false,
            25,
            None,
            false,
            "1080p",
            &known,
        );
        let s_80_seeders = score_candidate_with_context(
            "[Group] Kusuriya no Hitorigoto - 25 (720p).mkv",
            Some("Group"),
            Some("720p"),
            80,
            500 * 1024 * 1024,
            false,
            25,
            None,
            false,
            "1080p",
            &known,
        );
        assert!(s_80_seeders > s_1_seeder);

        // Ensure English-dub-only releases are heavily penalized (< -500) while Dual-Audio releases score high
        let s_eng_dub = score_candidate_with_context(
            "[Yameii] Kusuriya no Hitorigoto - 25 [English Dub] [CR WEB-DL 1080p]",
            Some("Yameii"),
            Some("1080p"),
            90,
            600 * 1024 * 1024,
            false,
            25,
            None,
            false,
            "1080p",
            &known,
        );
        let s_dual_audio = score_candidate_with_context(
            "[Ember] Kusuriya no Hitorigoto - 25 (1080p) [Dual Audio]",
            Some("Ember"),
            Some("1080p"),
            90,
            600 * 1024 * 1024,
            false,
            25,
            None,
            false,
            "1080p",
            &known,
        );
        assert!(s_eng_dub < -500);
        assert!(s_dual_audio > 1800);

        // Season splitting and punctuation cleaning (including possessive apostrophe removal)
        let (s_num, base) = split_season_from_title("Kusuriya no Hitorigoto: 2nd Season (TV)");
        assert_eq!(s_num, Some(2));
        assert_eq!(base, "Kusuriya no Hitorigoto");
        assert_eq!(
            clean_search_title("Frieren: Beyond Journey's End"),
            "Frieren Beyond Journey End"
        );

        let trs = extract_magnet_trackers(Some(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&tr=http%3A%2F%2Ftracker.example%3A7777%2Fannounce",
        ));
        assert!(trs
            .iter()
            .any(|t| t == "http://tracker.example:7777/announce"));
        assert!(parse_hex_hash_20("0123456789abcdef0123456789abcdef01234567").is_some());
        // SubsPlease 32-char RFC 4648 Base32 btih magnet info_hash decoding
        let sp_hex =
            normalize_btih_hex("omo5w7qs3cczv23xxa76jrl6hnspnmlb").expect("valid base32 btih");
        assert_eq!(sp_hex.len(), 40);
        assert_eq!(
            parse_hex_hash_20("omo5w7qs3cczv23xxa76jrl6hnspnmlb"),
            parse_hex_hash_20(&sp_hex)
        );

        // Modular extension URL classification & BEP 11 PEX compact peer parsing
        let ext_rss = build_extension_from_url("https://example-rss.org", None, None);
        assert_eq!(ext_rss.kind, "rss");
        let ext_sd = build_extension_from_url(
            "https://releases.moe/api/collections/entries/records",
            None,
            None,
        );
        assert_eq!(ext_sd.kind, "seadex-api");

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        // Extended msg id=1, bencoded `d5:added6:<1.2.3.4:6881>e`
        let pex_payload = b"\x01d5:added6:\x01\x02\x03\x04\x1a\xe1e";
        handle_extended_pex_msg(pex_payload, &tx);
        let peer = rx.try_recv().unwrap();
        assert_eq!(peer.to_string(), "1.2.3.4:6881");
    }
}
