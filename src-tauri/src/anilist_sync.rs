//! AniList OAuth2 login, two-way list synchronization, auto-scrobbling and offline queue.

use crate::db::{self, now};
use crate::service::AppState;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const DEFAULT_CLIENT_ID: &str = "55550";

static FLUSHING: AtomicBool = AtomicBool::new(false);
static OAUTH_LISTENING: AtomicBool = AtomicBool::new(false);
static CONNECTING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnilistAccount {
    pub connected: bool,
    pub user_id: Option<i64>,
    pub user_name: Option<String>,
    pub avatar_url: Option<String>,
    pub last_sync_at: Option<i64>,
    pub auto_scrobble: bool,
    pub queued_count: i64,
    pub tracked_count: i64,
    pub client_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnilistSyncReport {
    pub pulled_series: usize,
    pub pulled_episodes: usize,
    pub pushed_series: usize,
    pub flushed_queue: usize,
    pub total_remote: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnilistRemoteEntry {
    pub anilist_id: i64,
    pub status: String,
    pub progress: i64,
    pub score: Option<f64>,
    pub updated_at: i64,
}

pub fn get_account(conn: &Connection) -> AnilistAccount {
    let token = db::get_setting(conn, "anilist.token")
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty());
    let user_id = db::get_setting(conn, "anilist.user_id")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<i64>().ok());
    let user_name = db::get_setting(conn, "anilist.user_name")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
    let avatar_url = db::get_setting(conn, "anilist.avatar_url")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
    let last_sync_at = db::get_setting(conn, "anilist.last_sync_at")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<i64>().ok());
    let auto_scrobble = db::get_setting(conn, "pref.anilist_auto_scrobble")
        .ok()
        .flatten()
        .map(|v| v != "0" && v != "false")
        .unwrap_or(true);
    let client_id = db::get_setting(conn, "pref.anilist_client_id")
        .ok()
        .flatten()
        .unwrap_or_default();
    let queued_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM anilist_scrobble_queue", [], |r| r.get(0))
        .unwrap_or(0);
    let tracked_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM anilist_entries", [], |r| r.get(0))
        .unwrap_or(0);

    AnilistAccount {
        connected: token.is_some() && user_id.is_some(),
        user_id,
        user_name,
        avatar_url,
        last_sync_at,
        auto_scrobble,
        queued_count,
        tracked_count,
        client_id,
    }
}

pub fn get_remote_entry(conn: &Connection, anilist_id: i64) -> Option<AnilistRemoteEntry> {
    conn.query_row(
        "SELECT anilist_id, status, progress, score, updated_at FROM anilist_entries WHERE anilist_id = ?1",
        [anilist_id],
        |r| {
            Ok(AnilistRemoteEntry {
                anilist_id: r.get(0)?,
                status: r.get(1)?,
                progress: r.get(2)?,
                score: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    )
    .optional()
    .ok()
    .flatten()
}

pub fn disconnect(conn: &Connection) -> Result<()> {
    for k in [
        "anilist.token",
        "anilist.user_id",
        "anilist.user_name",
        "anilist.avatar_url",
        "anilist.last_sync_at",
    ] {
        conn.execute("DELETE FROM settings WHERE key = ?1", [k])?;
    }
    conn.execute("DELETE FROM anilist_scrobble_queue", [])?;
    crate::backup::mark_dirty();
    Ok(())
}

/// Extract the bare JWT if the user pasted a full redirect URL or `#access_token=...` fragment.
pub fn clean_token(raw: &str) -> String {
    let decoded = raw
        .trim()
        .replace("%3D", "=")
        .replace("%3d", "=")
        .replace("%26", "&")
        .replace("%23", "#");
    if let Ok(re) = regex::Regex::new(r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}") {
        if let Some(m) = re.find(&decoded) {
            return m.as_str().to_string();
        }
    }
    if let Some(idx) = decoded.rfind("access_token=") {
        let rest = &decoded[idx + "access_token=".len()..];
        let token = rest
            .split(['&', ' ', '#', '"', '\''])
            .next()
            .unwrap_or(rest)
            .trim();
        if !token.is_empty() {
            return token.to_string();
        }
    }
    decoded
        .trim_start_matches("Bearer ")
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

pub async fn connect_with_token(app: &AppHandle, st: &Arc<AppState>, raw_token: &str) -> Result<AnilistAccount> {
    let token = clean_token(raw_token);
    if token.is_empty() {
        bail!("Token cannot be empty.");
    }
    if CONNECTING.swap(true, Ordering::SeqCst) {
        return Ok(get_account(&st.db()));
    }
    struct ConnGuard;
    impl Drop for ConnGuard {
        fn drop(&mut self) {
            CONNECTING.store(false, Ordering::SeqCst);
        }
    }
    let _cg = ConnGuard;

    let viewer = match st.providers.anilist_viewer(&token).await {
        Ok(v) => v,
        Err(e) => {
            crate::klog!("[kura] couldn't verify token with AniList: {e}");
            if let Some(w) = app.get_webview_window("anilist-oauth") {
                let _ = w.close();
            }
            let acct = get_account(&st.db());
            let _ = app.emit("anilist-changed", &acct);
            bail!("Couldn't verify token with AniList: {e}");
        }
    };
    let avatar = viewer
        .avatar
        .and_then(|a| a.large.or(a.medium))
        .unwrap_or_default();
    {
        let conn = st.db();
        db::set_setting(&conn, "anilist.token", &token)?;
        db::set_setting(&conn, "anilist.user_id", &viewer.id.to_string())?;
        db::set_setting(&conn, "anilist.user_name", &viewer.name)?;
        db::set_setting(&conn, "anilist.avatar_url", &avatar)?;
    }
    crate::klog!("[kura] connected to AniList as {} (#{})", viewer.name, viewer.id);

    // Close the OAuth popup ONLY after the token has been verified by AniList!
    if let Some(w) = app.get_webview_window("anilist-oauth") {
        let _ = w.close();
    }

    // Emit immediately so the UI shows the connected profile & toast right away!
    let initial_acct = get_account(&st.db());
    let _ = app.emit("anilist-changed", &initial_acct);

    if let Err(e) = sync_now(app, st).await {
        crate::klog!("[kura] initial AniList sync failed: {e}");
    }
    let acct = get_account(&st.db());
    let _ = app.emit("anilist-changed", &acct);
    Ok(acct)
}

const OAUTH_INIT_SCRIPT: &str = r#"
(function() {
  let sent = false;
  const JWT_RE = /eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}/;
  function deliver(raw) {
    if (sent || !raw) return;
    const m = String(raw).match(JWT_RE);
    const token = m ? m[0] : String(raw).replace(/^.*access_token=/, "").split("&")[0].trim();
    if (!token || !token.startsWith("eyJ")) return;
    sent = true;
    try {
      window.location.replace("http://127.0.0.1:14876/oauth/callback?access_token=" + encodeURIComponent(token));
    } catch (_) {}
  }
  function check() {
    if (sent) return;
    const h = window.location.hash || "";
    const s = window.location.search || "";
    if (h.includes("access_token=")) {
      deliver(h);
      return;
    }
    if (s.includes("access_token=") && !window.location.host.startsWith("127.0.0.1")) {
      deliver(s);
      return;
    }
    if (window.location.pathname.includes("/oauth/pin")) {
      const els = document.querySelectorAll("textarea, input, code, pre, .token");
      for (const el of els) {
        const val = (el.value || el.textContent || "").trim();
        const m = val.match(JWT_RE);
        if (m) {
          deliver(m[0]);
          return;
        }
      }
      if (document.body) {
        const txt = document.body.innerText || "";
        const m = txt.match(JWT_RE);
        if (m) {
          deliver(m[0]);
          return;
        }
      }
    }
  }
  window.addEventListener("hashchange", check);
  window.addEventListener("DOMContentLoaded", check);
  window.addEventListener("load", check);
  setInterval(check, 250);
  check();
})();
"#;

/// Start a 1-click OAuth flow: opens an in-app AniList login window (with automatic token capture)
/// plus a local loopback listener so everything completes automatically with zero manual copy/paste.
pub fn start_oauth_flow(app: AppHandle, st: Arc<AppState>, client_id: Option<String>) -> Result<String> {
    let cid = client_id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_CLIENT_ID.to_string());

    let auth_url = format!("https://anilist.co/api/v2/oauth/authorize?client_id={cid}&response_type=token");

    if !OAUTH_LISTENING.swap(true, Ordering::SeqCst) {
        let app_bg = app.clone();
        let st_bg = st.clone();
        std::thread::spawn(move || {
            let res = run_loopback_listener(&app_bg, &st_bg);
            OAUTH_LISTENING.store(false, Ordering::SeqCst);
            if let Err(e) = res {
                crate::klog!("[kura] AniList OAuth loopback listener ended: {e}");
            }
        });
    }

    if let Some(existing) = app.get_webview_window("anilist-oauth") {
        let _ = existing.set_focus();
        return Ok(auth_url);
    }

    if let Ok(parsed) = auth_url.parse::<tauri::Url>() {
        let app_nav = app.clone();
        let st_nav = st.clone();
        let built = WebviewWindowBuilder::new(&app, "anilist-oauth", WebviewUrl::External(parsed))
            .title("Sign in to AniList — Kura")
            .inner_size(520.0, 700.0)
            .resizable(true)
            .center()
            .initialization_script(OAUTH_INIT_SCRIPT)
            .on_navigation(move |url| {
                let full = url.as_str();
                if full.contains("access_token=") {
                    let token = clean_token(full);
                    if !token.is_empty() && token.starts_with("eyJ") {
                        let app_c = app_nav.clone();
                        let st_c = st_nav.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = connect_with_token(&app_c, &st_c, &token).await;
                        });
                    }
                }
                true
            })
            .build();

        match built {
            Ok(win) => {
                let app_ev = app.clone();
                let st_ev = st.clone();
                win.on_window_event(move |ev| {
                    if matches!(ev, tauri::WindowEvent::Destroyed) {
                        let acct = get_account(&st_ev.db());
                        let _ = app_ev.emit("anilist-changed", &acct);
                    }
                });
            }
            Err(e) => {
                crate::klog!("[kura] couldn't open in-app AniList login window ({e}), opening system browser");
                let _ = tauri_plugin_opener::open_url(&auth_url, None::<&str>);
            }
        }
    } else {
        let _ = tauri_plugin_opener::open_url(&auth_url, None::<&str>);
    }

    Ok(auth_url)
}

fn run_loopback_listener(app: &AppHandle, st: &Arc<AppState>) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:14876")
        .or_else(|_| TcpListener::bind("127.0.0.1:14877"))
        .context("couldn't bind loopback port 14876/14877")?;
    listener.set_nonblocking(true)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(180);

    while std::time::Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                let mut buf = [0u8; 16384];
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    continue;
                }
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");

                if first_line.starts_with("POST /oauth/token") || first_line.contains("access_token=") {
                    let body = if first_line.contains("access_token=") {
                        first_line.to_string()
                    } else {
                        req.split("\r\n\r\n").nth(1).unwrap_or("").trim().to_string()
                    };
                    let token = clean_token(&body);
                    let done_html = r#"<!doctype html><html><head><meta charset="utf-8"><title>Kura · AniList</title>
<style>body{background:#181012;color:#f7ede2;font-family:system-ui,sans-serif;display:grid;place-items:center;height:100vh;margin:0}
.card{background:#23181b;border:2.5px solid #140c0d;border-radius:18px;padding:32px 36px;box-shadow:5px 5px 0 #f76d4d;text-align:center;max-width:420px}
h1{margin:0 0 10px;color:#f76d4d;font-size:24px}p{margin:0;color:#cdbab0;line-height:1.5}</style></head>
<body><div class="card"><h1>Connected to Kura!</h1><p>Finishing sign-in and syncing your AniList…</p></div></body></html>"#;
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        done_html.len(),
                        done_html
                    );
                    let _ = stream.write_all(reply.as_bytes());
                    if !token.is_empty() && token.starts_with("eyJ") {
                        let app_c = app.clone();
                        let st_c = st.clone();
                        tauri::async_runtime::spawn(async move {
                            let _ = connect_with_token(&app_c, &st_c, &token).await;
                        });
                        return Ok(());
                    }
                } else {
                    // Serve the HTML page that reads `window.location.hash` and posts it back to `/oauth/token`.
                    let html = r#"<!doctype html><html><head><meta charset="utf-8"><title>Kura · AniList</title>
<style>body{background:#181012;color:#f7ede2;font-family:system-ui,sans-serif;display:grid;place-items:center;height:100vh;margin:0}
.card{background:#23181b;border:2.5px solid #140c0d;border-radius:18px;padding:32px 36px;box-shadow:5px 5px 0 #f76d4d;text-align:center;max-width:420px}
h1{margin:0 0 10px;color:#f76d4d;font-size:24px}p{margin:0;color:#cdbab0;line-height:1.5}</style></head>
<body><div class="card"><h1 id="h">Connecting to Kura…</h1><p id="p">Finishing AniList sign-in.</p></div>
<script>
const h = location.hash.slice(1) || location.search.slice(1);
if (h.includes("access_token=")) {
  fetch("/oauth/token", { method: "POST", body: h }).then(() => {
    document.getElementById("h").textContent = "Connected to Kura!";
    document.getElementById("p").textContent = "You can close this tab and return to Kura.";
  });
} else {
  document.getElementById("h").textContent = "No token found in URL";
  document.getElementById("p").textContent = "Copy the token from AniList and paste it into Kura Settings → Integrations.";
}
</script></body></html>"#;
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        html.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(html.as_bytes());
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Compute local regular watched episode progress for `anilist_id`.
pub fn local_watched_progress(conn: &Connection, anilist_id: i64) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM watch_state WHERE anilist_id = ?1 AND ep_key NOT LIKE 'S%' AND ep_key NOT LIKE 'extra:%'",
        [anilist_id],
        |r| r.get(0),
    )
    .unwrap_or(0)
}

/// Decide the target AniList status and progress when merging local and remote state.
/// Never regresses progress on either side.
pub fn decide_merge(
    local_watched: i64,
    total_episodes: Option<i64>,
    remote_progress: Option<i64>,
    remote_status: Option<&str>,
    owned_in_library: bool,
) -> (i64, Option<String>, bool, bool) {
    let rem_prog = remote_progress.unwrap_or(0);
    let merged_prog = local_watched.max(rem_prog);
    let pull_to_local = rem_prog > local_watched;
    let push_to_remote = local_watched > rem_prog && (owned_in_library || remote_progress.is_some());

    let new_status = if push_to_remote {
        if let Some(total) = total_episodes.filter(|&t| t > 0) {
            if merged_prog >= total {
                Some("COMPLETED".to_string())
            } else if matches!(remote_status, None | Some("PLANNING")) {
                Some("CURRENT".to_string())
            } else {
                None
            }
        } else if matches!(remote_status, None | Some("PLANNING")) {
            Some("CURRENT".to_string())
        } else {
            None
        }
    } else {
        None
    };

    (merged_prog, new_status, pull_to_local, push_to_remote)
}

/// Apply remote progress to local `watch_state` without regressing or overwriting existing timestamps.
pub fn apply_remote_progress_to_local(conn: &Connection, anilist_id: i64, remote_progress: i64) -> Result<usize> {
    if remote_progress <= 0 {
        return Ok(0);
    }
    let ts = now();
    let mut added = 0usize;
    for n in 1..=remote_progress {
        let changed = conn.execute(
            "INSERT OR IGNORE INTO watch_state (anilist_id, ep_key, watched_at) VALUES (?1, ?2, ?3)",
            params![anilist_id, n.to_string(), ts],
        )?;
        added += changed;
    }
    Ok(added)
}

/// Enqueue a scrobble when an episode is marked watched locally, and trigger an async flush.
pub fn on_episode_watched(app: &AppHandle, st: &Arc<AppState>, anilist_id: i64) {
    let should_flush = {
        let conn = st.db();
        let acct = get_account(&conn);
        if !acct.connected || !acct.auto_scrobble {
            return;
        }
        let local_prog = local_watched_progress(&conn, anilist_id);
        if local_prog <= 0 {
            return;
        }
        let total_eps: Option<i64> = conn
            .query_row("SELECT episodes FROM media WHERE anilist_id = ?1", [anilist_id], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
            .flatten();
        let remote = get_remote_entry(&conn, anilist_id);
        let (merged_prog, status, _, push) = decide_merge(
            local_prog,
            total_eps,
            remote.as_ref().map(|r| r.progress),
            remote.as_ref().map(|r| r.status.as_str()),
            true,
        );
        if !push {
            return;
        }
        let effective_status = status
            .clone()
            .or_else(|| remote.as_ref().map(|r| r.status.clone()))
            .unwrap_or_else(|| "CURRENT".to_string());
        let _ = conn.execute(
            "INSERT INTO anilist_entries (anilist_id, entry_id, status, progress, score, updated_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5)
             ON CONFLICT(anilist_id) DO UPDATE SET
               status = excluded.status,
               progress = excluded.progress,
               updated_at = excluded.updated_at",
            params![
                anilist_id,
                effective_status,
                merged_prog,
                remote.and_then(|r| r.score),
                now()
            ],
        );
        let _ = conn.execute(
            "INSERT INTO anilist_scrobble_queue (anilist_id, progress, status, queued_at, last_error)
             VALUES (?1, ?2, ?3, ?4, NULL)
             ON CONFLICT(anilist_id) DO UPDATE SET
               progress = MAX(anilist_scrobble_queue.progress, excluded.progress),
               status = COALESCE(excluded.status, anilist_scrobble_queue.status),
               queued_at = excluded.queued_at",
            params![anilist_id, merged_prog, status, now()],
        );
        true
    };
    if should_flush {
        let app_c = app.clone();
        let st_c = st.clone();
        tauri::async_runtime::spawn(async move {
            let _ = flush_queue(&app_c, &st_c).await;
        });
    }
}

/// Directly update an anime's AniList list status, episode progress, or score from the detail page.
/// Works for both local library shows and external/seasonal shows not downloaded locally.
pub async fn update_entry(
    app: &AppHandle,
    st: &Arc<AppState>,
    anilist_id: i64,
    status: Option<String>,
    progress: Option<i64>,
    score: Option<f64>,
) -> Result<AnilistRemoteEntry> {
    let (token, mut target_status, mut target_progress, target_score) = {
        let conn = st.db();
        let _ = conn.execute_batch(db::SCHEMA_V5);
        let token = db::get_setting(&conn, "anilist.token")?.filter(|s| !s.trim().is_empty());
        let existing = get_remote_entry(&conn, anilist_id);
        let total_eps: Option<i64> = conn
            .query_row("SELECT episodes FROM media WHERE anilist_id = ?1", [anilist_id], |r| r.get(0))
            .optional()?
            .flatten();

        let local_prog = local_watched_progress(&conn, anilist_id);
        let cur_prog = existing.as_ref().map(|e| e.progress).unwrap_or(local_prog);
        let mut p = progress.unwrap_or(cur_prog).max(0);
        if let Some(t) = total_eps.filter(|&t| t > 0) {
            if status.as_deref() == Some("COMPLETED") && progress.is_none() {
                p = t;
            } else {
                p = p.min(t);
            }
        }

        let mut s = status
            .clone()
            .or_else(|| existing.as_ref().map(|e| e.status.clone()))
            .unwrap_or_else(|| if p > 0 { "CURRENT".to_string() } else { "PLANNING".to_string() });

        if status.is_none() && progress.is_some() {
            if let Some(t) = total_eps.filter(|&t| t > 0) {
                if p >= t {
                    s = "COMPLETED".to_string();
                } else if p > 0 && s == "PLANNING" {
                    s = "CURRENT".to_string();
                }
            } else if p > 0 && s == "PLANNING" {
                s = "CURRENT".to_string();
            }
        }

        let sc = score.or_else(|| existing.as_ref().and_then(|e| e.score));

        // Keep local watch_state synchronized with episode progress.
        let _ = apply_remote_progress_to_local(&conn, anilist_id, p);
        let _ = conn.execute(
            "DELETE FROM watch_state
             WHERE anilist_id = ?1
               AND ep_key NOT LIKE 'S%'
               AND ep_key NOT LIKE 'extra:%'
               AND CAST(ep_key AS INTEGER) > ?2",
            params![anilist_id, p],
        );

        let ts = now();
        conn.execute(
            "INSERT INTO anilist_entries (anilist_id, entry_id, status, progress, score, updated_at)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5)
             ON CONFLICT(anilist_id) DO UPDATE SET
               status = excluded.status,
               progress = excluded.progress,
               score = excluded.score,
               updated_at = excluded.updated_at",
            params![anilist_id, s, p, sc, ts],
        )?;

        (token, s, p, sc)
    };

    if let Some(tok) = token {
        match st
            .providers
            .anilist_save_entry_full(&tok, anilist_id, Some(&target_status), Some(target_progress), target_score)
            .await
        {
            Ok(saved) => {
                target_status = saved.status;
                target_progress = saved.progress;
                let conn = st.db();
                let _ = conn.execute(
                    "INSERT INTO anilist_entries (anilist_id, entry_id, status, progress, score, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(anilist_id) DO UPDATE SET
                       entry_id = excluded.entry_id,
                       status = excluded.status,
                       progress = excluded.progress,
                       score = COALESCE(excluded.score, anilist_entries.score),
                       updated_at = excluded.updated_at",
                    params![
                        saved.media_id,
                        saved.id,
                        target_status,
                        target_progress,
                        saved.score.or(target_score),
                        saved.updated_at.unwrap_or_else(now)
                    ],
                );
                let _ = conn.execute("DELETE FROM anilist_scrobble_queue WHERE anilist_id = ?1", [anilist_id]);
            }
            Err(e) => {
                crate::klog!("[kura] AniList update queued offline (#{anilist_id}): {e}");
                let conn = st.db();
                let _ = conn.execute(
                    "INSERT INTO anilist_scrobble_queue (anilist_id, progress, status, queued_at, last_error)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(anilist_id) DO UPDATE SET
                       progress = excluded.progress,
                       status = excluded.status,
                       queued_at = excluded.queued_at,
                       last_error = excluded.last_error",
                    params![anilist_id, target_progress, target_status, now(), e.to_string()],
                );
            }
        }
    }

    crate::backup::mark_dirty();
    let _ = app.emit("library-changed", ());
    let acct = get_account(&st.db());
    let _ = app.emit("anilist-changed", &acct);

    Ok(get_remote_entry(&st.db(), anilist_id).unwrap_or(AnilistRemoteEntry {
        anilist_id,
        status: target_status,
        progress: target_progress,
        score: target_score,
        updated_at: now(),
    }))
}

/// Flush pending scrobbles in `anilist_scrobble_queue` to AniList. Keeps failed items in the queue for when back online.
pub async fn flush_queue(app: &AppHandle, st: &Arc<AppState>) -> Result<usize> {
    if FLUSHING.swap(true, Ordering::SeqCst) {
        return Ok(0);
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            FLUSHING.store(false, Ordering::SeqCst);
        }
    }
    let _g = Guard;

    let (token, items): (String, Vec<(i64, i64, Option<String>)>) = {
        let conn = st.db();
        let Some(token) = db::get_setting(&conn, "anilist.token")?.filter(|s| !s.is_empty()) else {
            return Ok(0);
        };
        let mut stmt = conn.prepare(
            "SELECT anilist_id, progress, status FROM anilist_scrobble_queue ORDER BY queued_at ASC",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .flatten()
            .collect();
        (token, rows)
    };

    if items.is_empty() {
        return Ok(0);
    }

    let mut flushed = 0usize;
    for (anilist_id, progress, status) in items {
        // Never regress if remote entry was updated to a higher progress in the meantime.
        let remote_prog = get_remote_entry(&st.db(), anilist_id).map(|r| r.progress).unwrap_or(0);
        if progress <= remote_prog {
            let _ = st.db().execute("DELETE FROM anilist_scrobble_queue WHERE anilist_id = ?1", [anilist_id]);
            continue;
        }
        match st
            .providers
            .anilist_save_entry(&token, anilist_id, progress, status.as_deref())
            .await
        {
            Ok(saved) => {
                let conn = st.db();
                let _ = conn.execute(
                    "INSERT INTO anilist_entries (anilist_id, entry_id, status, progress, score, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(anilist_id) DO UPDATE SET
                       entry_id = excluded.entry_id,
                       status = excluded.status,
                       progress = excluded.progress,
                       score = COALESCE(excluded.score, anilist_entries.score),
                       updated_at = excluded.updated_at",
                    params![
                        saved.media_id,
                        saved.id,
                        saved.status,
                        saved.progress,
                        saved.score,
                        saved.updated_at.unwrap_or_else(now)
                    ],
                );
                let _ = conn.execute("DELETE FROM anilist_scrobble_queue WHERE anilist_id = ?1", [anilist_id]);
                crate::klog!("[kura] scrobbled AniList #{anilist_id} → ep {progress}");
                flushed += 1;
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = st.db().execute(
                    "UPDATE anilist_scrobble_queue SET last_error = ?2 WHERE anilist_id = ?1",
                    params![anilist_id, msg],
                );
                crate::klog!("[kura] AniList scrobble queued for retry (#{anilist_id}): {msg}");
                break; // likely offline or rate-limited; retry later
            }
        }
    }

    if flushed > 0 {
        let acct = get_account(&st.db());
        let _ = app.emit("anilist-changed", &acct);
    }
    Ok(flushed)
}

/// Full two-way synchronization between Kura and AniList.
pub async fn sync_now(app: &AppHandle, st: &Arc<AppState>) -> Result<AnilistSyncReport> {
    let (token, user_id) = {
        let conn = st.db();
        let token = db::get_setting(&conn, "anilist.token")?
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("AniList is not connected."))?;
        let user_id = db::get_setting(&conn, "anilist.user_id")?
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| anyhow::anyhow!("AniList user ID missing."))?;
        (token, user_id)
    };

    let flushed_queue = flush_queue(app, st).await.unwrap_or(0);
    let remote_list = st.providers.anilist_user_list(&token, user_id).await?;
    let total_remote = remote_list.len();

    let mut remote_by_media: HashMap<i64, crate::providers::AlListEntry> = HashMap::new();
    for entry in remote_list {
        // If user has multiple lists containing the same media, keep the one with highest progress.
        match remote_by_media.get(&entry.media_id) {
            Some(prev) if prev.progress > entry.progress => {}
            _ => {
                remote_by_media.insert(entry.media_id, entry);
            }
        }
    }

    // Save remote snapshot and compare against local library.
    let (mut pulled_series, mut pulled_episodes, to_push) = {
        let mut conn = st.db();
        let tx = conn.transaction()?;
        for e in remote_by_media.values() {
            tx.execute(
                "INSERT INTO anilist_entries (anilist_id, entry_id, status, progress, score, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(anilist_id) DO UPDATE SET
                   entry_id = excluded.entry_id,
                   status = excluded.status,
                   progress = excluded.progress,
                   score = excluded.score,
                   updated_at = excluded.updated_at",
                params![
                    e.media_id,
                    e.id,
                    e.status,
                    e.progress,
                    e.score,
                    e.updated_at.unwrap_or_else(now)
                ],
            )?;
        }

        // Collect all local media IDs (owned files or local watch state).
        let mut owned_ids: HashSet<i64> = HashSet::new();
        {
            let mut stmt = tx.prepare("SELECT DISTINCT anilist_id FROM local_files WHERE anilist_id IS NOT NULL")?;
            for id in stmt.query_map([], |r| r.get::<_, i64>(0))?.flatten() {
                owned_ids.insert(id);
            }
        }
        let mut all_ids: HashSet<i64> = owned_ids.clone();
        all_ids.extend(remote_by_media.keys().copied());

        let mut pulled_s = 0usize;
        let mut pulled_eps = 0usize;
        let mut push_list: Vec<(i64, i64, Option<String>)> = Vec::new();

        for id in all_ids {
            let local_prog = local_watched_progress(&tx, id);
            let total_eps: Option<i64> = tx
                .query_row("SELECT episodes FROM media WHERE anilist_id = ?1", [id], |r| r.get(0))
                .optional()?
                .flatten();
            let rem = remote_by_media.get(&id);
            let (merged_prog, status, pull, push) = decide_merge(
                local_prog,
                total_eps,
                rem.map(|r| r.progress),
                rem.map(|r| r.status.as_str()),
                owned_ids.contains(&id),
            );
            if pull && owned_ids.contains(&id) {
                let added = apply_remote_progress_to_local(&tx, id, merged_prog)?;
                if added > 0 {
                    pulled_s += 1;
                    pulled_eps += added;
                }
            } else if push {
                push_list.push((id, merged_prog, status));
            }
        }

        tx.commit()?;
        (pulled_s, pulled_eps, push_list)
    };

    let mut pushed_series = 0usize;
    for (anilist_id, prog, status) in to_push {
        match st
            .providers
            .anilist_save_entry(&token, anilist_id, prog, status.as_deref())
            .await
        {
            Ok(saved) => {
                let conn = st.db();
                let _ = conn.execute(
                    "INSERT INTO anilist_entries (anilist_id, entry_id, status, progress, score, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(anilist_id) DO UPDATE SET
                       entry_id = excluded.entry_id,
                       status = excluded.status,
                       progress = excluded.progress,
                       score = COALESCE(excluded.score, anilist_entries.score),
                       updated_at = excluded.updated_at",
                    params![
                        saved.media_id,
                        saved.id,
                        saved.status,
                        saved.progress,
                        saved.score,
                        saved.updated_at.unwrap_or_else(now)
                    ],
                );
                pushed_series += 1;
            }
            Err(e) => {
                // Queue for retry so nothing is lost.
                let _ = st.db().execute(
                    "INSERT INTO anilist_scrobble_queue (anilist_id, progress, status, queued_at, last_error)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(anilist_id) DO UPDATE SET
                       progress = MAX(anilist_scrobble_queue.progress, excluded.progress),
                       last_error = excluded.last_error",
                    params![anilist_id, prog, status, now(), e.to_string()],
                );
            }
        }
    }

    {
        let conn = st.db();
        let _ = db::set_setting(&conn, "anilist.last_sync_at", &now().to_string());
    }
    if pulled_episodes > 0 {
        crate::backup::mark_dirty();
        let _ = app.emit("library-changed", ());
    }
    let acct = get_account(&st.db());
    let _ = app.emit("anilist-changed", &acct);

    // Ensure pulmonary unused mut warning doesn't fire
    let _ = (&mut pulled_series, &mut pulled_episodes);

    Ok(AnilistSyncReport {
        pulled_series,
        pulled_episodes,
        pushed_series,
        flushed_queue,
        total_remote,
    })
}

/// Background worker: retries offline scrobbles every 60s and fires airing notifications.
pub fn start_worker(app: AppHandle, st: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let _ = flush_queue(&app, &st).await;
            check_airing_notifications(&app, &st);
        }
    });
}

fn check_airing_notifications(app: &AppHandle, st: &Arc<AppState>) {
    let conn = st.db();
    let enabled = db::get_setting(&conn, "pref.airing_notifications")
        .ok()
        .flatten()
        .map(|v| v != "0" && v != "false")
        .unwrap_or(true);
    if !enabled {
        return;
    }
    let ts = now();
    // Shows in the user's library (or AniListWatching) whose next episode aired within the last 30 minutes.
    let Ok(mut stmt) = conn.prepare(
        "SELECT m.anilist_id, COALESCE(m.title_english, m.title_romaji), m.next_airing_episode, m.next_airing_at
         FROM media m
         WHERE m.next_airing_episode IS NOT NULL
           AND m.next_airing_at IS NOT NULL
           AND m.next_airing_at <= ?1
           AND m.next_airing_at >= ?2
           AND (
             EXISTS(SELECT 1 FROM local_files f WHERE f.anilist_id = m.anilist_id)
             OR EXISTS(SELECT 1 FROM anilist_entries a WHERE a.anilist_id = m.anilist_id AND a.status IN ('CURRENT', 'PLANNING'))
           )",
    ) else {
        return;
    };
    let rows: Vec<(i64, Option<String>, i64, i64)> = stmt
        .query_map(params![ts, ts - 1800], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .collect();

    for (id, title, ep, _) in rows {
        let key = format!("airing.notified.{id}.{ep}");
        if db::get_setting(&conn, &key).ok().flatten().is_some() {
            continue;
        }
        let _ = db::set_setting(&conn, &key, &ts.to_string());
        let name = title.unwrap_or_else(|| format!("Anime #{id}"));
        let _ = app.emit(
            "airing-notification",
            serde_json::json!({
                "anilistId": id,
                "title": name,
                "episode": ep,
                "message": format!("Episode {ep} of {name} just aired!")
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_tokens_and_fragments() {
        assert_eq!(clean_token("  abc.def.ghi  "), "abc.def.ghi");
        assert_eq!(clean_token("Bearer my_token_123"), "my_token_123");
        assert_eq!(
            clean_token("http://localhost:14876/oauth/callback#access_token=tok_xyz&token_type=Bearer&expires_in=31536000"),
            "tok_xyz"
        );
    }

    #[test]
    fn safe_merge_never_regresses_either_side() {
        // Remote ahead (8 vs 3 locally): pull to local, don't push.
        let (prog, status, pull, push) = decide_merge(3, Some(12), Some(8), Some("CURRENT"), true);
        assert_eq!(prog, 8);
        assert!(pull);
        assert!(!push);
        assert!(status.is_none());

        // Local ahead (6 vs 2 remotely): push to remote, don't pull.
        let (prog, status, pull, push) = decide_merge(6, Some(12), Some(2), Some("PLANNING"), true);
        assert_eq!(prog, 6);
        assert!(!pull);
        assert!(push);
        assert_eq!(status.as_deref(), Some("CURRENT"));

        // Local finishes the show (12 of 12 vs 11 remotely): push COMPLETED.
        let (prog, status, pull, push) = decide_merge(12, Some(12), Some(11), Some("CURRENT"), true);
        assert_eq!(prog, 12);
        assert!(!pull);
        assert!(push);
        assert_eq!(status.as_deref(), Some("COMPLETED"));

        // Equal progress: neither pull nor push.
        let (prog, _, pull, push) = decide_merge(5, Some(12), Some(5), Some("CURRENT"), true);
        assert_eq!(prog, 5);
        assert!(!pull);
        assert!(!push);
    }

    #[test]
    fn remote_progress_marks_local_episodes_idempotently() {
        let conn = Connection::open_in_memory().unwrap();
        db::migrate_for_tests(&conn).unwrap();
        conn.execute("INSERT INTO watch_state (anilist_id, ep_key, watched_at) VALUES (100, '1', 1000)", [])
            .unwrap();
        let added = apply_remote_progress_to_local(&conn, 100, 4).unwrap();
        assert_eq!(added, 3); // episodes 2, 3, 4 added; episode 1 kept
        assert_eq!(local_watched_progress(&conn, 100), 4);
        let ep1_ts: i64 = conn
            .query_row("SELECT watched_at FROM watch_state WHERE anilist_id = 100 AND ep_key = '1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ep1_ts, 1000);
    }
}

