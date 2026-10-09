//! Discord Rich Presence integration via local IPC socket / named pipe.
//!
//! Connects directly to Discord's local IPC interface:
//! - Windows: `\\.\pipe\discord-ipc-0` through `9`
//! - Linux/macOS: `$XDG_RUNTIME_DIR/discord-ipc-0`, Flatpak Discord paths, `/tmp/...`
//!
//! Uses Activity Type 3 (`Watching`) so user profiles display:
//!   WATCHING
//!   Kura
//!
//! Thread-safe, non-blocking, and handles automatic reconnection if Discord restarts.

use anyhow::{anyhow, Result};
use serde_json::json;
#[cfg(unix)]
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, WriteHalf};
use tokio::sync::mpsc;

pub const DEFAULT_CLIENT_ID: &str = "1558103009396002816";

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;
const OP_CLOSE: u32 = 2;

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

#[derive(Debug, Clone)]
pub struct ActivityPayload {
    pub client_id: String,
    pub title: String,
    pub episode_text: String,
    pub cover_url: Option<String>,
    pub anilist_id: Option<i64>,
    pub is_playing: bool,
    pub start_timestamp: Option<i64>,
    pub end_timestamp: Option<i64>,
    pub spoiler_protection: bool,
    pub show_buttons: bool,
}

enum DiscordCommand {
    Update(Box<ActivityPayload>),
    Clear,
}

#[derive(Clone)]
pub struct DiscordHandle {
    tx: mpsc::UnboundedSender<DiscordCommand>,
}

impl DiscordHandle {
    pub fn update(&self, activity: ActivityPayload) {
        let _ = self.tx.send(DiscordCommand::Update(Box::new(activity)));
    }

    pub fn clear(&self) {
        let _ = self.tx.send(DiscordCommand::Clear);
    }
}

pub fn start_service() -> DiscordHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(discord_actor(rx));
    DiscordHandle { tx }
}

#[cfg(windows)]
async fn connect_socket() -> Option<Box<dyn Io>> {
    for i in 0..10 {
        let name = format!(r"\\.\pipe\discord-ipc-{i}");
        if let Ok(client) = tokio::net::windows::named_pipe::ClientOptions::new().open(&name) {
            return Some(Box::new(client));
        }
    }
    None
}

#[cfg(unix)]
async fn connect_socket() -> Option<Box<dyn Io>> {
    let mut candidates = Vec::new();
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        let xdg = PathBuf::from(xdg);
        for i in 0..10 {
            candidates.push(xdg.join(format!("discord-ipc-{i}")));
            candidates.push(xdg.join(format!("app/com.discordapp.Discord/discord-ipc-{i}")));
            candidates.push(xdg.join(format!("app/dev.vencord.Vesktop/discord-ipc-{i}")));
        }
    }
    let tmp = std::env::temp_dir();
    for i in 0..10 {
        candidates.push(tmp.join(format!("discord-ipc-{i}")));
        candidates.push(PathBuf::from(format!("/tmp/discord-ipc-{i}")));
    }

    for path in candidates {
        if path.exists() {
            if let Ok(stream) = tokio::net::UnixStream::connect(&path).await {
                return Some(Box::new(stream));
            }
        }
    }
    None
}

#[cfg(not(any(windows, unix)))]
async fn connect_socket() -> Option<Box<dyn Io>> {
    None
}

async fn write_packet<W: AsyncWrite + Unpin + ?Sized>(tx: &mut W, opcode: u32, payload: &serde_json::Value) -> Result<()> {
    let body = serde_json::to_vec(payload)?;
    let len = body.len() as u32;
    tx.write_all(&opcode.to_le_bytes()).await?;
    tx.write_all(&len.to_le_bytes()).await?;
    tx.write_all(&body).await?;
    tx.flush().await?;
    Ok(())
}

async fn read_packet<R: AsyncRead + Unpin + ?Sized>(rx: &mut R) -> Result<(u32, serde_json::Value)> {
    let mut header = [0u8; 8];
    rx.read_exact(&mut header).await?;
    let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if len > 64 * 1024 {
        return Err(anyhow!("Discord packet too large ({len} bytes)"));
    }
    let mut body = vec![0u8; len];
    rx.read_exact(&mut body).await?;
    let val: serde_json::Value = serde_json::from_slice(&body)?;
    Ok((opcode, val))
}

fn sanitize_len(s: &str, min: usize, max: usize) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let count = trimmed.chars().count();
    if count < min {
        let mut padded = trimmed.to_string();
        while padded.chars().count() < min {
            padded.push(' ');
        }
        Some(padded)
    } else if count > max {
        let truncated: String = trimmed.chars().take(max).collect();
        Some(truncated)
    } else {
        Some(trimmed.to_string())
    }
}

fn build_activity_json(act: &ActivityPayload) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    // Type 3 = Watching!
    map.insert("type".into(), json!(3));

    // Details: Anime title
    if let Some(d) = sanitize_len(&act.title, 2, 128) {
        map.insert("details".into(), json!(d));
    }

    // State: Episode info or "Paused"
    let state_text = if act.spoiler_protection {
        if act.is_playing {
            act.episode_text.split("—").next().unwrap_or(&act.episode_text).trim().to_string()
        } else {
            "Paused".to_string()
        }
    } else if act.is_playing {
        act.episode_text.clone()
    } else {
        format!("Paused · {}", act.episode_text)
    };

    if let Some(s) = sanitize_len(&state_text, 2, 128) {
        map.insert("state".into(), json!(s));
    }

    // Timestamps: live start/end progress bar
    if act.is_playing {
        let mut ts = serde_json::Map::new();
        if let Some(start) = act.start_timestamp {
            ts.insert("start".into(), json!(start));
        }
        if let Some(end) = act.end_timestamp {
            ts.insert("end".into(), json!(end));
        }
        if !ts.is_empty() {
            map.insert("timestamps".into(), serde_json::Value::Object(ts));
        }
    }

    // Assets: Cover artwork + Kura badge
    let mut assets = serde_json::Map::new();
    if let Some(ref cover) = act.cover_url {
        if cover.starts_with("http://") || cover.starts_with("https://") {
            assets.insert("large_image".into(), json!(cover));
            if let Some(t) = sanitize_len(&act.title, 2, 128) {
                assets.insert("large_text".into(), json!(t));
            }
            assets.insert("small_image".into(), json!("kura_logo"));
            assets.insert("small_text".into(), json!("Kura"));
        } else {
            assets.insert("large_image".into(), json!("kura_logo"));
            assets.insert("large_text".into(), json!("Kura"));
        }
    } else {
        assets.insert("large_image".into(), json!("kura_logo"));
        assets.insert("large_text".into(), json!("Kura"));
    }
    map.insert("assets".into(), serde_json::Value::Object(assets));

    // Buttons: View on AniList & Get Kura on GitHub
    if act.show_buttons {
        let mut buttons = Vec::new();
        if let Some(id) = act.anilist_id {
            if id > 0 {
                buttons.push(json!({
                    "label": "View on AniList",
                    "url": format!("https://anilist.co/anime/{id}")
                }));
            }
        }
        buttons.push(json!({
            "label": "Get Kura on GitHub",
            "url": "https://github.com/wantingCat/kura"
        }));
        map.insert("buttons".into(), json!(buttons));
    }

    serde_json::Value::Object(map)
}

fn build_frame_payload(activity: Option<&ActivityPayload>, nonce: &str) -> serde_json::Value {
    let act_val = match activity {
        Some(a) => build_activity_json(a),
        None => serde_json::Value::Null,
    };
    json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": std::process::id(),
            "activity": act_val
        },
        "nonce": nonce
    })
}

struct DiscordConnection {
    tx: WriteHalf<Box<dyn Io>>,
    client_id: String,
    nonce: u64,
}

impl DiscordConnection {
    async fn try_connect(client_id: &str) -> Result<Self> {
        let io = connect_socket().await.ok_or_else(|| anyhow!("No Discord IPC socket found"))?;
        let (mut rx, mut tx) = tokio::io::split(io);

        // Handshake
        let handshake_payload = json!({
            "v": 1,
            "client_id": client_id
        });
        write_packet(&mut tx, OP_HANDSHAKE, &handshake_payload).await?;

        // Read READY frame with timeout
        let (op, resp) = tokio::time::timeout(Duration::from_secs(3), read_packet(&mut rx))
            .await
            .map_err(|_| anyhow!("Discord handshake timeout"))??;

        if op == OP_CLOSE {
            return Err(anyhow!("Discord rejected handshake: {resp:?}"));
        }

        // Spawn background reader to continuously drain incoming frames and avoid pipe buffer blockage
        tokio::spawn(async move {
            while let Ok((op, _)) = read_packet(&mut rx).await {
                if op == OP_CLOSE {
                    break;
                }
            }
        });

        Ok(Self {
            tx,
            client_id: client_id.to_string(),
            nonce: 1,
        })
    }

    async fn send_activity(&mut self, activity: Option<&ActivityPayload>) -> Result<()> {
        self.nonce += 1;
        let nonce_str = format!("kura-{}", self.nonce);
        let payload = build_frame_payload(activity, &nonce_str);
        write_packet(&mut self.tx, OP_FRAME, &payload).await
    }
}

async fn discord_actor(mut rx: mpsc::UnboundedReceiver<DiscordCommand>) {
    let mut current_activity: Option<ActivityPayload> = None;
    let mut conn: Option<DiscordConnection> = None;
    let mut last_reconnect_attempt = std::time::Instant::now() - Duration::from_secs(10);

    loop {
        // Wait for incoming command with periodic reconnect interval when we have active activity
        let cmd = if conn.is_none() && current_activity.is_some() {
            tokio::select! {
                c = rx.recv() => c,
                _ = tokio::time::sleep(Duration::from_secs(5)) => None,
            }
        } else {
            rx.recv().await
        };

        if let Some(cmd) = cmd {
            match cmd {
                DiscordCommand::Update(act) => {
                    current_activity = Some(*act);
                }
                DiscordCommand::Clear => {
                    current_activity = None;
                    if let Some(ref mut c) = conn {
                        let _ = c.send_activity(None).await;
                    }
                    continue;
                }
            }
        }

        // Check if we need to establish or re-establish connection
        if let Some(ref act) = current_activity {
            // Need reconnect if no connection or client_id changed
            let needs_connect = match &conn {
                None => last_reconnect_attempt.elapsed() > Duration::from_secs(4),
                Some(c) => c.client_id != act.client_id,
            };

            if needs_connect {
                last_reconnect_attempt = std::time::Instant::now();
                match DiscordConnection::try_connect(&act.client_id).await {
                    Ok(mut new_conn) => {
                        if new_conn.send_activity(Some(act)).await.is_ok() {
                            conn = Some(new_conn);
                        }
                    }
                    Err(_) => {
                        conn = None;
                    }
                }
            } else if let Some(ref mut c) = conn {
                if c.send_activity(Some(act)).await.is_err() {
                    conn = None;
                }
            }
        }
    }
}
