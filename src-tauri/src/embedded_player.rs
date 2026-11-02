//! Built-in Memento-style video player engine.
//!
//! Renders video directly inside Kura's window using a local HTTP byte-range media server
//! (with automatic live `mpv` H.264/AAC stream transcoding fallback for 10-bit HEVC / AC3),
//! pure-Rust Matroska embedded subtitle/chapter extraction, Skip OP/ED, and pause-gated
//! interactive Japanese subtitles with Yomitan dictionary lookup.

use crate::db;
use crate::player::{self, AniSkipTimes, Ep, Kind};
use crate::service::AppState;
use crate::subtitles::{self, SubtitleCue};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const EXTERNAL_SUB_TRACK_ID: i64 = 1000;

// ---------------------------------------------------------------------------
// State & IPC Models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackItem {
    pub id: i64,
    pub track_type: String, // "audio" | "sub"
    pub title: Option<String>,
    pub lang: Option<String>,
    pub codec: Option<String>,
    pub selected: bool,
    pub external: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChapterItem {
    pub title: String,
    pub time: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinPlayerState {
    pub active: bool,
    pub backend: String, // "html5" | "transcode"
    pub anilist_id: i64,
    pub ep_key: String,
    pub path: String,
    pub stream_url: String,
    pub transcode_base_url: Option<String>,
    pub show_title: String,
    pub episode_title: String,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub muted: bool,
    pub speed: f64,
    pub aid: Option<i64>,
    pub sid: Option<i64>,
    pub sub_text: String,
    pub sub_offset_sec: f64,
    pub audio_tracks: Vec<TrackItem>,
    pub sub_tracks: Vec<TrackItem>,
    pub chapters: Vec<ChapterItem>,
    pub op_start: Option<f64>,
    pub op_end: Option<f64>,
    pub ed_start: Option<f64>,
    pub ed_end: Option<f64>,
    pub has_next_episode: bool,
    pub external_sub_path: Option<String>,
}

impl Default for BuiltinPlayerState {
    fn default() -> Self {
        Self {
            active: false,
            backend: "html5".to_string(),
            anilist_id: 0,
            ep_key: String::new(),
            path: String::new(),
            stream_url: String::new(),
            transcode_base_url: None,
            show_title: String::new(),
            episode_title: String::new(),
            paused: false,
            position: 0.0,
            duration: 0.0,
            volume: 100.0,
            muted: false,
            speed: 1.0,
            aid: None,
            sid: None,
            sub_text: String::new(),
            sub_offset_sec: 0.0,
            audio_tracks: Vec::new(),
            sub_tracks: Vec::new(),
            chapters: Vec::new(),
            op_start: None,
            op_end: None,
            ed_start: None,
            ed_end: None,
            has_next_episode: false,
            external_sub_path: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BuiltinCommand {
    PlayPause,
    Pause,
    Resume,
    Seek { position: f64 },
    SeekRelative { delta: f64 },
    SetVolume { volume: f64 },
    ToggleMute,
    SetSpeed { speed: f64 },
    SetAudioTrack { id: i64 },
    SetSubTrack { id: i64 }, // 0 = off
    AddSubOffset { delta: f64 },
    LoadSubFile { path: String },
    NextEpisode,
    OpenExternalMpv,
    Html5Tick {
        position: f64,
        duration: f64,
        paused: bool,
        ended: bool,
    },
    SyncBounds,
    Stop,
}

struct RuntimeSession {
    state: BuiltinPlayerState,
    cues: Vec<SubtitleCue>,
    cues_by_track: HashMap<i64, Vec<SubtitleCue>>,
    threshold: f64,
    autoplay: bool,
    marked_watched: bool,
    aniskip_queried: bool,
    tick_count: u64,
    ext_auto_offset_sec: Option<f64>,
    ext_matched_cues: usize,
    user_adjusted_offset: bool,
}

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);
static RUNTIME: OnceLock<Mutex<RuntimeSession>> = OnceLock::new();

fn runtime() -> &'static Mutex<RuntimeSession> {
    RUNTIME.get_or_init(|| {
        Mutex::new(RuntimeSession {
            state: BuiltinPlayerState::default(),
            cues: Vec::new(),
            cues_by_track: HashMap::new(),
            threshold: 0.90,
            autoplay: true,
            marked_watched: false,
            aniskip_queried: false,
            tick_count: 0,
            ext_auto_offset_sec: None,
            ext_matched_cues: 0,
            user_adjusted_offset: false,
        })
    })
}

pub fn get_state() -> BuiltinPlayerState {
    runtime()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .state
        .clone()
}

pub fn sync_bounds(_app: &AppHandle) {}

pub fn reload_cues_if_active(anilist_id: i64, ep_key: &str, sub_path: &str) -> bool {
    let cues = subtitles::parse_subtitle_file_cues(Path::new(sub_path));
    if cues.is_empty() {
        return false;
    }
    let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
    if g.state.active && g.state.anilist_id == anilist_id && g.state.ep_key == ep_key {
        g.state.external_sub_path = Some(sub_path.to_string());
        let file_name = Path::new(sub_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "External Subtitle".to_string());

        for tr in &mut g.state.sub_tracks {
            tr.selected = false;
        }
        if let Some(existing) = g.state.sub_tracks.iter_mut().find(|t| t.id == EXTERNAL_SUB_TRACK_ID) {
            existing.title = Some(file_name);
            existing.selected = true;
        } else {
            g.state.sub_tracks.insert(
                0,
                TrackItem {
                    id: EXTERNAL_SUB_TRACK_ID,
                    track_type: "sub".to_string(),
                    title: Some(file_name),
                    lang: Some("ja".to_string()),
                    codec: Some("external".to_string()),
                    selected: true,
                    external: true,
                },
            );
        }
        g.state.sid = Some(EXTERNAL_SUB_TRACK_ID);
        g.user_adjusted_offset = false;
        g.ext_matched_cues = 0;
        if let Some((auto_off, matched)) =
            subtitles::compute_live_cue_offset_from_tracks(&cues, &g.cues_by_track, EXTERNAL_SUB_TRACK_ID)
        {
            g.ext_auto_offset_sec = Some(auto_off);
            g.ext_matched_cues = matched;
            g.state.sub_offset_sec = auto_off;
        }
        let pos = g.state.position;
        let off = g.state.sub_offset_sec;
        g.state.sub_text = find_cue_at(&cues, pos, off).unwrap_or_default();
        g.cues_by_track.insert(EXTERNAL_SUB_TRACK_ID, cues.clone());
        g.cues = cues;
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Local HTTP Byte-Range + Live MPV Transcode Server
// ---------------------------------------------------------------------------

async fn spawn_local_media_server(
    source_path: String,
    session_id: u64,
    has_mpv: bool,
) -> Result<(String, Option<String>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let is_remote = source_path.starts_with("http://") || source_path.starts_with("https://");

    let stream_url = if is_remote {
        source_path.clone()
    } else {
        format!("http://127.0.0.1:{port}/video")
    };
    let transcode_base = if has_mpv {
        Some(format!("http://127.0.0.1:{port}/transcode"))
    } else {
        None
    };

    let active_transcode_id = Arc::new(AtomicU64::new(0));

    tokio::spawn(async move {
        loop {
            if SESSION_COUNTER.load(Ordering::SeqCst) != session_id {
                break;
            }
            let accept_res = tokio::time::timeout(Duration::from_secs(2), listener.accept()).await;
            let Ok(Ok((socket, _))) = accept_res else {
                continue;
            };
            if SESSION_COUNTER.load(Ordering::SeqCst) != session_id {
                break;
            }
            let src = source_path.clone();
            let tc_counter = active_transcode_id.clone();
            tokio::spawn(async move {
                let _ = handle_media_http_client(socket, src, session_id, tc_counter).await;
            });
        }
    });

    Ok((stream_url, transcode_base))
}

fn parse_query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut kv = pair.splitn(2, '=');
        if kv.next()? == key {
            return kv.next().map(String::from);
        }
    }
    None
}

fn content_type_for_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".mp4") || lower.ends_with(".m4v") {
        "video/mp4"
    } else if lower.ends_with(".webm") {
        "video/webm"
    } else {
        "video/x-matroska"
    }
}

async fn handle_media_http_client(
    mut socket: TcpStream,
    source_path: String,
    session_id: u64,
    active_transcode_id: Arc<AtomicU64>,
) -> Result<()> {
    let mut req_buf = [0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut req_buf)).await??;
    if n == 0 {
        return Ok(());
    }
    let req_str = String::from_utf8_lossy(&req_buf[..n]);
    let first_line = req_str.lines().next().unwrap_or("");
    let is_head = first_line.starts_with("HEAD ");
    let is_options = first_line.starts_with("OPTIONS ");

    if is_options {
        let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, HEAD, OPTIONS\r\nAccess-Control-Allow-Headers: Range\r\nConnection: close\r\n\r\n";
        socket.write_all(resp.as_bytes()).await?;
        return Ok(());
    }

    let req_target = first_line.split_whitespace().nth(1).unwrap_or("/");
    let (route_path, query) = req_target.split_once('?').unwrap_or((req_target, ""));

    if route_path.starts_with("/transcode") {
        let start_sec = parse_query_param(query, "start")
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let aid = parse_query_param(query, "aid")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let copy_video = parse_query_param(query, "vc")
            .map(|v| v != "h264")
            .unwrap_or(true);
        return handle_transcode_stream(
            socket,
            &source_path,
            start_sec,
            aid,
            copy_video,
            session_id,
            active_transcode_id,
        )
        .await;
    }

    // Serve local file with HTTP 206 Partial Content byte-range support
    let file_meta = tokio::fs::metadata(&source_path).await?;
    let total_len = file_meta.len();
    if total_len == 0 {
        return Ok(());
    }

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
    let mime = content_type_for_path(&source_path);
    let header = if has_range {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nContent-Range: bytes {range_start}-{range_end}/{total_len}\r\nContent-Length: {content_len}\r\nConnection: close\r\n\r\n"
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {total_len}\r\nConnection: close\r\n\r\n"
        )
    };
    socket.write_all(header.as_bytes()).await?;
    if is_head {
        return Ok(());
    }

    let mut f = tokio::fs::File::open(&source_path).await?;
    f.seek(SeekFrom::Start(range_start)).await?;

    let mut remaining = content_len;
    let mut buf = vec![0u8; 65_536];
    while remaining > 0 {
        if SESSION_COUNTER.load(Ordering::SeqCst) != session_id {
            break;
        }
        let to_read = (remaining as usize).min(buf.len());
        let read_n = f.read(&mut buf[..to_read]).await?;
        if read_n == 0 {
            break;
        }
        if socket.write_all(&buf[..read_n]).await.is_err() {
            break;
        }
        remaining -= read_n as u64;
    }

    Ok(())
}

async fn handle_transcode_stream(
    mut socket: TcpStream,
    source_path: &str,
    start_sec: f64,
    aid: i64,
    copy_video: bool,
    session_id: u64,
    active_transcode_id: Arc<AtomicU64>,
) -> Result<()> {
    let Some(mpv_exe) = player::detect(Kind::Mpv) else {
        return Ok(());
    };

    let my_tc_id = active_transcode_id.fetch_add(1, Ordering::SeqCst) + 1;

    let header = "HTTP/1.1 200 OK\r\nContent-Type: video/x-matroska\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n";
    socket.write_all(header.as_bytes()).await?;

    let mut cmd = std::process::Command::new(&mpv_exe);
    cmd.arg("--no-config")
        .arg("--no-terminal")
        .arg("--sub=no")
        .arg("--vid=1");

    if copy_video {
        cmd.arg("--ovc=copy");
    } else {
        cmd.arg("--hwdec=auto-copy")
            .arg("--vf=format=yuv420p")
            .arg("--ovc=libx264")
            .arg("--ovcopts=preset=ultrafast,tune=zerolatency,crf=20");
    }

    cmd.arg("--audio-channels=stereo")
        .arg("--oac=aac")
        .arg("--oacopts=b=192k")
        .arg("--of=matroska")
        .arg("--o=-");

    if aid > 0 {
        cmd.arg(format!("--aid={aid}"));
    } else {
        cmd.arg("--alang=jpn,ja,jp,japanese,und,eng,en");
    }
    if start_sec > 0.5 {
        cmd.arg(format!("--start=+{start_sec:.1}"));
    }
    cmd.arg("--").arg(source_path);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn()?;
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Ok(());
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
    let tc_clone = active_transcode_id.clone();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 32_768];
        loop {
            if SESSION_COUNTER.load(Ordering::SeqCst) != session_id
                || tc_clone.load(Ordering::SeqCst) != my_tc_id
            {
                break;
            }
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    });

    while let Some(chunk) = rx.recv().await {
        if SESSION_COUNTER.load(Ordering::SeqCst) != session_id
            || active_transcode_id.load(Ordering::SeqCst) != my_tc_id
        {
            break;
        }
        if socket.write_all(&chunk).await.is_err() {
            break;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Metadata & Cue Lookup Helpers
// ---------------------------------------------------------------------------

fn is_japanese_lang(lang: &str) -> bool {
    let l = lang.trim().to_ascii_lowercase();
    l == "jpn" || l == "ja" || l == "jp" || l.contains("japan") || l.contains("日本語")
}

fn resolve_titles(conn: &rusqlite::Connection, anilist_id: i64, ep_key: &str) -> (String, String) {
    let media_info: Option<(Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT title_english, title_romaji, title_native FROM media WHERE anilist_id = ?1",
            params![anilist_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .unwrap_or(None);
    let show_title = media_info
        .and_then(|(en, ro, nat)| en.or(ro).or(nat))
        .unwrap_or_else(|| "Anime".to_string());

    let ep_info: Option<(Option<String>, Option<String>, Option<String>, i64, i64)> = conn
        .query_row(
            "SELECT title_en, title_romaji, title_ja, is_special, number FROM episodes WHERE anilist_id = ?1 AND ep_key = ?2",
            params![anilist_id, ep_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .unwrap_or(None);

    let episode_title = if let Some((en, ro, ja, is_special, num)) = ep_info {
        let ep_t = en.or(ro).or(ja);
        let prefix = if is_special != 0 || ep_key.starts_with('S') {
            format!("Special {:02}", num)
        } else {
            format!("Episode {:02}", num)
        };
        if let Some(t) = ep_t.filter(|t| !t.trim().is_empty()) {
            format!("{prefix} · {t}")
        } else {
            prefix
        }
    } else if let Ok(num) = ep_key.parse::<i64>() {
        format!("Episode {:02}", num)
    } else {
        format!("Episode {ep_key}")
    };

    (show_title, episode_title)
}

fn find_cue_at(cues: &[SubtitleCue], pos_sec: f64, offset_sec: f64) -> Option<String> {
    if cues.is_empty() {
        return None;
    }
    let target_ms = ((pos_sec - offset_sec) * 1000.0).round() as i64;
    let idx = cues.partition_point(|c| c.end_ms < target_ms);
    let mut lines = Vec::new();
    for c in &cues[idx..] {
        if c.start_ms > target_ms {
            break;
        }
        if target_ms >= c.start_ms && target_ms <= c.end_ms {
            if !lines.contains(&c.text) {
                lines.push(c.text.clone());
            }
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn infer_op_ed_from_chapters(
    chapters: &[ChapterItem],
    duration: f64,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    let mut op_s = None;
    let mut op_e = None;
    let mut ed_s = None;
    let mut ed_e = None;

    for (i, ch) in chapters.iter().enumerate() {
        let lower = ch.title.trim().to_ascii_lowercase();
        let next_t = chapters
            .get(i + 1)
            .map(|n| n.time)
            .unwrap_or_else(|| if duration > ch.time { duration } else { ch.time + 90.0 });
        let span = next_t - ch.time;
        if !(25.0..=210.0).contains(&span) {
            continue;
        }
        let is_op = lower == "op"
            || lower.starts_with("op ")
            || lower.contains("opening")
            || lower.contains("オープニング")
            || lower == "アバン+op";
        let is_ed = lower == "ed"
            || lower.starts_with("ed ")
            || lower.contains("ending")
            || lower.contains("エンディング");

        if is_op && op_s.is_none() {
            op_s = Some(ch.time);
            op_e = Some(next_t);
        } else if is_ed && ed_s.is_none() {
            ed_s = Some(ch.time);
            ed_e = Some(next_t);
        }
    }

    (op_s, op_e, ed_s, ed_e)
}

fn emit_state(app: &AppHandle, state: &BuiltinPlayerState) {
    let _ = app.emit("builtin-player-state", state);
}

// ---------------------------------------------------------------------------
// Launching & Controlling the Built-in Player
// ---------------------------------------------------------------------------

pub fn start(app: AppHandle, st: Arc<AppState>, ep: Ep) -> Result<()> {
    let session_id = SESSION_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    let _play_session = st.play_session.fetch_add(1, Ordering::SeqCst) + 1;

    let (start_pos, threshold, autoplay, show_title, episode_title, has_next, ext_sub_path, ext_cues) = {
        let conn = st.db();
        let p = player::prefs(&conn);
        let saved = player::saved_progress(&conn, ep.anilist_id, &ep.ep_key);
        let start_pos = if p.resume {
            player::resume_point(saved, p.threshold)
        } else {
            0.0
        };
        let (show_title, episode_title) = resolve_titles(&conn, ep.anilist_id, &ep.ep_key);
        let has_next = player::next_episode(&conn, &ep).is_some();
        let (ext_sub_path, ext_cues) = subtitles::load_cues_for_episode(&conn, ep.anilist_id, &ep.ep_key, &ep.path);
        (
            start_pos,
            p.threshold,
            p.autoplay,
            show_title,
            episode_title,
            has_next,
            ext_sub_path,
            ext_cues,
        )
    };

    // Ensure starter Japanese dictionary is seeded in background so first hover is instant
    let st_dict = st.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Ok(conn) = crate::dictionary::open_dict_db(&st_dict.data_dir) {
            let _ = crate::dictionary::ensure_starter_dictionary(&conn);
        }
    });

    let mut sub_tracks = Vec::new();
    let mut cues_by_track = HashMap::new();
    let mut active_sid = None;
    let mut initial_sub_text = String::new();

    if !ext_cues.is_empty() {
        let file_label = ext_sub_path
            .as_deref()
            .and_then(|p| Path::new(p).file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "External Subtitle".to_string());
        sub_tracks.push(TrackItem {
            id: EXTERNAL_SUB_TRACK_ID,
            track_type: "sub".to_string(),
            title: Some(file_label),
            lang: Some("ja".to_string()),
            codec: Some("external".to_string()),
            selected: true,
            external: true,
        });
        active_sid = Some(EXTERNAL_SUB_TRACK_ID);
        initial_sub_text = find_cue_at(&ext_cues, start_pos, 0.0).unwrap_or_default();
        cues_by_track.insert(EXTERNAL_SUB_TRACK_ID, ext_cues.clone());
    }

    let initial_state = BuiltinPlayerState {
        active: true,
        backend: "html5".to_string(),
        anilist_id: ep.anilist_id,
        ep_key: ep.ep_key.clone(),
        path: ep.path.clone(),
        stream_url: if ep.path.starts_with("http://") || ep.path.starts_with("https://") {
            ep.path.clone()
        } else {
            String::new()
        },
        transcode_base_url: None,
        show_title,
        episode_title,
        paused: false,
        position: start_pos,
        duration: 0.0,
        volume: 100.0,
        muted: false,
        speed: 1.0,
        aid: None,
        sid: active_sid,
        sub_text: initial_sub_text,
        sub_offset_sec: 0.0,
        audio_tracks: Vec::new(),
        sub_tracks,
        chapters: Vec::new(),
        op_start: None,
        op_end: None,
        ed_start: None,
        ed_end: None,
        has_next_episode: has_next,
        external_sub_path: ext_sub_path,
    };

    {
        let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        g.state = initial_state.clone();
        g.cues = ext_cues;
        g.cues_by_track = cues_by_track;
        g.threshold = threshold;
        g.autoplay = autoplay;
        g.marked_watched = false;
        g.aniskip_queried = false;
        g.tick_count = 0;
        g.ext_auto_offset_sec = None;
        g.ext_matched_cues = 0;
        g.user_adjusted_offset = false;
    }

    emit_state(&app, &initial_state);
    player::sync_discord_activity(&st, &ep, start_pos, 0.0, true);

    let has_mpv = player::detect(Kind::Mpv).is_some();
    let app_srv = app.clone();
    let st_srv = st.clone();
    let ep_srv = ep.clone();

    tauri::async_runtime::spawn(async move {
        if let Ok((stream_url, transcode_base)) =
            spawn_local_media_server(ep_srv.path.clone(), session_id, has_mpv).await
        {
            if SESSION_COUNTER.load(Ordering::SeqCst) == session_id {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                if g.state.active {
                    g.state.stream_url = stream_url;
                    g.state.transcode_base_url = transcode_base;
                    emit_state(&app_srv, &g.state);
                }
            }
        }

        // Also query AniSkip in parallel
        fetch_and_apply_aniskip(&app_srv, &st_srv, &ep_srv, 0.0, session_id).await;
    });

    // Extract embedded Matroska (.mkv) subtitle tracks, audio tracks, chapters, and duration in background
    let app_mkv = app.clone();
    let st_mkv = st.clone();
    let ep_mkv = ep.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let is_remote_stream = ep_mkv.path.starts_with("http://") || ep_mkv.path.starts_with("https://");
        let local_mkv_path: PathBuf = if is_remote_stream {
            let Some(p) = crate::torrent::save_path_for_stream_url(&ep_mkv.path) else {
                return;
            };
            p
        } else {
            PathBuf::from(&ep_mkv.path)
        };

        let mut aniskip_sent = false;
        let mut scan_iter = 0u32;
        loop {
            if SESSION_COUNTER.load(Ordering::SeqCst) != session_id {
                return;
            }
            if let Ok(embedded) = subtitles::extract_mkv_embedded_data(&local_mkv_path) {
                if SESSION_COUNTER.load(Ordering::SeqCst) != session_id {
                    return;
                }
                let dur_for_aniskip;
                {
                    let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                    if !g.state.active || SESSION_COUNTER.load(Ordering::SeqCst) != session_id {
                        return;
                    }

                    if let Some(dur) = embedded.duration_sec {
                        if g.state.duration <= 0.0 {
                            g.state.duration = dur;
                        }
                    }

                    // Live auto-synchronization of external Japanese subtitles (e.g. from Jimaku) against
                    // the streaming/local `.mkv`'s embedded subtitle cues or leading studio/distributor bumper
                    // (such as a 6–12s Viz Media / Crunchyroll / Netflix intro animation before episode start).
                    if !g.user_adjusted_offset {
                        if let Some(ext_track_cues) = g.cues_by_track.get(&EXTERNAL_SUB_TRACK_ID).cloned() {
                            if let Some((auto_off, matched)) =
                                subtitles::compute_live_cue_offset_sec(&ext_track_cues, &embedded)
                            {
                                if g.ext_auto_offset_sec.is_none() || matched >= g.ext_matched_cues {
                                    g.ext_auto_offset_sec = Some(auto_off);
                                    g.ext_matched_cues = matched;
                                    if g.state.sid == Some(EXTERNAL_SUB_TRACK_ID) {
                                        g.state.sub_offset_sec = auto_off;
                                        let pos = g.state.position;
                                        g.state.sub_text =
                                            find_cue_at(&g.cues, pos, auto_off).unwrap_or_default();
                                    }
                                }
                            }
                        }
                    }

                    // Audio tracks (populate once if empty; use 1-based audio ordinal matching `mpv --aid=1,2,...`)
                    if g.state.audio_tracks.is_empty() && !embedded.audio_tracks.is_empty() {
                        let mut a_items = Vec::new();
                        let mut chosen_aid: Option<i64> = None;
                        for (idx, tr) in embedded.audio_tracks.iter().enumerate() {
                            let ordinal = (idx + 1) as i64;
                            if chosen_aid.is_none() && is_japanese_lang(&tr.lang) {
                                chosen_aid = Some(ordinal);
                            }
                        }
                        if chosen_aid.is_none() {
                            chosen_aid = Some(1);
                        }
                        for (idx, tr) in embedded.audio_tracks.into_iter().enumerate() {
                            let ordinal = (idx + 1) as i64;
                            a_items.push(TrackItem {
                                id: ordinal,
                                track_type: "audio".to_string(),
                                title: tr.title,
                                lang: Some(tr.lang),
                                codec: Some(tr.codec),
                                selected: Some(ordinal) == chosen_aid,
                                external: false,
                            });
                        }
                        g.state.aid = chosen_aid;
                        g.state.audio_tracks = a_items;
                    }

                    // Embedded subtitle tracks
                    if !embedded.sub_tracks.is_empty() {
                        let current_sid = g.state.sid;
                        let mut chosen_sid = current_sid;
                        if chosen_sid.is_none() && g.state.sub_tracks.is_empty() {
                            for tr in &embedded.sub_tracks {
                                if is_japanese_lang(&tr.lang)
                                    || tr.title.as_deref().map(is_japanese_lang).unwrap_or(false)
                                {
                                    chosen_sid = Some(tr.track_num);
                                    break;
                                }
                            }
                            if chosen_sid.is_none() {
                                chosen_sid = embedded.sub_tracks.first().map(|t| t.track_num);
                            }
                        }

                        for tr in embedded.sub_tracks {
                            let is_sel = Some(tr.track_num) == chosen_sid;
                            if is_sel {
                                g.state.sid = Some(tr.track_num);
                                let pos = g.state.position;
                                let off = g.state.sub_offset_sec;
                                g.state.sub_text = find_cue_at(&tr.cues, pos, off).unwrap_or_default();
                                g.cues = tr.cues.clone();
                            }
                            g.cues_by_track.insert(tr.track_num, tr.cues);
                            if !g.state.sub_tracks.iter().any(|existing| existing.id == tr.track_num) {
                                g.state.sub_tracks.push(TrackItem {
                                    id: tr.track_num,
                                    track_type: "sub".to_string(),
                                    title: tr.title,
                                    lang: Some(tr.lang),
                                    codec: Some(tr.codec),
                                    selected: is_sel,
                                    external: false,
                                });
                            }
                        }
                    }

                    // Chapters & OP/ED inference
                    if g.state.chapters.is_empty() && !embedded.chapters.is_empty() {
                        let ch_items: Vec<ChapterItem> = embedded
                            .chapters
                            .into_iter()
                            .map(|c| ChapterItem {
                                title: c.title,
                                time: c.time_sec,
                            })
                            .collect();
                        let dur = g.state.duration;
                        let (op_s, op_e, ed_s, ed_e) = infer_op_ed_from_chapters(&ch_items, dur);
                        if g.state.op_start.is_none() && op_s.is_some() {
                            g.state.op_start = op_s;
                            g.state.op_end = op_e;
                        }
                        if g.state.ed_start.is_none() && ed_s.is_some() {
                            g.state.ed_start = ed_s;
                            g.state.ed_end = ed_e;
                        }
                        g.state.chapters = ch_items;
                    }

                    dur_for_aniskip = g.state.duration;
                    emit_state(&app_mkv, &g.state);
                }

                if !aniskip_sent && dur_for_aniskip > 0.0 {
                    aniskip_sent = true;
                    let app_c = app_mkv.clone();
                    let st_c = st_mkv.clone();
                    let ep_c = ep_mkv.clone();
                    tauri::async_runtime::spawn(async move {
                        fetch_and_apply_aniskip(&app_c, &st_c, &ep_c, dur_for_aniskip, session_id).await;
                    });
                }
            }

            if !is_remote_stream {
                break;
            }
            scan_iter += 1;
            let sleep_dur = if scan_iter <= 10 {
                Duration::from_millis(750)
            } else {
                Duration::from_secs(3)
            };
            std::thread::sleep(sleep_dur);
        }
    });

    Ok(())
}

async fn fetch_and_apply_aniskip(
    app: &AppHandle,
    st: &AppState,
    ep: &Ep,
    duration: f64,
    session_id: u64,
) {
    let mal_id: Option<i64> = st
        .db()
        .query_row(
            "SELECT id_mal FROM media WHERE anilist_id = ?1",
            [ep.anilist_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let ep_num: Option<i64> = ep.ep_key.parse().ok();
    let Some(mid) = mal_id else { return };
    let Some(enum_val) = ep_num else { return };
    let Ok(client) = reqwest::Client::builder().no_proxy().build() else { return };

    if let Some(AniSkipTimes {
        op_start,
        op_end,
        ed_start,
        ed_end,
    }) = player::fetch_aniskip(&client, mid, enum_val, duration).await
    {
        if SESSION_COUNTER.load(Ordering::SeqCst) != session_id {
            return;
        }
        let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        if g.state.active {
            if op_start.is_some() {
                g.state.op_start = op_start;
                g.state.op_end = op_end;
            }
            if ed_start.is_some() {
                g.state.ed_start = ed_start;
                g.state.ed_end = ed_end;
            }
            emit_state(app, &g.state);
        }
    }
}

fn save_ep_progress(conn: &rusqlite::Connection, ep: &Ep, pos: f64, dur: f64) {
    if ep.ep_key.starts_with("extra:") {
        return;
    }
    let _ = conn.execute(
        "INSERT INTO watch_progress (anilist_id, ep_key, position, duration, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(anilist_id, ep_key) DO UPDATE SET position = excluded.position, duration = excluded.duration,
           updated_at = excluded.updated_at",
        params![ep.anilist_id, ep.ep_key, pos, dur, db::now()],
    );
}

fn save_watched(conn: &rusqlite::Connection, ep: &Ep) {
    if ep.ep_key.starts_with("extra:") {
        return;
    }
    let _ = conn.execute(
        "INSERT OR IGNORE INTO watch_state (anilist_id, ep_key, watched_at) VALUES (?1, ?2, ?3)",
        params![ep.anilist_id, ep.ep_key, db::now()],
    );
    let _ = conn.execute(
        "DELETE FROM watch_progress WHERE anilist_id = ?1 AND ep_key = ?2",
        params![ep.anilist_id, ep.ep_key],
    );
}

pub async fn dispatch_command(
    app: AppHandle,
    st: Arc<AppState>,
    cmd: BuiltinCommand,
) -> Result<BuiltinPlayerState> {
    let cur_ep = {
        let g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        Ep {
            anilist_id: g.state.anilist_id,
            ep_key: g.state.ep_key.clone(),
            path: g.state.path.clone(),
        }
    };

    match cmd {
        BuiltinCommand::SyncBounds => {}
        BuiltinCommand::Stop => {
            SESSION_COUNTER.fetch_add(1, Ordering::SeqCst);
            let (pos, dur) = {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                let pos = g.state.position;
                let dur = g.state.duration;
                g.state.active = false;
                emit_state(&app, &g.state);
                (pos, dur)
            };
            if pos > 15.0 && dur > 0.0 {
                save_ep_progress(&st.db(), &cur_ep, pos, dur);
            }
            *st.active_mpv_ipc.lock().unwrap_or_else(|e| e.into_inner()) = None;
            st.discord.clear();
            crate::torrent::on_player_stopped(cur_ep.anilist_id, &cur_ep.ep_key);
            let _ = app.emit("library-changed", ());
        }
        BuiltinCommand::OpenExternalMpv => {
            SESSION_COUNTER.fetch_add(1, Ordering::SeqCst);
            let (pos, dur, ext_sub) = {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                let pos = g.state.position;
                let dur = g.state.duration;
                let ext_sub = g.state.external_sub_path.clone();
                g.state.active = false;
                emit_state(&app, &g.state);
                (pos, dur, ext_sub)
            };
            if pos > 15.0 && dur > 0.0 {
                save_ep_progress(&st.db(), &cur_ep, pos, dur);
            }
            if let Some(mpv_exe) = player::detect(Kind::Mpv) {
                let mut c = std::process::Command::new(mpv_exe);
                c.arg("--force-window=yes");
                if pos > 3.0 {
                    c.arg(format!("--start=+{pos:.0}"));
                }
                if let Some(sub) = ext_sub {
                    c.arg(format!("--sub-file={sub}"));
                }
                c.arg("--").arg(&cur_ep.path);
                let _ = c.spawn();
            }
        }
        BuiltinCommand::NextEpisode => {
            let next = player::next_episode(&st.db(), &cur_ep);
            if let Some(next_ep) = next {
                start(app.clone(), st.clone(), next_ep)?;
            }
        }
        BuiltinCommand::PlayPause => {
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.paused = !g.state.paused;
            emit_state(&app, &g.state);
        }
        BuiltinCommand::Pause => {
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.paused = true;
            emit_state(&app, &g.state);
        }
        BuiltinCommand::Resume => {
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.paused = false;
            emit_state(&app, &g.state);
        }
        BuiltinCommand::Seek { position } => {
            let target = position.max(0.0);
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.position = target;
            let offset = g.state.sub_offset_sec;
            g.state.sub_text = find_cue_at(&g.cues, target, offset).unwrap_or_default();
            emit_state(&app, &g.state);
        }
        BuiltinCommand::SeekRelative { delta } => {
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            let target = (g.state.position + delta).max(0.0);
            g.state.position = target;
            let offset = g.state.sub_offset_sec;
            g.state.sub_text = find_cue_at(&g.cues, target, offset).unwrap_or_default();
            emit_state(&app, &g.state);
        }
        BuiltinCommand::SetVolume { volume } => {
            let v = volume.clamp(0.0, 130.0);
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.volume = v;
            if v > 0.0 {
                g.state.muted = false;
            }
            emit_state(&app, &g.state);
        }
        BuiltinCommand::ToggleMute => {
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.muted = !g.state.muted;
            emit_state(&app, &g.state);
        }
        BuiltinCommand::SetSpeed { speed } => {
            let sp = speed.clamp(0.25, 3.0);
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.speed = sp;
            emit_state(&app, &g.state);
        }
        BuiltinCommand::SetAudioTrack { id } => {
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.aid = Some(id);
            for tr in &mut g.state.audio_tracks {
                tr.selected = tr.id == id;
            }
            emit_state(&app, &g.state);
        }
        BuiltinCommand::SetSubTrack { id } => {
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.sid = if id <= 0 { None } else { Some(id) };
            for tr in &mut g.state.sub_tracks {
                tr.selected = tr.id == id;
            }
            if !g.user_adjusted_offset {
                if id == EXTERNAL_SUB_TRACK_ID {
                    g.state.sub_offset_sec = g.ext_auto_offset_sec.unwrap_or(0.0);
                } else {
                    g.state.sub_offset_sec = 0.0;
                }
            }
            if id <= 0 {
                g.state.sub_text.clear();
                g.cues.clear();
            } else if let Some(track_cues) = g.cues_by_track.get(&id).cloned() {
                let pos = g.state.position;
                let off = g.state.sub_offset_sec;
                g.state.sub_text = find_cue_at(&track_cues, pos, off).unwrap_or_default();
                g.cues = track_cues;
            }
            emit_state(&app, &g.state);
        }
        BuiltinCommand::AddSubOffset { delta } => {
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.user_adjusted_offset = true;
            g.state.sub_offset_sec = ((g.state.sub_offset_sec + delta) * 100.0).round() / 100.0;
            let pos = g.state.position;
            let off = g.state.sub_offset_sec;
            g.state.sub_text = find_cue_at(&g.cues, pos, off).unwrap_or_default();
            emit_state(&app, &g.state);
        }
        BuiltinCommand::LoadSubFile { path } => {
            let cues = subtitles::parse_subtitle_file_cues(Path::new(&path));
            let file_label = Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "External Subtitle".to_string());
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.external_sub_path = Some(path);
            for tr in &mut g.state.sub_tracks {
                tr.selected = false;
            }
            if let Some(ext) = g.state.sub_tracks.iter_mut().find(|t| t.id == EXTERNAL_SUB_TRACK_ID) {
                ext.title = Some(file_label);
                ext.selected = true;
            } else {
                g.state.sub_tracks.insert(
                    0,
                    TrackItem {
                        id: EXTERNAL_SUB_TRACK_ID,
                        track_type: "sub".to_string(),
                        title: Some(file_label),
                        lang: Some("ja".to_string()),
                        codec: Some("external".to_string()),
                        selected: true,
                        external: true,
                    },
                );
            }
            g.state.sid = Some(EXTERNAL_SUB_TRACK_ID);
            g.user_adjusted_offset = false;
            g.ext_matched_cues = 0;
            if let Some((auto_off, matched)) =
                subtitles::compute_live_cue_offset_from_tracks(&cues, &g.cues_by_track, EXTERNAL_SUB_TRACK_ID)
            {
                g.ext_auto_offset_sec = Some(auto_off);
                g.ext_matched_cues = matched;
                g.state.sub_offset_sec = auto_off;
            }
            let pos = g.state.position;
            let off = g.state.sub_offset_sec;
            g.state.sub_text = find_cue_at(&cues, pos, off).unwrap_or_default();
            g.cues_by_track.insert(EXTERNAL_SUB_TRACK_ID, cues.clone());
            g.cues = cues;
            emit_state(&app, &g.state);
        }
        BuiltinCommand::Html5Tick {
            position,
            duration,
            paused,
            ended,
        } => {
            let (eff_dur, threshold, autoplay, should_mark_watched, should_save_prog, should_query_aniskip, sid_opt) = {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                if !g.state.active {
                    return Ok(g.state.clone());
                }
                g.state.position = position;
                if duration > 0.0 && duration.is_finite() {
                    g.state.duration = duration;
                }
                let eff_dur = g.state.duration;
                g.state.paused = paused;
                let off = g.state.sub_offset_sec;
                let sid_opt = g.state.sid;
                if sid_opt.is_some() {
                    g.state.sub_text = find_cue_at(&g.cues, position, off).unwrap_or_default();
                } else {
                    g.state.sub_text.clear();
                }

                g.tick_count += 1;
                let threshold = g.threshold;
                let autoplay = g.autoplay;
                let should_mark_watched = !g.marked_watched && player::reached(position, eff_dur, threshold);
                if should_mark_watched {
                    g.marked_watched = true;
                }
                let should_save_prog = !g.marked_watched && position > 15.0 && eff_dur > 0.0 && g.tick_count % 12 == 0;
                let should_query_aniskip = !g.aniskip_queried && eff_dur > 0.0;
                if should_query_aniskip {
                    g.aniskip_queried = true;
                }

                emit_state(&app, &g.state);
                (
                    eff_dur,
                    threshold,
                    autoplay,
                    should_mark_watched,
                    should_save_prog,
                    should_query_aniskip,
                    sid_opt,
                )
            };
            let _ = (threshold, sid_opt);

            if should_query_aniskip {
                let app_c = app.clone();
                let st_c = st.clone();
                let ep_c = cur_ep.clone();
                let sess_id = SESSION_COUNTER.load(Ordering::SeqCst);
                tauri::async_runtime::spawn(async move {
                    fetch_and_apply_aniskip(&app_c, &st_c, &ep_c, eff_dur, sess_id).await;
                });
            }

            if should_mark_watched {
                save_watched(&st.db(), &cur_ep);
                crate::anilist_sync::on_episode_watched(&app, &st, cur_ep.anilist_id);
                let _ = app.emit("library-changed", ());
            } else if should_save_prog {
                save_ep_progress(&st.db(), &cur_ep, position, eff_dur);
            }

            if ended && autoplay {
                let next = player::next_episode(&st.db(), &cur_ep);
                if let Some(next_ep) = next {
                    let _ = start(app.clone(), st.clone(), next_ep);
                }
            }
        }
    }

    Ok(get_state())
}
