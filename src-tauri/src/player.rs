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
            env("ProgramW6432"),
            env("LOCALAPPDATA").map(|p| p.join("Programs")),
            env("LOCALAPPDATA").map(|p| p.join(r"Microsoft\WinGet\Links")),
            env("ProgramFiles").map(|p| p.join(r"WinGet\Links")),
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
                // winget `shinchiro.mpv` (Inno installer) uses this folder name.
                "MPV Player",
                "mpv-x86_64",
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
        // winget portable packages: <root>\WinGet\Packages\<Id>_<Source>\[subdir\]exe
        let pkg_roots = [
            env("LOCALAPPDATA").map(|p| p.join(r"Microsoft\WinGet\Packages")),
            env("ProgramFiles").map(|p| p.join(r"WinGet\Packages")),
        ];
        for root in pkg_roots.into_iter().flatten() {
            let Ok(entries) = std::fs::read_dir(&root) else { continue };
            for e in entries.flatten() {
                let dir = e.path();
                let name = e.file_name().to_string_lossy().to_lowercase();
                let wanted = match kind {
                    Kind::Mpv => name.contains("mpv"),
                    Kind::Vlc => name.contains("vlc"),
                    Kind::Mpc => name.contains("mpc-"),
                    Kind::Memento => name.contains("memento"),
                    Kind::System => false,
                };
                if !wanted {
                    continue;
                }
                for exe in exe_names(kind) {
                    out.push(dir.join(exe));
                    // One level of nesting (zip archives often contain a top folder).
                    if let Ok(inner) = std::fs::read_dir(&dir) {
                        for i in inner.flatten() {
                            if i.path().is_dir() {
                                out.push(i.path().join(exe));
                            }
                        }
                    }
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

/// Directories on PATH. On Windows this also re-reads the *current* user and
/// machine PATH from the registry: installers (winget, choco, scoop) update the
/// registry, but Kura's own process PATH is a snapshot from when it (or Explorer)
/// started, so newly installed players would otherwise stay invisible.
fn path_dirs() -> Vec<PathBuf> {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    #[cfg(windows)]
    for raw in win::registry_path_values() {
        for part in raw.split(';') {
            let expanded = win::expand_env(part.trim());
            if !expanded.is_empty() {
                let p = PathBuf::from(expanded);
                if !dirs.contains(&p) {
                    dirs.push(p);
                }
            }
        }
    }
    dirs
}

fn on_path(names: &[&str]) -> Option<PathBuf> {
    for dir in path_dirs() {
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
mod win {
    use std::path::{Path, PathBuf};
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
    use winreg::RegKey;

    /// Expand `%VAR%` references (REG_EXPAND_SZ values are returned unexpanded).
    pub fn expand_env(s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('%') {
                Some(end) => {
                    let var = &after[..end];
                    match std::env::var(var) {
                        Ok(v) => out.push_str(&v),
                        Err(_) => {
                            out.push('%');
                            out.push_str(var);
                            out.push('%');
                        }
                    }
                    rest = &after[end + 1..];
                }
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    pub fn registry_path_values() -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Environment") {
            if let Ok(v) = k.get_value::<String, _>("Path") {
                out.push(v);
            }
        }
        if let Ok(k) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment")
        {
            if let Ok(v) = k.get_value::<String, _>("Path") {
                out.push(v);
            }
        }
        out
    }

    /// Turn a registry value like `"C:\x\mpv.exe",0` or `C:\x\mpv.exe "%1"` into a path.
    fn exe_from_command(raw: &str) -> Option<PathBuf> {
        let raw = expand_env(raw.trim());
        let s = if let Some(stripped) = raw.strip_prefix('"') {
            stripped.split('"').next()?.to_string()
        } else {
            let lower = raw.to_lowercase();
            let end = lower.find(".exe").map(|i| i + 4).unwrap_or(raw.len());
            raw[..end].to_string()
        };
        let s = s.split(',').next().unwrap_or(&s).trim().to_string();
        (!s.is_empty()).then(|| PathBuf::from(s))
    }

    const VIEWS: [u32; 2] = [KEY_WOW64_64KEY, KEY_WOW64_32KEY];

    /// `App Paths\<exe>` default value (HKLM + HKCU, both registry views).
    pub fn app_path(exe: &str) -> Option<PathBuf> {
        let sub = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
        for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            for view in VIEWS {
                let Ok(k) = RegKey::predef(hive).open_subkey_with_flags(&sub, KEY_READ | view) else { continue };
                if let Ok(v) = k.get_value::<String, _>("") {
                    if let Some(p) = exe_from_command(&v).filter(|p| p.is_file()) {
                        return Some(p);
                    }
                }
                if let Ok(dir) = k.get_value::<String, _>("Path") {
                    let p = Path::new(&expand_env(&dir)).join(exe);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
        None
    }

    /// Scan installed-programs entries (what "Apps & features" shows) for a
    /// matching DisplayName and resolve its InstallLocation / DisplayIcon.
    pub fn uninstall_entries(name_hint: &str, exes: &[&str]) -> Option<PathBuf> {
        let hint = name_hint.to_lowercase();
        let sub = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
        for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            for view in VIEWS {
                let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(sub, KEY_READ | view) else { continue };
                for key_name in root.enum_keys().flatten() {
                    let Ok(app) = root.open_subkey_with_flags(&key_name, KEY_READ | view) else { continue };
                    let display: String = app.get_value("DisplayName").unwrap_or_default();
                    // First word only: "MPV Player", "mpv.net", "VLC media player", "MPC-HC"…
                    // but not "Memento - An mpv-based video player".
                    let first_word = |s: &str| {
                        s.to_lowercase()
                            .split(|c: char| !c.is_alphanumeric())
                            .find(|w| !w.is_empty())
                            .map(str::to_string)
                    };
                    let matches = first_word(&display).as_deref() == Some(hint.as_str())
                        || (display.is_empty() && first_word(&key_name).as_deref() == Some(hint.as_str()));
                    if !matches {
                        continue;
                    }
                    if let Ok(loc) = app.get_value::<String, _>("InstallLocation") {
                        let dir = PathBuf::from(expand_env(loc.trim().trim_matches('"')));
                        for exe in exes {
                            let p = dir.join(exe);
                            if p.is_file() {
                                return Some(p);
                            }
                        }
                    }
                    for value in ["DisplayIcon", "UninstallString"] {
                        let Ok(v) = app.get_value::<String, _>(value) else { continue };
                        let Some(p) = exe_from_command(&v) else { continue };
                        let fname = p.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
                        if exes.iter().any(|e| e.eq_ignore_ascii_case(&fname)) && p.is_file() {
                            return Some(p);
                        }
                        // e.g. UninstallString points at unins000.exe in the app folder.
                        if let Some(dir) = p.parent() {
                            for exe in exes {
                                let c = dir.join(exe);
                                if c.is_file() {
                                    return Some(c);
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn parses_registry_commands() {
            assert_eq!(
                exe_from_command(r#""C:\Program Files\MPV Player\mpv.exe",0"#),
                Some(PathBuf::from(r"C:\Program Files\MPV Player\mpv.exe"))
            );
            assert_eq!(
                exe_from_command(r#"C:\Program Files\MPV Player\mpv.exe "%1""#),
                Some(PathBuf::from(r"C:\Program Files\MPV Player\mpv.exe"))
            );
            assert_eq!(
                exe_from_command(r#""C:\Program Files\MPV Player\unins000.exe""#),
                Some(PathBuf::from(r"C:\Program Files\MPV Player\unins000.exe"))
            );
        }

        #[test]
        fn expands_env_vars() {
            std::env::set_var("KURA_TEST_DIR", r"C:\kura");
            assert_eq!(expand_env(r"%KURA_TEST_DIR%\bin"), r"C:\kura\bin");
            assert_eq!(expand_env(r"%NOPE_NOT_SET%\x"), r"%NOPE_NOT_SET%\x");
            assert_eq!(expand_env("plain"), "plain");
        }
    }
}

pub fn detect(kind: Kind) -> Option<PathBuf> {
    let exes = exe_names(kind);
    if exes.is_empty() {
        return None;
    }

    #[cfg(windows)]
    for exe in exes {
        if let Some(p) = win::app_path(exe) {
            return Some(p);
        }
    }

    if let Some(p) = known_locations(kind).into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    if let Some(p) = on_path(exes) {
        return Some(p);
    }

    // Last resort: whatever "Apps & features" knows about (any installer, any folder).
    #[cfg(windows)]
    {
        let hint = match kind {
            Kind::Mpv => "mpv",
            Kind::Vlc => "vlc",
            Kind::Mpc => "mpc",
            Kind::Memento => "memento",
            Kind::System => return None,
        };
        if let Some(p) = win::uninstall_entries(hint, exes) {
            return Some(p);
        }
    }
    None
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
                        "--source",
                        "winget",
                        "--accept-source-agreements",
                        "--accept-package-agreements",
                    ])
                    .status()
            })
            .await;

            // Re-detect even on a non-zero exit: winget reports "already installed"
            // / "no upgrade available" as errors, but mpv is there all the same.
            if matches!(res, Ok(Ok(_))) {
                {
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

            if matches!(res, Ok(Ok(_))) {
                {
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
        // No package-manager install on Linux (distro-specific); just point to mpv.io.
        let _ = (&app, &st);
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
-- Provides Kura-styled OSD notifications, clickable ASS skip button, OP/ED skipping, and subtitle adjustments.

local mp = require 'mp'

-- Palette (ASS Color format: &HBBGGRR& or &HAABBGGRR&)
-- Coral: #F76D4D -> &H4D6DF7&
-- Plum:  #140C0D -> &H0D0C14&
-- Cream: #F7EDE2 -> &HE2EDF7&
-- White: #FFFFFF -> &HFFFFFF&

local skip_overlay = nil
if mp.create_osd_overlay then
    skip_overlay = mp.create_osd_overlay("ass-events")
    skip_overlay.res_x = 1920
    skip_overlay.res_y = 1080
end

-- Resolved OP/ED ranges used by the button.
local op_start, op_end, ed_start, ed_end = nil, nil, nil, nil
-- Source data. Priority: named chapters (from the file itself) > AniSkip > 90s chapter heuristic.
local ch = {}
local ask = {}
local chapters_scanned = false
local current_button = nil
local is_hovered = false
local mouse_bound = false
local btn_box = nil -- {x1, y1, x2, y2} in overlay coordinates

local update_state = nil

local function norm(title)
    return string.lower(title or ""):gsub("^%s*(.-)%s*$", "%1")
end

local function is_intro(title)
    local s = norm(title)
    if s == "" then return false end
    if s == "intro" or s == "avant" or s == "teaser" or s == "recap" or s == "cold open" then return true end
    return string.find(s, "prolog") ~= nil or string.find(s, "cold open") ~= nil or
           string.find(s, "avant") ~= nil or string.find(s, "アバン") ~= nil or
           string.find(s, "previously") ~= nil or string.find(s, "recap") ~= nil or
           string.find(s, "pre%-op") ~= nil or string.find(s, "preop") ~= nil
end

local function is_op(title)
    local s = norm(title)
    if s == "" or is_intro(s) then return false end
    if s == "op" or s == "ncop" or s == "opening" or s == "オープニング" then return true end
    if string.find(s, "^op[%s%d%-_]") or string.find(s, "[%s%-_%(%[]op[%s%d%-_%)%]]") or string.find(s, "[%s%-_%(%[]op$") then return true end
    if string.find(s, "opening") or string.find(s, "ncop") or string.find(s, "theme song") or string.find(s, "オープニング") then return true end
    return false
end

local function is_ed(title)
    local s = norm(title)
    if s == "" or is_intro(s) then return false end
    if s == "ed" or s == "nced" or s == "ending" or s == "エンディング" then return true end
    if string.find(s, "^ed[%s%d%-_]") or string.find(s, "[%s%-_%(%[]ed[%s%d%-_%)%]]") or string.find(s, "[%s%-_%(%[]ed$") then return true end
    if string.find(s, "ending") or string.find(s, "nced") or string.find(s, "credits") or string.find(s, "エンディング") then return true end
    return false
end

-- A real OP/ED song is roughly 1-3 minutes. Anything else is almost certainly a prologue, recap or a mislabelled chapter.
local function plausible(s, e)
    return s and e and e > s and (e - s) >= 30 and (e - s) <= 200
end

local function resolve()
    op_start, op_end, ed_start, ed_end = nil, nil, nil, nil
    if ch.op_named then
        op_start, op_end = ch.op_s, ch.op_e
    elseif plausible(ask.op_s, ask.op_e) then
        op_start, op_end = ask.op_s, ask.op_e
    elseif ch.op_s then
        op_start, op_end = ch.op_s, ch.op_e
    end
    if ch.ed_named then
        ed_start, ed_end = ch.ed_s, ch.ed_e
    elseif plausible(ask.ed_s, ask.ed_e) then
        ed_start, ed_end = ask.ed_s, ask.ed_e
    elseif ch.ed_s then
        ed_start, ed_end = ch.ed_s, ch.ed_e
    end
end

local function chapter_span(i, ch_count, dur)
    local time = mp.get_property_number(string.format("chapter-list/%d/time", i), 0.0)
    local next_time = nil
    if i + 1 < ch_count then
        next_time = mp.get_property_number(string.format("chapter-list/%d/time", i + 1), nil)
    elseif dur > 0 then
        next_time = dur
    end
    return time, next_time
end

local function scan_chapters()
    ch = {}
    local ch_count = mp.get_property_number("chapters", 0)
    local dur = mp.get_property_number("duration", 0.0)

    -- Phase 1: chapters explicitly named Opening / Ending.
    for i = 0, ch_count - 1 do
        local title = mp.get_property(string.format("chapter-list/%d/title", i), "")
        local time, next_time = chapter_span(i, ch_count, dur)
        if plausible(time, next_time) then
            if is_op(title) and not ch.op_s then
                ch.op_s, ch.op_e, ch.op_named = time, next_time, true
            elseif is_ed(title) and not ch.ed_s then
                ch.ed_s, ch.ed_e, ch.ed_named = time, next_time, true
            end
        end
    end

    -- Phase 2: generic chapters ("Chapter 1", ...). OP/ED songs are ~90s (85-95s).
    -- A ~90s prologue can sit right before the OP, so when several 90s chapters exist in the
    -- first half, prefer one that doesn't start at 0:00 (the cold open is always first).
    if (not ch.op_s or not ch.ed_s) and ch_count > 0 and dur > 0 then
        local op_cands = {}
        for i = 0, ch_count - 1 do
            local title = mp.get_property(string.format("chapter-list/%d/title", i), "")
            if not is_intro(title) then
                local time, next_time = chapter_span(i, ch_count, dur)
                if next_time then
                    local len = next_time - time
                    if len >= 85 and len <= 95 then
                        if time < dur * 0.5 then
                            table.insert(op_cands, { s = time, e = next_time })
                        elseif not ch.ed_s then
                            ch.ed_s, ch.ed_e = time, next_time
                        end
                    end
                end
            end
        end
        if not ch.op_s and #op_cands > 0 then
            local pick = op_cands[1]
            if pick.s < 1.0 and op_cands[2] then pick = op_cands[2] end
            ch.op_s, ch.op_e = pick.s, pick.e
        end
    end

    chapters_scanned = (ch_count > 0 or dur > 0)
    resolve()
end

-- IPC Script Message from Kura (AniSkip timestamps, already matched to this file's length)
mp.register_script_message("kura-skip-times", function(op_s, op_e, ed_s, ed_e)
    ask = { op_s = tonumber(op_s), op_e = tonumber(op_e), ed_s = tonumber(ed_s), ed_e = tonumber(ed_e) }
    resolve()
    if update_state then update_state() end
end)

-- ---------------------------------------------------------------------------
-- Skip button: rounded comic "sticker" pill, bottom-right, with a TAB key chip.
-- ---------------------------------------------------------------------------
local function rrect(w, h, r)
    return string.format(
        "m %d 0 l %d 0 b %d 0 %d %d %d %d l %d %d b %d %d %d %d %d %d l %d %d b %d %d 0 %d 0 %d l 0 %d b 0 %d %d 0 %d 0",
        r, w - r, w - r / 2, w, r / 2, w, r, w, h - r, w, h - r / 2, w - r / 2, h, w - r, h,
        r, h, r / 2, h, h - r / 2, h - r, r, r / 2, r / 2, r)
end

local function render_button()
    if not skip_overlay or not current_button then return end

    -- Keep the pill's proportions on any window shape: fixed height space, width follows aspect.
    local d = mp.get_property_native("osd-dimensions")
    local res_x = 1920
    if d and d.w and d.h and d.w > 0 and d.h > 0 then
        res_x = math.floor(1080 * d.w / d.h + 0.5)
    end
    skip_overlay.res_x = res_x
    skip_overlay.res_y = 1080

    local label = "SKIP " .. current_button:upper()
    local h = 66
    local icon_w, chip_w, chip_h = 26, 58, 32
    -- pad | icon | gap | label | gap | chip | pad   (bold caps at fs30 ≈ 18px per char)
    local w = 26 + icon_w + 16 + #label * 18 + 22 + chip_w + 16
    local x = res_x - 60 - w
    local y = 1080 - 130 - h
    btn_box = { x, y, x + w, y + h }

    local fill, text, chip_fill, chip_text
    if is_hovered then
        fill, text, chip_fill, chip_text = "&H4D6DF7&", "&H0D0C14&", "&H0D0C14&", "&HE2EDF7&"
    else
        fill, text, chip_fill, chip_text = "&H0D0C14&", "&HE2EDF7&", "&H4D6DF7&", "&H0D0C14&"
    end

    local chip_x = x + w - 16 - chip_w
    local chip_y = y + (h - chip_h) / 2
    local icon_x = x + 26
    local icon_y = y + (h - 22) / 2

    local events = {
        -- Pill body with coral outline and a hard offset "sticker" shadow.
        string.format("{\\an7\\pos(%d,%d)\\bord3.5\\shad6\\1c%s\\1a&H10&\\3c&H4D6DF7&\\4c&H0D0C14&\\4a&H40&\\p1}%s{\\p0}",
            x, y, fill, rrect(w, h, 18)),
        -- Fast-forward icon drawn as vectors (no font glyph needed).
        string.format("{\\an7\\pos(%d,%d)\\bord0\\shad0\\1c%s\\p1}m 0 0 l 13 11 l 0 22 m 13 0 l 26 11 l 13 22{\\p0}",
            icon_x, icon_y, chip_fill == "&H4D6DF7&" and "&H4D6DF7&" or text),
        -- Label.
        string.format("{\\an4\\pos(%d,%d)\\bord0\\shad0\\b1\\fs30\\fsp1\\1c%s}%s",
            icon_x + icon_w + 16, y + h / 2, text, label),
        -- TAB key chip.
        string.format("{\\an7\\pos(%d,%d)\\bord0\\shad0\\1c%s\\p1}%s{\\p0}",
            chip_x, chip_y, chip_fill, rrect(chip_w, chip_h, 8)),
        string.format("{\\an5\\pos(%d,%d)\\bord0\\shad0\\b1\\fs18\\fsp1\\1c%s}TAB",
            chip_x + chip_w / 2, chip_y + chip_h / 2, chip_text),
    }
    skip_overlay.data = table.concat(events, "\n")
    skip_overlay:update()
end

local function show_button(label)
    if not skip_overlay then return end
    if current_button == label then return end
    current_button = label
    render_button()
end

local function hide_button()
    if not skip_overlay then return end
    if current_button ~= nil then
        current_button = nil
        is_hovered = false
        btn_box = nil
        if mouse_bound then
            mp.remove_key_binding("kura_skip_click")
            mouse_bound = false
        end
        skip_overlay:remove()
    end
end

-- mp.osd_message only parses ASS styling when prefixed with osd-ass-cc/0; without it the raw
-- {\an8...} tags were printed on screen.
local function osd_ass(text, dur)
    mp.osd_message(mp.get_property_osd("osd-ass-cc/0") .. text, dur)
end

local function ass_escape(s)
    return (s:gsub("\\", "\\\226\129\160"):gsub("{", "\\{"):gsub("\n", " "))
end

local function osd(msg)
    osd_ass("{\\an9\\b1\\fs20\\bord2.5\\shad0\\3c&H0D0C14&\\1c&H4D6DF7&}" .. msg, 2.0)
end

local function trigger_skip()
    local pos = mp.get_property_number("time-pos", 0.0)

    if op_start and op_end and pos >= (op_start - 2) and pos < op_end then
        mp.commandv("seek", op_end + 0.5, "absolute", "exact")
        osd("Skipped Opening")
    elseif ed_start and ed_end and pos >= (ed_start - 2) and pos < ed_end then
        mp.commandv("seek", ed_end, "absolute", "exact")
        osd("Skipped Ending")
    else
        -- No known OP/ED here: plain song-length jump.
        mp.commandv("seek", 85, "relative", "exact")
        osd("Skipped +85s")
    end

    hide_button()
    if update_state then update_state() end
end

local function set_hover(v)
    if is_hovered == v then return end
    is_hovered = v
    render_button()
    if v and not mouse_bound then
        mp.add_forced_key_binding("MBTN_LEFT", "kura_skip_click", trigger_skip)
        mouse_bound = true
    elseif not v and mouse_bound then
        mp.remove_key_binding("kura_skip_click")
        mouse_bound = false
    end
end

local function check_mouse()
    if not current_button or not btn_box then
        set_hover(false)
        return
    end

    local m = mp.get_property_native("mouse-pos")
    local d = mp.get_property_native("osd-dimensions")
    if not m or not d or not m.hover or not d.w or d.w == 0 or d.h == 0 then
        set_hover(false)
        return
    end

    -- The overlay spans the whole window (not just the video area), so map against the window.
    local ox = (m.x / d.w) * skip_overlay.res_x
    local oy = (m.y / d.h) * 1080
    local b = btn_box
    set_hover(ox >= b[1] and ox <= b[3] and oy >= b[2] and oy <= b[4])
end

update_state = function()
    if not chapters_scanned then
        scan_chapters()
    end

    local pos = mp.get_property_number("time-pos", nil)
    if not pos then
        hide_button()
        return
    end

    -- Only show the button while actually inside the OP/ED, never during a prologue / cold open.
    if op_start and op_end and pos >= (op_start - 1) and pos < (op_end - 1) then
        show_button("Opening")
    elseif ed_start and ed_end and pos >= (ed_start - 1) and pos < (ed_end - 1) then
        show_button("Ending")
    else
        hide_button()
    end
end

mp.observe_property("osd-dimensions", "native", function()
    if current_button then render_button() end
end)

-- The last chapter's length depends on the duration, which can arrive after file-loaded.
mp.observe_property("duration", "number", function()
    chapters_scanned = false
end)

-- 1. Styled "Now Playing" Banner on file start
mp.register_event("file-loaded", function()
    ask = {}
    chapters_scanned = false
    hide_button()
    scan_chapters()
    update_state()
    local title = mp.get_property("media-title", "")
    if title ~= "" then
        osd_ass("{\\an7\\b1\\fs17\\bord2.5\\shad0\\3c&H0D0C14&\\1c&H4D6DF7&}KURA  {\\b0\\fs15\\1c&HE2EDF7&}" .. ass_escape(title), 3.5)
    end
end)

-- 2. Subtitle Delay Shortcuts: 'z' (-100ms) and 'x' (+100ms)
local function sub_delay(step)
    local d = mp.get_property_number("sub-delay", 0.0) + step
    mp.set_property_number("sub-delay", d)
    local ms = math.floor(d * 1000 + 0.5)
    osd_ass(string.format("{\\an8\\b1\\fs18\\bord2.5\\shad0\\3c&H0D0C14&\\1c&H4D6DF7&}SUBTITLE DELAY  {\\1c&HE2EDF7&}%+d ms", ms), 1.5)
end
mp.add_forced_key_binding("z", "kura_sub_delay_down", function() sub_delay(-0.1) end)
mp.add_forced_key_binding("x", "kura_sub_delay_up", function() sub_delay(0.1) end)

-- 3. Smart OP/ED Skip on 'Tab'
mp.add_forced_key_binding("Tab", "kura_skip_oped", trigger_skip)

-- Continuously observe time-pos, chapter, and mouse-pos
mp.observe_property("time-pos", "number", update_state)
mp.observe_property("chapter", "number", update_state)
mp.observe_property("mouse-pos", "native", check_mouse)
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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AniSkipTimes {
    pub op_start: Option<f64>,
    pub op_end: Option<f64>,
    pub ed_start: Option<f64>,
    pub ed_end: Option<f64>,
}

/// Fetch OP/ED times from AniSkip. `episode_length` (seconds) should be the actual file's duration:
/// AniSkip timestamps are only valid for a release with the same length — a TV vs. BD cut with a
/// longer prologue shifts the OP, which made Kura flag prologues as openings. Pass `0.0` to skip the check.
pub async fn fetch_aniskip(client: &reqwest::Client, mal_id: i64, ep_num: i64, episode_length: f64) -> Option<AniSkipTimes> {
    let len = if episode_length > 0.0 { episode_length.round() as i64 } else { 0 };
    let url = format!("https://api.aniskip.com/v2/skip-times/{mal_id}/{ep_num}?types=op&types=ed&episodeLength={len}");
    let resp = client
        .get(&url)
        .header("User-Agent", "Kura/0.3.6")
        .timeout(Duration::from_millis(2500))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let val: serde_json::Value = resp.json().await.ok()?;
    if !val.get("found").and_then(|v| v.as_bool()).unwrap_or(false) {
        return None;
    }
    parse_aniskip(&val, episode_length)
}

/// Results come sorted best-first, so the first plausible entry of each type wins.
fn parse_aniskip(val: &serde_json::Value, episode_length: f64) -> Option<AniSkipTimes> {
    let results = val.get("results")?.as_array()?;
    let mut skip = AniSkipTimes::default();

    for item in results {
        let skip_type = item.get("skipType").and_then(|s| s.as_str()).unwrap_or("");
        let (Some(start), Some(end)) = (
            item.get("interval").and_then(|i| i.get("startTime")).and_then(|s| s.as_f64()),
            item.get("interval").and_then(|i| i.get("endTime")).and_then(|e| e.as_f64()),
        ) else {
            continue;
        };
        // A real OP/ED song is ~30s-200s; anything else is bad data.
        if !(30.0..=200.0).contains(&(end - start)) {
            continue;
        }
        // Submitted for a different cut of the episode → timestamps would be offset.
        if episode_length > 0.0 {
            if let Some(sub_len) = item.get("episodeLength").and_then(|l| l.as_f64()) {
                if sub_len > 0.0 && (sub_len - episode_length).abs() > 4.0 {
                    continue;
                }
            }
        }
        if skip_type == "op" && skip.op_start.is_none() {
            skip.op_start = Some(start);
            skip.op_end = Some(end);
        } else if skip_type == "ed" && skip.ed_start.is_none() {
            skip.ed_start = Some(start);
            skip.ed_end = Some(end);
        }
    }

    if skip == AniSkipTimes::default() {
        return None;
    }
    Some(skip)
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

    async fn send_skip_times(&mut self, skip: AniSkipTimes) -> Option<()> {
        let op_s = skip.op_start.map(|v| v.to_string()).unwrap_or_default();
        let op_e = skip.op_end.map(|v| v.to_string()).unwrap_or_default();
        let ed_s = skip.ed_start.map(|v| v.to_string()).unwrap_or_default();
        let ed_e = skip.ed_end.map(|v| v.to_string()).unwrap_or_default();

        self.call(serde_json::json!([
            "script-message",
            "kura-skip-times",
            op_s,
            op_e,
            ed_s,
            ed_e
        ])).await?;
        Some(())
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

async fn sync_aniskip_to_player(ctl: &mut Controller, st: &AppState, ep: &Ep) {
    if let Controller::Mpv(ref mut mpv) = ctl {
        let http_client = reqwest::Client::builder().no_proxy().build().ok();
        let mal_id: Option<i64> = st.db().query_row(
            "SELECT id_mal FROM media WHERE anilist_id = ?1",
            [ep.anilist_id],
            |r| r.get(0),
        ).ok().flatten();
        let ep_num: Option<i64> = ep.ep_key.parse().ok();

        if let (Some(ref client), Some(mid), Some(enum_val)) = (http_client.as_ref(), mal_id, ep_num) {
            // Wait (up to ~6s) until mpv has loaded *this* file and knows its duration,
            // so AniSkip results can be matched to this exact cut of the episode.
            let mut duration = 0.0;
            for _ in 0..20 {
                let path_ok = match mpv.prop("path").await {
                    Some(serde_json::Value::String(p)) => same_file(&p, &ep.path),
                    None => return, // player gone
                    _ => false,
                };
                let d = mpv.prop("duration").await.and_then(|v| v.as_f64()).unwrap_or(0.0);
                if path_ok && d > 0.0 {
                    duration = d;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            if duration <= 0.0 {
                return; // unknown length → can't trust AniSkip timestamps; chapters are used instead
            }
            if let Some(skip) = fetch_aniskip(client, mid, enum_val, duration).await {
                let _ = mpv.send_skip_times(skip).await;
            }
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

    // Dispatch AniSkip frame-perfect skip times to MPV
    sync_aniskip_to_player(&mut ctl, &st, &ep).await;

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
            sync_aniskip_to_player(&mut ctl, &st, &ep).await;
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

    #[tokio::test]
    async fn aniskip_fetch() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // Trapped in a Dating Sim S01E01 (MAL ID: 50461)
        if let Some(skip) = fetch_aniskip(&client, 50461, 1, 0.0).await {
            assert!(skip.op_start.is_some());
            assert!(skip.op_end.is_some());
            let op_len = skip.op_end.unwrap() - skip.op_start.unwrap();
            assert!((op_len - 90.0).abs() < 2.0);
        }
    }

    #[test]
    fn aniskip_rejects_other_cuts_and_bad_lengths() {
        let val = serde_json::json!({
            "found": true,
            "results": [
                // Prologue mislabelled as OP (too short) → ignored
                { "skipType": "op", "episodeLength": 1420.0, "interval": { "startTime": 0.0, "endTime": 20.0 } },
                // Submitted for a BD cut 40s longer → ignored
                { "skipType": "op", "episodeLength": 1460.0, "interval": { "startTime": 150.0, "endTime": 240.0 } },
                // Matches this file → used
                { "skipType": "op", "episodeLength": 1421.5, "interval": { "startTime": 110.0, "endTime": 200.0 } },
                // Lower-voted duplicate → ignored (first one wins)
                { "skipType": "op", "episodeLength": 1420.0, "interval": { "startTime": 112.0, "endTime": 202.0 } },
                { "skipType": "ed", "episodeLength": 1420.0, "interval": { "startTime": 1300.0, "endTime": 1390.0 } }
            ]
        });
        let s = parse_aniskip(&val, 1420.0).unwrap();
        assert_eq!((s.op_start, s.op_end), (Some(110.0), Some(200.0)));
        assert_eq!((s.ed_start, s.ed_end), (Some(1300.0), Some(1390.0)));

        // Nothing plausible → None, so mpv falls back to chapters.
        let bad = serde_json::json!({ "found": true, "results": [
            { "skipType": "op", "episodeLength": 1420.0, "interval": { "startTime": 0.0, "endTime": 15.0 } }
        ]});
        assert!(parse_aniskip(&bad, 1420.0).is_none());
    }
}
