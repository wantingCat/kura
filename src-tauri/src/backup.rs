//! User-data backups.
//!
//! One JSON format is used for both:
//! - the automatic *safety copy* (`kura_backup.json`, written to a few locations outside the
//!   app data folder so a reinstall / wiped database can be restored on the next start), and
//! - manual *Export / Import* from Settings → Backup & Restore.
//!
//! Only user-owned data is stored — settings, libraries & folders, watch history, resume
//! progress, per-show track preferences and manual match overrides. Metadata and artwork are
//! re-downloaded by a scan, so they are never part of a backup.
//!
//! The format is forward/backward compatible: every field added after v1 is `#[serde(default)]`,
//! and older Kura builds simply ignore the fields they don't know.

use crate::db::{now, BackupLibrary};
use crate::service::AppState;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Current backup format. v1 = settings + libraries only (≤ 0.3.x).
pub const FORMAT: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Backup {
    #[serde(default)]
    pub app: String,
    pub version: u32,
    pub updated_at: i64,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub settings: HashMap<String, String>,
    #[serde(default)]
    pub libraries: Vec<BackupLibrary>,
    #[serde(default)]
    pub watched: Vec<WatchedRow>,
    #[serde(default)]
    pub progress: Vec<ProgressRow>,
    #[serde(default)]
    pub track_prefs: Vec<TrackPrefRow>,
    #[serde(default)]
    pub manual_matches: Vec<ManualMatch>,
    #[serde(default)]
    pub extensions: Vec<ExtensionBackupRow>,
    #[serde(default)]
    pub onlinestream_mappings: Vec<OnlinestreamMappingBackupRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionBackupRow {
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
    pub user_config_json: String,
    pub enabled: bool,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlinestreamMappingBackupRow {
    pub provider_id: String,
    pub anilist_id: i64,
    pub provider_anime_id: String,
    pub provider_title: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchedRow {
    pub anilist_id: i64,
    pub ep_key: String,
    pub watched_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressRow {
    pub anilist_id: i64,
    pub ep_key: String,
    pub position: f64,
    pub duration: f64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackPrefRow {
    pub anilist_id: i64,
    pub audio_pref: Option<String>,
    pub sub_pref: Option<String>,
    pub sub_fallback: Option<String>,
    pub updated_at: i64,
}

/// A "Fix match" the user made. Group keys embed the library folder path, so we store the
/// folder (`root`) separately and remap it on import if the folder moved (e.g. new drive letter).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualMatch {
    pub library: String,
    pub root: String,
    pub rest: String,
    pub folder_path: Option<String>,
    pub display_name: String,
    pub title_guess: String,
    pub title_full: String,
    pub year_hint: Option<i64>,
    pub anilist_id: i64,
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

fn rows<T>(conn: &Connection, sql: &str, f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>) -> Vec<T> {
    // Tolerant on purpose: a missing table (very old DB) just yields nothing.
    let Ok(mut stmt) = conn.prepare(sql) else { return Vec::new() };
    let Ok(iter) = stmt.query_map([], f) else { return Vec::new() };
    iter.flatten().collect()
}

fn is_sensitive_setting_key(k: &str) -> bool {
    let low = k.trim().to_ascii_lowercase();
    matches!(
        low.as_str(),
        "anilist.token"
            | "pref.player_path"
            | "pref.jimaku_api_key"
            | "pref.fanart_api_key"
            | "fanart_api_key"
            | "jimaku_api_key"
    ) || low.contains("token")
        || low.contains("api_key")
        || low.contains("apikey")
        || low.contains("passkey")
        || low.contains("secret")
        || low.contains("password")
}

fn is_sensitive_field_name(name: &str) -> bool {
    let low = name.trim().to_ascii_lowercase();
    low.contains("token")
        || low.contains("key")
        || low.contains("secret")
        || low.contains("password")
        || low.contains("passwd")
        || low.contains("pwd")
        || low.contains("auth")
        || low.contains("cookie")
}

fn strip_sensitive_url_params(raw: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(raw.trim()) else {
        return crate::db::redact_sensitive_text(raw);
    };
    let pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| !is_sensitive_field_name(k))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if pairs.is_empty() {
        parsed.set_query(None);
    } else {
        let mut qp = parsed.query_pairs_mut();
        qp.clear();
        for (k, v) in pairs {
            qp.append_pair(&k, &v);
        }
    }
    parsed.to_string()
}

fn sanitize_torrent_extensions_json(raw: &str) -> String {
    let Ok(mut arr) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return raw.to_string();
    };
    for item in &mut arr {
        if let Some(obj) = item.as_object_mut() {
            if let Some(u) = obj.get("url").and_then(|v| v.as_str()).map(str::to_string) {
                obj.insert("url".to_string(), serde_json::Value::String(strip_sensitive_url_params(&u)));
            }
        }
    }
    serde_json::to_string(&arr).unwrap_or_else(|_| raw.to_string())
}

fn sanitize_extension_user_config_json(raw: &str) -> String {
    let Ok(mut val) = serde_json::from_str::<serde_json::Value>(raw) else {
        return "{}".to_string();
    };
    let mut password_fields = std::collections::HashSet::new();
    if let Some(fields) = val
        .get("definition")
        .and_then(|d| d.get("fields"))
        .and_then(|f| f.as_array())
    {
        for f in fields {
            let fname = f.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let ftype = f.get("type").and_then(|x| x.as_str()).unwrap_or("");
            if ftype.eq_ignore_ascii_case("password") || is_sensitive_field_name(fname) {
                password_fields.insert(fname.to_string());
            }
        }
    }
    if let Some(values_obj) = val.get_mut("values").and_then(|v| v.as_object_mut()) {
        values_obj.retain(|k, _| !password_fields.contains(k) && !is_sensitive_field_name(k));
    } else if let Some(top_obj) = val.as_object_mut() {
        top_obj.retain(|k, _| k == "definition" || (!password_fields.contains(k) && !is_sensitive_field_name(k)));
    }
    serde_json::to_string(&val).unwrap_or_else(|_| "{}".to_string())
}

/// Read every piece of user data into a [`Backup`].
pub fn snapshot(conn: &Connection) -> Backup {
    let settings: HashMap<String, String> = rows(
        conn,
        "SELECT key, value FROM settings",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )
    .into_iter()
    .filter(|(k, _)| !is_sensitive_setting_key(k))
    .map(|(k, v)| {
        if k == "pref.torrent_extensions" {
            (k, sanitize_torrent_extensions_json(&v))
        } else {
            (k, v)
        }
    })
    .collect();

    let libs: Vec<(i64, String, String)> =
        rows(conn, "SELECT id, name, kind FROM libraries ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)));
    let folders: Vec<(i64, String)> =
        rows(conn, "SELECT library_id, path FROM library_folders ORDER BY id", |r| Ok((r.get(0)?, r.get(1)?)));
    let libraries: Vec<BackupLibrary> = libs
        .iter()
        .map(|(id, name, kind)| BackupLibrary {
            name: name.clone(),
            kind: kind.clone(),
            folders: folders.iter().filter(|(l, _)| l == id).map(|(_, p)| p.clone()).collect(),
        })
        .collect();

    let watched = rows(conn, "SELECT anilist_id, ep_key, watched_at FROM watch_state ORDER BY anilist_id, ep_key", |r| {
        Ok(WatchedRow { anilist_id: r.get(0)?, ep_key: r.get(1)?, watched_at: r.get(2)? })
    });
    let progress = rows(
        conn,
        "SELECT anilist_id, ep_key, position, duration, updated_at FROM watch_progress ORDER BY anilist_id, ep_key",
        |r| {
            Ok(ProgressRow {
                anilist_id: r.get(0)?,
                ep_key: r.get(1)?,
                position: r.get(2)?,
                duration: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    );
    let track_prefs = rows(
        conn,
        "SELECT anilist_id, audio_pref, sub_pref, sub_fallback, updated_at FROM media_track_prefs ORDER BY anilist_id",
        |r| {
            Ok(TrackPrefRow {
                anilist_id: r.get(0)?,
                audio_pref: r.get(1)?,
                sub_pref: r.get(2)?,
                sub_fallback: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    );

    let lib_name = |id: i64| libs.iter().find(|(l, ..)| *l == id).map(|(_, n, _)| n.clone());
    let groups: Vec<(i64, String, Option<String>, String, String, String, Option<i64>, i64)> = rows(
        conn,
        "SELECT library_id, group_key, folder_path, display_name, title_guess, title_full, year_hint, anilist_id
         FROM match_groups WHERE manual = 1 AND anilist_id IS NOT NULL ORDER BY id",
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
    );
    let mut manual_matches = Vec::new();
    for (lib, key, folder_path, display_name, title_guess, title_full, year_hint, anilist_id) in groups {
        let Some(library) = lib_name(lib) else { continue };
        // Longest matching folder wins (nested library folders).
        let root = folders
            .iter()
            .filter(|(l, p)| *l == lib && key.starts_with(&format!("{p}|")))
            .map(|(_, p)| p.clone())
            .max_by_key(|p| p.len());
        let Some(root) = root else { continue };
        let rest = key[root.len() + 1..].to_string();
        manual_matches.push(ManualMatch {
            library,
            root,
            rest,
            folder_path,
            display_name,
            title_guess,
            title_full,
            year_hint,
            anilist_id,
        });
    }

    let extensions = rows(
        conn,
        "SELECT id, name, version, ext_type, language, manifest_uri, icon_url, author, description,
                payload, user_config_json, enabled, updated_at
         FROM installed_extensions ORDER BY id",
        |r| {
            Ok(ExtensionBackupRow {
                id: r.get(0)?,
                name: r.get(1)?,
                version: r.get(2)?,
                ext_type: r.get(3)?,
                language: r.get(4)?,
                manifest_uri: strip_sensitive_url_params(&r.get::<_, String>(5)?),
                icon_url: r.get(6)?,
                author: r.get(7)?,
                description: r.get(8)?,
                payload: r.get(9)?,
                user_config_json: sanitize_extension_user_config_json(&r.get::<_, String>(10)?),
                enabled: r.get::<_, i64>(11)? != 0,
                updated_at: r.get(12)?,
            })
        },
    );

    let onlinestream_mappings = rows(
        conn,
        "SELECT provider_id, anilist_id, provider_anime_id, provider_title, updated_at
         FROM onlinestream_mappings ORDER BY provider_id, anilist_id",
        |r| {
            Ok(OnlinestreamMappingBackupRow {
                provider_id: r.get(0)?,
                anilist_id: r.get(1)?,
                provider_anime_id: r.get(2)?,
                provider_title: r.get(3)?,
                updated_at: r.get(4)?,
            })
        },
    );

    Backup {
        app: "kura".into(),
        version: FORMAT,
        updated_at: now(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        settings,
        libraries,
        watched,
        progress,
        track_prefs,
        manual_matches,
        extensions,
        onlinestream_mappings,
    }
}

// ---------------------------------------------------------------------------
// Apply (import / restore)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    pub settings: bool,
    pub libraries: bool,
    pub history: bool,
    pub track_prefs: bool,
    pub matches: bool,
    #[serde(default)]
    pub extensions: bool,
}

impl ImportOptions {
    pub const ALL: Self = Self {
        settings: true,
        libraries: true,
        history: true,
        track_prefs: true,
        matches: true,
        extensions: true,
    };
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub settings: usize,
    pub libraries_added: usize,
    pub folders_added: usize,
    pub watched_added: usize,
    pub progress_added: usize,
    pub track_prefs: usize,
    pub matches: usize,
    pub matches_skipped: usize,
    pub extensions: usize,
}

fn is_safe_library_folder(raw: &str) -> bool {
    let s = raw.trim();
    if s.is_empty() || s.contains('\0') {
        return false;
    }
    // Reject UNC network shares and Win32 device namespace paths (`\\server\share`, `\\?\`, `\\.\`)
    if s.starts_with("\\\\") || s.starts_with("//") {
        return false;
    }
    // Reject parent traversal segments
    if s.split(['/', '\\']).any(|seg| seg == "..") {
        return false;
    }

    let bytes = s.as_bytes();
    let is_win_drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let is_unix_abs = s.starts_with('/');
    if !is_win_drive && !is_unix_abs {
        return false;
    }

    let trimmed = s.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return false;
    }
    if is_win_drive && trimmed.len() <= 2 {
        // Bare drive root like `C:\` or `D:/`
        return false;
    }
    if Path::new(s).parent().is_none() {
        return false;
    }

    let norm = trimmed.replace('\\', "/").to_ascii_lowercase();
    if is_win_drive {
        let after_drive = &norm[2..]; // starts with `/`
        for forbidden in [
            "/windows",
            "/program files",
            "/program files (x86)",
            "/programdata",
            "/system volume information",
            "/$recycle.bin",
        ] {
            if after_drive == forbidden
                || after_drive
                    .strip_prefix(forbidden)
                    .map_or(false, |rest| rest.starts_with('/'))
            {
                return false;
            }
        }
        if after_drive == "/users" {
            return false;
        }
    } else {
        for forbidden in [
            "/etc", "/usr", "/bin", "/sbin", "/lib", "/lib64", "/proc", "/sys", "/dev", "/boot",
            "/system", "/var/run", "/private/etc",
        ] {
            if norm == forbidden
                || norm
                    .strip_prefix(forbidden)
                    .map_or(false, |rest| rest.starts_with('/'))
            {
                return false;
            }
        }
        if matches!(norm.as_str(), "/home" | "/users" | "/root" | "/var" | "/tmp") {
            return false;
        }
    }

    true
}

/// Pick which current folder of `lib_id` a backed-up folder `root` corresponds to.
fn remap_root(conn: &Connection, lib_id: i64, root: &str) -> Option<String> {
    let folders: Vec<String> = {
        let mut stmt = conn.prepare("SELECT path FROM library_folders WHERE library_id = ?1").ok()?;
        let r = stmt.query_map([lib_id], |r| r.get(0)).ok()?.flatten().collect();
        r
    };
    if folders.iter().any(|f| f == root) {
        return Some(root.to_string());
    }
    if folders.len() == 1 {
        return folders.into_iter().next();
    }
    let base = |p: &str| p.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
    let want = base(root);
    let mut same: Vec<String> = folders.into_iter().filter(|f| !want.is_empty() && base(f) == want).collect();
    if same.len() == 1 {
        same.pop()
    } else {
        None
    }
}

/// Merge a backup into the database. Never deletes anything: existing watch history wins over
/// older entries, newer progress / track preferences replace older ones.
pub fn apply(conn: &Connection, b: &Backup, o: &ImportOptions) -> Result<ImportReport> {
    let tx = conn.unchecked_transaction()?;
    let mut rep = ImportReport::default();

    if o.settings {
        for (k, v) in &b.settings {
            // Never import arbitrary executable paths or tokens/keys from a backup file.
            if is_sensitive_setting_key(k) {
                continue;
            }
            if !k.starts_with("pref.") && !k.starts_with("anilist.") {
                continue;
            }
            tx.execute(
                "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v],
            )?;
            rep.settings += 1;
        }
    }

    if o.extensions {
        for ext in &b.extensions {
            if ext.id.trim().is_empty() || !crate::extensions::is_supported_ext_type(&ext.ext_type) {
                continue;
            }
            // Restored JS/TS extensions are always inserted disabled (`enabled = 0`) and never
            // auto-enable an existing disabled extension on conflict (`V-C03` / `C-02`).
            rep.extensions += tx.execute(
                "INSERT INTO installed_extensions
                 (id, name, version, ext_type, language, manifest_uri, icon_url, author, description,
                  payload, user_config_json, enabled, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, ?12)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    version = excluded.version,
                    ext_type = excluded.ext_type,
                    language = excluded.language,
                    manifest_uri = excluded.manifest_uri,
                    icon_url = excluded.icon_url,
                    author = excluded.author,
                    description = excluded.description,
                    payload = excluded.payload,
                    user_config_json = excluded.user_config_json,
                    updated_at = excluded.updated_at
                 WHERE excluded.updated_at >= installed_extensions.updated_at",
                params![
                    ext.id.trim(),
                    ext.name,
                    ext.version,
                    ext.ext_type.trim(),
                    ext.language,
                    strip_sensitive_url_params(&ext.manifest_uri),
                    ext.icon_url,
                    ext.author,
                    ext.description,
                    ext.payload,
                    sanitize_extension_user_config_json(&ext.user_config_json),
                    ext.updated_at,
                ],
            )?;
        }
    }

    // Libraries are matched by name; also needed (read-only) for manual matches.
    let lib_id = |tx: &Connection, name: &str| -> Result<Option<i64>> {
        Ok(tx.query_row("SELECT id FROM libraries WHERE name = ?1 ORDER BY id LIMIT 1", [name], |r| r.get(0)).optional()?)
    };
    if o.libraries {
        for lib in &b.libraries {
            let id = match lib_id(&tx, &lib.name)? {
                Some(id) => id,
                None => {
                    tx.execute(
                        "INSERT INTO libraries (name, kind, created_at) VALUES (?1, ?2, ?3)",
                        params![lib.name, lib.kind, now()],
                    )?;
                    rep.libraries_added += 1;
                    tx.last_insert_rowid()
                }
            };
            for f in &lib.folders {
                let trimmed = f.trim();
                if !is_safe_library_folder(trimmed) {
                    continue;
                }
                rep.folders_added += tx.execute(
                    "INSERT OR IGNORE INTO library_folders (library_id, path) VALUES (?1, ?2)",
                    params![id, trimmed],
                )?;
            }
        }
    }

    if o.history {
        for w in &b.watched {
            rep.watched_added += tx.execute(
                "INSERT OR IGNORE INTO watch_state (anilist_id, ep_key, watched_at) VALUES (?1, ?2, ?3)",
                params![w.anilist_id, w.ep_key, w.watched_at],
            )?;
        }
        for p in &b.progress {
            rep.progress_added += tx.execute(
                "INSERT INTO watch_progress (anilist_id, ep_key, position, duration, updated_at)
                 SELECT ?1, ?2, ?3, ?4, ?5
                 WHERE NOT EXISTS (SELECT 1 FROM watch_state WHERE anilist_id = ?1 AND ep_key = ?2)
                 ON CONFLICT(anilist_id, ep_key) DO UPDATE SET
                    position = excluded.position, duration = excluded.duration, updated_at = excluded.updated_at
                 WHERE excluded.updated_at > watch_progress.updated_at",
                params![p.anilist_id, p.ep_key, p.position, p.duration, p.updated_at],
            )?;
        }
    }

    if o.track_prefs {
        for t in &b.track_prefs {
            rep.track_prefs += tx.execute(
                "INSERT INTO media_track_prefs (anilist_id, audio_pref, sub_pref, sub_fallback, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(anilist_id) DO UPDATE SET
                    audio_pref = excluded.audio_pref, sub_pref = excluded.sub_pref,
                    sub_fallback = excluded.sub_fallback, updated_at = excluded.updated_at
                 WHERE excluded.updated_at >= media_track_prefs.updated_at",
                params![t.anilist_id, t.audio_pref, t.sub_pref, t.sub_fallback, t.updated_at],
            )?;
        }
    }

    if o.matches {
        for m in &b.manual_matches {
            let Some(lib) = lib_id(&tx, &m.library)? else {
                rep.matches_skipped += 1;
                continue;
            };
            let Some(root) = remap_root(&tx, lib, &m.root) else {
                rep.matches_skipped += 1;
                continue;
            };
            let key = format!("{root}|{}", m.rest);
            let folder_path = m.folder_path.as_ref().map(|fp| match fp.strip_prefix(&m.root) {
                Some(tail) => format!("{root}{tail}"),
                None => fp.clone(),
            });
            let current: Option<(i64, Option<i64>)> = tx
                .query_row(
                    "SELECT id, anilist_id FROM match_groups WHERE library_id = ?1 AND group_key = ?2",
                    params![lib, key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match current {
                Some((gid, aid)) => {
                    tx.execute(
                        "UPDATE match_groups SET anilist_id = ?2, confidence = 1.0, manual = 1, attempted = 1 WHERE id = ?1",
                        params![gid, m.anilist_id],
                    )?;
                    if aid != Some(m.anilist_id) {
                        // Let the next scan re-resolve these files against the restored match.
                        tx.execute("UPDATE local_files SET anilist_id = NULL, ep_key = NULL WHERE group_id = ?1", [gid])?;
                    }
                }
                None => {
                    // Files arrive with the next scan and attach to this group by key.
                    tx.execute(
                        "INSERT INTO match_groups (library_id, group_key, folder_path, display_name, title_guess, title_full,
                            year_hint, anilist_id, confidence, manual, attempted, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1.0, 1, 1, ?9)",
                        params![lib, key, folder_path, m.display_name, m.title_guess, m.title_full, m.year_hint, m.anilist_id, now()],
                    )?;
                }
            }
            rep.matches += 1;
        }
        for m in &b.onlinestream_mappings {
            let _ = tx.execute(
                "INSERT INTO onlinestream_mappings
                 (provider_id, anilist_id, provider_anime_id, provider_title, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(provider_id, anilist_id) DO UPDATE SET
                    provider_anime_id = excluded.provider_anime_id,
                    provider_title = excluded.provider_title,
                    updated_at = excluded.updated_at
                 WHERE excluded.updated_at >= onlinestream_mappings.updated_at",
                params![
                    m.provider_id,
                    m.anilist_id,
                    m.provider_anime_id,
                    m.provider_title,
                    m.updated_at,
                ],
            );
        }
    }

    tx.commit()?;
    Ok(rep)
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

pub fn parse(json: &str) -> Result<Backup> {
    let b: Backup = serde_json::from_str(json).context("This file isn't a Kura backup (couldn't read it as JSON).")?;
    if !b.app.is_empty() && b.app != "kura" {
        bail!("This file isn't a Kura backup.");
    }
    if b.version == 0 || b.version > FORMAT {
        bail!("This backup was made by a newer version of Kura. Update Kura and try again.");
    }
    Ok(b)
}

pub fn read_file(path: &Path) -> Result<Backup> {
    let json = std::fs::read_to_string(path).with_context(|| format!("Couldn't read {}", path.display()))?;
    parse(&json)
}

fn write_bytes_atomic(path: &Path, json_bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json_bytes)?;
    std::fs::rename(&tmp, path).or_else(|_| {
        // rename can fail across some network shares; fall back to a direct write.
        let r = std::fs::copy(&tmp, path).map(|_| ());
        let _ = std::fs::remove_file(&tmp);
        r
    })?;
    Ok(())
}

/// Write via a temp file + rename so a crash mid-write never leaves a truncated backup.
pub fn write_file(path: &Path, b: &Backup) -> Result<()> {
    let json_bytes = serde_json::to_vec_pretty(b)?;
    write_bytes_atomic(path, &json_bytes)
}

// ---------------------------------------------------------------------------
// Automatic safety copy
// ---------------------------------------------------------------------------

pub fn safety_paths(data_dir: &Path) -> Vec<PathBuf> {
    let mut paths = vec![data_dir.join("kura_backup.json")];
    if cfg!(test) {
        // Never touch the real user's safety copies from unit tests.
        return paths;
    }

    #[cfg(windows)]
    {
        if let Some(user_profile) = std::env::var_os("USERPROFILE") {
            paths.push(PathBuf::from(user_profile).join(".kura").join("kura_backup.json"));
        }
        if let Some(app_data) = std::env::var_os("APPDATA") {
            paths.push(PathBuf::from(app_data).join("Kura").join("kura_backup.json"));
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            paths.push(home.join(".kura").join("kura_backup.json"));
            paths.push(home.join(".config").join("kura").join("kura_backup.json"));
        }
    }

    paths
}

/// Whether a snapshot holds anything worth protecting on an initial untouched install.
fn meaningful(b: &Backup) -> bool {
    !b.libraries.is_empty()
        || !b.watched.is_empty()
        || !b.progress.is_empty()
        || !b.extensions.is_empty()
        || !b.onlinestream_mappings.is_empty()
}

/// Write `b` once as pretty JSON to all safety paths (including when `meaningful(b)` became
/// `false` after deleting the last library or extension so stale data is never left behind).
pub fn write_safety_copy_from(b: &Backup, data_dir: &Path) {
    let Ok(json_bytes) = serde_json::to_vec_pretty(b) else {
        return;
    };
    let paths = safety_paths(data_dir);
    let any_exists = paths.iter().any(|p| p.exists());
    if !meaningful(b) && !any_exists && DIRTY_REVISION.load(Ordering::Acquire) == 0 {
        return;
    }
    for path in paths {
        if let Err(e) = write_bytes_atomic(&path, &json_bytes) {
            crate::klog!("[kura] safety backup to {} failed: {e:#}", path.display());
        }
    }
}

/// Synchronous safety copy helper for a raw connection + data_dir.
pub fn write_safety_copy_conn(conn: &Connection, data_dir: &Path) {
    write_safety_copy_from(&snapshot(conn), data_dir);
}

/// Synchronous safety copy flush on app shutdown (writes if dirty or if no safety copy exists yet for meaningful data).
pub fn write_safety_copy(st: &AppState) {
    let cur_rev = DIRTY_REVISION.load(Ordering::Acquire);
    let last_rev = FLUSHED_REVISION.load(Ordering::Acquire);
    let paths = safety_paths(&st.data_dir);
    let any_exists = paths.iter().any(|p| p.exists());
    if cur_rev == last_rev && any_exists {
        return;
    }
    let snap = snapshot(&st.db());
    if cur_rev != last_rev || meaningful(&snap) {
        write_safety_copy_from(&snap, &st.data_dir);
        FLUSHED_REVISION.store(cur_rev, Ordering::Release);
    }
}

/// On startup: restore ONLY when the database is genuinely empty (fresh install / wiped DB),
/// never resurrecting deleted libraries/extensions or overwriting live settings.
pub fn restore_if_needed(conn: &Connection, data_dir: &Path) -> Result<bool> {
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0) };
    let watch_state = count("SELECT COUNT(*) FROM watch_state") + count("SELECT COUNT(*) FROM watch_progress");
    let libraries = count("SELECT COUNT(*) FROM libraries");
    let user_prefs = count("SELECT COUNT(*) FROM settings WHERE key LIKE 'pref.%' AND key NOT LIKE 'pref.%_v1'");
    let extensions = count("SELECT COUNT(*) FROM installed_extensions");

    if !(watch_state == 0 && libraries == 0 && user_prefs == 0 && extensions == 0) {
        return Ok(false);
    }

    let newest = safety_paths(data_dir)
        .into_iter()
        .filter_map(|p| read_file(&p).ok().map(|b| (p, b)))
        .max_by_key(|(_, b)| b.updated_at);
    let Some((path, b)) = newest else { return Ok(false) };

    let opts = ImportOptions {
        settings: !b.settings.is_empty(),
        libraries: !b.libraries.is_empty(),
        history: !b.watched.is_empty() || !b.progress.is_empty(),
        track_prefs: !b.track_prefs.is_empty(),
        matches: !b.manual_matches.is_empty() || !b.onlinestream_mappings.is_empty(),
        extensions: false,
    };
    if !(opts.settings || opts.libraries || opts.history || opts.track_prefs || opts.matches) {
        return Ok(false);
    }
    apply(conn, &b, &opts)?;
    crate::klog!("[kura] restored user data from backup: {}", path.display());
    Ok(true)
}

static DIRTY_REVISION: AtomicU64 = AtomicU64::new(0);
static FLUSHED_REVISION: AtomicU64 = AtomicU64::new(0);

/// Ask for a safety copy soon. Cheap atomic revision bump — flushed on a 60s debounce in the background.
pub fn mark_dirty() {
    DIRTY_REVISION.fetch_add(1, Ordering::AcqRel);
}

/// Background writer: debounced to 60s and driven by [`DIRTY_REVISION`] without full-table `MAX(...)` scans.
pub fn start_auto(st: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let cur_rev = DIRTY_REVISION.load(Ordering::Acquire);
            if cur_rev == FLUSHED_REVISION.load(Ordering::Acquire) {
                continue;
            }
            let st_clone = st.clone();
            let flushed = tauri::async_runtime::spawn_blocking(move || {
                let snap = {
                    let conn = st_clone.db_read();
                    snapshot(&conn)
                };
                write_safety_copy_from(&snap, &st_clone.data_dir);
                cur_rev
            })
            .await;
            if let Ok(done_rev) = flushed {
                FLUSHED_REVISION.store(done_rev, Ordering::Release);
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Summaries for the UI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    pub name: String,
    pub folders: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub path: String,
    pub created_at: i64,
    pub app_version: String,
    pub format: u32,
    pub settings: usize,
    pub libraries: Vec<LibrarySummary>,
    pub watched: usize,
    pub shows_watched: usize,
    pub progress: usize,
    pub track_prefs: usize,
    pub manual_matches: usize,
    pub extensions: usize,
    pub extension_names: Vec<String>,
}

pub fn summarize(path: &Path, b: &Backup) -> BackupSummary {
    let mut shows: Vec<i64> = b.watched.iter().map(|w| w.anilist_id).collect();
    shows.sort_unstable();
    shows.dedup();
    let extension_names: Vec<String> = b
        .extensions
        .iter()
        .filter(|e| !e.id.trim().is_empty() && crate::extensions::is_supported_ext_type(&e.ext_type))
        .map(|e| {
            let name = e.name.trim();
            if name.is_empty() {
                e.id.trim().to_string()
            } else {
                name.to_string()
            }
        })
        .collect();
    BackupSummary {
        path: path.to_string_lossy().to_string(),
        created_at: b.updated_at,
        app_version: b.app_version.clone(),
        format: b.version,
        settings: b.settings.keys().filter(|k| k.starts_with("pref.")).count(),
        libraries: b.libraries.iter().map(|l| LibrarySummary { name: l.name.clone(), folders: l.folders.len() }).collect(),
        watched: b.watched.len(),
        shows_watched: shows.len(),
        progress: b.progress.len(),
        track_prefs: b.track_prefs.len(),
        manual_matches: b.manual_matches.len(),
        extensions: extension_names.len(),
        extension_names,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrate_for_tests(&conn).unwrap();
        conn
    }

    fn seed(conn: &Connection) {
        crate::db::set_setting(conn, "pref.theme", "manga").unwrap();
        conn.execute("INSERT INTO libraries (id, name, kind, created_at) VALUES (1, 'Anime', 'anime', 0)", []).unwrap();
        conn.execute("INSERT INTO library_folders (library_id, path) VALUES (1, 'D:\\Anime')", []).unwrap();
        conn.execute("INSERT INTO watch_state VALUES (21, '1', 100), (21, '2', 200)", []).unwrap();
        conn.execute("INSERT INTO watch_progress VALUES (21, '3', 600.0, 1400.0, 300)", []).unwrap();
        conn.execute("INSERT INTO media_track_prefs VALUES (21, 'eng', 'off', NULL, 50)", []).unwrap();
        conn.execute(
            "INSERT INTO match_groups (library_id, group_key, folder_path, display_name, title_guess, title_full, anilist_id, confidence, manual, attempted, created_at)
             VALUES (1, 'D:\\Anime|dir:One Piece', 'D:\\Anime\\One Piece', 'One Piece', 'One Piece', 'One Piece', 21, 1.0, 1, 1, 0)",
            [],
        )
        .unwrap();
    }

    #[test]
    fn roundtrip_restores_everything() {
        let a = fresh();
        seed(&a);
        let b = snapshot(&a);
        assert_eq!(b.watched.len(), 2);
        assert_eq!(b.manual_matches.len(), 1);
        assert_eq!(b.manual_matches[0].root, "D:\\Anime");
        assert_eq!(b.manual_matches[0].rest, "dir:One Piece");

        let json = serde_json::to_string(&b).unwrap();
        let parsed = parse(&json).unwrap();

        let c = fresh();
        let rep = apply(&c, &parsed, &ImportOptions::ALL).unwrap();
        assert_eq!(rep.libraries_added, 1);
        assert_eq!(rep.watched_added, 2);
        assert_eq!(rep.progress_added, 1);
        assert_eq!(rep.matches, 1);
        let key: String = c.query_row("SELECT group_key FROM match_groups WHERE manual = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(key, "D:\\Anime|dir:One Piece");
        assert_eq!(crate::db::get_setting(&c, "pref.theme").unwrap().as_deref(), Some("manga"));

        // Importing twice changes nothing.
        let again = apply(&c, &parsed, &ImportOptions::ALL).unwrap();
        assert_eq!(again.libraries_added + again.folders_added + again.watched_added + again.progress_added, 0);
    }

    #[test]
    fn manual_match_follows_moved_folder() {
        let a = fresh();
        seed(&a);
        let b = snapshot(&a);

        let c = fresh();
        c.execute("INSERT INTO libraries (id, name, kind, created_at) VALUES (1, 'Anime', 'anime', 0)", []).unwrap();
        c.execute("INSERT INTO library_folders (library_id, path) VALUES (1, 'E:\\Anime')", []).unwrap();
        let opts = ImportOptions { libraries: false, ..ImportOptions::ALL };
        apply(&c, &b, &opts).unwrap();
        let (key, fp): (String, String) = c
            .query_row("SELECT group_key, folder_path FROM match_groups WHERE manual = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(key, "E:\\Anime|dir:One Piece");
        assert_eq!(fp, "E:\\Anime\\One Piece");
    }

    #[test]
    fn newer_local_progress_wins_and_watched_blocks_progress() {
        let a = fresh();
        seed(&a);
        let b = snapshot(&a);

        let c = fresh();
        c.execute("INSERT INTO watch_progress VALUES (21, '3', 900.0, 1400.0, 999)", []).unwrap();
        c.execute("INSERT INTO watch_state VALUES (21, '2', 5)", []).unwrap();
        apply(&c, &b, &ImportOptions::ALL).unwrap();
        let pos: f64 = c.query_row("SELECT position FROM watch_progress WHERE ep_key = '3'", [], |r| r.get(0)).unwrap();
        assert_eq!(pos, 900.0);
        let at: i64 = c.query_row("SELECT watched_at FROM watch_state WHERE ep_key = '2'", [], |r| r.get(0)).unwrap();
        assert_eq!(at, 5, "existing watch history is kept");
    }

    #[test]
    fn reads_v1_backups_and_rejects_garbage() {
        let v1 = r#"{"version":1,"updated_at":5,"settings":{"pref.theme":"manga"},"libraries":[{"name":"Anime","kind":"anime","folders":["C:\\A"]}]}"#;
        let b = parse(v1).unwrap();
        assert_eq!(b.libraries.len(), 1);
        assert!(b.watched.is_empty());
        assert!(parse("{\"hello\":1}").is_err());
        assert!(parse(r#"{"app":"kura","version":99,"updated_at":1}"#).is_err());
    }

    #[test]
    fn strips_secrets_player_path_and_blocks_zombie_resurrection() {
        let a = fresh();
        seed(&a);
        crate::db::set_setting(&a, "anilist.token", "secret-jwt").unwrap();
        crate::db::set_setting(&a, "pref.player_path", r"C:\Windows\System32\cmd.exe").unwrap();
        crate::db::set_setting(&a, "pref.jimaku_api_key", "jimaku-secret").unwrap();
        crate::db::set_setting(&a, "pref.fanart_api_key", "fanart-secret").unwrap();

        let mut snap = snapshot(&a);
        assert!(!snap.settings.contains_key("anilist.token"));
        assert!(!snap.settings.contains_key("pref.player_path"));
        assert!(!snap.settings.contains_key("pref.jimaku_api_key"));
        assert!(!snap.settings.contains_key("pref.fanart_api_key"));

        // Even if crafted JSON injects player_path or unsupported ext_type, apply() must reject them
        snap.settings
            .insert("pref.player_path".into(), r"C:\Windows\System32\cmd.exe".into());
        snap.settings
            .insert("anilist.token".into(), "evil-token".into());
        snap.extensions.push(ExtensionBackupRow {
            id: "bad-ext".into(),
            name: "Bad".into(),
            version: "1.0".into(),
            ext_type: "arbitrary-rce".into(),
            language: "javascript".into(),
            manifest_uri: String::new(),
            icon_url: None,
            author: None,
            description: None,
            payload: "console.log(1)".into(),
            user_config_json: "{}".into(),
            enabled: true,
            updated_at: 1,
        });

        let c = fresh();
        apply(&c, &snap, &ImportOptions::ALL).unwrap();
        assert_eq!(crate::db::get_setting(&c, "pref.player_path").unwrap(), None);
        assert_eq!(crate::db::get_setting(&c, "anilist.token").unwrap(), None);
        let ext_cnt: i64 = c
            .query_row("SELECT COUNT(*) FROM installed_extensions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ext_cnt, 0);

        // Deleting last library flushes empty libraries to kura_backup.json and never resurrects when user_prefs > 0
        let tmp = std::env::temp_dir().join(format!("kura_h14_test_{}", crate::db::random_hex_token(6)));
        std::fs::create_dir_all(&tmp).unwrap();
        write_safety_copy_from(&snapshot(&a), &tmp);
        assert_eq!(read_file(&tmp.join("kura_backup.json")).unwrap().libraries.len(), 1);

        let cleared = fresh();
        crate::db::seed_default_prefs_if_missing(&cleared).unwrap();
        mark_dirty();
        write_safety_copy_from(&snapshot(&cleared), &tmp);
        assert_eq!(read_file(&tmp.join("kura_backup.json")).unwrap().libraries.len(), 0);
        assert!(!restore_if_needed(&cleared, &tmp).unwrap());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn extension_opt_in_disabled_on_import_and_safe_folder_validation() {
        let mut b = Backup {
            app: "kura".into(),
            version: FORMAT,
            updated_at: 100,
            app_version: "0.5.0".into(),
            ..Default::default()
        };
        b.libraries.push(BackupLibrary {
            name: "Mixed".into(),
            kind: "anime".into(),
            folders: vec![
                r"D:\Anime".into(),
                r"C:\".into(),
                r"C:\Windows".into(),
                r"C:\Windows\System32".into(),
                r"\\evil\share\anime".into(),
                "relative/folder".into(),
                "/".into(),
                "/etc".into(),
                "/usr".into(),
                "/System".into(),
                String::new(),
            ],
        });
        b.extensions.push(ExtensionBackupRow {
            id: "stream-ext".into(),
            name: "Stream Provider".into(),
            version: "1.0.0".into(),
            ext_type: "onlinestream-provider".into(),
            language: "javascript".into(),
            manifest_uri: "https://example.com/manifest.json".into(),
            icon_url: None,
            author: None,
            description: None,
            payload: "class Provider {}".into(),
            user_config_json: "{}".into(),
            enabled: true,
            updated_at: 50,
        });

        let summary = summarize(Path::new("backup.json"), &b);
        assert_eq!(summary.extensions, 1);
        assert_eq!(summary.extension_names, vec!["Stream Provider".to_string()]);

        // 1. `extensions: false` skips extension import completely and filters unsafe folders
        let c1 = fresh();
        let rep1 = apply(
            &c1,
            &b,
            &ImportOptions {
                extensions: false,
                ..ImportOptions::ALL
            },
        )
        .unwrap();
        assert_eq!(rep1.extensions, 0);
        assert_eq!(rep1.folders_added, 1);
        let ext_cnt1: i64 = c1
            .query_row("SELECT COUNT(*) FROM installed_extensions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ext_cnt1, 0);
        let saved_folder: String = c1
            .query_row("SELECT path FROM library_folders", [], |r| r.get(0))
            .unwrap();
        assert_eq!(saved_folder, r"D:\Anime");

        // 2. `extensions: true` imports valid extensions with `enabled = 0` (never auto-enabled!)
        let c2 = fresh();
        let rep2 = apply(&c2, &b, &ImportOptions::ALL).unwrap();
        assert_eq!(rep2.extensions, 1);
        let enabled_val: i64 = c2
            .query_row(
                "SELECT enabled FROM installed_extensions WHERE id = 'stream-ext'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(enabled_val, 0);

        // 3. `restore_if_needed` never auto-imports JS extensions on fresh boot
        let tmp = std::env::temp_dir().join(format!(
            "kura_ext_restore_test_{}",
            crate::db::random_hex_token(6)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        write_file(&tmp.join("kura_backup.json"), &b).unwrap();
        let c3 = fresh();
        assert!(restore_if_needed(&c3, &tmp).unwrap());
        let ext_cnt3: i64 = c3
            .query_row("SELECT COUNT(*) FROM installed_extensions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ext_cnt3, 0);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
