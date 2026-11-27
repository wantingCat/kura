//! Direct soft-sub web streaming backend:
//! - Ephemeral `127.0.0.1` HTTP stream proxy that injects `Referer`/`Origin`/`Cookie`/`User-Agent`
//!   headers and rewrites `.m3u8` master/media playlists (including disguised `#EXTM3U` responses,
//!   relative paths, `#EXT-X-KEY`, `#EXT-X-MAP`, and `#EXT-X-MEDIA` `URI="..."` attributes).
//! - Manual AniList -> provider anime mapping overrides (`onlinestream_mappings` table).

use crate::db;
use crate::extensions::{base64_decode, base64_encode};
use crate::klog;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::header::{
    HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, COOKIE, LOCATION, PROXY_AUTHORIZATION,
    RANGE, USER_AGENT,
};
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

const MAX_M3U8_LINES: usize = 25_000;
const MAX_M3U8_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

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
        let token = crate::db::random_hex_token(24);

        let token_bg = token.clone();
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
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = handle_proxy_connection(socket, port, &tok).await {
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
    let parsed = reqwest::Url::parse(target_url.trim()).context("Invalid stream URL")?;
    if !crate::extensions::is_safe_redirect_url(&parsed) {
        bail!("Blocked unsafe stream URL targeting local or private network");
    }
    let info = ensure_stream_proxy()?;
    let hdrs_json = serde_json::to_vec(headers)?;
    let hdrs_b64 = base64_encode(&hdrs_json);
    let proxy_base = format!("http://127.0.0.1:{}", info.port);
    Ok(format_proxied_url(
        &proxy_base,
        &info.token,
        parsed.as_str(),
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
        let is_safe = reqwest::Url::parse(&resolved)
            .map(|u| crate::extensions::is_safe_redirect_url(&u))
            .unwrap_or(false);
        if !is_safe {
            return String::new();
        }
        format_proxied_url(proxy_base, token, &resolved, hdrs_b64)
    };

    let mut out = String::with_capacity(content.len().min(MAX_M3U8_OUTPUT_BYTES) + 1024);
    for (idx, line) in content.lines().take(MAX_M3U8_LINES).enumerate() {
        let trimmed = line.trim();
        let next_line = if trimmed.is_empty() {
            String::new()
        } else if trimmed.starts_with('#') {
            rewrite_tag_uri_attributes(trimmed, &resolve_and_wrap)
                .unwrap_or_else(|| trimmed.to_string())
        } else {
            resolve_and_wrap(trimmed)
        };
        let extra = usize::from(idx > 0) + next_line.len();
        if out.len().saturating_add(extra) > MAX_M3U8_OUTPUT_BYTES {
            break;
        }
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&next_line);
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
        let resp = "HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n";
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

    let is_head = method.eq_ignore_ascii_case("HEAD");
    let mut current_url = target_url.clone();
    let mut final_resp_opt: Option<(reqwest::Response, String)> = None;

    for hop in 0..=8 {
        let Ok((hop_url, domain_opt, addrs)) =
            crate::extensions::validate_and_resolve_public_url(&current_url).await
        else {
            stream
                .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n")
                .await?;
            return Ok(());
        };
        let client = crate::extensions::build_pinned_http_client(
            domain_opt.as_deref(),
            &addrs,
            Duration::from_secs(15),
            Some(Duration::from_secs(30)),
        )
        .map_err(|e| anyhow!("{e}"))?;

        let req_method = if is_head {
            reqwest::Method::HEAD
        } else {
            reqwest::Method::GET
        };
        let resp = client
            .request(req_method, hop_url.clone())
            .headers(upstream_headers.clone())
            .send()
            .await
            .map_err(|e| anyhow::Error::new(e.without_url()))
            .context("upstream proxy GET failed")?;

        if matches!(resp.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            if let Some(loc_val) = resp.headers().get(LOCATION) {
                if hop >= 8 {
                    bail!("upstream stream exceeded 8 redirects");
                }
                let Ok(loc_str) = loc_val.to_str() else {
                    bail!("invalid upstream redirect Location header");
                };
                let Ok(next_url) = hop_url.join(loc_str) else {
                    bail!("invalid upstream redirect Location URL");
                };
                if !crate::extensions::same_origin(&hop_url, &next_url) {
                    upstream_headers.remove(COOKIE);
                    upstream_headers.remove(AUTHORIZATION);
                    upstream_headers.remove(PROXY_AUTHORIZATION);
                }
                current_url = next_url.to_string();
                continue;
            }
        }

        final_resp_opt = Some((resp, hop_url.to_string()));
        break;
    }

    let Some((mut resp, final_url)) = final_resp_opt else {
        stream
            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(());
    };

    let status = resp.status();
    let status_code = status.as_u16();
    let reason = status.canonical_reason().unwrap_or("OK");

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

    // Peek just enough leading bytes to detect `#EXTM3U` (including BOM / leading whitespace
    // or disguised `.txt`/`.png` playlists) WITHOUT buffering >4 KiB of disguised MP4/TS video streams!
    let mut prefix_buf: Vec<u8> = Vec::new();
    if !is_head {
        loop {
            let without_bom = prefix_buf
                .strip_prefix(&[0xEF, 0xBB, 0xBF])
                .unwrap_or(&prefix_buf);
            let non_ws_start = without_bom
                .iter()
                .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
                .map(|idx| &without_bom[idx..])
                .unwrap_or(&[]);

            if non_ws_start.len() >= 7
                || (!non_ws_start.is_empty() && !b"#EXTM3U".starts_with(non_ws_start))
                || prefix_buf.len() >= 4096
            {
                break;
            }
            let Some(chunk) = resp
                .chunk()
                .await
                .map_err(|e| anyhow::Error::new(e.without_url()))?
            else {
                break;
            };
            prefix_buf.extend_from_slice(&chunk);
        }
    }

    let trimmed_start = prefix_buf
        .strip_prefix(&[0xEF, 0xBB, 0xBF])
        .unwrap_or(&prefix_buf);
    let trimmed_after_ws = trimmed_start
        .iter()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .map(|idx| &trimmed_start[idx..])
        .unwrap_or(&[]);
    let starts_with_extm3u = trimmed_after_ws.starts_with(b"#EXTM3U");

    // ONLY buffer & rewrite as an HLS playlist when the body actually begins with `#EXTM3U`!
    if status.is_success() && !is_head && starts_with_extm3u {
        let mut body = prefix_buf;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| anyhow::Error::new(e.without_url()))?
        {
            if body.len().saturating_add(chunk.len()) > 8 * 1024 * 1024 {
                bail!("upstream m3u8 playlist exceeds 8 MiB limit");
            }
            body.extend_from_slice(&chunk);
        }
        let text = String::from_utf8_lossy(&body);
        let proxy_base = format!("http://127.0.0.1:{port}");
        let rewritten =
            rewrite_m3u8_playlist(&text, &final_url, &proxy_base, expected_token, &hdrs_b64);
        let out_bytes = rewritten.as_bytes();
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
            out_bytes.len()
        );
        stream.write_all(header.as_bytes()).await?;
        stream.write_all(out_bytes).await?;
        stream.flush().await?;
        return Ok(());
    }

    // Binary stream (MPEG-TS segment, fMP4 segment, AES-128 key, MP4 video, disguised video at `.m3u8`, or VTT subtitle):
    // stream `prefix_buf` followed by all remaining `resp.chunk()` data without any 8 MiB truncation!
    let effective_ct = if prefix_buf.first() == Some(&0x47)
        && (upstream_ct.to_ascii_lowercase().starts_with("image/") || looks_like_m3u8_by_meta)
    {
        "video/mp2t"
    } else if looks_like_m3u8_by_meta && !starts_with_extm3u && !is_head {
        "application/octet-stream"
    } else if upstream_ct.is_empty() {
        "application/octet-stream"
    } else {
        upstream_ct.as_str()
    };

    let mut resp_hdr = format!(
        "HTTP/1.1 {status_code} {reason}\r\nContent-Type: {effective_ct}\r\nConnection: close\r\n"
    );
    for passthrough in ["content-length", "content-range", "accept-ranges"] {
        if let Some(val) = resp.headers().get(passthrough).and_then(|v| v.to_str().ok()) {
            resp_hdr.push_str(&format!("{passthrough}: {val}\r\n"));
        }
    }
    resp_hdr.push_str("\r\n");

    stream.write_all(resp_hdr.as_bytes()).await?;
    if !is_head {
        if !prefix_buf.is_empty() {
            stream.write_all(&prefix_buf).await?;
        }
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| anyhow::Error::new(e.without_url()))?
        {
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

    #[test]
    fn m3u8_rewriter_refuses_loopback_private_urls_and_caps_lines() {
        let malicious = "\
#EXTM3U
#EXT-X-KEY:METHOD=AES-128,URI=\"http://127.0.0.1:8765/steal\"
#EXTINF:4.0,
http://192.168.1.1/admin
#EXTINF:4.0,
http://localhost:14876/oauth/token
#EXTINF:4.0,
https://cdn.example.org/valid.ts
";
        let out = rewrite_m3u8_playlist(
            malicious,
            "https://cdn.example.org/live/index.m3u8",
            "http://127.0.0.1:50000",
            "secret_tok",
            "aGRycw==",
        );
        assert!(!out.contains("127.0.0.1%3A8765"));
        assert!(!out.contains("192.168.1.1"));
        assert!(!out.contains("localhost"));
        assert!(out.contains(&url_encode_component("https://cdn.example.org/valid.ts")));

        let huge = format!("#EXTM3U\n{}", "seg.ts\n".repeat(30_000));
        let capped = rewrite_m3u8_playlist(
            &huge,
            "https://cdn.example.org/live/index.m3u8",
            "http://127.0.0.1:50000",
            "secret_tok",
            "aGRycw==",
        );
        assert!(capped.lines().count() <= MAX_M3U8_LINES);
        assert!(capped.len() <= MAX_M3U8_OUTPUT_BYTES);
    }
}
