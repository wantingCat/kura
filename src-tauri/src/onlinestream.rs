//! Direct soft-sub web streaming backend:
//! - Ephemeral `127.0.0.1` HTTP stream proxy that injects `Referer`/`Origin`/`Cookie`/`User-Agent`
//!   headers and rewrites `.m3u8` master/media playlists (including disguised `#EXTM3U` responses,
//!   relative paths, `#EXT-X-KEY`, `#EXT-X-MAP`, and `#EXT-X-MEDIA` `URI="..."` attributes).
//! - Manual AniList -> provider anime mapping overrides (`onlinestream_mappings` table).

use crate::db;
use crate::extensions::{base64_decode, base64_encode};
use crate::klog;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, RANGE, USER_AGENT};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const DEFAULT_STREAM_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

// ---------------------------------------------------------------------------
// Manual AniList -> Provider Mapping Overrides
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlinestreamMapping {
    pub provider_id: String,
    pub anilist_id: i64,
    pub provider_anime_id: String,
    pub provider_title: Option<String>,
    pub updated_at: i64,
}

pub fn get_mapping(
    conn: &Connection,
    provider_id: &str,
    anilist_id: i64,
) -> Result<Option<OnlinestreamMapping>> {
    let _ = conn.execute_batch(db::SCHEMA_V9);
    conn.query_row(
        "SELECT provider_id, anilist_id, provider_anime_id, provider_title, updated_at
         FROM onlinestream_mappings
         WHERE provider_id = ?1 AND anilist_id = ?2",
        params![provider_id, anilist_id],
        |r| {
            Ok(OnlinestreamMapping {
                provider_id: r.get(0)?,
                anilist_id: r.get(1)?,
                provider_anime_id: r.get(2)?,
                provider_title: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn set_mapping(
    conn: &Connection,
    provider_id: &str,
    anilist_id: i64,
    provider_anime_id: &str,
    provider_title: Option<&str>,
) -> Result<OnlinestreamMapping> {
    let _ = conn.execute_batch(db::SCHEMA_V9);
    let ts = db::now();
    conn.execute(
        "INSERT INTO onlinestream_mappings
         (provider_id, anilist_id, provider_anime_id, provider_title, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(provider_id, anilist_id) DO UPDATE SET
            provider_anime_id = excluded.provider_anime_id,
            provider_title = excluded.provider_title,
            updated_at = excluded.updated_at",
        params![provider_id, anilist_id, provider_anime_id, provider_title, ts],
    )?;
    crate::backup::mark_dirty();
    Ok(OnlinestreamMapping {
        provider_id: provider_id.to_string(),
        anilist_id,
        provider_anime_id: provider_anime_id.to_string(),
        provider_title: provider_title.map(|s| s.to_string()),
        updated_at: ts,
    })
}

pub fn remove_mapping(conn: &Connection, provider_id: &str, anilist_id: i64) -> Result<()> {
    let _ = conn.execute_batch(db::SCHEMA_V9);
    conn.execute(
        "DELETE FROM onlinestream_mappings WHERE provider_id = ?1 AND anilist_id = ?2",
        params![provider_id, anilist_id],
    )?;
    crate::backup::mark_dirty();
    Ok(())
}

// ---------------------------------------------------------------------------
// URL Encoding Helpers
// ---------------------------------------------------------------------------

pub fn url_encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

pub fn url_decode_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn parse_query_params(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(url_decode_component(k), url_decode_component(v));
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Local 127.0.0.1 HLS / Stream Header Proxy
// ---------------------------------------------------------------------------

struct ProxyInfo {
    port: u16,
    token: String,
}

static STREAM_PROXY: OnceLock<Result<ProxyInfo, String>> = OnceLock::new();

fn generate_proxy_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mixed = nanos ^ (pid.wrapping_mul(0x9E37_79B9_7F4A_7C15_u128));
    format!("{mixed:032x}")
}

fn ensure_stream_proxy() -> Result<&'static ProxyInfo> {
    let res = STREAM_PROXY.get_or_init(|| {
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to bind local stream proxy: {e}"))?;
        std_listener
            .set_nonblocking(true)
            .map_err(|e| format!("Failed to configure nonblocking listener: {e}"))?;
        let port = std_listener
            .local_addr()
            .map_err(|e| format!("Failed to inspect proxy port: {e}"))?
            .port();
        let token = generate_proxy_token();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(24)
            .tcp_nodelay(true)
            .build()
            .map_err(|e| format!("Failed to build proxy HTTP client: {e}"))?;

        let token_bg = token.clone();
        let client_bg = client.clone();
        tauri::async_runtime::spawn(async move {
            let Ok(listener) = TcpListener::from_std(std_listener) else {
                return;
            };
            klog!("[onlinestream] Local HLS/header proxy listening on 127.0.0.1:{port}");
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    continue;
                };
                let _ = socket.set_nodelay(true);
                let tok = token_bg.clone();
                let cli = client_bg.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = handle_proxy_connection(socket, port, &tok, &cli).await {
                        let msg = e.to_string();
                        // Broken pipe / aborted segment seeks from mpv are normal during scrubbing
                        if !msg.contains("os error 10053")
                            && !msg.contains("os error 10054")
                            && !msg.contains("Broken pipe")
                        {
                            klog!("[onlinestream] proxy request error: {msg}");
                        }
                    }
                });
            }
        });

        Ok(ProxyInfo { port, token })
    });

    match res {
        Ok(info) => Ok(info),
        Err(msg) => Err(anyhow!("{msg}")),
    }
}

/// Build a local `http://127.0.0.1:<port>/proxy?...` URL that injects custom upstream headers
/// and transparently rewrites `.m3u8` manifests so `libmpv` never hits CDN 403 hotlink checks.
pub fn build_proxy_url(target_url: &str, headers: &HashMap<String, String>) -> Result<String> {
    let info = ensure_stream_proxy()?;
    let hdrs_json = serde_json::to_vec(headers)?;
    let hdrs_b64 = base64_encode(&hdrs_json);
    let proxy_base = format!("http://127.0.0.1:{}", info.port);
    Ok(format_proxied_url(
        &proxy_base,
        &info.token,
        target_url,
        &hdrs_b64,
    ))
}

fn format_proxied_url(proxy_base: &str, token: &str, target_url: &str, hdrs_b64: &str) -> String {
    format!(
        "{proxy_base}/proxy?token={}&url={}&hdrs={}",
        url_encode_component(token),
        url_encode_component(target_url),
        url_encode_component(hdrs_b64),
    )
}

/// Rewrite an HLS `.m3u8` master or media playlist so all relative/absolute segment lines,
/// variant playlists, and `URI="..."` attributes (`#EXT-X-KEY`, `#EXT-X-MAP`, `#EXT-X-MEDIA`)
/// route back through Kura's local header-injecting proxy.
pub fn rewrite_m3u8_playlist(
    content: &str,
    final_base_url: &str,
    proxy_base: &str,
    token: &str,
    hdrs_b64: &str,
) -> String {
    let base_url = reqwest::Url::parse(final_base_url).ok();

    let resolve_and_wrap = |raw_ref: &str| -> String {
        let trimmed = raw_ref.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        // Already proxied
        if trimmed.starts_with(proxy_base) {
            return trimmed.to_string();
        }
        let resolved = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            trimmed.to_string()
        } else if let Some(ref base) = base_url {
            base.join(trimmed)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| trimmed.to_string())
        } else {
            trimmed.to_string()
        };
        format_proxied_url(proxy_base, token, &resolved, hdrs_b64)
    };

    let mut out = String::with_capacity(content.len() + 1024);
    for (idx, line) in content.lines().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            if let Some(rewritten_tag) = rewrite_tag_uri_attributes(trimmed, &resolve_and_wrap) {
                out.push_str(&rewritten_tag);
            } else {
                out.push_str(trimmed);
            }
        } else {
            out.push_str(&resolve_and_wrap(trimmed));
        }
    }
    out
}

fn rewrite_tag_uri_attributes<F>(line: &str, wrap_fn: &F) -> Option<String>
where
    F: Fn(&str) -> String,
{
    if !line.contains("URI=\"") {
        return None;
    }
    let mut result = String::with_capacity(line.len() + 128);
    let mut rest = line;
    while let Some(pos) = rest.find("URI=\"") {
        let prefix_end = pos + 5; // includes `URI="`
        result.push_str(&rest[..prefix_end]);
        let after_quote = &rest[prefix_end..];
        if let Some(end_quote) = after_quote.find('"') {
            let inner_uri = &after_quote[..end_quote];
            result.push_str(&wrap_fn(inner_uri));
            result.push('"');
            rest = &after_quote[end_quote + 1..];
        } else {
            result.push_str(after_quote);
            rest = "";
            break;
        }
    }
    result.push_str(rest);
    Some(result)
}

async fn handle_proxy_connection(
    mut stream: TcpStream,
    port: u16,
    expected_token: &str,
    client: &reqwest::Client,
) -> Result<()> {
    let mut req_buf = vec![0u8; 16_384];
    let mut read_len = 0usize;
    loop {
        if read_len >= req_buf.len() {
            bail!("proxy HTTP request header too large");
        }
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut req_buf[read_len..]))
            .await
            .context("proxy request read timeout")??;
        if n == 0 {
            return Ok(());
        }
        read_len += n;
        if req_buf[..read_len].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    let req_str = String::from_utf8_lossy(&req_buf[..read_len]);
    let mut lines = req_str.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let raw_target = parts.next().unwrap_or("/");

    if method.eq_ignore_ascii_case("OPTIONS") {
        let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: *\r\nConnection: close\r\n\r\n";
        stream.write_all(resp.as_bytes()).await?;
        return Ok(());
    }

    let mut client_range: Option<String> = None;
    for hdr_line in lines {
        if hdr_line.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = hdr_line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                client_range = Some(v.trim().to_string());
            }
        }
    }

    let (path, query) = raw_target.split_once('?').unwrap_or((raw_target, ""));
    if path != "/proxy" {
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(());
    }

    let params = parse_query_params(query);
    if params.get("token").map(String::as_str) != Some(expected_token) {
        stream
            .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(());
    }

    let Some(target_url) = params.get("url").filter(|u| {
        u.starts_with("http://") || u.starts_with("https://")
    }) else {
        stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(());
    };

    let hdrs_b64 = params.get("hdrs").cloned().unwrap_or_default();
    let injected_headers: HashMap<String, String> = if hdrs_b64.is_empty() {
        HashMap::new()
    } else {
        base64_decode(&hdrs_b64)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    };

    let mut upstream_headers = HeaderMap::new();
    let mut has_ua = false;
    for (k, v) in &injected_headers {
        if k.eq_ignore_ascii_case("user-agent") {
            has_ua = true;
        }
        if let (Ok(hn), Ok(hv)) = (
            HeaderName::from_bytes(k.trim().as_bytes()),
            HeaderValue::from_str(v),
        ) {
            upstream_headers.insert(hn, hv);
        }
    }
    if !has_ua {
        upstream_headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_STREAM_UA));
    }
    if let Some(range_val) = client_range {
        if let Ok(hv) = HeaderValue::from_str(&range_val) {
            upstream_headers.insert(RANGE, hv);
        }
    }

    let mut resp = client
        .get(target_url)
        .headers(upstream_headers)
        .send()
        .await
        .with_context(|| format!("upstream proxy GET failed for {target_url}"))?;

    let status = resp.status();
    let status_code = status.as_u16();
    let reason = status.canonical_reason().unwrap_or("OK");
    let final_url = resp.url().to_string();

    let upstream_ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let url_path_lower = final_url
        .split('?')
        .next()
        .unwrap_or(&final_url)
        .to_ascii_lowercase();

    let looks_like_m3u8_by_meta = upstream_ct.to_ascii_lowercase().contains("mpegurl")
        || url_path_lower.ends_with(".m3u8");

    // Peek first chunk so we can also detect disguised `#EXTM3U` playlists (e.g. served as `.txt`/`.png`)
    // AND fix disguised `0x47` MPEG-TS segments served with `image/*` MIME types!
    let first_chunk = resp.chunk().await?;
    let first_bytes = first_chunk.as_deref().unwrap_or(&[]);
    let trimmed_start = first_bytes
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .unwrap_or(first_bytes);
    let starts_with_extm3u = trimmed_start.starts_with(b"#EXTM3U");

    if status.is_success() && (looks_like_m3u8_by_meta || starts_with_extm3u) {
        let mut body = Vec::from(first_bytes);
        while let Some(chunk) = resp.chunk().await? {
            body.extend_from_slice(&chunk);
            if body.len() > 8 * 1024 * 1024 {
                break;
            }
        }
        let text = String::from_utf8_lossy(&body);
        if text.trim_start_matches('\u{feff}').trim_start().starts_with("#EXTM3U") {
            let proxy_base = format!("http://127.0.0.1:{port}");
            let rewritten =
                rewrite_m3u8_playlist(&text, &final_url, &proxy_base, expected_token, &hdrs_b64);
            let out_bytes = rewritten.as_bytes();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
                out_bytes.len()
            );
            stream.write_all(header.as_bytes()).await?;
            if !method.eq_ignore_ascii_case("HEAD") {
                stream.write_all(out_bytes).await?;
            }
            stream.flush().await?;
            return Ok(());
        }

        // Not actually `#EXTM3U`; write buffered body as-is
        let header = format!(
            "HTTP/1.1 {status_code} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
            if upstream_ct.is_empty() { "application/octet-stream" } else { &upstream_ct },
            body.len()
        );
        stream.write_all(header.as_bytes()).await?;
        if !method.eq_ignore_ascii_case("HEAD") {
            stream.write_all(&body).await?;
        }
        stream.flush().await?;
        return Ok(());
    }

    // Binary stream (MPEG-TS segment, fMP4 segment, AES-128 key, MP4 video, or VTT subtitle)
    let effective_ct = if first_bytes.first() == Some(&0x47)
        && upstream_ct.to_ascii_lowercase().starts_with("image/")
    {
        "video/mp2t"
    } else if upstream_ct.is_empty() {
        "application/octet-stream"
    } else {
        upstream_ct.as_str()
    };

    let mut resp_hdr = format!(
        "HTTP/1.1 {status_code} {reason}\r\nContent-Type: {effective_ct}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n"
    );
    for passthrough in ["content-length", "content-range", "accept-ranges"] {
        if let Some(val) = resp.headers().get(passthrough).and_then(|v| v.to_str().ok()) {
            resp_hdr.push_str(&format!("{passthrough}: {val}\r\n"));
        }
    }
    resp_hdr.push_str("\r\n");

    stream.write_all(resp_hdr.as_bytes()).await?;
    if !method.eq_ignore_ascii_case("HEAD") {
        if !first_bytes.is_empty() {
            stream.write_all(first_bytes).await?;
        }
        while let Some(chunk) = resp.chunk().await? {
            stream.write_all(&chunk).await?;
        }
    }
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_master_and_media_m3u8_playlists_with_keys_and_relative_paths() {
        let playlist = "\
#EXTM3U
#EXT-X-VERSION:3
#EXT-X-KEY:METHOD=AES-128,URI=\"keys/enc.key\",IV=0x00000000000000000000000000000001
#EXT-X-MAP:URI=\"/hls/init.mp4\"
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aac\",NAME=\"Japanese\",URI=\"audio/ja.m3u8\"
#EXTINF:6.006,
seg-001.ts?sig=abc&exp=99
#EXTINF:6.006,
https://cdn2.example.org/seg-002.ts
";

        let rewritten = rewrite_m3u8_playlist(
            playlist,
            "https://cdn.example.org/anime/ep01/index.m3u8?token=xyz",
            "http://127.0.0.1:49152",
            "tok123",
            "eyJSZWZlcmVyIjoiaHR0cHM6Ly9leGFtcGxlLmNvbSJ9",
        );

        assert!(rewritten.starts_with("#EXTM3U\n"));
        // AES key relative path resolved against `/anime/ep01/`
        let expected_key = url_encode_component("https://cdn.example.org/anime/ep01/keys/enc.key");
        assert!(
            rewritten.contains(&expected_key),
            "Missing rewritten key URI in:\n{rewritten}"
        );
        // Root-relative init segment resolved against origin
        let expected_map = url_encode_component("https://cdn.example.org/hls/init.mp4");
        assert!(rewritten.contains(&expected_map));
        // Audio rendition URI rewritten
        let expected_audio = url_encode_component("https://cdn.example.org/anime/ep01/audio/ja.m3u8");
        assert!(rewritten.contains(&expected_audio));
        // Relative segment with query string preserved
        let expected_seg1 =
            url_encode_component("https://cdn.example.org/anime/ep01/seg-001.ts?sig=abc&exp=99");
        assert!(rewritten.contains(&expected_seg1));
        // Absolute external CDN segment wrapped
        let expected_seg2 = url_encode_component("https://cdn2.example.org/seg-002.ts");
        assert!(rewritten.contains(&expected_seg2));
    }

    #[test]
    fn manual_mapping_crud_works() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate_for_tests(&conn).unwrap();

        assert!(get_mapping(&conn, "demo-provider", 154587)
            .unwrap()
            .is_none());
        set_mapping(
            &conn,
            "demo-provider",
            154587,
            "sousou-no-frieren-18542",
            Some("Sousou no Frieren"),
        )
        .unwrap();

        let m = get_mapping(&conn, "demo-provider", 154587)
            .unwrap()
            .unwrap();
        assert_eq!(m.provider_anime_id, "sousou-no-frieren-18542");

        remove_mapping(&conn, "demo-provider", 154587).unwrap();
        assert!(get_mapping(&conn, "demo-provider", 154587)
            .unwrap()
            .is_none());
    }
}
