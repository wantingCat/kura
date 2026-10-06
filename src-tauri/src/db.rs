//! SQLite storage: schema, migrations and small query helpers.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

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

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA synchronous = NORMAL;",
    )?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(&format!("BEGIN; {SCHEMA_V1} PRAGMA user_version = 1; COMMIT;"))?;
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
    pub format: Option<String>,
    pub status: Option<String>,
    pub episodes: Option<i64>,
    pub next_airing_episode: Option<i64>,
    pub fetched_at: i64,
    pub episodes_fetched_at: Option<i64>,
}

pub fn media_lite(conn: &Connection, id: i64) -> Result<Option<MediaLite>> {
    Ok(conn
        .query_row(
            "SELECT anilist_id, id_mal, format, status, episodes, next_airing_episode, fetched_at, episodes_fetched_at
             FROM media WHERE anilist_id = ?1",
            [id],
            |r| {
                Ok(MediaLite {
                    id: r.get(0)?,
                    id_mal: r.get(1)?,
                    format: r.get(2)?,
                    status: r.get(3)?,
                    episodes: r.get(4)?,
                    next_airing_episode: r.get(5)?,
                    fetched_at: r.get(6)?,
                    episodes_fetched_at: r.get(7)?,
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
