//! Seanime-compatible extension manager (`onlinestream-provider` & `anime-torrent-provider`)
//! and CORS-free HTTP `fetch` bridge for the isolated Web Worker runtime.

use crate::db;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, SET_COOKIE, USER_AGENT};
use reqwest::redirect::Policy;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

const DEFAULT_EXT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

// ---------------------------------------------------------------------------
// Base64 helper (RFC 4648 standard alphabet)
// ---------------------------------------------------------------------------

pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub fn base64_decode(input: &str) -> Result<Vec<u8>> {
    fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    }
    let clean: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        if chunk.len() < 2 {
            break;
        }
        let c0 = val(chunk[0]).ok_or_else(|| anyhow!("invalid base64 byte"))? as u32;
        let c1 = val(chunk[1]).ok_or_else(|| anyhow!("invalid base64 byte"))? as u32;
        out.push(((c0 << 2) | (c1 >> 4)) as u8);
        if chunk.len() > 2 {
            let c2 = val(chunk[2]).ok_or_else(|| anyhow!("invalid base64 byte"))? as u32;
            out.push((((c1 & 0x0F) << 4) | (c2 >> 2)) as u8);
            if chunk.len() > 3 {
                let c3 = val(chunk[3]).ok_or_else(|| anyhow!("invalid base64 byte"))? as u32;
                out.push((((c2 & 0x03) << 6) | c3) as u8);
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Extension Manifest & Database Models
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledExtension {
    pub id: String,
    pub name: String,
    pub version: String,
    pub ext_type: String,
    pub language: String,
    pub manifest_uri: String,
    pub icon_url: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub payload: String,
    pub user_config: Value,
    pub enabled: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceExtension {
    pub id: String,
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(rename = "type", alias = "extType")]
    pub ext_type: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default, alias = "manifestURI", alias = "manifest_uri")]
    pub manifest_uri: String,
    #[serde(default, alias = "icon")]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub installed_version: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawExtensionManifest {
    id: String,
    name: String,
    #[serde(default = "default_version")]
    version: String,
    #[serde(rename = "type", alias = "extType")]
    ext_type: String,
    #[serde(default = "default_language")]
    language: String,
    #[serde(default, alias = "manifestURI", alias = "manifestUri", alias = "manifest_uri")]
    manifest_uri: String,
    #[serde(default, alias = "icon", alias = "iconUrl")]
    icon_url: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    website: Option<String>,
    #[serde(default)]
    payload: Option<String>,
    #[serde(default, alias = "payloadURI", alias = "payloadUri")]
    payload_uri: Option<String>,
    #[serde(default, alias = "userConfig")]
    user_config: Option<Value>,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

fn default_language() -> String {
    "javascript".to_string()
}

pub fn is_supported_ext_type(ext_type: &str) -> bool {
    matches!(
        ext_type.trim(),
        "onlinestream-provider" | "anime-torrent-provider"
    )
}

/// Convert `https://github.com/<owner>/<repo>/blob/<ref>/<path>` into `raw.githubusercontent.com`.
fn normalize_github_raw_url(input: &str) -> String {
    let trimmed = input.trim();
    if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
    {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 5 && parts[2] == "blob" {
            let owner = parts[0];
            let repo = parts[1];
            let branch = parts[3];
            let file_path = parts[4..].join("/");
            return format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{file_path}");
        }
    }
    trimmed.to_string()
}

/// Parse `gh:owner/repo` or `https://github.com/owner/repo(/tree/<branch>...)` if it points to a repo/folder rather than a `.json`/`.xml` file.
fn parse_github_repo_spec(input: &str) -> Option<(String, String, Option<String>)> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.ends_with(".json") || trimmed.ends_with(".xml") || trimmed.ends_with(".rss") {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("gh:") {
        let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() >= 2 {
            return Some((parts[0].to_string(), parts[1].to_string(), None));
        }
    }
    if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
    {
        let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() >= 2 && parts.get(2).copied() != Some("blob") && parts.get(2).copied() != Some("raw") {
            let branch = if parts.len() >= 4 && parts[2] == "tree" {
                Some(parts[3].to_string())
            } else {
                None
            };
            return Some((
                parts[0].to_string(),
                parts[1].trim_end_matches(".git").to_string(),
                branch,
            ));
        }
    }
    None
}

pub fn list_installed_extensions(conn: &Connection) -> Result<Vec<InstalledExtension>> {
    let _ = conn.execute_batch(db::SCHEMA_V9);
    let mut stmt = conn.prepare(
        "SELECT id, name, version, ext_type, language, manifest_uri, icon_url, author, description,
                payload, user_config_json, enabled, updated_at
         FROM installed_extensions
         ORDER BY ext_type ASC, name ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            let cfg_str: String = r.get(10)?;
            let user_config = serde_json::from_str::<Value>(&cfg_str).unwrap_or_else(|_| json!({}));
            Ok(InstalledExtension {
                id: r.get(0)?,
                name: r.get(1)?,
                version: r.get(2)?,
                ext_type: r.get(3)?,
                language: r.get(4)?,
                manifest_uri: r.get(5)?,
                icon_url: r.get(6)?,
                author: r.get(7)?,
                description: r.get(8)?,
                payload: r.get(9)?,
                user_config,
                enabled: r.get::<_, i64>(11)? != 0,
                updated_at: r.get(12)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn get_installed_extension(conn: &Connection, id: &str) -> Result<Option<InstalledExtension>> {
    let _ = conn.execute_batch(db::SCHEMA_V9);
    conn.query_row(
        "SELECT id, name, version, ext_type, language, manifest_uri, icon_url, author, description,
                payload, user_config_json, enabled, updated_at
         FROM installed_extensions WHERE id = ?1",
        [id],
        |r| {
            let cfg_str: String = r.get(10)?;
            let user_config = serde_json::from_str::<Value>(&cfg_str).unwrap_or_else(|_| json!({}));
            Ok(InstalledExtension {
                id: r.get(0)?,
                name: r.get(1)?,
                version: r.get(2)?,
                ext_type: r.get(3)?,
                language: r.get(4)?,
                manifest_uri: r.get(5)?,
                icon_url: r.get(6)?,
                author: r.get(7)?,
                description: r.get(8)?,
                payload: r.get(9)?,
                user_config,
                enabled: r.get::<_, i64>(11)? != 0,
                updated_at: r.get(12)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub async fn fetch_marketplace_extensions(
    http: &reqwest::Client,
    conn_installed: &[InstalledExtension],
    url: &str,
) -> Result<Vec<MarketplaceExtension>> {
    let clean_url = normalize_github_raw_url(url);
    if clean_url.is_empty() {
        bail!("Enter a Marketplace JSON URL, GitHub repo URL, or Extension Manifest URL first.");
    }

    let installed_map: HashMap<&str, &str> = conn_installed
        .iter()
        .map(|e| (e.id.as_str(), e.version.as_str()))
        .collect();

    // Support pasting a GitHub repository directly (e.g. `https://github.com/pal-droid/seanime-providers` or `gh:owner/repo`)!
    if let Some((owner, repo, branch_opt)) = parse_github_repo_spec(&clean_url) {
        let tree_ref = branch_opt.as_deref().unwrap_or("HEAD");
        let tree_api = format!(
            "https://api.github.com/repos/{owner}/{repo}/git/trees/{tree_ref}?recursive=1"
        );
        let resp = http
            .get(&tree_api)
            .header(USER_AGENT, DEFAULT_EXT_UA)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .with_context(|| format!("Failed to inspect GitHub repository {owner}/{repo}"))?;
        if !resp.status().is_success() {
            bail!("GitHub API returned HTTP {} for {owner}/{repo}", resp.status());
        }
        let tree_val: Value = resp.json().await?;
        let sha_or_ref = tree_val
            .get("sha")
            .and_then(|v| v.as_str())
            .or(branch_opt.as_deref())
            .unwrap_or("main");
        let mut manifest_urls = Vec::new();
        if let Some(entries) = tree_val.get("tree").and_then(|t| t.as_array()) {
            for entry in entries {
                if entry.get("type").and_then(|t| t.as_str()) != Some("blob") {
                    continue;
                }
                let Some(path) = entry.get("path").and_then(|p| p.as_str()) else {
                    continue;
                };
                let low = path.to_ascii_lowercase();
                // Skip manga/plugin folders upfront so we only fetch candidate anime/root manifests
                if low.contains("/manga/") || low.starts_with("manga/") || low.contains("/plugins/") {
                    continue;
                }
                if low.ends_with("/manifest.json")
                    || low == "manifest.json"
                    || (low.ends_with(".json") && (low.contains("onlinestream") || low.contains("anime") || low.contains("marketplace")))
                {
                    let branch_for_raw = branch_opt.as_deref().unwrap_or(sha_or_ref);
                    manifest_urls.push(format!(
                        "https://raw.githubusercontent.com/{owner}/{repo}/{branch_for_raw}/{path}"
                    ));
                }
            }
        }
        manifest_urls.truncate(45);
        let mut set = tokio::task::JoinSet::new();
        for m_url in manifest_urls {
            let client = http.clone();
            set.spawn(async move {
                let r = client
                    .get(&m_url)
                    .header(USER_AGENT, DEFAULT_EXT_UA)
                    .timeout(Duration::from_secs(12))
                    .send()
                    .await
                    .ok()?;
                if !r.status().is_success() {
                    return None;
                }
                let txt = r.text().await.ok()?;
                Some((m_url, txt))
            });
        }
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        while let Some(joined) = set.join_next().await {
            let Some((m_url, txt)) = joined.ok().flatten() else {
                continue;
            };
            let Ok(val) = serde_json::from_str::<Value>(&txt) else {
                continue;
            };
            let candidates: Vec<RawExtensionManifest> = if let Some(arr) = val.as_array() {
                arr.iter()
                    .filter_map(|x| serde_json::from_value::<RawExtensionManifest>(x.clone()).ok())
                    .collect()
            } else if let Ok(single) = serde_json::from_value::<RawExtensionManifest>(val) {
                vec![single]
            } else {
                Vec::new()
            };
            for mut m in candidates {
                if m.id.trim().is_empty() || !is_supported_ext_type(&m.ext_type) || !seen.insert(m.id.clone()) {
                    continue;
                }
                if m.manifest_uri.trim().is_empty() {
                    m.manifest_uri = m_url.clone();
                }
                let inst_ver = installed_map.get(m.id.as_str()).map(|v| (*v).to_string());
                out.push(MarketplaceExtension {
                    id: m.id,
                    name: m.name,
                    version: m.version,
                    ext_type: m.ext_type,
                    language: m.language.to_ascii_lowercase(),
                    manifest_uri: m.manifest_uri,
                    icon_url: m.icon_url,
                    author: m.author,
                    description: m.description,
                    website: m.website,
                    installed: inst_ver.is_some(),
                    installed_version: inst_ver,
                });
            }
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        return Ok(out);
    }

    if !clean_url.starts_with("http://") && !clean_url.starts_with("https://") {
        bail!("Marketplace URL must start with http://, https://, or gh:owner/repo");
    }

    let resp = http
        .get(&clean_url)
        .header(USER_AGENT, DEFAULT_EXT_UA)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .with_context(|| format!("Failed to fetch marketplace from {clean_url}"))?;

    if !resp.status().is_success() {
        bail!("Marketplace returned HTTP {}", resp.status());
    }

    let body = resp.text().await?;
    let val: Value =
        serde_json::from_str(&body).context("Marketplace URL did not return valid JSON")?;

    let raw_items: Vec<RawExtensionManifest> = if val.is_array() {
        let mut out = Vec::new();
        if let Some(arr) = val.as_array() {
            for item in arr {
                if let Ok(m) = serde_json::from_value::<RawExtensionManifest>(item.clone()) {
                    out.push(m);
                }
            }
        }
        out
    } else if val.is_object() {
        // Also support `{ "extensions": [...] }` wrapper or a single manifest object
        if let Some(arr) = val.get("extensions").and_then(|x| x.as_array()) {
            arr.iter()
                .filter_map(|item| serde_json::from_value::<RawExtensionManifest>(item.clone()).ok())
                .collect()
        } else if let Ok(single) = serde_json::from_value::<RawExtensionManifest>(val) {
            vec![single]
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let mut out = Vec::new();
    for mut m in raw_items {
        if m.id.trim().is_empty() || !is_supported_ext_type(&m.ext_type) {
            continue;
        }
        if m.manifest_uri.trim().is_empty() && m.payload.is_some() {
            m.manifest_uri = clean_url.to_string();
        }
        if m.manifest_uri.trim().is_empty() {
            continue;
        }
        let inst_ver = installed_map.get(m.id.as_str()).map(|v| (*v).to_string());
        out.push(MarketplaceExtension {
            id: m.id,
            name: m.name,
            version: m.version,
            ext_type: m.ext_type,
            language: m.language.to_ascii_lowercase(),
            manifest_uri: m.manifest_uri,
            icon_url: m.icon_url,
            author: m.author,
            description: m.description,
            website: m.website,
            installed: inst_ver.is_some(),
            installed_version: inst_ver,
        });
    }

    Ok(out)
}

pub async fn install_extension_from_source(
    http: &reqwest::Client,
    source: &str,
) -> Result<(
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<Value>,
)> {
    let trimmed = normalize_github_raw_url(source);
    if trimmed.is_empty() {
        bail!("Extension manifest URL or JSON cannot be empty.");
    }

    let (manifest_json, source_url) =
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            let resp = http
                .get(&trimmed)
                .header(USER_AGENT, DEFAULT_EXT_UA)
                .timeout(Duration::from_secs(15))
                .send()
                .await
                .with_context(|| format!("Failed to download manifest from {trimmed}"))?;
            if !resp.status().is_success() {
                bail!("Manifest URL returned HTTP {}", resp.status());
            }
            (resp.text().await?, Some(trimmed))
        } else {
            (trimmed, None)
        };

    let raw: RawExtensionManifest = serde_json::from_str(&manifest_json)
        .context("Invalid Seanime extension manifest JSON")?;

    let id = raw.id.trim().to_string();
    if id.is_empty() {
        bail!("Extension manifest is missing an 'id' field.");
    }
    let ext_type = raw.ext_type.trim().to_string();
    if !is_supported_ext_type(&ext_type) {
        bail!(
            "UNSUPPORTED_SEANIME_TYPE: '{}' is a '{}' extension (e.g. manga/plugin). Kura is an anime player and supports 'onlinestream-provider' (under src/anime/) and 'anime-torrent-provider'. Tip: paste the GitHub repo URL itself into Browse Marketplace to list all compatible anime extensions!",
            raw.name.trim(),
            ext_type
        );
    }

    let payload = match raw.payload.filter(|p| !p.trim().is_empty()) {
        Some(inline) => inline,
        None => {
            let raw_p_uri = raw
                .payload_uri
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .ok_or_else(|| anyhow!("SEANIME_MANIFEST_ERR: Extension manifest has neither 'payload' nor 'payloadURI'."))?;
            let resolved_p_uri = if raw_p_uri.starts_with("http://") || raw_p_uri.starts_with("https://") {
                normalize_github_raw_url(raw_p_uri)
            } else if let Some(base) = source_url.as_deref() {
                reqwest::Url::parse(base)
                    .and_then(|u| u.join(raw_p_uri))
                    .map(|u| u.to_string())
                    .unwrap_or_else(|_| raw_p_uri.to_string())
            } else {
                raw_p_uri.to_string()
            };
            let resp = http
                .get(&resolved_p_uri)
                .header(USER_AGENT, DEFAULT_EXT_UA)
                .timeout(Duration::from_secs(15))
                .send()
                .await
                .with_context(|| format!("SEANIME_MANIFEST_ERR: Failed to fetch payloadURI {resolved_p_uri}"))?;
            if !resp.status().is_success() {
                bail!("SEANIME_MANIFEST_ERR: payloadURI returned HTTP {}", resp.status());
            }
            resp.text().await?
        }
    };

    let manifest_uri = if !raw.manifest_uri.trim().is_empty() {
        raw.manifest_uri.trim().to_string()
    } else {
        source_url.unwrap_or_default()
    };

    let language = match raw.language.trim().to_ascii_lowercase().as_str() {
        "typescript" | "ts" => "typescript".to_string(),
        _ => "javascript".to_string(),
    };

    Ok((
        id,
        raw.name.trim().to_string(),
        raw.version.trim().to_string(),
        ext_type,
        language,
        manifest_uri,
        raw.icon_url,
        raw.author,
        raw.description,
        payload,
        raw.user_config,
    ))
}

pub fn upsert_installed_extension(
    conn: &Connection,
    id: &str,
    name: &str,
    version: &str,
    ext_type: &str,
    language: &str,
    manifest_uri: &str,
    icon_url: Option<&str>,
    author: Option<&str>,
    description: Option<&str>,
    payload: &str,
    definition: Option<Value>,
) -> Result<InstalledExtension> {
    let _ = conn.execute_batch(db::SCHEMA_V9);

    // Preserve existing user preference values and enabled flag if updating
    let existing = get_installed_extension(conn, id)?;
    let prev_values = existing
        .as_ref()
        .and_then(|e| e.user_config.get("values").cloned())
        .unwrap_or_else(|| json!({}));
    let enabled = existing.as_ref().map(|e| e.enabled).unwrap_or(true);

    let merged_cfg = json!({
        "definition": definition.unwrap_or(Value::Null),
        "values": prev_values,
    });
    let cfg_json = serde_json::to_string(&merged_cfg)?;
    let ts = db::now();

    conn.execute(
        "INSERT INTO installed_extensions
         (id, name, version, ext_type, language, manifest_uri, icon_url, author, description,
          payload, user_config_json, enabled, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            version = excluded.version,
            ext_type = excluded.ext_type,
            language = excluded.language,
            manifest_uri = CASE WHEN excluded.manifest_uri != '' THEN excluded.manifest_uri ELSE installed_extensions.manifest_uri END,
            icon_url = excluded.icon_url,
            author = excluded.author,
            description = excluded.description,
            payload = excluded.payload,
            user_config_json = excluded.user_config_json,
            updated_at = excluded.updated_at",
        params![
            id,
            name,
            version,
            ext_type,
            language,
            manifest_uri,
            icon_url,
            author,
            description,
            payload,
            cfg_json,
            if enabled { 1i64 } else { 0i64 },
            ts,
        ],
    )?;
    crate::backup::mark_dirty();

    get_installed_extension(conn, id)?
        .ok_or_else(|| anyhow!("Failed to read installed extension after upsert"))
}

pub fn toggle_extension(conn: &Connection, id: &str, enabled: bool) -> Result<()> {
    let _ = conn.execute_batch(db::SCHEMA_V9);
    conn.execute(
        "UPDATE installed_extensions SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, if enabled { 1i64 } else { 0i64 }, db::now()],
    )?;
    crate::backup::mark_dirty();
    Ok(())
}

pub fn remove_extension(conn: &Connection, id: &str) -> Result<()> {
    let _ = conn.execute_batch(db::SCHEMA_V9);
    conn.execute("DELETE FROM installed_extensions WHERE id = ?1", [id])?;
    conn.execute(
        "DELETE FROM onlinestream_mappings WHERE provider_id = ?1",
        [id],
    )?;
    crate::backup::mark_dirty();
    Ok(())
}

pub fn set_extension_user_config(
    conn: &Connection,
    id: &str,
    values: Value,
) -> Result<InstalledExtension> {
    let _ = conn.execute_batch(db::SCHEMA_V9);
    let existing = get_installed_extension(conn, id)?
        .ok_or_else(|| anyhow!("Extension '{id}' is not installed"))?;
    let definition = existing
        .user_config
        .get("definition")
        .cloned()
        .unwrap_or(Value::Null);
    let merged = json!({
        "definition": definition,
        "values": values,
    });
    conn.execute(
        "UPDATE installed_extensions SET user_config_json = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, serde_json::to_string(&merged)?, db::now()],
    )?;
    crate::backup::mark_dirty();
    get_installed_extension(conn, id)?
        .ok_or_else(|| anyhow!("Failed to load extension '{id}'"))
}

// ---------------------------------------------------------------------------
// CORS-Free HTTP Fetch Bridge for Extension Worker (`core.d.ts` `fetch`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionFetchReq {
    pub url: String,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_base64: Option<String>,
    #[serde(default)]
    pub redirect: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionFetchResp {
    pub status: u16,
    pub status_text: String,
    pub method: String,
    pub ok: bool,
    pub url: String,
    pub redirected: bool,
    pub content_type: String,
    pub content_length: i64,
    pub headers: HashMap<String, String>,
    pub raw_headers: HashMap<String, Vec<String>>,
    pub cookies: HashMap<String, String>,
    pub body_text: String,
    pub body_base64: String,
}

pub async fn extension_http_fetch(req: ExtensionFetchReq) -> Result<ExtensionFetchResp> {
    let target_url = req.url.trim();
    if !target_url.starts_with("http://") && !target_url.starts_with("https://") {
        bail!("extension fetch only permits http:// and https:// URLs");
    }

    let timeout_secs = req.timeout.unwrap_or(35).clamp(2, 120);
    let redirect_mode = req
        .redirect
        .as_deref()
        .unwrap_or("follow")
        .to_ascii_lowercase();

    let policy = match redirect_mode.as_str() {
        "manual" => Policy::none(),
        "error" => Policy::custom(|attempt| attempt.error("redirect disallowed by redirect=error")),
        _ => Policy::limited(10),
    };

    let client = reqwest::Client::builder()
        .redirect(policy)
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .context("Failed to initialize extension HTTP client")?;

    let method_str = req
        .method
        .as_deref()
        .unwrap_or("GET")
        .trim()
        .to_ascii_uppercase();
    let method = reqwest::Method::from_bytes(method_str.as_bytes())
        .with_context(|| format!("Invalid HTTP method '{method_str}'"))?;

    let mut header_map = HeaderMap::new();
    let mut has_ua = false;
    if let Some(hdrs) = &req.headers {
        for (k, v) in hdrs {
            if k.eq_ignore_ascii_case("user-agent") {
                has_ua = true;
            }
            if let (Ok(hn), Ok(hv)) = (
                HeaderName::from_bytes(k.trim().as_bytes()),
                HeaderValue::from_str(v),
            ) {
                header_map.insert(hn, hv);
            }
        }
    }
    if !has_ua {
        header_map.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_EXT_UA));
    }

    let mut builder = client.request(method, target_url).headers(header_map);
    if let Some(b64) = &req.body_base64 {
        let bytes = base64_decode(b64)?;
        builder = builder.body(bytes);
    } else if let Some(text_body) = req.body {
        builder = builder.body(text_body);
    }

    let resp = builder
        .send()
        .await
        .with_context(|| format!("HTTP {method_str} {target_url} failed"))?;

    let status = resp.status();
    let status_code = status.as_u16();
    let status_text = status.canonical_reason().unwrap_or("").to_string();
    let ok = status.is_success();
    let final_url = resp.url().to_string();
    let redirected = final_url != target_url;

    let mut headers = HashMap::new();
    let mut raw_headers: HashMap<String, Vec<String>> = HashMap::new();
    let mut cookies = HashMap::new();

    for (name, val) in resp.headers().iter() {
        let val_str = val.to_str().unwrap_or("").to_string();
        let key_lower = name.as_str().to_ascii_lowercase();
        headers.insert(key_lower.clone(), val_str.clone());
        raw_headers
            .entry(key_lower)
            .or_default()
            .push(val_str.clone());

        if name == SET_COOKIE {
            if let Some(first_pair) = val_str.split(';').next() {
                if let Some((ck, cv)) = first_pair.split_once('=') {
                    cookies.insert(ck.trim().to_string(), cv.trim().to_string());
                }
            }
        }
    }

    let content_type = headers.get("content-type").cloned().unwrap_or_default();
    let bytes = resp.bytes().await?;
    let content_length = bytes.len() as i64;
    let body_text = String::from_utf8_lossy(&bytes).into_owned();
    let body_base64 = base64_encode(&bytes);

    Ok(ExtensionFetchResp {
        status: status_code,
        status_text,
        method: method_str,
        ok,
        url: final_url,
        redirected,
        content_type,
        content_length,
        headers,
        raw_headers,
        cookies,
        body_text,
        body_base64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip_works() {
        let samples: &[&[u8]] = &[b"", b"a", b"ab", b"abc", b"Kura anime streaming \xE8\x94\xB5!"];
        for &s in samples {
            let enc = base64_encode(s);
            let dec = base64_decode(&enc).unwrap();
            assert_eq!(dec, s);
        }
    }

    #[test]
    fn extension_db_crud_and_config_preservation() {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate_for_tests(&conn).unwrap();

        let ext = upsert_installed_extension(
            &conn,
            "test-stream",
            "Test Stream",
            "1.0.0",
            "onlinestream-provider",
            "typescript",
            "https://example.com/manifest.json",
            None,
            Some("Author"),
            Some("Clean softsub provider"),
            "class Provider {}",
            Some(json!({ "fields": [{ "name": "domain", "default": "example.com" }] })),
        )
        .unwrap();
        assert_eq!(ext.id, "test-stream");
        assert!(ext.enabled);

        let updated =
            set_extension_user_config(&conn, "test-stream", json!({ "domain": "mirror.example.com" }))
                .unwrap();
        assert_eq!(updated.user_config["values"]["domain"], "mirror.example.com");

        // Re-upserting newer version preserves saved user preference values
        let upgraded = upsert_installed_extension(
            &conn,
            "test-stream",
            "Test Stream",
            "1.1.0",
            "onlinestream-provider",
            "typescript",
            "https://example.com/manifest.json",
            None,
            Some("Author"),
            Some("Clean softsub provider"),
            "class Provider { /* v1.1 */ }",
            Some(json!({ "fields": [{ "name": "domain", "default": "example.com" }] })),
        )
        .unwrap();
        assert_eq!(upgraded.version, "1.1.0");
        assert_eq!(upgraded.user_config["values"]["domain"], "mirror.example.com");

        toggle_extension(&conn, "test-stream", false).unwrap();
        let listed = list_installed_extensions(&conn).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].enabled);

        remove_extension(&conn, "test-stream").unwrap();
        assert!(list_installed_extensions(&conn).unwrap().is_empty());
    }
}
