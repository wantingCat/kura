//! External player integration.
//!
//! Kura launches the user's player in a mode it can talk to, then polls it about
//! once a second to:
//! - remember the playback position (resume + progress bars),
//! - mark the episode watched once it passes the user's threshold (default 90%),
//! - optionally send the next owned episode to the same window when one ends (autoplay).
//!
//! | Player          | Control channel                                         |
//! |-----------------|---------------------------------------------------------|
//! | mpv / Memento   | JSON IPC (`--input-ipc-server`, named pipe / unix socket)|
//! | VLC             | Built-in HTTP interface on 127.0.0.1 with a random password |
//! | MPC-HC / MPC-BE | Web interface (user enables it once; default port 13579) |
//! | System default  | none — opened with the OS handler, not tracked          |

use crate::db::{self, now};
use crate::franchise;
use crate::service::AppState;
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines, ReadHalf, WriteHalf};

// ---------------------------------------------------------------------------
// Player kinds & detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    System,
    Mpv,
    Memento,
    Vlc,
    Mpc,
}

impl Kind {
    pub fn parse(s: &str) -> Kind {
        match s {
            "mpv" => Kind::Mpv,
            "memento" => Kind::Memento,
            "vlc" => Kind::Vlc,
            "mpc" => Kind::Mpc,
            _ => Kind::System,
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Kind::System => "system",
            Kind::Mpv => "mpv",
            Kind::Memento => "memento",
            Kind::Vlc => "vlc",
            Kind::Mpc => "mpc",
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    kind: &'static str,
    path: String,
}

fn exe_names(kind: Kind) -> &'static [&'static str] {
    if cfg!(windows) {
        match kind {
            Kind::Mpv => &["mpv.exe"],
            Kind::Memento => &["memento.exe"],
            Kind::Vlc => &["vlc.exe"],
            Kind::Mpc => &["mpc-hc64.exe", "mpc-hc.exe", "mpc-be64.exe", "mpc-be.exe"],
            Kind::System => &[],
        }
    } else {
        match kind {
            Kind::Mpv => &["mpv"],
            Kind::Memento => &["memento"],
            Kind::Vlc => &["vlc"],
            Kind::Mpc => &[],
            Kind::System => &[],
        }
    }
}

fn known_locations(kind: Kind) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let env = |k: &str| std::env::var(k).ok().map(PathBuf::from);
    if cfg!(windows) {
        let roots: Vec<PathBuf> = [
            env("ProgramFiles"),
            env("ProgramFiles(x86)"),
            env("LOCALAPPDATA").map(|p| p.join("Programs")),
            env("LOCALAPPDATA").map(|p| p.join(r"Microsoft\WinGet\Links")),
            Some(PathBuf::from(r"C:\ProgramData\chocolatey\bin")),
            Some(PathBuf::from(r"C:\ProgramData")),
            Some(PathBuf::from(r"C:\tools")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let sub: &[&str] = match kind {
            Kind::Mpv => &[
                "mpv",
                "mpv.net",
                "",
                r"chocolatey\lib\mpvio.install\tools",
                r"chocolatey\lib\mpv.install\tools",
                r"chocolatey\lib\mpvio\tools",
                r"chocolatey\lib\mpv\tools",
            ],
            Kind::Memento => &["Memento", ""],
            Kind::Vlc => &["VideoLAN\\VLC", ""],
            Kind::Mpc => &["MPC-HC", "MPC-HC x64", "MPC-BE x64", "MPC-BE", "K-Lite Codec Pack\\MPC-HC64", ""],
            Kind::System => &[],
        };
        for r in &roots {
            for s in sub {
                for exe in exe_names(kind) {
                    let candidate = if s.is_empty() { r.join(exe) } else { r.join(s).join(exe) };
                    out.push(candidate);
                }
            }
        }
        if let Some(home) = env("USERPROFILE") {
            let scoop = match kind {
                Kind::Mpv => Some("mpv"),
                Kind::Vlc => Some("vlc"),
                Kind::Mpc => Some("mpc-hc-fork"),
                _ => None,
            };
            if let Some(app) = scoop {
                for exe in exe_names(kind) {
                    out.push(home.join("scoop\\apps").join(app).join("current").join(exe));
                }
            }
        }
    } else if cfg!(target_os = "macos") {
        match kind {
            Kind::Mpv => {
                out.push("/Applications/mpv.app/Contents/MacOS/mpv".into());
                out.push("/opt/homebrew/bin/mpv".into());
                out.push("/usr/local/bin/mpv".into());
            }
            Kind::Vlc => out.push("/Applications/VLC.app/Contents/MacOS/VLC".into()),
            Kind::Memento => out.push("/Applications/Memento.app/Contents/MacOS/memento".into()),
            _ => {}
        }
    }
    out
}

fn on_path(names: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for n in names {
            let p = dir.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(windows)]
fn query_app_path(exe: &str) -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    for root in &["HKLM", "HKCU"] {
        let key = format!(r"{root}\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
        let mut cmd = std::process::Command::new("reg");
        cmd.args(["query", &key, "/ve"]);
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    if let Some(pos) = line.find("REG_SZ") {
                        let path_str = line[pos + 6..].trim();
                        let p = PathBuf::from(path_str.trim_matches('"'));
                        if p.is_file() {
                            return Some(p);
                        }
                    }
                }
            }
        }
    }
    None
}

pub fn detect(kind: Kind) -> Option<PathBuf> {
    #[cfg(windows)]
    for exe in exe_names(kind) {
        if let Some(p) = query_app_path(exe) {
            return Some(p);
        }
    }

    known_locations(kind).into_iter().find(|p| p.is_file()).or_else(|| on_path(exe_names(kind)))
}

pub fn detect_all() -> Vec<Detected> {
    [Kind::Mpv, Kind::Vlc, Kind::Mpc, Kind::Memento]
        .into_iter()
        .filter_map(|k| detect(k).map(|p| Detected { kind: k.id(), path: p.to_string_lossy().to_string() }))
        .collect()
}

// ---------------------------------------------------------------------------
// MPV Detection, Auto-Installer & Kura Theme Script
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MpvStatus {
    pub installed: bool,
    pub path: Option<String>,
    pub has_winget: bool,
    pub has_choco: bool,
    pub package_manager: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstallMpvResult {
    pub success: bool,
    pub method: String,
    pub message: String,
    pub path: Option<String>,
}

pub fn check_tool_exists(name: &str) -> bool {
    if cfg!(windows) {
        let exe = if name.ends_with(".exe") { name.to_string() } else { format!("{name}.exe") };
        if on_path(&[&exe]).is_some() {
            return true;
        }
        let env = |k: &str| std::env::var(k).ok().map(PathBuf::from);
        if name == "winget" {
            if let Some(local) = env("LOCALAPPDATA") {
                if local.join(r"Microsoft\WindowsApps\winget.exe").is_file() {
                    return true;
                }
            }
        }
        if name == "choco" {
            if PathBuf::from(r"C:\ProgramData\chocolatey\bin\choco.exe").is_file() {
                return true;
            }
        }
        std::process::Command::new("where.exe")
            .arg(name)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        std::process::Command::new("which")
            .arg(name)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

pub fn check_mpv_status() -> MpvStatus {
    let detected = detect(Kind::Mpv);
    let has_winget = check_tool_exists("winget");
    let has_choco = check_tool_exists("choco");
    let package_manager = if has_winget {
        Some("winget".to_string())
    } else if has_choco {
        Some("choco".to_string())
    } else {
        None
    };

    MpvStatus {
        installed: detected.is_some(),
        path: detected.map(|p| p.to_string_lossy().to_string()),
        has_winget,
        has_choco,
        package_manager,
    }
}

pub async fn install_mpv(app: AppHandle, st: Arc<AppState>) -> Result<InstallMpvResult> {
    let status = check_mpv_status();
    if status.installed {
        return Ok(InstallMpvResult {
            success: true,
            method: "already_installed".to_string(),
            message: "MPV is already installed.".to_string(),
            path: status.path,
        });
    }

    #[cfg(windows)]
    {
        if status.has_winget {
            let _ = app.emit("mpv-install-progress", "Installing MPV via Windows Package Manager (winget)...");
            let res = tokio::task::spawn_blocking(|| {
                std::process::Command::new("winget")
                    .args([
                        "install",
                        "--id",
                        "shinchiro.mpv",
                        "-e",
                        "--accept-source-agreements",
                        "--accept-package-agreements",
                    ])
                    .status()
            })
            .await;

            if let Ok(Ok(exit)) = res {
                if exit.success() {
                    let _ = app.emit("mpv-install-progress", "Verifying MPV installation...");
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    if let Some(p) = detect(Kind::Mpv) {
                        let path_str = p.to_string_lossy().to_string();
                        let conn = st.db();
                        let _ = db::set_setting(&conn, "pref.player", "mpv");
                        let _ = db::set_setting(&conn, "pref.player_path", &path_str);
                        return Ok(InstallMpvResult {
                            success: true,
                            method: "winget".to_string(),
                            message: "MPV successfully installed and configured!".to_string(),
                            path: Some(path_str),
                        });
                    }
                }
            }
        }

        if status.has_choco {
            let _ = app.emit("mpv-install-progress", "Installing MPV via Chocolatey...");
            let res = tokio::task::spawn_blocking(|| {
                std::process::Command::new("choco")
                    .args(["install", "mpvio", "-y"])
                    .status()
            })
            .await;

            if let Ok(Ok(exit)) = res {
                if exit.success() {
                    let _ = app.emit("mpv-install-progress", "Verifying MPV installation...");
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    if let Some(p) = detect(Kind::Mpv) {
                        let path_str = p.to_string_lossy().to_string();
                        let conn = st.db();
                        let _ = db::set_setting(&conn, "pref.player", "mpv");
                        let _ = db::set_setting(&conn, "pref.player_path", &path_str);
                        return Ok(InstallMpvResult {
                            success: true,
                            method: "choco".to_string(),
                            message: "MPV successfully installed via Chocolatey!".to_string(),
                            path: Some(path_str),
                        });
                    }
                }
            }
        }

        let _ = app.emit("mpv-install-progress", "Opening MPV official download page...");
        let _ = tauri_plugin_opener::open_url("https://mpv.io/installation/", None::<&str>);
        Ok(InstallMpvResult {
            success: false,
            method: "website".to_string(),
            message: "Opened MPV installation page in your default browser.".to_string(),
            path: None,
        })
    }

    #[cfg(target_os = "macos")]
    {
        if check_tool_exists("brew") {
            let _ = app.emit("mpv-install-progress", "Installing MPV via Homebrew...");
            let res = tokio::task::spawn_blocking(|| {
                std::process::Command::new("brew")
                    .args(["install", "mpv"])
                    .status()
            })
            .await;
            if let Ok(Ok(exit)) = res {
                if exit.success() {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    if let Some(p) = detect(Kind::Mpv) {
                        let path_str = p.to_string_lossy().to_string();
                        let conn = st.db();
                        let _ = db::set_setting(&conn, "pref.player", "mpv");
                        let _ = db::set_setting(&conn, "pref.player_path", &path_str);
                        return Ok(InstallMpvResult {
                            success: true,
                            method: "brew".to_string(),
                            message: "MPV successfully installed via Homebrew!".to_string(),
                            path: Some(path_str),
                        });
                    }
                }
            }
        }
        let _ = tauri_plugin_opener::open_url("https://mpv.io/installation/", None::<&str>);
        Ok(InstallMpvResult {
            success: false,
            method: "website".to_string(),
            message: "Opened MPV installation page in your default browser.".to_string(),
            path: None,
        })
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = tauri_plugin_opener::open_url("https://mpv.io/installation/", None::<&str>);
        Ok(InstallMpvResult {
            success: false,
            method: "website".to_string(),
            message: "Opened MPV installation page in your default browser.".to_string(),
            path: None,
        })
    }
}

pub fn ensure_kura_mpv_script(data_dir: &std::path::Path) -> Result<PathBuf> {
    let dir = data_dir.join("scripts");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("kura-theme.lua");

    const SCRIPT: &str = r#"-- Kura MPV Theme & Integration Script
-- Provides Kura-styled OSD notifications, ASS overlays, OP/ED skipping, and subtitle adjustments.

local mp = require 'mp'

-- Palette (ASS Color format: &HBBGGRR& or &HAABBGGRR&)
-- Coral: #F76D4D -> &H4D6DF7&
-- Plum:  #140C0D -> &H0D0C14&
-- Cream: #F7EDE2 -> &HE2EDF7&

local skip_overlay = nil
if mp.create_osd_overlay then
    skip_overlay = mp.create_osd_overlay("ass-events")
    skip_overlay.res_x = 1920
    skip_overlay.res_y = 1080
end

local op_start = nil
local op_end = nil
local ed_start = nil
local ed_end = nil
local chapters_scanned = false
local current_button = nil

local function is_op(title)
    if not title or title == "" then return false end
    local s = string.lower(title):gsub("^%s*(.-)%s*$", "%1")
    if s == "op" or s == "ncop" or s == "opening" then return true end
    if string.find(s, "^op[%s%d%-_]") or string.find(s, "[%s%-_%(%[]op[%s%d%-_%)%]]") or string.find(s, "[%s%-_%(%[]op$") then return true end
    if string.find(s, "opening") or string.find(s, "ncop") or string.find(s, "theme song") then return true end
    return false
end

local function is_ed(title)
    if not title or title == "" then return false end
    local s = string.lower(title):gsub("^%s*(.-)%s*$", "%1")
    if s == "ed" or s == "nced" or s == "ending" then return true end
    if string.find(s, "^ed[%s%d%-_]") or string.find(s, "[%s%-_%(%[]ed[%s%d%-_%)%]]") or string.find(s, "[%s%-_%(%[]ed$") then return true end
    if string.find(s, "ending") or string.find(s, "nced") or string.find(s, "credits") or string.find(s, "credit") or string.find(s, "preview") or string.find(s, "pv") then return true end
    return false
end

local function scan_chapters()
    op_start = nil
    op_end = nil
    ed_start = nil
    ed_end = nil

    local ch_count = mp.get_property_number("chapters", 0)
    local dur = mp.get_property_number("duration", 0.0)

    for i = 0, ch_count - 1 do
        local title = mp.get_property(string.format("chapter-list/%d/title", i), "")
        local time = mp.get_property_number(string.format("chapter-list/%d/time", i), 0.0)
        local next_time = nil
        if i + 1 < ch_count then
            next_time = mp.get_property_number(string.format("chapter-list/%d/time", i + 1), nil)
        elseif dur > 0 then
            next_time = dur
        end

        if is_op(title) and not op_start then
            op_start = time
            op_end = next_time
        elseif is_ed(title) and not ed_start then
            ed_start = time
            ed_end = next_time
        end
    end

    -- Fallback for anime episodes without named chapters:
    -- Standard anime OP is around 10s-100s, ED is ~90s before end
    if not op_start and dur > 300 then
        op_start = 10.0
        op_end = 100.0
    end
    if not ed_start and dur > 300 then
        ed_start = dur - 105.0
        ed_end = dur - 15.0
    end

    chapters_scanned = (ch_count > 0 or dur > 0)
end

local function show_button(label)
    if not skip_overlay then return end
    if current_button == label then return end
    current_button = label
    skip_overlay.data = string.format("{\\an3\\pos(1860,960)\\b1\\fs28\\bord3.5\\shad2.5\\3c&H0D0C14&\\c&H4D6DF7&}[ ⏭ SKIP %s {\\fs22\\c&HE2EDF7&}(Tab) ]", label:upper())
    skip_overlay:update()
end

local function hide_button()
    if not skip_overlay then return end
    if current_button ~= nil then
        current_button = nil
        skip_overlay:remove()
    end
end

local function update_state()
    if not chapters_scanned then
        scan_chapters()
    end

    local pos = mp.get_property_number("time-pos", 0.0)
    if not pos then
        hide_button()
        return
    end

    -- 1. Currently inside Opening sequence (or near start if OP starts within first 25s)
    if op_start and op_end and pos < op_end and (pos >= op_start or (op_start <= 25 and pos <= op_end)) then
        show_button("Opening")
        return
    end

    -- 2. Currently inside Ending sequence
    if ed_start and ed_end and pos >= ed_start and pos < ed_end then
        show_button("Ending")
        return
    end

    -- 3. At the start of the episode (first 15s) before a delayed OP (cold open / prologue)
    if op_start and op_start > 25 and pos >= 0 and pos <= 15 then
        show_button("to Opening")
        return
    end

    hide_button()
end

-- 1. Styled "Now Playing" Banner on file start
mp.register_event("file-loaded", function()
    scan_chapters()
    update_state()
    local title = mp.get_property("media-title", "")
    if title ~= "" then
        mp.osd_message("{\\an7\\pos(30,30)\\b1\\fs24\\bord3\\shad2\\3c&H0D0C14&\\c&H4D6DF7&}🌸 KURA {\\c&HE2EDF7&}• {\\fs20\\c&HE2EDF7&}" .. title, 3.5)
    end
end)

-- 2. Subtitle Delay Shortcuts: 'z' (-100ms) and 'x' (+100ms)
mp.add_forced_key_binding("z", "kura_sub_delay_down", function()
    local d = mp.get_property_number("sub-delay", 0.0) - 0.1
    mp.set_property_number("sub-delay", d)
    local ms = math.floor(d * 1000 + 0.5)
    mp.osd_message(string.format("{\\an8\\pos(0,40)\\b1\\fs24\\bord3\\shad2\\3c&H0D0C14&\\c&H4D6DF7&}💬 SUBTITLE DELAY: {\\c&HE2EDF7&}%+d ms", ms), 1.5)
end)

mp.add_forced_key_binding("x", "kura_sub_delay_up", function()
    local d = mp.get_property_number("sub-delay", 0.0) + 0.1
    mp.set_property_number("sub-delay", d)
    local ms = math.floor(d * 1000 + 0.5)
    mp.osd_message(string.format("{\\an8\\pos(0,40)\\b1\\fs24\\bord3\\shad2\\3c&H0D0C14&\\c&H4D6DF7&}💬 SUBTITLE DELAY: {\\c&HE2EDF7&}%+d ms", ms), 1.5)
end)

-- 3. Smart OP/ED Skip on 'Tab'
local function trigger_skip()
    local pos = mp.get_property_number("time-pos", 0.0)

    -- If in Opening -> jump past OP
    if op_start and op_end and pos < op_end and (pos >= (op_start - 2) or op_start <= 25) then
        mp.commandv("seek", op_end + 0.5, "absolute", "exact")
        mp.osd_message("{\\an8\\pos(0,50)\\b1\\fs26\\bord3\\shad2\\3c&H0D0C14&\\c&H4D6DF7&}⏭ Skipped Opening", 2.0)
    -- If in Ending -> jump past ED
    elseif ed_start and ed_end and pos >= (ed_start - 2) and pos < ed_end then
        mp.commandv("seek", ed_end, "absolute", "exact")
        mp.osd_message("{\\an8\\pos(0,50)\\b1\\fs26\\bord3\\shad2\\3c&H0D0C14&\\c&H4D6DF7&}⏭ Skipped Ending", 2.0)
    -- If in cold open before OP -> jump to start of OP
    elseif op_start and pos < op_start then
        mp.commandv("seek", op_start + 0.2, "absolute", "exact")
        mp.osd_message("{\\an8\\pos(0,50)\\b1\\fs26\\bord3\\shad2\\3c&H0D0C14&\\c&H4D6DF7&}⏭ Jumped to Opening", 2.0)
    else
        -- General anime song skip (+85s)
        mp.commandv("seek", 85, "relative", "exact")
        mp.osd_message("{\\an8\\pos(0,50)\\b1\\fs26\\bord3\\shad2\\3c&H0D0C14&\\c&H4D6DF7&}⏭ Skipped +85s", 2.0)
    end
    update_state()
end

mp.add_forced_key_binding("Tab", "kura_skip_oped", trigger_skip)

-- Continuously observe time-pos and chapter to toggle the skip button
mp.observe_property("time-pos", "number", update_state)
mp.observe_property("chapter", "number", update_state)
"#;

    std::fs::write(&path, SCRIPT)?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Prefs {
    pub kind: Kind,
    pub path: Option<String>,
    /// 0.5 ..= 1.0
    pub threshold: f64,
    pub resume: bool,
    pub autoplay: bool,
    pub mpc_port: u16,
}

pub fn prefs(conn: &Connection) -> Prefs {
    let get = |k: &str| db::get_setting(conn, &format!("pref.{k}")).ok().flatten();
    Prefs {
        kind: match get("player").filter(|s| !s.is_empty()) {
            Some(s) => Kind::parse(&s),
            None => [Kind::Mpv, Kind::Vlc, Kind::Mpc].into_iter().find(|k| detect(*k).is_some()).unwrap_or(Kind::System),
        },
        path: get("player_path").filter(|s| !s.trim().is_empty()),
        threshold: get("watched_threshold").and_then(|s| s.parse::<f64>().ok()).map(|v| (v / 100.0).clamp(0.5, 1.0)).unwrap_or(0.9),
        resume: get("resume").as_deref() != Some("0"),
        autoplay: get("autoplay").as_deref() != Some("0"),
        mpc_port: get("mpc_port").and_then(|s| s.parse().ok()).unwrap_or(13579),
    }
}

#[derive(Debug, Clone, Default)]
pub struct EffectiveTrackPrefs {
    pub audio: Option<String>,
    pub sub: Option<String>,
    pub sub_fallback: Option<String>,
}

pub fn effective_track_prefs(conn: &Connection, anilist_id: i64) -> EffectiveTrackPrefs {
    let show = db::get_media_track_pref(conn, anilist_id).ok().flatten();
    let get_global = |k: &str| db::get_setting(conn, &format!("pref.{k}")).ok().flatten().filter(|s| !s.trim().is_empty());

    let audio = show
        .as_ref()
        .and_then(|s| s.audio_pref.clone())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| get_global("audio_lang"))
        .unwrap_or_else(|| "jpn".into());

    let sub = show
        .as_ref()
        .and_then(|s| s.sub_pref.clone())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| get_global("sub_lang"))
        .unwrap_or_else(|| "jpn".into());

    let sub_fallback = show
        .as_ref()
        .and_then(|s| s.sub_fallback.clone())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| get_global("sub_fallback"))
        .unwrap_or_else(|| "eng".into());

    EffectiveTrackPrefs {
        audio: Some(audio),
        sub: Some(sub),
        sub_fallback: Some(sub_fallback),
    }
}

pub fn lang_aliases(code: &str) -> Vec<&'static str> {
    match code.trim().to_ascii_lowercase().as_str() {
        "jpn" | "ja" | "japanese" => vec!["jpn", "ja", "Japanese", "jp"],
        "eng" | "en" | "english" => vec!["eng", "en", "English"],
        "ger" | "de" | "deu" | "german" => vec!["ger", "deu", "de", "German"],
        "spa" | "es" | "spanish" => vec!["spa", "es", "Spanish", "Castilian"],
        "fre" | "fr" | "fra" | "french" => vec!["fre", "fra", "fr", "French"],
        "ita" | "it" | "italian" => vec!["ita", "it", "Italian"],
        "por" | "pt" | "portuguese" => vec!["por", "pt", "Portuguese", "Brazilian"],
        "chi" | "zho" | "zh" | "chinese" => vec!["chi", "zho", "zh", "Chinese"],
        "kor" | "ko" | "korean" => vec!["kor", "ko", "Korean"],
        "rus" | "ru" | "russian" => vec!["rus", "ru", "Russian"],
        "ara" | "ar" | "arabic" => vec!["ara", "ar", "Arabic"],
        _ => vec![],
    }
}

pub fn expand_lang_list(code: &str) -> Vec<String> {
    let trimmed = code.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("any") || trimmed.eq_ignore_ascii_case("default") {
        return vec![];
    }
    let aliases = lang_aliases(trimmed);
    if aliases.is_empty() {
        vec![trimmed.to_string()]
    } else {
        aliases.into_iter().map(String::from).collect()
    }
}

// ---------------------------------------------------------------------------
// Progress storage
// ---------------------------------------------------------------------------

/// Saved (position, duration) in seconds.
pub fn saved_progress(conn: &Connection, anilist_id: i64, ep_key: &str) -> Option<(f64, f64)> {
    conn.query_row(
        "SELECT position, duration FROM watch_progress WHERE anilist_id = ?1 AND ep_key = ?2",
        params![anilist_id, ep_key],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .ok()
    .flatten()
}

fn save_progress(conn: &Connection, ep: &Ep, pos: f64, dur: f64) {
    if ep.ep_key.starts_with("extra:") {
        return;
    }
    let _ = conn.execute(
        "INSERT INTO watch_progress (anilist_id, ep_key, position, duration, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(anilist_id, ep_key) DO UPDATE SET position = excluded.position, duration = excluded.duration,
           updated_at = excluded.updated_at",
        params![ep.anilist_id, ep.ep_key, pos, dur, now()],
    );
}

fn mark_watched(conn: &Connection, ep: &Ep) {
    if ep.ep_key.starts_with("extra:") {
        return;
    }
    let _ = conn.execute(
        "INSERT OR IGNORE INTO watch_state (anilist_id, ep_key, watched_at) VALUES (?1, ?2, ?3)",
        params![ep.anilist_id, ep.ep_key, now()],
    );
    let _ = conn.execute("DELETE FROM watch_progress WHERE anilist_id = ?1 AND ep_key = ?2", params![ep.anilist_id, ep.ep_key]);
}

/// Whether a position counts as "watched" for a given threshold.
pub fn reached(pos: f64, dur: f64, threshold: f64) -> bool {
    dur > 0.0 && pos / dur >= threshold
}

/// Where to start: resume saved progress unless it's trivially short or basically finished.
pub fn resume_point(saved: Option<(f64, f64)>, threshold: f64) -> f64 {
    match saved {
        Some((pos, dur)) if pos > 15.0 && !reached(pos, dur, threshold) => (pos - 3.0).max(0.0),
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// Episodes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ep {
    pub anilist_id: i64,
    pub ep_key: String,
    pub path: String,
}

/// The next owned regular episode after `cur` — in the same entry, else the first one of the next
/// owned season / movie of the franchise.
pub fn next_episode(conn: &Connection, cur: &Ep) -> Option<Ep> {
    let num: i64 = cur.ep_key.parse().ok()?; // specials ("S1") have no "next"
    let first_after = |id: i64, after: i64| -> Option<Ep> {
        conn.query_row(
            "SELECT ep_key, path FROM local_files
             WHERE anilist_id = ?1 AND ep_key IS NOT NULL AND ep_key NOT LIKE 'S%' AND CAST(ep_key AS INTEGER) > ?2
             ORDER BY CAST(ep_key AS INTEGER), path LIMIT 1",
            params![id, after],
            |r| Ok(Ep { anilist_id: id, ep_key: r.get(0)?, path: r.get(1)? }),
        )
        .optional()
        .ok()
        .flatten()
    };
    if let Some(e) = first_after(cur.anilist_id, num) {
        return Some(e);
    }
    let map = franchise::load(conn).ok()?;
    let me = map.get(&cur.anilist_id)?;
    let mut later: Vec<(usize, i64)> =
        map.iter().filter(|(_, p)| p.root == me.root && p.index > me.index).map(|(id, p)| (p.index, *id)).collect();
    later.sort();
    later.into_iter().find_map(|(_, id)| first_after(id, 0))
}

// ---------------------------------------------------------------------------
// Controllers
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct Status {
    pos: f64,
    dur: f64,
    /// Path / URI of the file currently loaded, if the player reports it.
    file: Option<String>,
    #[allow(dead_code)] // reported by every controller; not used for decisions yet
    playing: bool,
    ended: bool,
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

struct Mpv {
    lines: Lines<BufReader<ReadHalf<Box<dyn Io>>>>,
    tx: WriteHalf<Box<dyn Io>>,
    next_id: u64,
}

impl Mpv {
    async fn connect(addr: &str) -> std::io::Result<Mpv> {
        #[cfg(windows)]
        let io: Box<dyn Io> = Box::new(tokio::net::windows::named_pipe::ClientOptions::new().open(addr)?);
        #[cfg(unix)]
        let io: Box<dyn Io> = Box::new(tokio::net::UnixStream::connect(addr).await?);
        let (rx, tx) = tokio::io::split(io);
        Ok(Mpv { lines: BufReader::new(rx).lines(), tx, next_id: 1 })
    }

    async fn call(&mut self, args: serde_json::Value) -> Option<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = serde_json::json!({ "command": args, "request_id": id });
        self.tx.write_all(format!("{msg}\n").as_bytes()).await.ok()?;
        loop {
            let line = tokio::time::timeout(Duration::from_secs(2), self.lines.next_line()).await.ok()?.ok()??;
            let v: serde_json::Value = serde_json::from_str(&line).ok()?;
            if v.get("request_id").and_then(|x| x.as_u64()) == Some(id) {
                return Some(v);
            }
            // Async events ("event": ...) are interleaved; skip them.
        }
    }

    async fn prop(&mut self, name: &str) -> Option<serde_json::Value> {
        let v = self.call(serde_json::json!(["get_property", name])).await?;
        if v.get("error").and_then(|e| e.as_str()) == Some("success") {
            v.get("data").cloned()
        } else {
            Some(serde_json::Value::Null) // property unavailable (e.g. nothing loaded yet) — still connected
        }
    }
}

struct Vlc {
    http: reqwest::Client,
    base: String,
    password: String,
    seen_playing: bool,
    last_ratio: f64,
}

struct Mpc {
    http: reqwest::Client,
    base: String,
    exe: PathBuf,
    last_ratio: f64,
}

enum Controller {
    Mpv(Mpv),
    Vlc(Vlc),
    Mpc(Mpc),
}

fn file_uri(path: &str) -> String {
    reqwest::Url::from_file_path(path).map(|u| u.to_string()).unwrap_or_else(|_| path.to_string())
}

fn same_file(reported: &str, expected: &str) -> bool {
    let norm = |s: &str| {
        let s = s.strip_prefix("file:///").or_else(|| s.strip_prefix("file://")).unwrap_or(s);
        percent_decode(s).replace('\\', "/").to_lowercase()
    };
    let (a, b) = (norm(reported), norm(expected));
    a == b || a.ends_with(&b) || b.ends_with(&a)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn mpc_var(html: &str, id: &str) -> Option<String> {
    let re = regex::Regex::new(&format!(r#"id="{id}">([^<]*)<"#)).ok()?;
    re.captures(html).map(|c| c[1].to_string())
}

impl Controller {
    /// `None` = player unreachable (closed, or control channel not available).
    async fn status(&mut self, threshold: f64) -> Option<Status> {
        match self {
            Controller::Mpv(m) => {
                let pos = m.prop("time-pos").await?;
                let dur = m.prop("duration").await?;
                let eof = m.prop("eof-reached").await?;
                let path = m.prop("path").await?;
                let paused = m.prop("pause").await?;
                Some(Status {
                    pos: pos.as_f64().unwrap_or(0.0),
                    dur: dur.as_f64().unwrap_or(0.0),
                    file: path.as_str().map(String::from),
                    playing: !paused.as_bool().unwrap_or(false),
                    ended: eof.as_bool().unwrap_or(false),
                })
            }
            Controller::Vlc(v) => {
                let res = v
                    .http
                    .get(format!("{}/requests/status.json", v.base))
                    .basic_auth("", Some(&v.password))
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await
                    .ok()?;
                let j: serde_json::Value = res.json().await.ok()?;
                let state = j.get("state").and_then(|s| s.as_str()).unwrap_or("");
                let pos = j.get("time").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let dur = j.get("length").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let file = j
                    .pointer("/information/category/meta/filename")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                if state == "playing" {
                    v.seen_playing = true;
                }
                if dur > 0.0 {
                    v.last_ratio = pos / dur;
                }
                // At the end of a single-item playlist VLC drops to "stopped".
                let ended = v.seen_playing && state == "stopped" && v.last_ratio >= threshold.min(0.90);
                Some(Status { pos, dur, file, playing: state == "playing", ended })
            }
            Controller::Mpc(m) => {
                let html = m
                    .http
                    .get(format!("{}/variables.html", m.base))
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await
                    .ok()?
                    .text()
                    .await
                    .ok()?;
                let ms = |id: &str| mpc_var(&html, id).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0) / 1000.0;
                let (pos, dur) = (ms("position"), ms("duration"));
                let state: i64 = mpc_var(&html, "state").and_then(|s| s.trim().parse().ok()).unwrap_or(-1);
                if dur > 0.0 && state != 0 {
                    m.last_ratio = pos / dur;
                }
                let ended = (dur > 0.0 && pos / dur >= 0.995) || (state == 0 && m.last_ratio >= threshold.min(0.90));
                Some(Status { pos, dur, file: mpc_var(&html, "filepath"), playing: state == 2, ended })
            }
        }
    }

    /// Load another file into the same player window (autoplay).
    async fn load(&mut self, path: &str, start: f64, tracks: &EffectiveTrackPrefs) -> bool {
        match self {
            Controller::Mpv(m) => {
                let start_opt = if start > 0.0 { format!("+{start:.0}") } else { "none".into() };
                let _ = m.call(serde_json::json!(["set_property", "start", start_opt])).await;
                if let Some(ref a) = tracks.audio {
                    let list = expand_lang_list(a);
                    if !list.is_empty() {
                        let _ = m.call(serde_json::json!(["set_property", "alang", list.join(",")])).await;
                    }
                }
                match tracks.sub.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
                    Some(s) if s == "off" || s == "none" => {
                        let _ = m.call(serde_json::json!(["set_property", "sid", "no"])).await;
                    }
                    Some(s) if !s.is_empty() && s != "any" && s != "default" => {
                        let mut list = expand_lang_list(&s);
                        if let Some(fb) = tracks.sub_fallback.as_deref() {
                            let fb_clean = fb.trim().to_ascii_lowercase();
                            if fb_clean != "off" && fb_clean != "none" && fb_clean != "any" && fb_clean != s {
                                for lang in expand_lang_list(&fb_clean) {
                                    if !list.contains(&lang) {
                                        list.push(lang);
                                    }
                                }
                            }
                        }
                        if !list.is_empty() {
                            let _ = m.call(serde_json::json!(["set_property", "slang", list.join(",")])).await;
                            let _ = m.call(serde_json::json!(["set_property", "subs-with-matching-audio", "yes"])).await;
                        }
                    }
                    _ => {}
                }
                let ok = m.call(serde_json::json!(["loadfile", path, "replace"])).await.is_some();
                let _ = m.call(serde_json::json!(["set_property", "pause", false])).await;
                ok
            }
            Controller::Vlc(v) => {
                v.seen_playing = false;
                v.last_ratio = 0.0;
                // Clear the finished item first so VLC does not accumulate stale items in the playlist
                let _ = v
                    .http
                    .get(format!("{}/requests/status.json?command=pl_empty", v.base))
                    .basic_auth("", Some(&v.password))
                    .send()
                    .await;
                let Ok(url) = reqwest::Url::parse_with_params(
                    &format!("{}/requests/status.json", v.base),
                    &[("command", "in_play"), ("input", file_uri(path).as_str())],
                ) else {
                    return false;
                };
                let ok = v.http.get(url).basic_auth("", Some(&v.password)).send().await.is_ok();
                // Ensure VLC is unpaused / playing in case in_play only queued it
                let _ = v
                    .http
                    .get(format!("{}/requests/status.json?command=pl_play", v.base))
                    .basic_auth("", Some(&v.password))
                    .send()
                    .await;
                if ok && start > 0.0 {
                    tokio::time::sleep(Duration::from_millis(800)).await;
                    let _ = v
                        .http
                        .get(format!("{}/requests/status.json?command=seek&val={}", v.base, start as i64))
                        .basic_auth("", Some(&v.password))
                        .send()
                        .await;
                }
                ok
            }
            Controller::Mpc(m) => {
                m.last_ratio = 0.0;
                // MPC reuses its running window by default ("Use the same player for each media file").
                let mut cmd = std::process::Command::new(&m.exe);
                cmd.arg(path);
                if start > 0.0 {
                    cmd.arg("/start").arg(((start * 1000.0) as i64).to_string());
                }
                cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
                cmd.spawn().is_ok()
            }
        }
    }

    async fn notify(&mut self, text: &str) {
        if let Controller::Mpv(m) = self {
            let _ = m.call(serde_json::json!(["show-text", text, 4000])).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackEvent {
    /// "tracking" | "untracked" | "stopped"
    state: &'static str,
    player: &'static str,
    anilist_id: i64,
    ep_key: String,
    position: f64,
    duration: f64,
    hint: Option<String>,
}

fn emit(app: &AppHandle, state: &'static str, kind: Kind, ep: &Ep, pos: f64, dur: f64, hint: Option<String>) {
    let _ = app.emit(
        "playback",
        PlaybackEvent { state, player: kind.id(), anilist_id: ep.anilist_id, ep_key: ep.ep_key.clone(), position: pos, duration: dur, hint },
    );
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(48123)
}

fn token() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{:x}{:x}", n, std::process::id())
}

fn ipc_address(tag: &str) -> String {
    if cfg!(windows) {
        format!(r"\\.\pipe\kura-mpv-{tag}")
    } else {
        std::env::temp_dir().join(format!("kura-mpv-{tag}.sock")).to_string_lossy().to_string()
    }
}

pub fn sync_discord_activity(st: &AppState, ep: &Ep, pos: f64, dur: f64, is_playing: bool) {
    let conn = st.db();
    let rpc_enabled = db::get_setting(&conn, "pref.discord_rpc")
        .ok()
        .flatten()
        .map(|v| v != "0")
        .unwrap_or(true);
    if !rpc_enabled {
        st.discord.clear();
        return;
    }

    let client_id = db::get_setting(&conn, "pref.discord_client_id")
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| crate::discord::DEFAULT_CLIENT_ID.to_string());

    let spoiler_protection = db::get_setting(&conn, "pref.discord_spoilers")
        .ok()
        .flatten()
        .map(|v| v == "1")
        .unwrap_or(false);

    let show_buttons = db::get_setting(&conn, "pref.discord_buttons")
        .ok()
        .flatten()
        .map(|v| v != "0")
        .unwrap_or(true);

    let media_info: Option<(Option<String>, Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT title_english, title_romaji, title_native, cover_url FROM media WHERE anilist_id = ?1",
            params![ep.anilist_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .unwrap_or(None);

    let (title_en, title_ro, title_nat, cover_url) = media_info.unwrap_or((None, None, None, None));
    let title = title_en.or(title_ro).or(title_nat).unwrap_or_else(|| "Anime".to_string());

    let ep_info: Option<(Option<String>, Option<String>, Option<String>, i64, i64)> = conn
        .query_row(
            "SELECT title_en, title_romaji, title_ja, is_special, number FROM episodes WHERE anilist_id = ?1 AND ep_key = ?2",
            params![ep.anilist_id, ep.ep_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .unwrap_or(None);

    let episode_text = if let Some((en, ro, ja, is_special, num)) = ep_info {
        let ep_title = en.or(ro).or(ja);
        let prefix = if is_special != 0 || ep.ep_key.starts_with('S') {
            format!("Special {:02}", num)
        } else {
            format!("Episode {:02}", num)
        };
        if let Some(t) = ep_title.filter(|t| !t.trim().is_empty()) {
            format!("{prefix} — {t}")
        } else {
            prefix
        }
    } else if ep.ep_key.starts_with("extra:") {
        let name = ep.ep_key.strip_prefix("extra:").unwrap_or(&ep.ep_key);
        format!("Extra — {name}")
    } else if let Ok(num) = ep.ep_key.parse::<i64>() {
        format!("Episode {:02}", num)
    } else {
        format!("Episode {}", ep.ep_key)
    };

    let now = chrono::Utc::now().timestamp();
    let (start_timestamp, end_timestamp) = if is_playing && dur > 0.0 {
        let start = now - pos.max(0.0) as i64;
        let end = now + (dur - pos).max(0.0) as i64;
        (Some(start), Some(end))
    } else if is_playing {
        (Some(now), None)
    } else {
        (None, None)
    };

    st.discord.update(crate::discord::ActivityPayload {
        client_id,
        title,
        episode_text,
        cover_url,
        anilist_id: Some(ep.anilist_id),
        is_playing,
        start_timestamp,
        end_timestamp,
        spoiler_protection,
        show_buttons,
    });
}

/// Start playing an episode with the configured player. Returns immediately; tracking runs in the background.
pub fn play(app: AppHandle, st: Arc<AppState>, ep: Ep) -> Result<()> {
    let (p, start, track_prefs) = {
        let conn = st.db();
        let p = prefs(&conn);
        let start = if p.resume { resume_point(saved_progress(&conn, ep.anilist_id, &ep.ep_key), p.threshold) } else { 0.0 };
        let track_prefs = effective_track_prefs(&conn, ep.anilist_id);
        (p, start, track_prefs)
    };
    // Supersede any previous tracking session.
    let session = st.play_session.fetch_add(1, Ordering::SeqCst) + 1;

    if p.kind == Kind::System {
        tauri_plugin_opener::open_path(&ep.path, None::<&str>).map_err(|e| anyhow!(e.to_string()))?;
        sync_discord_activity(&st, &ep, 0.0, 0.0, true);
        emit(&app, "untracked", p.kind, &ep, 0.0, 0.0, None);
        return Ok(());
    }

    let exe = p
        .path
        .clone()
        .map(PathBuf::from)
        .filter(|x| x.is_file())
        .or_else(|| detect(p.kind))
        .ok_or_else(|| anyhow!("Couldn't find {} — set its location in Settings → Playback.", p.kind.id()))?;

    let tag = token();
    let mut cmd = std::process::Command::new(&exe);

    let audio_args: Option<String> = track_prefs.audio.as_deref().and_then(|a| {
        let list = expand_lang_list(a);
        if list.is_empty() { None } else { Some(list.join(",")) }
    });

    enum SubArg {
        Off,
        Langs(String),
        Default,
    }

    let sub_arg = match track_prefs.sub.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if s == "off" || s == "none" => SubArg::Off,
        Some(s) if !s.is_empty() && s != "any" && s != "default" => {
            let mut list = expand_lang_list(&s);
            if let Some(fb) = track_prefs.sub_fallback.as_deref() {
                let fb_clean = fb.trim().to_ascii_lowercase();
                if fb_clean != "off" && fb_clean != "none" && fb_clean != "any" && fb_clean != s {
                    for lang in expand_lang_list(&fb_clean) {
                        if !list.contains(&lang) {
                            list.push(lang);
                        }
                    }
                }
            }
            if list.is_empty() {
                SubArg::Default
            } else {
                SubArg::Langs(list.join(","))
            }
        }
        _ => SubArg::Default,
    };

    let pending = match p.kind {
        Kind::Mpv | Kind::Memento => {
            let addr = ipc_address(&tag);
            cmd.arg(format!("--input-ipc-server={addr}")).arg("--force-window=yes");
            if p.autoplay {
                cmd.arg("--keep-open=yes"); // stay open at the end so we can load the next episode
            }
            if start > 0.0 {
                cmd.arg(format!("--start=+{start:.0}"));
            }
            if let Some(ref a) = audio_args {
                cmd.arg(format!("--alang={a}"));
            }
            match &sub_arg {
                SubArg::Off => {
                    cmd.arg("--sid=no");
                }
                SubArg::Langs(s) => {
                    cmd.arg(format!("--slang={s}"));
                    cmd.arg("--subs-with-matching-audio=yes");
                }
                SubArg::Default => {}
            }

            // Kura custom styling & Lua integration
            if let Ok(script_path) = ensure_kura_mpv_script(&st.data_dir) {
                cmd.arg(format!("--script={}", script_path.display()));
            }
            cmd.args([
                "--osd-font=sans-serif",
                "--osd-font-size=28",
                "--osd-color=#F7EDE2",
                "--osd-border-color=#140C0D",
                "--osd-border-size=2.4",
                "--osd-shadow-offset=1.5",
                "--osd-shadow-color=#140C0D",
                "--osd-bar-align-y=0.92",
                "--osd-bar-h=3.2",
                "--osd-bar-w=65",
                "--sub-font=sans-serif",
                "--sub-color=#F7EDE2",
                "--sub-border-color=#140C0D",
                "--sub-border-size=2.5",
                "--sub-shadow-offset=1.2",
                "--sub-shadow-color=#140C0D",
                "--script-opts=osc-layout=bottombar,osc-seekbarstyle=bar,osc-seekrangestyle=bar,osc-deadzonesize=0.5,osc-minmousemove=3",
            ]);

            cmd.arg("--").arg(&ep.path);
            Pending::Mpv(addr)
        }
        Kind::Vlc => {
            let port = free_port();
            let password = tag.clone();
            cmd.args(["--no-one-instance", "--extraintf=http", "--http-host=127.0.0.1"])
                .arg(format!("--http-port={port}"))
                .arg(format!("--http-password={password}"));
            if start > 0.0 {
                cmd.arg(format!("--start-time={start:.0}"));
            }
            if let Some(ref a) = audio_args {
                cmd.arg(format!("--audio-language={a}"));
            }
            match &sub_arg {
                SubArg::Off => {
                    cmd.args(["--no-sub-autodetect-file", "--sub-track=99999"]);
                }
                SubArg::Langs(s) => {
                    cmd.arg(format!("--sub-language={s}"));
                }
                SubArg::Default => {}
            }
            cmd.arg(&ep.path);
            Pending::Vlc { base: format!("http://127.0.0.1:{port}"), password }
        }
        Kind::Mpc => {
            cmd.arg(&ep.path);
            if start > 0.0 {
                cmd.arg("/start").arg(((start * 1000.0) as i64).to_string());
            }
            Pending::Mpc { base: format!("http://127.0.0.1:{}", p.mpc_port), exe: exe.clone() }
        }
        Kind::System => unreachable!(),
    };
    // The player's own logging (e.g. VLC's "direct3d11 vout display error") is not ours to show.
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let child = cmd.spawn().map_err(|e| anyhow!("Couldn't start {}: {e}", exe.display()))?;
    tauri::async_runtime::spawn(track(app, st, p, ep, pending, child, session));
    Ok(())
}

enum Pending {
    Mpv(String),
    Vlc { base: String, password: String },
    Mpc { base: String, exe: PathBuf },
}

async fn connect(p: &Pending) -> Option<Controller> {
    let http = || reqwest::Client::builder().no_proxy().build().ok();
    match p {
        Pending::Mpv(addr) => Mpv::connect(addr).await.ok().map(Controller::Mpv),
        Pending::Vlc { base, password } => {
            let c = Vlc { http: http()?, base: base.clone(), password: password.clone(), seen_playing: false, last_ratio: 0.0 };
            let mut ctl = Controller::Vlc(c);
            ctl.status(0.9).await.map(|_| ctl)
        }
        Pending::Mpc { base, exe } => {
            let c = Mpc { http: http()?, base: base.clone(), exe: exe.clone(), last_ratio: 0.0 };
            let mut ctl = Controller::Mpc(c);
            ctl.status(0.9).await.map(|_| ctl)
        }
    }
}

async fn track(app: AppHandle, st: Arc<AppState>, p: Prefs, mut ep: Ep, pending: Pending, mut child: std::process::Child, session: u64) {
    let alive = |st: &AppState| st.play_session.load(Ordering::SeqCst) == session;

    // 1. Wait for the control channel (players take a moment to start).
    let mut ctl = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if !alive(&st) {
            return;
        }
        if let Some(c) = connect(&pending).await {
            ctl = Some(c);
            break;
        }
        if matches!(child.try_wait(), Ok(Some(_))) && !matches!(pending, Pending::Mpc { .. }) {
            break; // exited immediately (MPC hands the file to an existing instance and exits — keep waiting for it)
        }
    }
    let Some(mut ctl) = ctl else {
        let hint = match p.kind {
            Kind::Mpc => Some("Turn on MPC's web interface (Options → Player → Web Interface → \"Listen on port\") so Kura can track progress.".into()),
            Kind::Memento => Some("This Memento build doesn't accept mpv's IPC option, so progress can't be tracked.".into()),
            _ => Some("Couldn't connect to the player to track progress.".into()),
        };
        emit(&app, "untracked", p.kind, &ep, 0.0, 0.0, hint);
        return;
    };

    // 2. Poll.
    let (mut pos, mut dur) = (0.0_f64, 0.0_f64);
    let mut marked = false;
    let mut misses = 0;
    let mut ticks: u64 = 0;
    let mut loaded_at = std::time::Instant::now();
    let mut last_discord_playing: Option<bool> = None;
    let mut last_discord_sync = std::time::Instant::now();
    let mut last_discord_pos: f64 = 0.0;
    sync_discord_activity(&st, &ep, 0.0, 0.0, true);
    emit(&app, "tracking", p.kind, &ep, 0.0, 0.0, None);
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if !alive(&st) {
            break;
        }
        ticks += 1;
        let Some(s) = ctl.status(p.threshold).await else {
            misses += 1;
            if misses >= 3 {
                break; // player closed
            }
            continue;
        };
        misses = 0;

        // The user opened something else in the player — stop attributing progress to our episode.
        if let Some(f) = &s.file {
            if !f.is_empty() && !same_file(f, &ep.path) && loaded_at.elapsed() > Duration::from_secs(5) {
                break;
            }
        }
        if s.dur > 0.0 {
            pos = s.pos;
            dur = s.dur;
        }

        let playing = s.playing;
        let playing_changed = last_discord_playing != Some(playing);
        let seeked = (pos - (last_discord_pos + last_discord_sync.elapsed().as_secs_f64())).abs() > 8.0;
        let periodic = last_discord_sync.elapsed() > Duration::from_secs(20);

        if playing_changed || (playing && (seeked || periodic)) {
            sync_discord_activity(&st, &ep, pos, dur, playing);
            last_discord_playing = Some(playing);
            last_discord_sync = std::time::Instant::now();
            last_discord_pos = pos;
        }

        if !marked && reached(pos, dur, p.threshold) {
            marked = true;
            mark_watched(&st.db(), &ep);
            let _ = app.emit("library-changed", ());
        } else if !marked && pos > 15.0 && ticks % 5 == 0 {
            save_progress(&st.db(), &ep, pos, dur);
        }
        if ticks % 3 == 0 {
            emit(&app, "tracking", p.kind, &ep, pos, dur, None);
        }

        if s.ended && loaded_at.elapsed() > Duration::from_secs(3) {
            if !marked && dur > 0.0 {
                mark_watched(&st.db(), &ep);
                let _ = app.emit("library-changed", ());
            }
            if !p.autoplay {
                if matches!(ctl, Controller::Mpv(_)) {
                    continue; // mpv will exit on its own
                }
                break;
            }
            let next = next_episode(&st.db(), &ep);
            let Some(next) = next else {
                ctl.notify("That was the last episode in your library").await;
                break;
            };
            let start = if p.resume { resume_point(saved_progress(&st.db(), next.anilist_id, &next.ep_key), p.threshold) } else { 0.0 };
            ctl.notify(&format!("Up next: episode {}", next.ep_key)).await;
            let next_tracks = effective_track_prefs(&st.db(), next.anilist_id);
            if !ctl.load(&next.path, start, &next_tracks).await {
                break;
            }
            ep = next;
            (pos, dur, marked) = (0.0, 0.0, false);
            loaded_at = std::time::Instant::now();
            sync_discord_activity(&st, &ep, 0.0, 0.0, true);
            last_discord_playing = Some(true);
            last_discord_sync = std::time::Instant::now();
            last_discord_pos = 0.0;
            let _ = app.emit("library-changed", ());
        }
    }

    // 3. Final save.
    if !marked && pos > 15.0 && dur > 0.0 {
        if reached(pos, dur, p.threshold) {
            mark_watched(&st.db(), &ep);
        } else {
            save_progress(&st.db(), &ep, pos, dur);
        }
    }
    let _ = child.try_wait();
    if alive(&st) {
        st.discord.clear();
    }
    emit(&app, "stopped", p.kind, &ep, pos, dur, None);
    let _ = app.emit("library-changed", ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold() {
        assert!(reached(1300.0, 1440.0, 0.9));
        assert!(!reached(1200.0, 1440.0, 0.9));
        assert!(!reached(10.0, 0.0, 0.9));
    }

    #[test]
    fn resume_rules() {
        assert_eq!(resume_point(None, 0.9), 0.0);
        assert_eq!(resume_point(Some((10.0, 1440.0)), 0.9), 0.0); // too early to bother
        assert_eq!(resume_point(Some((600.0, 1440.0)), 0.9), 597.0);
        assert_eq!(resume_point(Some((1400.0, 1440.0)), 0.9), 0.0); // basically finished
    }

    #[test]
    fn mpc_variables() {
        let html = r#"<p id="filepath">C:\Anime\Show - 01.mkv</p><p id="state">2</p><p id="position">615000</p><p id="duration">1440000</p>"#;
        assert_eq!(mpc_var(html, "state").as_deref(), Some("2"));
        assert_eq!(mpc_var(html, "position").as_deref(), Some("615000"));
        assert_eq!(mpc_var(html, "filepath").as_deref(), Some(r"C:\Anime\Show - 01.mkv"));
    }

    #[test]
    fn file_matching() {
        assert!(same_file("file:///C:/Anime/Show%20-%2001.mkv", r"C:\Anime\Show - 01.mkv"));
        assert!(same_file(r"C:\Anime\Show - 01.mkv", r"c:\anime\show - 01.mkv"));
        assert!(same_file("Show - 01.mkv", r"C:\Anime\Show - 01.mkv")); // VLC reports just the file name
        assert!(!same_file(r"C:\Anime\Show - 02.mkv", r"C:\Anime\Show - 01.mkv"));
    }

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE media (anilist_id INTEGER PRIMARY KEY, format TEXT, start_date TEXT, season_year INTEGER);
             CREATE TABLE media_relations (anilist_id INTEGER, related_id INTEGER, relation_type TEXT, format TEXT, media_type TEXT);
             CREATE TABLE local_files (id INTEGER PRIMARY KEY, anilist_id INTEGER, ep_key TEXT, path TEXT);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn next_episode_within_and_across_seasons() {
        let conn = mem_db();
        conn.execute_batch(
            "INSERT INTO media VALUES (1, 'TV', '2013-04-07', 2013), (2, 'TV', '2017-04-01', 2017);
             INSERT INTO media_relations VALUES (1, 2, 'SEQUEL', 'TV', 'ANIME');
             INSERT INTO local_files (anilist_id, ep_key, path) VALUES
               (1, '1', 'a1'), (1, '2', 'a2'), (1, '10', 'a10'), (1, 'S1', 'aS1'), (2, '1', 'b1');",
        )
        .unwrap();
        let ep = |id, k: &str| Ep { anilist_id: id, ep_key: k.into(), path: String::new() };
        assert_eq!(next_episode(&conn, &ep(1, "1")).unwrap().path, "a2");
        assert_eq!(next_episode(&conn, &ep(1, "2")).unwrap().path, "a10"); // numeric, not lexical, order
        assert_eq!(next_episode(&conn, &ep(1, "10")).unwrap().path, "b1"); // next season
        assert!(next_episode(&conn, &ep(2, "1")).is_none());
        assert!(next_episode(&conn, &ep(1, "S1")).is_none());
    }

    #[test]
    fn track_lang_expansion() {
        let jp = expand_lang_list("jpn");
        assert!(jp.contains(&"jpn".to_string()));
        assert!(jp.contains(&"ja".to_string()));
        assert!(jp.contains(&"Japanese".to_string()));

        let en = expand_lang_list("eng");
        assert!(en.contains(&"eng".to_string()));
        assert!(en.contains(&"en".to_string()));
        assert!(en.contains(&"English".to_string()));

        let custom = expand_lang_list("kor");
        assert_eq!(custom, vec!["kor".to_string(), "ko".to_string(), "Korean".to_string()]);
    }

    #[test]
    fn track_prefs_resolution() {
        let conn = mem_db();
        conn.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE media_track_prefs (
                 anilist_id INTEGER PRIMARY KEY,
                 audio_pref TEXT,
                 sub_pref TEXT,
                 sub_fallback TEXT,
                 updated_at INTEGER NOT NULL
             );"
        ).unwrap();

        // 1. Defaults when nothing is configured
        let eff = effective_track_prefs(&conn, 100);
        assert_eq!(eff.audio.as_deref(), Some("jpn"));
        assert_eq!(eff.sub.as_deref(), Some("jpn"));
        assert_eq!(eff.sub_fallback.as_deref(), Some("eng"));

        // 2. Global settings
        crate::db::set_setting(&conn, "pref.audio_lang", "eng").unwrap();
        crate::db::set_setting(&conn, "pref.sub_lang", "off").unwrap();
        crate::db::set_setting(&conn, "pref.sub_fallback", "none").unwrap();
        let eff_global = effective_track_prefs(&conn, 100);
        assert_eq!(eff_global.audio.as_deref(), Some("eng"));
        assert_eq!(eff_global.sub.as_deref(), Some("off"));
        assert_eq!(eff_global.sub_fallback.as_deref(), Some("none"));

        // 3. Show-specific override
        crate::db::set_media_track_pref(
            &conn,
            100,
            Some(crate::db::MediaTrackPref {
                audio_pref: Some("ger".into()),
                sub_pref: Some("ger".into()),
                sub_fallback: Some("eng".into()),
            }),
        ).unwrap();

        // Show 100 uses override
        let eff_show = effective_track_prefs(&conn, 100);
        assert_eq!(eff_show.audio.as_deref(), Some("ger"));
        assert_eq!(eff_show.sub.as_deref(), Some("ger"));
        assert_eq!(eff_show.sub_fallback.as_deref(), Some("eng"));

        // Other shows still use global
        let eff_other = effective_track_prefs(&conn, 200);
        assert_eq!(eff_other.audio.as_deref(), Some("eng"));
        assert_eq!(eff_other.sub.as_deref(), Some("off"));

        // 4. Reset override
        crate::db::set_media_track_pref(&conn, 100, None).unwrap();
        let eff_reset = effective_track_prefs(&conn, 100);
        assert_eq!(eff_reset.audio.as_deref(), Some("eng"));
        assert_eq!(eff_reset.sub.as_deref(), Some("off"));
    }

    #[test]
    fn player_prefs_defaults() {
        let conn = mem_db();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT);").unwrap();
        let p = prefs(&conn);
        assert!(p.resume);
        assert!(p.autoplay, "Autoplay should default to enabled");
        assert_eq!(p.threshold, 0.9);

        // Explicitly disabled
        crate::db::set_setting(&conn, "pref.autoplay", "0").unwrap();
        let p_off = prefs(&conn);
        assert!(!p_off.autoplay);

        // Explicitly enabled
        crate::db::set_setting(&conn, "pref.autoplay", "1").unwrap();
        let p_on = prefs(&conn);
        assert!(p_on.autoplay);
    }

    #[test]
    fn kura_mpv_script_generation() {
        let temp = std::env::temp_dir().join(format!("kura_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        let path = ensure_kura_mpv_script(&temp).expect("script created");
        assert!(path.is_file());
        let content = std::fs::read_to_string(&path).expect("read script");
        assert!(content.contains("Kura MPV Theme"));
        assert!(content.contains("kura_sub_delay_down"));
        assert!(content.contains("kura_skip_oped"));
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn mpv_status_check() {
        let status = check_mpv_status();
        // Should not panic, returns valid flags
        println!("Detected mpv status: {:?}", status);
    }
}
