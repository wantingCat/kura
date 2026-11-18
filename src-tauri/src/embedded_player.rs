//! Built-in Kura video player engine powered by native `libmpv`.
//!
//! Renders video directly via `libmpv` (`libmpv-2.dll` / `libmpv.so.2` / `libmpv.2.dylib`)
//! onto a hardware-accelerated surface synchronized behind Kura's transparent WebView2 HUD,
//! with zero-transcode multi-audio/subtitle track switching, Skip OP/ED, and pause-gated
//! interactive Japanese subtitles with Yomitan dictionary lookup.

use crate::db;
use crate::klog;
use crate::libmpv::{
    self, MpvEventEndFile, MpvEventProperty, MpvInstance, MPV_END_FILE_REASON_EOF,
    MPV_EVENT_END_FILE, MPV_EVENT_FILE_LOADED, MPV_EVENT_NONE, MPV_EVENT_PROPERTY_CHANGE,
    MPV_EVENT_SHUTDOWN, MPV_FORMAT_DOUBLE, MPV_FORMAT_FLAG, MPV_FORMAT_INT64, MPV_FORMAT_STRING,
};
use crate::player::{self, AniSkipTimes, Ep, Kind};
use crate::service::AppState;
use crate::subtitles::{self, SubtitleCue};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::CStr;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use tauri::{AppHandle, Emitter};

const EXTERNAL_SUB_TRACK_ID: i64 = 1000;

#[inline]
fn is_overlay_cue_track(sid: Option<i64>) -> bool {
    matches!(sid, Some(id) if id >= EXTERNAL_SUB_TRACK_ID)
}

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
    pub backend: String, // "libmpv"
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
    pub today_immersion_sec: i64,
    pub shader_preset: String,
    pub stream_kind: Option<String>,
}

impl Default for BuiltinPlayerState {
    fn default() -> Self {
        Self {
            active: false,
            backend: "libmpv".to_string(),
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
            today_immersion_sec: 0,
            shader_preset: "off".to_string(),
            stream_kind: None,
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
    SetShaderPreset { preset: String },
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebStreamSubtitleTrack {
    pub url: String,
    pub language: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartWebStreamReq {
    pub anilist_id: i64,
    pub ep_key: String,
    pub url: String,
    #[serde(default)]
    pub stream_type: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub subtitles: Vec<WebStreamSubtitleTrack>,
    #[serde(default)]
    pub provider_name: Option<String>,
    #[serde(default)]
    pub server_name: Option<String>,
}

struct RuntimeSession {
    state: BuiltinPlayerState,
    mpv: Option<MpvInstance>,
    cues: Vec<SubtitleCue>,
    cues_by_track: HashMap<i64, Vec<SubtitleCue>>,
    threshold: f64,
    autoplay: bool,
    marked_watched: bool,
    next_preloaded: bool,
    aniskip_queried: bool,
    tick_count: u64,
    pending_start_pos: f64,
    user_adjusted_offset: bool,
    auto_aligned_offset: bool,
    user_selected_sub_track: bool,
    track_prefs: player::EffectiveTrackPrefs,
    last_unpaused_tick: Option<Instant>,
    unflushed_immersion_sec: f64,
    last_emit_at: Option<Instant>,
}

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);
static RUNTIME: OnceLock<Mutex<RuntimeSession>> = OnceLock::new();
static MPV_EVENT_LOOP_STARTED: OnceLock<()> = OnceLock::new();
static MPV_INIT_LOCK: Mutex<()> = Mutex::new(());

fn runtime() -> &'static Mutex<RuntimeSession> {
    RUNTIME.get_or_init(|| {
        Mutex::new(RuntimeSession {
            state: BuiltinPlayerState::default(),
            mpv: None,
            cues: Vec::new(),
            cues_by_track: HashMap::new(),
            threshold: 0.90,
            autoplay: true,
            marked_watched: false,
            next_preloaded: false,
            aniskip_queried: false,
            tick_count: 0,
            pending_start_pos: 0.0,
            user_adjusted_offset: false,
            auto_aligned_offset: false,
            user_selected_sub_track: false,
            track_prefs: player::EffectiveTrackPrefs::default(),
            last_unpaused_tick: None,
            unflushed_immersion_sec: 0.0,
            last_emit_at: None,
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

pub fn sync_bounds(app: &AppHandle) {
    let active = runtime()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .state
        .active;
    libmpv::sync_surface_bounds(app, active);
}

fn ensure_mpv(app: &AppHandle, st: &Arc<AppState>) -> Result<MpvInstance> {
    {
        let g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mpv) = g.mpv {
            return Ok(mpv);
        }
    }

    // Serialize cold libmpv + Win32 surface init so background warm-up and an immediate
    // Play click coalesce into one initialization instead of racing.
    let _init_guard = MPV_INIT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    {
        let g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mpv) = g.mpv {
            return Ok(mpv);
        }
    }

    let wid = libmpv::ensure_video_surface()?;
    let mpv = MpvInstance::new(wid)?;
    {
        let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        g.mpv = Some(mpv);
    }

    let app_ev = app.clone();
    let st_ev = st.clone();
    MPV_EVENT_LOOP_STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("kura-libmpv-events".to_string())
            .spawn(move || run_mpv_event_loop(app_ev, st_ev, mpv))
            .ok();
    });

    Ok(mpv)
}

/// Pre-warm `libmpv` and the hidden Win32 video surface in the background after app startup
/// so the first episode Play opens immediately with zero DLL/GPU driver cold-start latency.
pub fn warm_up(app: AppHandle, st: Arc<AppState>) {
    let use_builtin = player::prefs(&st.db()).kind == Kind::Builtin;
    if !use_builtin {
        return;
    }
    if let Err(e) = ensure_mpv(&app, &st) {
        klog!("[libmpv] background warm-up skipped: {e}");
    }
}

fn clean_mpv_sub_text(raw: &str) -> String {
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Ignore raw ASS vector drawing coordinate strings (`m 0 0 l 100 100 ...`)
        let is_vector_draw = trimmed.starts_with("m ")
            && trimmed
                .chars()
                .all(|c| c.is_ascii_digit() || matches!(c, 'm' | 'l' | 'b' | 's' | 'p' | 'c' | 'n' | ' ' | '-' | '.'));
        if is_vector_draw {
            continue;
        }
        lines.push(trimmed);
    }
    lines.join("\n")
}

fn is_bitmap_sub_codec(codec: Option<&str>) -> bool {
    let c = codec.unwrap_or("").to_ascii_lowercase();
    c.contains("pgs") || c.contains("vobsub") || c.contains("dvd_subtitle") || c.contains("dvb_subtitle")
}

fn apply_sub_track_to_mpv(mpv: &MpvInstance, sub_tracks: &[TrackItem], chosen_sid: Option<i64>) {
    match chosen_sid {
        None => {
            let _ = mpv.set_property_string("sid", "no");
            let _ = mpv.set_property_string("sub-visibility", "no");
        }
        Some(id) if id >= EXTERNAL_SUB_TRACK_ID => {
            // External / companion cues are rendered directly by Kura's interactive subtitle overlay
            let _ = mpv.set_property_string("sid", "no");
            let _ = mpv.set_property_string("sub-visibility", "no");
        }
        Some(id) => {
            let is_bitmap = sub_tracks
                .iter()
                .find(|t| t.id == id)
                .and_then(|t| t.codec.as_deref())
                .map(|c| is_bitmap_sub_codec(Some(c)))
                .unwrap_or(false);
            let _ = mpv.set_property_i64("sid", id);
            if is_bitmap {
                let _ = mpv.set_property_string("sub-visibility", "yes");
            } else {
                // Hide libass on-video OSD so Kura's interactive Yomitan DOM overlay renders cleanly
                let _ = mpv.set_property_string("sub-visibility", "no");
            }
        }
    }
}

fn refresh_tracks_and_chapters_from_mpv(mpv: &MpvInstance, g: &mut RuntimeSession) {
    if let Some(dur) = mpv.get_property_double("duration") {
        if dur > 0.0 && dur.is_finite() {
            g.state.duration = dur;
        }
    }

    let track_count = mpv.get_property_i64("track-list/count").unwrap_or(0);
    let mut audio_tracks = Vec::new();
    let mut sub_tracks: Vec<TrackItem> = g
        .state
        .sub_tracks
        .iter()
        .filter(|t| t.id >= EXTERNAL_SUB_TRACK_ID)
        .cloned()
        .collect();

    for i in 0..track_count {
        let t_type = mpv
            .get_property_string(&format!("track-list/{i}/type"))
            .unwrap_or_default();
        let Some(id) = mpv.get_property_i64(&format!("track-list/{i}/id")) else {
            continue;
        };
        let title = mpv.get_property_string(&format!("track-list/{i}/title"));
        let lang = mpv.get_property_string(&format!("track-list/{i}/lang"));
        let codec = mpv.get_property_string(&format!("track-list/{i}/codec"));
        let selected = mpv
            .get_property_bool(&format!("track-list/{i}/selected"))
            .unwrap_or(false);
        let external = mpv
            .get_property_bool(&format!("track-list/{i}/external"))
            .unwrap_or(false);

        if t_type == "audio" {
            audio_tracks.push(TrackItem {
                id,
                track_type: "audio".to_string(),
                title,
                lang,
                codec,
                selected,
                external,
            });
        } else if t_type == "sub" {
            sub_tracks.push(TrackItem {
                id,
                track_type: "sub".to_string(),
                title,
                lang,
                codec,
                selected: false,
                external,
            });
        }
    }

    // Choose preferred audio track
    if !audio_tracks.is_empty() {
        let mut chosen_aid: Option<i64> = None;
        if let Some(audio_pref) = g.track_prefs.audio.as_deref() {
            for tr in &audio_tracks {
                if player::lang_matches_code(
                    tr.lang.as_deref().unwrap_or(""),
                    tr.title.as_deref(),
                    audio_pref,
                ) {
                    chosen_aid = Some(tr.id);
                    break;
                }
            }
        }
        if chosen_aid.is_none() {
            for tr in &audio_tracks {
                if is_japanese_lang(tr.lang.as_deref().unwrap_or("")) {
                    chosen_aid = Some(tr.id);
                    break;
                }
            }
        }
        if chosen_aid.is_none() {
            chosen_aid = audio_tracks.first().map(|t| t.id);
        }
        if let Some(aid) = chosen_aid {
            let _ = mpv.set_property_i64("aid", aid);
        }
        for tr in &mut audio_tracks {
            tr.selected = Some(tr.id) == chosen_aid;
        }
        g.state.aid = chosen_aid;
        g.state.audio_tracks = audio_tracks;
    }

    // Choose preferred subtitle track
    if !g.user_selected_sub_track {
        let chosen_sid = pick_best_sub_track_id(&sub_tracks, &g.track_prefs);
        for tr in &mut sub_tracks {
            tr.selected = Some(tr.id) == chosen_sid;
        }
        g.state.sid = chosen_sid;
        g.state.sub_tracks = sub_tracks;
        apply_sub_track_to_mpv(mpv, &g.state.sub_tracks, chosen_sid);

        if is_overlay_cue_track(chosen_sid) {
            if let Some(id) = chosen_sid {
                if let Some(ext_cues) = g.cues_by_track.get(&id).cloned() {
                    g.state.sub_text =
                        find_cue_at(&ext_cues, g.state.position, g.state.sub_offset_sec)
                            .unwrap_or_default();
                    g.cues = ext_cues;
                }
            }
        } else if chosen_sid.is_some() {
            let raw = mpv.get_property_string("sub-text").unwrap_or_default();
            g.state.sub_text = clean_mpv_sub_text(&raw);
        } else {
            g.state.sub_text.clear();
        }
    } else {
        let active_sid = g.state.sid;
        for tr in &mut sub_tracks {
            tr.selected = Some(tr.id) == active_sid;
        }
        g.state.sub_tracks = sub_tracks;
    }

    // Read chapters & infer OP/ED
    let ch_count = mpv.get_property_i64("chapter-list/count").unwrap_or(0);
    if ch_count > 0 {
        let mut ch_items = Vec::new();
        for i in 0..ch_count {
            let title = mpv
                .get_property_string(&format!("chapter-list/{i}/title"))
                .unwrap_or_else(|| format!("Chapter {}", i + 1));
            if let Some(time) = mpv.get_property_double(&format!("chapter-list/{i}/time")) {
                ch_items.push(ChapterItem { title, time });
            }
        }
        if !ch_items.is_empty() {
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
    }
}

fn run_mpv_event_loop(app: AppHandle, st: Arc<AppState>, mpv: MpvInstance) {
    let mut last_surface_sync: Option<Instant> = None;
    loop {
        let ev_ptr = mpv.wait_event(0.20);

        // Periodically keep the video surface aligned with the main window.
        // Read `active` and RELEASE the runtime lock before syncing (never hold it across
        // anything that touches the UI thread), and throttle to ~4x/sec instead of every event.
        if last_surface_sync
            .map(|t| t.elapsed().as_millis() >= 250)
            .unwrap_or(true)
        {
            last_surface_sync = Some(Instant::now());
            let active = runtime().lock().unwrap_or_else(|e| e.into_inner()).state.active;
            libmpv::sync_surface_bounds(&app, active);
        }

        if ev_ptr.is_null() {
            continue;
        }
        let ev = unsafe { &*ev_ptr };
        let event_id = ev.event_id;

        if event_id == MPV_EVENT_NONE {
            continue;
        }
        if event_id == MPV_EVENT_SHUTDOWN {
            klog!("[libmpv] Event loop received MPV_EVENT_SHUTDOWN");
            break;
        }

        if event_id == MPV_EVENT_FILE_LOADED {
            let (dur_for_aniskip, cur_ep, sess_id) = {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                if !g.state.active {
                    continue;
                }
                if g.pending_start_pos > 1.0 {
                    let pos = g.pending_start_pos;
                    g.pending_start_pos = 0.0;
                    let _ = mpv.set_property_double("time-pos", pos);
                    g.state.position = pos;
                }
                refresh_tracks_and_chapters_from_mpv(&mpv, &mut g);
                emit_state(&app, &g.state);
                (
                    g.state.duration,
                    Ep {
                        anilist_id: g.state.anilist_id,
                        ep_key: g.state.ep_key.clone(),
                        path: g.state.path.clone(),
                    },
                    SESSION_COUNTER.load(Ordering::SeqCst),
                )
            };

            if dur_for_aniskip > 0.0 {
                let app_c = app.clone();
                let st_c = st.clone();
                tauri::async_runtime::spawn(async move {
                    fetch_and_apply_aniskip(&app_c, &st_c, &cur_ep, dur_for_aniskip, sess_id).await;
                });
            }
            continue;
        }

        if event_id == MPV_EVENT_END_FILE && !ev.data.is_null() {
            let end_ev = unsafe { &*(ev.data as *const MpvEventEndFile) };
            if end_ev.reason == MPV_END_FILE_REASON_EOF {
                let (active, autoplay, has_next_ep, stream_kind, cur_ep) = {
                    let g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                    (
                        g.state.active,
                        g.autoplay,
                        g.state.has_next_episode,
                        g.state.stream_kind.clone(),
                        Ep {
                            anilist_id: g.state.anilist_id,
                            ep_key: g.state.ep_key.clone(),
                            path: g.state.path.clone(),
                        },
                    )
                };
                if active && autoplay {
                    let next = player::next_episode(&st.db(), &cur_ep);
                    if let Some(next_ep) = next {
                        let _ = start(app.clone(), st.clone(), next_ep);
                    } else if has_next_ep {
                        if let Ok(num) = cur_ep.ep_key.parse::<i64>() {
                            let _ = app.emit(
                                "play-next-stream-episode",
                                serde_json::json!({
                                    "anilistId": cur_ep.anilist_id,
                                    "nextEpisode": num + 1,
                                    "streamKind": stream_kind.unwrap_or_else(|| "web".to_string()),
                                }),
                            );
                        }
                    }
                }
            }
            continue;
        }

        if event_id == MPV_EVENT_PROPERTY_CHANGE && !ev.data.is_null() {
            let prop = unsafe { &*(ev.data as *const MpvEventProperty) };
            if prop.name.is_null() {
                continue;
            }
            let prop_name = unsafe { CStr::from_ptr(prop.name).to_string_lossy() };

            match prop_name.as_ref() {
                "time-pos" if prop.format == MPV_FORMAT_DOUBLE && !prop.data.is_null() => {
                    let pos = unsafe { *(prop.data as *const f64) };
                    if !pos.is_finite() || pos < 0.0 {
                        continue;
                    }
                    handle_mpv_time_tick(&app, &st, pos);
                }
                "duration" if prop.format == MPV_FORMAT_DOUBLE && !prop.data.is_null() => {
                    let dur = unsafe { *(prop.data as *const f64) };
                    if dur > 0.0 && dur.is_finite() {
                        let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                        if g.state.active {
                            g.state.duration = dur;
                            emit_state(&app, &g.state);
                        }
                    }
                }
                "pause" if prop.format == MPV_FORMAT_FLAG && !prop.data.is_null() => {
                    let paused = unsafe { *(prop.data as *const i32) != 0 };
                    let (flush_sec, cur_ep, show_title) = {
                        let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                        if !g.state.active {
                            continue;
                        }
                        g.state.paused = paused;
                        if paused {
                            g.last_unpaused_tick = None;
                        }
                        let flush_sec = if paused && g.unflushed_immersion_sec >= 1.0 {
                            let sec = g.unflushed_immersion_sec.floor() as i64;
                            g.unflushed_immersion_sec -= sec as f64;
                            g.state.today_immersion_sec += sec;
                            sec
                        } else {
                            0
                        };
                        emit_state(&app, &g.state);
                        (
                            flush_sec,
                            Ep {
                                anilist_id: g.state.anilist_id,
                                ep_key: g.state.ep_key.clone(),
                                path: g.state.path.clone(),
                            },
                            g.state.show_title.clone(),
                        )
                    };
                    if flush_sec > 0 {
                        let _ = db::record_immersion_time(
                            &st.db(),
                            Some(cur_ep.anilist_id),
                            Some(&cur_ep.ep_key),
                            Some(&show_title),
                            flush_sec,
                        );
                    }
                }
                "sub-text" if prop.format == MPV_FORMAT_STRING => {
                    let raw = if prop.data.is_null() {
                        String::new()
                    } else {
                        unsafe {
                            let cstr_ptr = *(prop.data as *const *const std::ffi::c_char);
                            if cstr_ptr.is_null() {
                                String::new()
                            } else {
                                CStr::from_ptr(cstr_ptr).to_string_lossy().into_owned()
                            }
                        }
                    };
                    let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                    if g.state.active
                        && g.state.sid.is_some()
                        && !is_overlay_cue_track(g.state.sid)
                    {
                        let cleaned = clean_mpv_sub_text(&raw);
                        if g.state.sub_text != cleaned {
                            g.state.sub_text = cleaned;
                            emit_state(&app, &g.state);
                        }
                    }
                }
                "aid" if prop.format == MPV_FORMAT_INT64 && !prop.data.is_null() => {
                    let aid = unsafe { *(prop.data as *const i64) };
                    let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                    if g.state.active {
                        g.state.aid = Some(aid);
                        for tr in &mut g.state.audio_tracks {
                            tr.selected = tr.id == aid;
                        }
                        emit_state(&app, &g.state);
                    }
                }
                _ => {}
            }
        }
    }
}

fn resolve_mkv_disk_path(anilist_id: i64, ep_key: &str, play_path: &str) -> Option<std::path::PathBuf> {
    let p = Path::new(play_path);
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    crate::torrent::active_save_path_for_episode(anilist_id, ep_key).filter(|f| f.is_file())
}

fn handle_mpv_time_tick(app: &AppHandle, st: &Arc<AppState>, position: f64) {
    let (
        eff_dur,
        should_mark_watched,
        should_save_prog,
        flush_immersion_sec,
        show_title,
        cur_ep,
        try_live_align,
        preload_stream_kind,
    ) = {
        let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        if !g.state.active {
            return;
        }
        let paused = g.state.paused;
        let now_inst = Instant::now();
        if !paused {
            if let Some(prev) = g.last_unpaused_tick {
                let dt = now_inst.duration_since(prev).as_secs_f64();
                if (0.01..=2.5).contains(&dt) {
                    g.unflushed_immersion_sec += dt;
                }
            }
            g.last_unpaused_tick = Some(now_inst);
        } else {
            g.last_unpaused_tick = None;
        }

        let flush_immersion_sec = if g.unflushed_immersion_sec >= 5.0 {
            let sec = g.unflushed_immersion_sec.floor() as i64;
            g.unflushed_immersion_sec -= sec as f64;
            g.state.today_immersion_sec += sec;
            sec
        } else {
            0
        };
        let show_title = g.state.show_title.clone();

        g.state.position = position;
        let eff_dur = g.state.duration;

        let mut sub_changed = false;
        if is_overlay_cue_track(g.state.sid) {
            let off = g.state.sub_offset_sec;
            let next_sub = find_cue_at(&g.cues, position, off).unwrap_or_default();
            if g.state.sub_text != next_sub {
                g.state.sub_text = next_sub;
                sub_changed = true;
            }
        }

        g.tick_count += 1;
        let try_live_align = !g.user_adjusted_offset
            && !g.auto_aligned_offset
            && g.state.sid == Some(EXTERNAL_SUB_TRACK_ID)
            && g.state.stream_kind.as_deref() != Some("web")
            && !g.cues.is_empty()
            && (g.tick_count == 5 || g.tick_count % 60 == 0)
            && g.tick_count <= 600;
        let threshold = g.threshold;
        let should_mark_watched =
            !g.marked_watched && player::reached(position, eff_dur, threshold);
        if should_mark_watched {
            g.marked_watched = true;
        }
        let should_preload_next = !g.next_preloaded
            && g.autoplay
            && g.state.has_next_episode
            && g.state.stream_kind.is_some()
            && eff_dur > 60.0
            && (position / eff_dur) >= 0.85;
        if should_preload_next {
            g.next_preloaded = true;
        }
        let preload_stream_kind = if should_preload_next {
            g.state.stream_kind.clone()
        } else {
            None
        };
        let should_save_prog =
            !g.marked_watched && position > 15.0 && eff_dur > 0.0 && g.tick_count % 25 == 0;

        let should_emit = sub_changed
            || flush_immersion_sec > 0
            || g.last_emit_at
                .map(|t| now_inst.duration_since(t).as_millis() >= 90)
                .unwrap_or(true);
        if should_emit {
            g.last_emit_at = Some(now_inst);
            emit_state(app, &g.state);
        }

        (
            eff_dur,
            should_mark_watched,
            should_save_prog,
            flush_immersion_sec,
            show_title,
            Ep {
                anilist_id: g.state.anilist_id,
                ep_key: g.state.ep_key.clone(),
                path: g.state.path.clone(),
            },
            if try_live_align {
                Some(g.cues.clone())
            } else {
                None
            },
            preload_stream_kind,
        )
    };

    if let Some(sk) = preload_stream_kind {
        if let Ok(num) = cur_ep.ep_key.parse::<i64>() {
            let _ = app.emit(
                "preload-next-episode",
                serde_json::json!({
                    "anilistId": cur_ep.anilist_id,
                    "epKey": cur_ep.ep_key,
                    "nextEpisode": num + 1,
                    "streamKind": sk,
                }),
            );
        }
    }

    if let Some(cues_snapshot) = try_live_align {
        let ep_clone = cur_ep.clone();
        let app_clone = app.clone();
        let sess_id = SESSION_COUNTER.load(Ordering::SeqCst);
        tauri::async_runtime::spawn_blocking(move || {
            let Some(mkv_path) =
                resolve_mkv_disk_path(ep_clone.anilist_id, &ep_clone.ep_key, &ep_clone.path)
            else {
                return;
            };
            if let Some(off_sec) = subtitles::compute_live_mkv_cue_offset_sec(&cues_snapshot, &mkv_path)
            {
                if SESSION_COUNTER.load(Ordering::SeqCst) != sess_id {
                    return;
                }
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                if g.state.active
                    && !g.user_adjusted_offset
                    && !g.auto_aligned_offset
                    && g.state.anilist_id == ep_clone.anilist_id
                    && g.state.ep_key == ep_clone.ep_key
                {
                    g.auto_aligned_offset = true;
                    g.state.sub_offset_sec = off_sec;
                    let pos = g.state.position;
                    g.state.sub_text = find_cue_at(&g.cues, pos, off_sec).unwrap_or_default();
                    emit_state(&app_clone, &g.state);
                }
            }
        });
    }

    if flush_immersion_sec > 0 {
        let _ = db::record_immersion_time(
            &st.db(),
            Some(cur_ep.anilist_id),
            Some(&cur_ep.ep_key),
            Some(&show_title),
            flush_immersion_sec,
        );
    }

    if should_mark_watched {
        save_watched(&st.db(), &cur_ep);
        crate::anilist_sync::on_episode_watched(app, st, cur_ep.anilist_id);
        let _ = app.emit("library-changed", ());
    } else if should_save_prog {
        save_ep_progress(&st.db(), &cur_ep, position, eff_dur);
    }
}

pub fn reload_cues_if_active(anilist_id: i64, ep_key: &str, sub_path: &str) -> bool {
    let cues = subtitles::parse_subtitle_file_cues(Path::new(sub_path));
    if cues.is_empty() {
        return false;
    }

    // Heavy MKV audio/sub scan must NOT run while holding the runtime lock (it would block
    // the libmpv event loop and every UI call that reads player state for seconds).
    let (is_target, user_adjusted, play_path) = {
        let g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        (
            g.state.active && g.state.anilist_id == anilist_id && g.state.ep_key == ep_key,
            g.user_adjusted_offset,
            g.state.path.clone(),
        )
    };
    if !is_target {
        return false;
    }
    let precomputed_offset = if user_adjusted {
        None
    } else {
        resolve_mkv_disk_path(anilist_id, ep_key, &play_path)
            .and_then(|mkv_path| subtitles::compute_live_mkv_cue_offset_sec(&cues, &mkv_path))
    };

    let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
    if g.state.active && g.state.anilist_id == anilist_id && g.state.ep_key == ep_key {
        g.state.external_sub_path = Some(sub_path.to_string());
        let file_name = Path::new(sub_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "External Subtitle".to_string());
        let ext_lang =
            subtitles::detect_sub_language(&file_name).unwrap_or_else(|| "ja".to_string());
        let should_select = match g.track_prefs.sub.as_deref() {
            Some(pref) => player::lang_matches_code(&ext_lang, Some(&file_name), pref),
            None => true,
        };

        g.cues_by_track.insert(EXTERNAL_SUB_TRACK_ID, cues.clone());

        if should_select {
            for tr in &mut g.state.sub_tracks {
                tr.selected = false;
            }
        }
        if let Some(existing) = g
            .state
            .sub_tracks
            .iter_mut()
            .find(|t| t.id == EXTERNAL_SUB_TRACK_ID)
        {
            existing.title = Some(file_name);
            existing.lang = Some(ext_lang);
            if should_select {
                existing.selected = true;
            }
        } else {
            g.state.sub_tracks.insert(
                0,
                TrackItem {
                    id: EXTERNAL_SUB_TRACK_ID,
                    track_type: "sub".to_string(),
                    title: Some(file_name),
                    lang: Some(ext_lang),
                    codec: Some("external".to_string()),
                    selected: should_select,
                    external: true,
                },
            );
        }
        if should_select {
            g.state.sid = Some(EXTERNAL_SUB_TRACK_ID);
            if let Some(ref mpv) = g.mpv {
                apply_sub_track_to_mpv(mpv, &g.state.sub_tracks, Some(EXTERNAL_SUB_TRACK_ID));
            }
            if !g.user_adjusted_offset {
                g.auto_aligned_offset = false;
                if let Some(off_sec) = precomputed_offset {
                    g.auto_aligned_offset = true;
                    g.state.sub_offset_sec = off_sec;
                }
            }
            let pos = g.state.position;
            let off = g.state.sub_offset_sec;
            g.state.sub_text = find_cue_at(&cues, pos, off).unwrap_or_default();
            g.cues = cues;
        }
        return true;
    }
    false
}

/// Hot-load an already piecewise-retimed Japanese subtitle file into an active web/torrent stream
/// session and emit updated player state immediately.
pub fn reload_stream_cues_if_active(
    app: &AppHandle,
    anilist_id: i64,
    ep_key: &str,
    sub_path: &str,
) -> bool {
    let cues = subtitles::parse_subtitle_file_cues(Path::new(sub_path));
    if cues.is_empty() {
        return false;
    }

    let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
    if !g.state.active || g.state.anilist_id != anilist_id || g.state.ep_key != ep_key {
        return false;
    }

    g.state.external_sub_path = Some(sub_path.to_string());
    let file_name = Path::new(sub_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Jimaku Japanese".to_string());
    let ext_lang = subtitles::detect_sub_language(&file_name).unwrap_or_else(|| "ja".to_string());
    let should_select = !g.user_selected_sub_track
        && match g.track_prefs.sub.as_deref() {
            Some(pref) => player::lang_matches_code(&ext_lang, Some(&file_name), pref),
            None => true,
        };

    g.cues_by_track.insert(EXTERNAL_SUB_TRACK_ID, cues.clone());
    if should_select {
        for tr in &mut g.state.sub_tracks {
            tr.selected = false;
        }
    }
    if let Some(existing) = g
        .state
        .sub_tracks
        .iter_mut()
        .find(|t| t.id == EXTERNAL_SUB_TRACK_ID)
    {
        existing.title = Some(file_name);
        existing.lang = Some(ext_lang);
        if should_select {
            existing.selected = true;
        }
    } else {
        g.state.sub_tracks.insert(
            0,
            TrackItem {
                id: EXTERNAL_SUB_TRACK_ID,
                track_type: "sub".to_string(),
                title: Some(file_name),
                lang: Some(ext_lang),
                codec: Some("external".to_string()),
                selected: should_select,
                external: true,
            },
        );
    }

    if should_select {
        g.state.sid = Some(EXTERNAL_SUB_TRACK_ID);
        if let Some(ref mpv) = g.mpv {
            apply_sub_track_to_mpv(mpv, &g.state.sub_tracks, Some(EXTERNAL_SUB_TRACK_ID));
        }
        if !g.user_adjusted_offset {
            g.auto_aligned_offset = true;
            g.state.sub_offset_sec = 0.0;
        }
        let pos = g.state.position;
        let off = g.state.sub_offset_sec;
        g.state.sub_text = find_cue_at(&cues, pos, off).unwrap_or_default();
        g.cues = cues;
    }

    emit_state(app, &g.state);
    true
}

fn has_next_stream_or_local_episode(conn: &rusqlite::Connection, ep: &Ep) -> bool {
    if player::next_episode(conn, ep).is_some() {
        return true;
    }
    let Ok(num) = ep.ep_key.parse::<i64>() else {
        return false;
    };
    let next_num = num + 1;
    let in_episodes: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM episodes WHERE anilist_id = ?1 AND number = ?2 AND is_special = 0)",
            params![ep.anilist_id, next_num],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if in_episodes {
        return true;
    }
    let media_bounds: Option<(Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT episodes, next_airing_episode FROM media WHERE anilist_id = ?1",
            [ep.anilist_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap_or(None);
    if let Some((total_eps, next_airing)) = media_bounds {
        let aired_max = next_airing
            .map(|n| (n - 1).max(0))
            .or(total_eps)
            .unwrap_or(0);
        return next_num <= aired_max;
    }
    false
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
            .unwrap_or_else(|| {
                if duration > ch.time {
                    duration
                } else {
                    ch.time + 90.0
                }
            });
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

fn pick_sub_track_for_code(tracks: &[TrackItem], code: &str) -> Option<i64> {
    let c = code.trim().to_ascii_lowercase();
    if c.is_empty() || c == "off" || c == "none" {
        return None;
    }
    if c == "any" || c == "default" {
        return tracks
            .iter()
            .find(|t| !t.external && !player::is_signs_or_songs_title(t.title.as_deref()))
            .or_else(|| tracks.iter().find(|t| !t.external))
            .or_else(|| tracks.first())
            .map(|t| t.id);
    }
    let prefer_external = matches!(c.as_str(), "jpn" | "ja" | "japanese" | "jp");
    if let Some(t) = tracks
        .iter()
        .filter(|t| {
            player::lang_matches_code(t.lang.as_deref().unwrap_or(""), t.title.as_deref(), &c)
                && !player::is_signs_or_songs_title(t.title.as_deref())
        })
        .min_by_key(|t| if prefer_external { !t.external } else { t.external })
    {
        return Some(t.id);
    }
    tracks
        .iter()
        .filter(|t| {
            player::lang_matches_code(t.lang.as_deref().unwrap_or(""), t.title.as_deref(), &c)
        })
        .min_by_key(|t| if prefer_external { !t.external } else { t.external })
        .map(|t| t.id)
}

fn pick_best_sub_track_id(
    tracks: &[TrackItem],
    prefs: &player::EffectiveTrackPrefs,
) -> Option<i64> {
    let sub_pref = prefs.sub.as_deref().unwrap_or("jpn").trim();
    if sub_pref.eq_ignore_ascii_case("off") || sub_pref.eq_ignore_ascii_case("none") {
        return None;
    }
    if let Some(id) = pick_sub_track_for_code(tracks, sub_pref) {
        return Some(id);
    }
    if let Some(fb) = prefs.sub_fallback.as_deref() {
        let fb_trim = fb.trim();
        if !fb_trim.is_empty()
            && !fb_trim.eq_ignore_ascii_case("off")
            && !fb_trim.eq_ignore_ascii_case("none")
        {
            if let Some(id) = pick_sub_track_for_code(tracks, fb_trim) {
                return Some(id);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Launching & Controlling the Built-in libmpv Player
// ---------------------------------------------------------------------------

pub fn start(app: AppHandle, st: Arc<AppState>, ep: Ep) -> Result<()> {
    let session_id = SESSION_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    let _play_session = st.play_session.fetch_add(1, Ordering::SeqCst) + 1;

    let mpv = ensure_mpv(&app, &st)?;

    let (
        start_pos,
        threshold,
        autoplay,
        show_title,
        episode_title,
        has_next,
        candidate_sub_paths,
        track_prefs,
        today_immersion_sec,
        prev_volume,
        prev_muted,
        shader_preset,
    ) = {
        let conn = st.db();
        let p = player::prefs(&conn);
        let saved = player::saved_progress(&conn, ep.anilist_id, &ep.ep_key);
        let start_pos = if p.resume {
            player::resume_point(saved, p.threshold)
        } else {
            0.0
        };
        let (show_title, episode_title) = resolve_titles(&conn, ep.anilist_id, &ep.ep_key);
        let has_next = has_next_stream_or_local_episode(&conn, &ep);
        let candidate_sub_paths =
            subtitles::candidate_subtitle_paths_for_episode(&conn, ep.anilist_id, &ep.ep_key);
        let track_prefs = player::effective_track_prefs(&conn, ep.anilist_id);
        let today_sec: i64 = conn
            .query_row(
                "SELECT active_seconds FROM immersion_daily WHERE day_key = ?1",
                [db::today_day_key()],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let preset = db::get_setting(&conn, "pref.anime4k_preset")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "off".to_string());
        let (vol, muted) = {
            let g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            (g.state.volume, g.state.muted)
        };
        (
            start_pos,
            p.threshold,
            p.autoplay,
            show_title,
            episode_title,
            has_next,
            candidate_sub_paths,
            track_prefs,
            today_sec,
            vol,
            muted,
            preset,
        )
    };

    let inferred_stream_kind = if ep.path.starts_with("http://127.0.0.1:") {
        Some("torrent".to_string())
    } else {
        None
    };

    let initial_state = BuiltinPlayerState {
        active: true,
        backend: "libmpv".to_string(),
        anilist_id: ep.anilist_id,
        ep_key: ep.ep_key.clone(),
        path: ep.path.clone(),
        stream_url: String::new(),
        transcode_base_url: None,
        show_title,
        episode_title,
        paused: false,
        position: start_pos,
        duration: 0.0,
        volume: prev_volume,
        muted: prev_muted,
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
        has_next_episode: has_next,
        external_sub_path: None,
        today_immersion_sec,
        shader_preset: shader_preset.clone(),
        stream_kind: inferred_stream_kind,
    };

    {
        let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        g.state = initial_state.clone();
        g.cues = Vec::new();
        g.cues_by_track = HashMap::new();
        g.threshold = threshold;
        g.autoplay = autoplay;
        g.marked_watched = false;
        g.next_preloaded = false;
        g.aniskip_queried = false;
        g.tick_count = 0;
        g.pending_start_pos = start_pos;
        g.user_adjusted_offset = false;
        g.auto_aligned_offset = false;
        g.user_selected_sub_track = false;
        g.track_prefs = track_prefs;
        g.last_unpaused_tick = None;
        g.unflushed_immersion_sec = 0.0;
        g.last_emit_at = None;
    }

    // Open the player HUD & reveal the Win32 hardware surface immediately before any
    // filesystem subtitle parsing or Discord RPC work.
    libmpv::sync_surface_bounds(&app, true);
    emit_state(&app, &initial_state);

    // Load file into native libmpv right away so demux + D3D11 decode start immediately.
    let shader_paths = libmpv::resolve_shader_preset_paths(&shader_preset);
    let _ = mpv.set_property_string("glsl-shaders", &shader_paths);
    let _ = mpv.set_property_double("volume", prev_volume);
    let _ = mpv.set_property_bool("mute", prev_muted);
    let _ = mpv.set_property_double("speed", 1.0);
    let _ = mpv.set_property_double("sub-delay", 0.0);
    let _ = mpv.set_property_bool("pause", false);
    mpv.command(&["loadfile", &ep.path, "replace"])?;

    // Parse external subtitle cues (.ass/.srt) & update Discord Rich Presence in parallel
    // without blocking player open or video decode startup.
    let app_bg = app.clone();
    let st_bg = st.clone();
    let ep_bg = ep.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (ext_sub_path, ext_cues) =
            subtitles::load_cues_from_candidates(candidate_sub_paths, &ep_bg.path);
        if !ext_cues.is_empty() && SESSION_COUNTER.load(Ordering::SeqCst) == session_id {
            let file_label = ext_sub_path
                .as_deref()
                .and_then(|p| Path::new(p).file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "External Subtitle".to_string());
            let ext_lang =
                subtitles::detect_sub_language(&file_label).unwrap_or_else(|| "ja".to_string());

            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            if g.state.active && SESSION_COUNTER.load(Ordering::SeqCst) == session_id {
                g.state.external_sub_path = ext_sub_path;
                g.cues_by_track
                    .insert(EXTERNAL_SUB_TRACK_ID, ext_cues.clone());
                if !g
                    .state
                    .sub_tracks
                    .iter()
                    .any(|t| t.id == EXTERNAL_SUB_TRACK_ID)
                {
                    g.state.sub_tracks.insert(
                        0,
                        TrackItem {
                            id: EXTERNAL_SUB_TRACK_ID,
                            track_type: "sub".to_string(),
                            title: Some(file_label),
                            lang: Some(ext_lang),
                            codec: Some("external".to_string()),
                            selected: false,
                            external: true,
                        },
                    );
                }
                if !g.user_selected_sub_track {
                    let chosen_sid = pick_best_sub_track_id(&g.state.sub_tracks, &g.track_prefs);
                    for tr in &mut g.state.sub_tracks {
                        tr.selected = Some(tr.id) == chosen_sid;
                    }
                    g.state.sid = chosen_sid;
                    if let Some(ref mpv_inst) = g.mpv {
                        apply_sub_track_to_mpv(mpv_inst, &g.state.sub_tracks, chosen_sid);
                    }
                    if chosen_sid == Some(EXTERNAL_SUB_TRACK_ID) {
                        let pos = g.state.position;
                        let off = g.state.sub_offset_sec;
                        g.state.sub_text = find_cue_at(&ext_cues, pos, off).unwrap_or_default();
                        g.cues = ext_cues;
                    }
                }
                emit_state(&app_bg, &g.state);
            }
        }
        player::sync_discord_activity(&st_bg, &ep_bg, start_pos, 0.0, true);
    });

    // Query AniSkip in parallel
    let app_srv = app.clone();
    let st_srv = st.clone();
    let ep_srv = ep.clone();
    tauri::async_runtime::spawn(async move {
        fetch_and_apply_aniskip(&app_srv, &st_srv, &ep_srv, 0.0, session_id).await;
    });

    Ok(())
}

/// Start playing a direct soft-sub web stream (`.m3u8` / `.mp4`) in native `libmpv` via Kura's
/// local header-injecting proxy, load companion `.vtt` tracks, and auto-fetch + piecewise-retime
/// Jimaku Japanese subtitles against the stream's soft-sub cues.
pub fn start_web_stream(app: AppHandle, st: Arc<AppState>, req: StartWebStreamReq) -> Result<()> {
    let session_id = SESSION_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    let _play_session = st.play_session.fetch_add(1, Ordering::SeqCst) + 1;

    let mpv = ensure_mpv(&app, &st)?;
    let proxy_url = crate::onlinestream::build_proxy_url(&req.url, &req.headers)?;
    let ep = Ep {
        anilist_id: req.anilist_id,
        ep_key: req.ep_key.clone(),
        path: proxy_url.clone(),
    };

    let (
        start_pos,
        threshold,
        autoplay,
        show_title,
        episode_title,
        has_next,
        track_prefs,
        today_immersion_sec,
        prev_volume,
        prev_muted,
        shader_preset,
    ) = {
        let conn = st.db();
        let p = player::prefs(&conn);
        let saved = player::saved_progress(&conn, ep.anilist_id, &ep.ep_key);
        let start_pos = if p.resume {
            player::resume_point(saved, p.threshold)
        } else {
            0.0
        };
        let (show_title, episode_title) = resolve_titles(&conn, ep.anilist_id, &ep.ep_key);
        let has_next = has_next_stream_or_local_episode(&conn, &ep);
        let track_prefs = player::effective_track_prefs(&conn, ep.anilist_id);
        let today_sec: i64 = conn
            .query_row(
                "SELECT active_seconds FROM immersion_daily WHERE day_key = ?1",
                [db::today_day_key()],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let preset = db::get_setting(&conn, "pref.anime4k_preset")
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "off".to_string());
        let (vol, muted) = {
            let g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            (g.state.volume, g.state.muted)
        };
        (
            start_pos,
            p.threshold,
            p.autoplay,
            show_title,
            episode_title,
            has_next,
            track_prefs,
            today_sec,
            vol,
            muted,
            preset,
        )
    };

    let initial_state = BuiltinPlayerState {
        active: true,
        backend: "libmpv".to_string(),
        anilist_id: ep.anilist_id,
        ep_key: ep.ep_key.clone(),
        path: proxy_url.clone(),
        stream_url: req.url.clone(),
        transcode_base_url: None,
        show_title,
        episode_title,
        paused: false,
        position: start_pos,
        duration: 0.0,
        volume: prev_volume,
        muted: prev_muted,
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
        has_next_episode: has_next,
        external_sub_path: None,
        today_immersion_sec,
        shader_preset: shader_preset.clone(),
        stream_kind: Some("web".to_string()),
    };

    {
        let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        g.state = initial_state.clone();
        g.cues = Vec::new();
        g.cues_by_track = HashMap::new();
        g.threshold = threshold;
        g.autoplay = autoplay;
        g.marked_watched = false;
        g.next_preloaded = false;
        g.aniskip_queried = false;
        g.tick_count = 0;
        g.pending_start_pos = start_pos;
        g.user_adjusted_offset = false;
        g.auto_aligned_offset = false;
        g.user_selected_sub_track = false;
        g.track_prefs = track_prefs;
        g.last_unpaused_tick = None;
        g.unflushed_immersion_sec = 0.0;
        g.last_emit_at = None;
    }

    libmpv::sync_surface_bounds(&app, true);
    emit_state(&app, &initial_state);

    let shader_paths = libmpv::resolve_shader_preset_paths(&shader_preset);
    let _ = mpv.set_property_string("glsl-shaders", &shader_paths);
    let _ = mpv.set_property_double("volume", prev_volume);
    let _ = mpv.set_property_bool("mute", prev_muted);
    let _ = mpv.set_property_double("speed", 1.0);
    let _ = mpv.set_property_double("sub-delay", 0.0);
    let _ = mpv.set_property_bool("pause", false);
    mpv.command(&["loadfile", &proxy_url, "replace"])?;

    // Background task: fetch companion soft-sub `.vtt` tracks, register them in the player HUD,
    // and auto-download + piecewise-retime Jimaku Japanese subtitles against the `.vtt` reference!
    let app_bg = app.clone();
    let st_bg = st.clone();
    let ep_bg = ep.clone();
    tauri::async_runtime::spawn(async move {
        player::sync_discord_activity(&st_bg, &ep_bg, start_pos, 0.0, true);

        let mut loaded_companion_tracks: Vec<(i64, TrackItem, Vec<SubtitleCue>)> = Vec::new();
        for (idx, sub) in req.subtitles.iter().enumerate() {
            if sub.url.trim().is_empty() {
                continue;
            }
            // Skip thumbnail sprite VTT tracks (`thumbnails.vtt` / `sprite`)
            let lang_low = sub.language.to_ascii_lowercase();
            let lbl_low = sub.label.as_deref().unwrap_or("").to_ascii_lowercase();
            if lang_low.contains("thumb") || lbl_low.contains("thumb") || sub.url.to_ascii_lowercase().contains("thumbnails") {
                continue;
            }
            if let Ok(cues) =
                subtitles::fetch_remote_subtitle_cues(&st_bg.providers.http, &sub.url, &req.headers)
                    .await
            {
                if cues.is_empty() {
                    continue;
                }
                let track_id = EXTERNAL_SUB_TRACK_ID + 1 + (idx as i64);
                let title = sub
                    .label
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| format!("Stream · {}", sub.language));
                let norm_lang = if lang_low.contains("eng") || lang_low == "en" {
                    "eng".to_string()
                } else if lang_low.contains("jap") || lang_low == "ja" || lang_low == "jpn" {
                    "jpn".to_string()
                } else {
                    sub.language.clone()
                };
                loaded_companion_tracks.push((
                    track_id,
                    TrackItem {
                        id: track_id,
                        track_type: "sub".to_string(),
                        title: Some(title),
                        lang: Some(norm_lang),
                        codec: Some("vtt".to_string()),
                        selected: false,
                        external: true,
                    },
                    cues,
                ));
            }
        }

        // Register companion `.vtt` tracks so fallback English/other subtitles are selectable right away
        if !loaded_companion_tracks.is_empty()
            && SESSION_COUNTER.load(Ordering::SeqCst) == session_id
        {
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            if g.state.active && SESSION_COUNTER.load(Ordering::SeqCst) == session_id {
                for (tid, item, cues) in &loaded_companion_tracks {
                    g.cues_by_track.insert(*tid, cues.clone());
                    if !g.state.sub_tracks.iter().any(|t| t.id == *tid) {
                        g.state.sub_tracks.push(item.clone());
                    }
                }
                if !g.user_selected_sub_track {
                    let chosen_sid = pick_best_sub_track_id(&g.state.sub_tracks, &g.track_prefs);
                    for tr in &mut g.state.sub_tracks {
                        tr.selected = Some(tr.id) == chosen_sid;
                    }
                    g.state.sid = chosen_sid;
                    if let Some(ref mpv_inst) = g.mpv {
                        apply_sub_track_to_mpv(mpv_inst, &g.state.sub_tracks, chosen_sid);
                    }
                    if let Some(cid) = chosen_sid.filter(|id| *id >= EXTERNAL_SUB_TRACK_ID) {
                        if let Some(cues) = g.cues_by_track.get(&cid).cloned() {
                            g.state.sub_text =
                                find_cue_at(&cues, g.state.position, g.state.sub_offset_sec)
                                    .unwrap_or_default();
                            g.cues = cues;
                        }
                    }
                }
                emit_state(&app_bg, &g.state);
            }
        }

        // Pick best reference cue set (prefer English dialogue with >= 10 cues, else largest cue set)
        let best_ref: Vec<SubtitleCue> = loaded_companion_tracks
            .iter()
            .filter(|(_, item, cues)| {
                cues.len() >= 10
                    && item
                        .lang
                        .as_deref()
                        .map(|l| l == "eng")
                        .unwrap_or(false)
                    && !player::is_signs_or_songs_title(item.title.as_deref())
            })
            .max_by_key(|(_, _, cues)| cues.len())
            .or_else(|| {
                loaded_companion_tracks
                    .iter()
                    .filter(|(_, _, cues)| cues.len() >= 10)
                    .max_by_key(|(_, _, cues)| cues.len())
            })
            .map(|(_, _, cues)| cues.clone())
            .unwrap_or_default();

        let ep_num = req.ep_key.parse::<i64>().unwrap_or(0);
        if SESSION_COUNTER.load(Ordering::SeqCst) == session_id {
            let _ = subtitles::auto_prepare_stream_subtitles(
                &app_bg,
                &st_bg,
                req.anilist_id,
                &req.ep_key,
                ep_num,
                &best_ref,
                "Web Stream Soft-Sub (.vtt)",
            )
            .await;
        }
    });

    // Query AniSkip in parallel
    let app_srv = app.clone();
    let st_srv = st.clone();
    tauri::async_runtime::spawn(async move {
        fetch_and_apply_aniskip(&app_srv, &st_srv, &ep, 0.0, session_id).await;
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
    let Ok(client) = reqwest::Client::builder().no_proxy().build() else {
        return;
    };

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
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO watch_state (anilist_id, ep_key, watched_at) VALUES (?1, ?2, ?3)",
            params![ep.anilist_id, ep.ep_key, db::now()],
        )
        .unwrap_or(0);
    if inserted > 0 {
        let _ = conn.execute(
            "INSERT INTO immersion_daily (day_key, active_seconds, lookup_count, words_tracked, episodes_completed)
             VALUES (?1, 0, 0, 0, 1)
             ON CONFLICT(day_key) DO UPDATE SET episodes_completed = episodes_completed + 1",
            params![db::today_day_key()],
        );
    }
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
    let (cur_ep, mpv_opt) = {
        let g = runtime().lock().unwrap_or_else(|e| e.into_inner());
        (
            Ep {
                anilist_id: g.state.anilist_id,
                ep_key: g.state.ep_key.clone(),
                path: g.state.path.clone(),
            },
            g.mpv,
        )
    };

    match cmd {
        BuiltinCommand::SyncBounds => {
            sync_bounds(&app);
        }
        BuiltinCommand::Stop => {
            SESSION_COUNTER.fetch_add(1, Ordering::SeqCst);
            if let Some(mpv) = mpv_opt {
                let _ = mpv.command(&["stop"]);
            }
            libmpv::sync_surface_bounds(&app, false);

            let (pos, dur, flush_sec, show_title) = {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                let pos = g.state.position;
                let dur = g.state.duration;
                let flush_sec = g.unflushed_immersion_sec.round() as i64;
                let show_title = g.state.show_title.clone();
                g.unflushed_immersion_sec = 0.0;
                g.last_unpaused_tick = None;
                g.state.active = false;
                emit_state(&app, &g.state);
                (pos, dur, flush_sec, show_title)
            };
            if flush_sec > 0 {
                let _ = db::record_immersion_time(
                    &st.db(),
                    Some(cur_ep.anilist_id),
                    Some(&cur_ep.ep_key),
                    Some(&show_title),
                    flush_sec,
                );
            }
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
            if let Some(mpv) = mpv_opt {
                let _ = mpv.command(&["stop"]);
            }
            libmpv::sync_surface_bounds(&app, false);

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
            } else {
                let (has_next_ep, stream_kind) = {
                    let g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                    (g.state.has_next_episode, g.state.stream_kind.clone())
                };
                if has_next_ep {
                    if let Ok(num) = cur_ep.ep_key.parse::<i64>() {
                        let _ = app.emit(
                            "play-next-stream-episode",
                            serde_json::json!({
                                "anilistId": cur_ep.anilist_id,
                                "nextEpisode": num + 1,
                                "streamKind": stream_kind.unwrap_or_else(|| "web".to_string()),
                            }),
                        );
                    }
                }
            }
        }
        BuiltinCommand::PlayPause => {
            let next_paused = {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                g.state.paused = !g.state.paused;
                emit_state(&app, &g.state);
                g.state.paused
            };
            if let Some(mpv) = mpv_opt {
                let _ = mpv.set_property_bool("pause", next_paused);
            }
        }
        BuiltinCommand::Pause => {
            {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                g.state.paused = true;
                emit_state(&app, &g.state);
            }
            if let Some(mpv) = mpv_opt {
                let _ = mpv.set_property_bool("pause", true);
            }
        }
        BuiltinCommand::Resume => {
            {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                g.state.paused = false;
                emit_state(&app, &g.state);
            }
            if let Some(mpv) = mpv_opt {
                let _ = mpv.set_property_bool("pause", false);
            }
        }
        BuiltinCommand::Seek { position } => {
            let target = position.max(0.0);
            if let Some(mpv) = mpv_opt {
                let _ = mpv.command(&["seek", &format!("{target:.3}"), "absolute", "exact"]);
            }
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.position = target;
            if is_overlay_cue_track(g.state.sid) {
                let offset = g.state.sub_offset_sec;
                g.state.sub_text = find_cue_at(&g.cues, target, offset).unwrap_or_default();
            }
            emit_state(&app, &g.state);
        }
        BuiltinCommand::SeekRelative { delta } => {
            if let Some(mpv) = mpv_opt {
                let _ = mpv.command(&["seek", &format!("{delta:.3}"), "relative", "exact"]);
            }
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            let target = (g.state.position + delta).max(0.0);
            g.state.position = target;
            if is_overlay_cue_track(g.state.sid) {
                let offset = g.state.sub_offset_sec;
                g.state.sub_text = find_cue_at(&g.cues, target, offset).unwrap_or_default();
            }
            emit_state(&app, &g.state);
        }
        BuiltinCommand::SetVolume { volume } => {
            let v = volume.clamp(0.0, 130.0);
            if let Some(mpv) = mpv_opt {
                let _ = mpv.set_property_double("volume", v);
                if v > 0.0 {
                    let _ = mpv.set_property_bool("mute", false);
                }
            }
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.volume = v;
            if v > 0.0 {
                g.state.muted = false;
            }
            emit_state(&app, &g.state);
        }
        BuiltinCommand::ToggleMute => {
            let next_muted = {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                g.state.muted = !g.state.muted;
                emit_state(&app, &g.state);
                g.state.muted
            };
            if let Some(mpv) = mpv_opt {
                let _ = mpv.set_property_bool("mute", next_muted);
            }
        }
        BuiltinCommand::SetSpeed { speed } => {
            let sp = speed.clamp(0.25, 3.0);
            if let Some(mpv) = mpv_opt {
                let _ = mpv.set_property_double("speed", sp);
            }
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.speed = sp;
            emit_state(&app, &g.state);
        }
        BuiltinCommand::SetAudioTrack { id } => {
            if let Some(mpv) = mpv_opt {
                let _ = mpv.set_property_i64("aid", id);
            }
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.aid = Some(id);
            for tr in &mut g.state.audio_tracks {
                tr.selected = tr.id == id;
            }
            emit_state(&app, &g.state);
        }
        BuiltinCommand::SetSubTrack { id } => {
            let mut chosen_lang_pref: Option<String> = None;
            {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                g.user_selected_sub_track = true;
                let chosen_sid = if id <= 0 { None } else { Some(id) };
                g.state.sid = chosen_sid;
                for tr in &mut g.state.sub_tracks {
                    tr.selected = tr.id == id;
                    if tr.selected {
                        if let Some(ref l) = tr.lang {
                            let low = l.trim().to_ascii_lowercase();
                            if matches!(low.as_str(), "eng" | "en" | "english") {
                                chosen_lang_pref = Some("eng".to_string());
                            } else if matches!(low.as_str(), "jpn" | "ja" | "japanese" | "jp") {
                                chosen_lang_pref = Some("jpn".to_string());
                            }
                        }
                    }
                }
                if id <= 0 {
                    chosen_lang_pref = Some("off".to_string());
                }
                if let Some(ref lp) = chosen_lang_pref {
                    g.track_prefs.sub = Some(lp.clone());
                }

                if let Some(ref mpv) = g.mpv {
                    apply_sub_track_to_mpv(mpv, &g.state.sub_tracks, chosen_sid);
                }

                if id <= 0 {
                    g.state.sub_text.clear();
                    g.cues.clear();
                } else if id >= EXTERNAL_SUB_TRACK_ID {
                    if let Some(track_cues) = g.cues_by_track.get(&id).cloned() {
                        let pos = g.state.position;
                        let off = g.state.sub_offset_sec;
                        g.state.sub_text = find_cue_at(&track_cues, pos, off).unwrap_or_default();
                        g.cues = track_cues;
                    }
                } else if let Some(ref mpv) = g.mpv {
                    let raw = mpv.get_property_string("sub-text").unwrap_or_default();
                    g.state.sub_text = clean_mpv_sub_text(&raw);
                }
                emit_state(&app, &g.state);
            }
            if let Some(lp) = chosen_lang_pref {
                let conn = st.db();
                let _ = db::set_setting(&conn, "pref.sub_lang", &lp);
            }
        }
        BuiltinCommand::AddSubOffset { delta } => {
            let new_off = {
                let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
                g.user_adjusted_offset = true;
                g.state.sub_offset_sec =
                    ((g.state.sub_offset_sec + delta) * 100.0).round() / 100.0;
                if is_overlay_cue_track(g.state.sid) {
                    let pos = g.state.position;
                    let off = g.state.sub_offset_sec;
                    g.state.sub_text = find_cue_at(&g.cues, pos, off).unwrap_or_default();
                }
                emit_state(&app, &g.state);
                g.state.sub_offset_sec
            };
            if let Some(mpv) = mpv_opt {
                let _ = mpv.set_property_double("sub-delay", new_off);
            }
        }
        BuiltinCommand::SetShaderPreset { preset } => {
            let norm = match preset.trim().to_ascii_lowercase().as_str() {
                "a_fast" => "a_fast",
                "a_hq" => "a_hq",
                "b_soft" => "b_soft",
                "c_upscale" => "c_upscale",
                _ => "off",
            }
            .to_string();
            let shader_paths = libmpv::resolve_shader_preset_paths(&norm);
            if let Some(mpv) = mpv_opt {
                let _ = mpv.set_property_string("glsl-shaders", &shader_paths);
            }
            {
                let conn = st.db();
                let _ = db::set_setting(&conn, "pref.anime4k_preset", &norm);
            }
            crate::backup::mark_dirty();
            let mut g = runtime().lock().unwrap_or_else(|e| e.into_inner());
            g.state.shader_preset = norm;
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
            g.user_selected_sub_track = true;
            for tr in &mut g.state.sub_tracks {
                tr.selected = false;
            }
            if let Some(ext) = g
                .state
                .sub_tracks
                .iter_mut()
                .find(|t| t.id == EXTERNAL_SUB_TRACK_ID)
            {
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
            if let Some(ref mpv) = g.mpv {
                apply_sub_track_to_mpv(mpv, &g.state.sub_tracks, Some(EXTERNAL_SUB_TRACK_ID));
            }
            let pos = g.state.position;
            let off = g.state.sub_offset_sec;
            g.state.sub_text = find_cue_at(&cues, pos, off).unwrap_or_default();
            g.cues_by_track.insert(EXTERNAL_SUB_TRACK_ID, cues.clone());
            g.cues = cues;
            emit_state(&app, &g.state);
        }
        BuiltinCommand::Html5Tick { .. } => {
            // Legacy no-op when running on native libmpv
        }
    }

    Ok(get_state())
}
