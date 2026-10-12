//! SQLite storage: schema, migrations and small query helpers.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SCHEMA_V1: &str = r#"
CREATE TABLE libraries (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    kind        TEXT NOT NULL DEFAULT 'anime',
    created_at  INTEGER NOT NULL
);

CREATE TABLE library_folders (
    id          INTEGER PRIMARY KEY,
    library_id  INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    path        TEXT NOT NULL,
    UNIQUE(library_id, path)
);

CREATE TABLE media (
    anilist_id          INTEGER PRIMARY KEY,
    id_mal              INTEGER,
    title_romaji        TEXT,
    title_english       TEXT,
    title_native        TEXT,
    synonyms            TEXT NOT NULL DEFAULT '[]',
    format              TEXT,
    status              TEXT,
    episodes            INTEGER,
    duration            INTEGER,
    season              TEXT,
    season_year         INTEGER,
    start_date          TEXT,
    end_date            TEXT,
    description         TEXT,
    genres              TEXT NOT NULL DEFAULT '[]',
    tags                TEXT NOT NULL DEFAULT '[]',
    studios             TEXT NOT NULL DEFAULT '[]',
    cover_url           TEXT,
    cover_color         TEXT,
    banner_url          TEXT,
    cover_path          TEXT,
    banner_path         TEXT,
    average_score       INTEGER,
    next_airing_episode INTEGER,
    next_airing_at      INTEGER,
    fetched_at          INTEGER NOT NULL,
    episodes_fetched_at INTEGER,
    episodes_source     TEXT
);

CREATE TABLE media_relations (
    anilist_id      INTEGER NOT NULL,
    related_id      INTEGER NOT NULL,
    relation_type   TEXT NOT NULL,
    title           TEXT,
    format          TEXT,
    media_type      TEXT,
    status          TEXT,
    episodes        INTEGER,
    season_year     INTEGER,
    cover_url       TEXT,
    PRIMARY KEY (anilist_id, related_id)
);

CREATE TABLE episodes (
    anilist_id      INTEGER NOT NULL,
    ep_key          TEXT NOT NULL,
    number          INTEGER NOT NULL,
    is_special      INTEGER NOT NULL DEFAULT 0,
    title_en        TEXT,
    title_ja        TEXT,
    title_romaji    TEXT,
    overview        TEXT,
    air_date        TEXT,
    runtime         INTEGER,
    thumb_url       TEXT,
    thumb_path      TEXT,
    tvdb_season     INTEGER,
    tvdb_episode    INTEGER,
    absolute_number INTEGER,
    filler          INTEGER NOT NULL DEFAULT 0,
    recap           INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (anilist_id, ep_key)
);

CREATE TABLE match_groups (
    id              INTEGER PRIMARY KEY,
    library_id      INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    group_key       TEXT NOT NULL,
    folder_path     TEXT,
    display_name    TEXT NOT NULL,
    title_guess     TEXT NOT NULL,
    title_full      TEXT NOT NULL,
    year_hint       INTEGER,
    anilist_id      INTEGER,
    confidence      REAL,
    manual          INTEGER NOT NULL DEFAULT 0,
    attempted       INTEGER NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL,
    UNIQUE(library_id, group_key)
);

CREATE TABLE local_files (
    id          INTEGER PRIMARY KEY,
    library_id  INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    group_id    INTEGER REFERENCES match_groups(id) ON DELETE SET NULL,
    path        TEXT NOT NULL UNIQUE,
    size        INTEGER NOT NULL,
    mtime       INTEGER NOT NULL,
    parsed      TEXT NOT NULL,
    season_hint INTEGER,
    special_hint TEXT,
    anilist_id  INTEGER,
    ep_key      TEXT,
    added_at    INTEGER NOT NULL
);
CREATE INDEX idx_files_media ON local_files(anilist_id);
CREATE INDEX idx_files_group ON local_files(group_id);

CREATE TABLE watch_state (
    anilist_id  INTEGER NOT NULL,
    ep_key      TEXT NOT NULL,
    watched_at  INTEGER NOT NULL,
    PRIMARY KEY (anilist_id, ep_key)
);

CREATE TABLE settings (
    key     TEXT PRIMARY KEY,
    value   TEXT NOT NULL
);
"#;

/// v0.2: per-episode playback position for resume + progress bars.
const SCHEMA_V2: &str = r#"
CREATE TABLE IF NOT EXISTS watch_progress (
    anilist_id  INTEGER NOT NULL,
    ep_key      TEXT NOT NULL,
    position    REAL NOT NULL,
    duration    REAL NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (anilist_id, ep_key)
);
"#;

/// v0.3: per-show audio & subtitle track preferences.
const SCHEMA_V3: &str = r#"
CREATE TABLE IF NOT EXISTS media_track_prefs (
    anilist_id    INTEGER PRIMARY KEY,
    audio_pref    TEXT,
    sub_pref      TEXT,
    sub_fallback  TEXT,
    updated_at    INTEGER NOT NULL
);
"#;

/// v0.4: ClearLogos and priority artwork (logo_url, logo_path).
const SCHEMA_V4: &str = r#"
ALTER TABLE media ADD COLUMN logo_url TEXT;
ALTER TABLE media ADD COLUMN logo_path TEXT;
"#;

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;",
    )?;
    migrate(&conn)?;
    let _ = conn.execute("UPDATE media SET logo_url = NULL, logo_path = NULL WHERE logo_url LIKE '%/icons/%'", []);

    // Check if user data needs to be restored from persistent backup
    let data_dir = path.parent().unwrap_or(Path::new("."));
    let restored = restore_user_data_if_needed(&conn, data_dir).unwrap_or(false);
    let _ = seed_default_prefs_if_missing(&conn);
    if !restored {
        backup_user_data(&conn, data_dir);
    }

    Ok(conn)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupLibrary {
    pub name: String,
    pub kind: String,
    pub folders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDataBackup {
    pub version: u32,
    pub updated_at: i64,
    pub settings: HashMap<String, String>,
    pub libraries: Vec<BackupLibrary>,
}

fn get_backup_paths(data_dir: &Path) -> Vec<PathBuf> {
    let mut paths = vec![data_dir.join("kura_backup.json")];

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

pub fn backup_user_data(conn: &Connection, data_dir: &Path) {
    let _ = (|| -> Result<()> {
        let mut settings_stmt = conn.prepare("SELECT key, value FROM settings")?;
        let settings_map: HashMap<String, String> = settings_stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .flatten()
            .collect();

        let mut lib_stmt = conn.prepare("SELECT id, name, kind FROM libraries")?;
        let mut libraries = Vec::new();
        let lib_rows = lib_stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for lib_res in lib_rows.flatten() {
            let (lib_id, name, kind) = lib_res;
            let mut folder_stmt =
                conn.prepare("SELECT path FROM library_folders WHERE library_id = ?1")?;
            let folders: Vec<String> = folder_stmt
                .query_map([lib_id], |r| r.get(0))?
                .flatten()
                .collect();
            libraries.push(BackupLibrary {
                name,
                kind,
                folders,
            });
        }

        // Never overwrite a good backup with an empty database
        if settings_map.is_empty() && libraries.is_empty() {
            return Ok(());
        }

        let backup = UserDataBackup {
            version: 1,
            updated_at: now(),
            settings: settings_map,
            libraries,
        };

        let json = serde_json::to_string_pretty(&backup)?;
        let backup_paths = get_backup_paths(data_dir);
        for path in backup_paths {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, &json);
        }
        Ok(())
    })();
}

pub fn restore_user_data_if_needed(conn: &Connection, data_dir: &Path) -> Result<bool> {
    let settings_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE key LIKE 'pref.%'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let libraries_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM libraries", [], |r| r.get(0))
        .unwrap_or(0);

    // If we already have preferences and libraries, nothing to restore.
    if settings_count > 0 && libraries_count > 0 {
        return Ok(false);
    }

    let backup_paths = get_backup_paths(data_dir);
    for path in backup_paths {
        if path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(backup) = serde_json::from_str::<UserDataBackup>(&content) {
                    let mut restored_something = false;

                    // Restore settings if empty
                    if settings_count == 0 && !backup.settings.is_empty() {
                        for (k, v) in &backup.settings {
                            let _ = conn.execute(
                                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                                params![k, v],
                            );
                        }
                        restored_something = true;
                    }

                    // Restore libraries if empty
                    if libraries_count == 0 && !backup.libraries.is_empty() {
                        for lib in &backup.libraries {
                            let _ = conn.execute(
                                "INSERT INTO libraries (name, kind, created_at) VALUES (?1, ?2, ?3)",
                                params![lib.name, lib.kind, now()],
                            );
                            let lib_id = conn.last_insert_rowid();
                            for folder in &lib.folders {
                                let _ = conn.execute(
                                    "INSERT OR IGNORE INTO library_folders (library_id, path) VALUES (?1, ?2)",
                                    params![lib_id, folder],
                                );
                            }
                        }
                        restored_something = true;
                    }

                    if restored_something {
                        eprintln!("[kura] restored user data from backup: {:?}", path);
                        return Ok(true);
                    }
                }
            }
        }
    }

    Ok(false)
}

pub fn seed_default_prefs_if_missing(conn: &Connection) -> Result<()> {
    let defaults = [
        ("pref.theme", "comic"),
        ("pref.player", "mpv"),
        ("pref.audio_lang", "jpn"),
        ("pref.sub_lang", "jpn"),
        ("pref.sub_fallback", "eng"),
        ("pref.watched_threshold", "90"),
        ("pref.resume", "1"),
        ("pref.autoplay", "1"),
        ("pref.group_seasons", "1"),
        ("pref.auto_update", "1"),
        ("pref.folder_watcher", "1"),
        ("pref.discord_rpc", "1"),
        ("pref.discord_spoilers", "0"),
        ("pref.discord_buttons", "1"),
        ("pref.mpc_port", "13579"),
    ];

    for (k, v) in defaults {
        conn.execute(
            "INSERT OR IGNORE INTO settings(key, value) VALUES (?1, ?2)",
            params![k, v],
        )?;
    }
    Ok(())
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V1} PRAGMA user_version = 1; COMMIT;"))?;
    }
    if version < 2 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V2} PRAGMA user_version = 2; COMMIT;"))?;
    }
    if version < 3 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V3} PRAGMA user_version = 3; COMMIT;"))?;
    }
    if version < 4 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V4} PRAGMA user_version = 4; COMMIT;"))?;
    }
    Ok(())
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn get_setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// Minimal media info used by the matcher / resolver.
#[derive(Debug, Clone)]
pub struct MediaLite {
    pub id: i64,
    pub id_mal: Option<i64>,
    pub title_romaji: Option<String>,
    pub title_english: Option<String>,
    pub format: Option<String>,
    pub status: Option<String>,
    pub episodes: Option<i64>,
    pub season_year: Option<u32>,
    pub start_date: Option<String>,
    pub next_airing_episode: Option<i64>,
    pub fetched_at: i64,
    pub episodes_fetched_at: Option<i64>,
}

pub fn media_lite(conn: &Connection, id: i64) -> Result<Option<MediaLite>> {
    Ok(conn
        .query_row(
            "SELECT anilist_id, id_mal, title_romaji, title_english, format, status, episodes, season_year, start_date, next_airing_episode, fetched_at, episodes_fetched_at
             FROM media WHERE anilist_id = ?1",
            [id],
            |r| {
                Ok(MediaLite {
                    id: r.get(0)?,
                    id_mal: r.get(1)?,
                    title_romaji: r.get(2)?,
                    title_english: r.get(3)?,
                    format: r.get(4)?,
                    status: r.get(5)?,
                    episodes: r.get(6)?,
                    season_year: r.get(7)?,
                    start_date: r.get(8)?,
                    next_airing_episode: r.get(9)?,
                    fetched_at: r.get(10)?,
                    episodes_fetched_at: r.get(11)?,
                })
            },
        )
        .optional()?)
}

#[derive(Debug, Clone)]
pub struct RelationLite {
    pub related_id: i64,
    pub relation_type: String,
    pub format: Option<String>,
    pub media_type: Option<String>,
    pub episodes: Option<i64>,
}

pub fn relations(conn: &Connection, id: i64) -> Result<Vec<RelationLite>> {
    let mut stmt = conn.prepare(
        "SELECT related_id, relation_type, format, media_type, episodes FROM media_relations WHERE anilist_id = ?1",
    )?;
    let rows = stmt
        .query_map([id], |r| {
            Ok(RelationLite {
                related_id: r.get(0)?,
                relation_type: r.get(1)?,
                format: r.get(2)?,
                media_type: r.get(3)?,
                episodes: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// (ep_key, tvdb_season, tvdb_episode, absolute_number, is_special)
pub fn episode_index(conn: &Connection, id: i64) -> Result<Vec<(String, Option<i64>, Option<i64>, Option<i64>, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT ep_key, tvdb_season, tvdb_episode, absolute_number, is_special FROM episodes WHERE anilist_id = ?1",
    )?;
    let rows = stmt
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? != 0)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaTrackPref {
    pub audio_pref: Option<String>,
    pub sub_pref: Option<String>,
    pub sub_fallback: Option<String>,
}

pub fn get_media_track_pref(conn: &Connection, anilist_id: i64) -> Result<Option<MediaTrackPref>> {
    conn.query_row(
        "SELECT audio_pref, sub_pref, sub_fallback FROM media_track_prefs WHERE anilist_id = ?1",
        [anilist_id],
        |r| {
            Ok(MediaTrackPref {
                audio_pref: r.get(0)?,
                sub_pref: r.get(1)?,
                sub_fallback: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn set_media_track_pref(conn: &Connection, anilist_id: i64, pref: Option<MediaTrackPref>) -> Result<()> {
    match pref {
        Some(p) => {
            conn.execute(
                "INSERT INTO media_track_prefs (anilist_id, audio_pref, sub_pref, sub_fallback, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(anilist_id) DO UPDATE SET
                    audio_pref = excluded.audio_pref,
                    sub_pref = excluded.sub_pref,
                    sub_fallback = excluded.sub_fallback,
                    updated_at = excluded.updated_at",
                rusqlite::params![anilist_id, p.audio_pref, p.sub_pref, p.sub_fallback, now()],
            )?;
        }
        None => {
            conn.execute("DELETE FROM media_track_prefs WHERE anilist_id = ?1", [anilist_id])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seed_defaults_preserves_custom_and_adds_missing() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA_V1).unwrap();

        // Simulate user having custom dark theme and custom threshold
        set_setting(&conn, "pref.theme", "manga").unwrap();
        set_setting(&conn, "pref.watched_threshold", "85").unwrap();

        seed_default_prefs_if_missing(&conn).unwrap();

        // Custom prefs must be preserved
        assert_eq!(get_setting(&conn, "pref.theme").unwrap(), Some("manga".into()));
        assert_eq!(get_setting(&conn, "pref.watched_threshold").unwrap(), Some("85".into()));

        // Default prefs must be populated
        assert_eq!(get_setting(&conn, "pref.player").unwrap(), Some("mpv".into()));
        assert_eq!(get_setting(&conn, "pref.audio_lang").unwrap(), Some("jpn".into()));
    }

    #[test]
    fn test_backup_and_restore_user_data() {
        let temp_dir = std::env::temp_dir().join(format!("kura_test_{}", now()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        // 1. Initial db with user libraries & prefs
        let conn1 = Connection::open_in_memory().unwrap();
        conn1.execute_batch(SCHEMA_V1).unwrap();
        set_setting(&conn1, "pref.player", "mpv").unwrap();
        set_setting(&conn1, "pref.theme", "manga").unwrap();
        conn1.execute("INSERT INTO libraries (id, name, kind, created_at) VALUES (1, 'Anime', 'anime', ?1)", [now()]).unwrap();
        conn1.execute("INSERT INTO library_folders (library_id, path) VALUES (1, 'C:\\AnimeFolder')", []).unwrap();

        // Trigger backup
        backup_user_data(&conn1, &temp_dir);

        let backup_file = temp_dir.join("kura_backup.json");
        assert!(backup_file.is_file(), "Backup file should be created");

        // 2. New blank database (simulating upgrade or wiped db)
        let conn2 = Connection::open_in_memory().unwrap();
        conn2.execute_batch(SCHEMA_V1).unwrap();

        let restored = restore_user_data_if_needed(&conn2, &temp_dir).unwrap();
        assert!(restored, "Should restore from backup");

        assert_eq!(get_setting(&conn2, "pref.player").unwrap(), Some("mpv".into()));
        assert_eq!(get_setting(&conn2, "pref.theme").unwrap(), Some("manga".into()));

        let lib_name: String = conn2.query_row("SELECT name FROM libraries WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(lib_name, "Anime");

        let folder_path: String = conn2.query_row("SELECT path FROM library_folders WHERE library_id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(folder_path, "C:\\AnimeFolder");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
