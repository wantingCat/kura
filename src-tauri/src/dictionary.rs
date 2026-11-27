//! Yomitan-compatible Japanese dictionary engine, condition-checked deinflector (`japanese-transforms`),
//! furigana segmenter (`distributeFurigana`), Yomitan `.zip` v3 importer, and AnkiConnect client.
//!
//! Licensed under GPL-3.0-or-later (compatible with Yomitan).

use crate::db::now;
use anyhow::{anyhow, bail, Context, Result};
use flate2::read::DeflateDecoder;
use rusqlite::{params, params_from_iter, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter};

const DICT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS dictionaries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    title       TEXT NOT NULL UNIQUE,
    revision    TEXT NOT NULL DEFAULT '1',
    format      INTEGER NOT NULL DEFAULT 3,
    term_count  INTEGER NOT NULL DEFAULT 0,
    kanji_count INTEGER NOT NULL DEFAULT 0,
    meta_count  INTEGER NOT NULL DEFAULT 0,
    imported_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS terms (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    dict_id         INTEGER NOT NULL REFERENCES dictionaries(id) ON DELETE CASCADE,
    expression      TEXT NOT NULL,
    reading         TEXT NOT NULL,
    definition_tags TEXT NOT NULL DEFAULT '',
    rules           TEXT NOT NULL DEFAULT '',
    score           INTEGER NOT NULL DEFAULT 0,
    glossary_json   TEXT NOT NULL,
    sequence        INTEGER NOT NULL DEFAULT 0,
    term_tags       TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_terms_expr ON terms(expression);
CREATE INDEX IF NOT EXISTS idx_terms_read ON terms(reading);

CREATE TABLE IF NOT EXISTS term_meta (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    dict_id    INTEGER NOT NULL REFERENCES dictionaries(id) ON DELETE CASCADE,
    expression TEXT NOT NULL,
    mode       TEXT NOT NULL,
    data_json  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_term_meta_expr ON term_meta(expression);

CREATE TABLE IF NOT EXISTS kanji (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    dict_id       INTEGER NOT NULL REFERENCES dictionaries(id) ON DELETE CASCADE,
    character     TEXT NOT NULL,
    onyomi        TEXT NOT NULL DEFAULT '',
    kunyomi       TEXT NOT NULL DEFAULT '',
    tags          TEXT NOT NULL DEFAULT '',
    meanings_json TEXT NOT NULL,
    stats_json    TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_kanji_char ON kanji(character);
"#;

static INITIALIZED_DICT_DIRS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
static DICT_READ_POOL: OnceLock<Mutex<HashMap<PathBuf, Vec<Connection>>>> = OnceLock::new();

const MAX_DICT_READ_POOL_PER_DIR: usize = 4;

fn ensure_dict_db_initialized(data_dir: &Path) -> Result<PathBuf> {
    let db_path = data_dir.join("dictionary.db");
    let initialized = INITIALIZED_DICT_DIRS.get_or_init(|| Mutex::new(HashSet::new()));
    {
        let guard = initialized.lock().unwrap();
        if guard.contains(&db_path) && db_path.exists() {
            return Ok(db_path);
        }
    }

    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(&db_path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    conn.execute_batch(DICT_SCHEMA)?;
    ensure_starter_dictionary(&conn)?;
    drop(conn);

    initialized.lock().unwrap().insert(db_path.clone());
    Ok(db_path)
}

pub fn open_dict_db(data_dir: &Path) -> Result<Connection> {
    let db_path = ensure_dict_db_initialized(data_dir)?;
    let conn = Connection::open(db_path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(conn)
}

struct PooledDictReadConn {
    db_path: PathBuf,
    conn: Option<Connection>,
}

impl Drop for PooledDictReadConn {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let pool = DICT_READ_POOL.get_or_init(|| Mutex::new(HashMap::new()));
            if let Ok(mut map) = pool.lock() {
                let list = map.entry(self.db_path.clone()).or_default();
                if list.len() < MAX_DICT_READ_POOL_PER_DIR {
                    list.push(conn);
                }
            }
        }
    }
}

/// Fast read-only pooled lookup against `data_dir/dictionary.db` (`C-07`).
pub fn lookup_in_dir(data_dir: &Path, text: &str, char_offset: usize) -> Result<LookupResult> {
    let db_path = ensure_dict_db_initialized(data_dir)?;
    let pool = DICT_READ_POOL.get_or_init(|| Mutex::new(HashMap::new()));
    let conn = pool
        .lock()
        .ok()
        .and_then(|mut map| map.get_mut(&db_path).and_then(|v| v.pop()));

    let conn = match conn {
        Some(c) => c,
        None => {
            let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
            let c = Connection::open_with_flags(&db_path, flags)?;
            c.execute_batch(
                "PRAGMA busy_timeout = 5000;
                 PRAGMA query_only = ON;",
            )?;
            c
        }
    };

    let guard = PooledDictReadConn {
        db_path,
        conn: Some(conn),
    };
    lookup_at(guard.conn.as_ref().unwrap(), text, char_offset)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryInfo {
    pub id: i64,
    pub title: String,
    pub revision: String,
    pub term_count: i64,
    pub kanji_count: i64,
    pub meta_count: i64,
    pub imported_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuriganaSegment {
    pub text: String,
    pub reading: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitchAccent {
    pub reading: String,
    pub downstep: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TermEntry {
    pub expression: String,
    pub reading: String,
    pub furigana: Vec<FuriganaSegment>,
    pub matched_text: String,
    pub match_len: usize,
    pub deinflection_reasons: Vec<String>,
    pub definition_tags: Vec<String>,
    pub term_tags: Vec<String>,
    pub rules: Vec<String>,
    pub glossaries: Vec<String>,
    pub dictionary_title: String,
    pub score: i64,
    pub frequencies: Vec<String>,
    pub pitches: Vec<PitchAccent>,
    pub word_status: Option<String>,
    pub lookup_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KanjiEntry {
    pub character: String,
    pub onyomi: Vec<String>,
    pub kunyomi: Vec<String>,
    pub meanings: Vec<String>,
    pub tags: Vec<String>,
    pub stats: HashMap<String, String>,
    pub dictionary_title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupResult {
    pub matched_text: String,
    pub match_len: usize,
    pub terms: Vec<TermEntry>,
    pub kanji: Vec<KanjiEntry>,
}

// ---------------------------------------------------------------------------
// Yomitan Condition-Checked Japanese Deinflector (`japanese-transforms`)
// ---------------------------------------------------------------------------

// Condition bitmasks matching Yomitan's grammar groups:
const C_V1: u32 = 1 << 0;       // Ichidan verb (一段)
const C_V5: u32 = 1 << 1;       // Godan verb (五段)
const C_VS: u32 = 1 << 2;       // Suru verb (する)
const C_VK: u32 = 1 << 3;       // Kuru verb (くる)
const C_VZ: u32 = 1 << 4;       // Zuru verb (ずる)
const C_ADJ_I: u32 = 1 << 5;    // I-adjective (形容詞)
const C_IRU: u32 = 1 << 6;      // -iru / -eru auxiliary (ている, てある, etc.)
const C_TE: u32 = 1 << 8;       // -te / -de form
const C_TA: u32 = 1 << 9;       // -ta / -da past form
const C_N: u32 = 1 << 13;       // Noun / suru-noun prefix

#[derive(Clone, Copy)]
struct TransformRule {
    inflected_suffix: &'static str,
    deinflected_suffix: &'static str,
    reason: &'static str,
    /// Conditions required on the outer (surface) form, or `0` if valid from surface.
    conditions_in: u32,
    /// Conditions produced on the inner (deinflected) form that the dictionary entry (or next rule) must satisfy.
    conditions_out: u32,
}

macro_rules! rule {
    ($in_suf:expr, $out_suf:expr, $reason:expr, $c_in:expr, $c_out:expr) => {
        TransformRule {
            inflected_suffix: $in_suf,
            deinflected_suffix: $out_suf,
            reason: $reason,
            conditions_in: $c_in,
            conditions_out: $c_out,
        }
    };
}

static TRANSFORM_RULES: &[TransformRule] = &[
    // --- Polite (-ます) ---
    rule!("ます", "る", "polite", 0, C_V1 | C_IRU),
    rule!("ました", "る", "polite past", 0, C_V1 | C_IRU),
    rule!("ません", "る", "polite negative", 0, C_V1 | C_IRU),
    rule!("ませんでした", "る", "polite past negative", 0, C_V1 | C_IRU),
    rule!("ましょう", "る", "polite volitional", 0, C_V1 | C_IRU),
    rule!("まして", "る", "polite te-form", 0, C_V1 | C_IRU),

    rule!("います", "う", "polite", 0, C_V5),
    rule!("きます", "く", "polite", 0, C_V5),
    rule!("ぎます", "ぐ", "polite", 0, C_V5),
    rule!("します", "す", "polite", 0, C_V5),
    rule!("ちます", "つ", "polite", 0, C_V5),
    rule!("にます", "ぬ", "polite", 0, C_V5),
    rule!("びます", "ぶ", "polite", 0, C_V5),
    rule!("みます", "む", "polite", 0, C_V5),
    rule!("ります", "る", "polite", 0, C_V5),

    rule!("いました", "う", "polite past", 0, C_V5),
    rule!("きました", "く", "polite past", 0, C_V5),
    rule!("ぎました", "ぐ", "polite past", 0, C_V5),
    rule!("しました", "す", "polite past", 0, C_V5),
    rule!("ちました", "つ", "polite past", 0, C_V5),
    rule!("にました", "ぬ", "polite past", 0, C_V5),
    rule!("びました", "ぶ", "polite past", 0, C_V5),
    rule!("みました", "む", "polite past", 0, C_V5),
    rule!("りました", "る", "polite past", 0, C_V5),

    rule!("いません", "う", "polite negative", 0, C_V5),
    rule!("きません", "く", "polite negative", 0, C_V5),
    rule!("ぎません", "ぐ", "polite negative", 0, C_V5),
    rule!("しません", "す", "polite negative", 0, C_V5),
    rule!("ちません", "つ", "polite negative", 0, C_V5),
    rule!("にません", "ぬ", "polite negative", 0, C_V5),
    rule!("びません", "ぶ", "polite negative", 0, C_V5),
    rule!("みません", "む", "polite negative", 0, C_V5),
    rule!("りません", "る", "polite negative", 0, C_V5),

    rule!("いませんでした", "う", "polite past negative", 0, C_V5),
    rule!("きませんでした", "く", "polite past negative", 0, C_V5),
    rule!("ぎませんでした", "ぐ", "polite past negative", 0, C_V5),
    rule!("しませんでした", "す", "polite past negative", 0, C_V5),
    rule!("ちませんでした", "つ", "polite past negative", 0, C_V5),
    rule!("にませんでした", "ぬ", "polite past negative", 0, C_V5),
    rule!("びませんでした", "ぶ", "polite past negative", 0, C_V5),
    rule!("みませんでした", "む", "polite past negative", 0, C_V5),
    rule!("りませんでした", "る", "polite past negative", 0, C_V5),

    rule!("いましょう", "う", "polite volitional", 0, C_V5),
    rule!("きましょう", "く", "polite volitional", 0, C_V5),
    rule!("ぎましょう", "ぐ", "polite volitional", 0, C_V5),
    rule!("しましょう", "す", "polite volitional", 0, C_V5),
    rule!("ちましょう", "つ", "polite volitional", 0, C_V5),
    rule!("にましょう", "ぬ", "polite volitional", 0, C_V5),
    rule!("びましょう", "ぶ", "polite volitional", 0, C_V5),
    rule!("みましょう", "む", "polite volitional", 0, C_V5),
    rule!("りましょう", "る", "polite volitional", 0, C_V5),

    rule!("します", "する", "polite", 0, C_VS),
    rule!("しました", "する", "polite past", 0, C_VS),
    rule!("しません", "する", "polite negative", 0, C_VS),
    rule!("しませんでした", "する", "polite past negative", 0, C_VS),
    rule!("しましょう", "する", "polite volitional", 0, C_VS),
    rule!("きます", "くる", "polite", 0, C_VK),
    rule!("きました", "くる", "polite past", 0, C_VK),
    rule!("きません", "くる", "polite negative", 0, C_VK),
    rule!("きませんでした", "くる", "polite past negative", 0, C_VK),
    rule!("きましょう", "くる", "polite volitional", 0, C_VK),
    rule!("来ます", "来る", "polite", 0, C_VK),
    rule!("来ました", "来る", "polite past", 0, C_VK),
    rule!("来ません", "来る", "polite negative", 0, C_VK),

    // --- Past (-た / -だ) ---
    rule!("た", "る", "past", C_TA, C_V1 | C_IRU),
    rule!("った", "う", "past", C_TA, C_V5),
    rule!("った", "つ", "past", C_TA, C_V5),
    rule!("った", "る", "past", C_TA, C_V5),
    rule!("いた", "く", "past", C_TA, C_V5),
    rule!("いだ", "ぐ", "past", C_TA, C_V5),
    rule!("した", "す", "past", C_TA, C_V5),
    rule!("んだ", "ぬ", "past", C_TA, C_V5),
    rule!("んだ", "ぶ", "past", C_TA, C_V5),
    rule!("んだ", "む", "past", C_TA, C_V5),
    rule!("行った", "行く", "past", C_TA, C_V5),
    rule!("いった", "いく", "past", C_TA, C_V5),
    rule!("した", "する", "past", C_TA, C_VS),
    rule!("きた", "くる", "past", C_TA, C_VK),
    rule!("来た", "来る", "past", C_TA, C_VK),

    // --- Te-form (-て / -で) ---
    rule!("て", "る", "te-form", C_TE, C_V1 | C_IRU),
    rule!("って", "う", "te-form", C_TE, C_V5),
    rule!("って", "つ", "te-form", C_TE, C_V5),
    rule!("って", "る", "te-form", C_TE, C_V5),
    rule!("いて", "く", "te-form", C_TE, C_V5),
    rule!("いで", "ぐ", "te-form", C_TE, C_V5),
    rule!("して", "す", "te-form", C_TE, C_V5),
    rule!("んで", "ぬ", "te-form", C_TE, C_V5),
    rule!("んで", "ぶ", "te-form", C_TE, C_V5),
    rule!("んで", "む", "te-form", C_TE, C_V5),
    rule!("行って", "行く", "te-form", C_TE, C_V5),
    rule!("いって", "いく", "te-form", C_TE, C_V5),
    rule!("して", "する", "te-form", C_TE, C_VS),
    rule!("きて", "くる", "te-form", C_TE, C_VK),
    rule!("来て", "来る", "te-form", C_TE, C_VK),

    // --- Progressive / Resultative (-ている / -てる / -ておく / -てしまう / -ちゃう) ---
    rule!("ている", "て", "progressive", C_V1, C_TE),
    rule!("でいる", "で", "progressive", C_V1, C_TE),
    rule!("てる", "て", "progressive", C_V1, C_TE),
    rule!("でる", "で", "progressive", C_V1, C_TE),
    rule!("ておる", "て", "progressive", C_V5, C_TE),
    rule!("ておく", "て", "preparatory", C_V5, C_TE),
    rule!("でおく", "で", "preparatory", C_V5, C_TE),
    rule!("とく", "て", "preparatory", C_V5, C_TE),
    rule!("どく", "で", "preparatory", C_V5, C_TE),
    rule!("てしまう", "て", "completion", C_V5, C_TE),
    rule!("でしまう", "で", "completion", C_V5, C_TE),
    rule!("ちゃう", "て", "completion", C_V5, C_TE),
    rule!("じゃう", "で", "completion", C_V5, C_TE),
    rule!("ちまう", "て", "completion", C_V5, C_TE),
    rule!("じまう", "で", "completion", C_V5, C_TE),
    rule!("ていく", "て", "continuative", C_V5, C_TE),
    rule!("でいく", "で", "continuative", C_V5, C_TE),
    rule!("てくる", "て", "continuative", C_VK, C_TE),
    rule!("でくる", "で", "continuative", C_VK, C_TE),
    rule!("てみる", "て", "try doing", C_V1, C_TE),
    rule!("でみる", "で", "try doing", C_V1, C_TE),
    rule!("てある", "て", "resultative", C_V5, C_TE),
    rule!("である", "で", "resultative", C_V5, C_TE),
    rule!("てあげる", "て", "benefactive", C_V1, C_TE),
    rule!("てくれる", "て", "benefactive", C_V1, C_TE),
    rule!("てもらう", "て", "benefactive", C_V5, C_TE),

    // --- Negative (-ない / -ず / -ぬ) ---
    rule!("ない", "る", "negative", C_ADJ_I, C_V1 | C_IRU),
    rule!("わない", "う", "negative", C_ADJ_I, C_V5),
    rule!("かない", "く", "negative", C_ADJ_I, C_V5),
    rule!("がない", "ぐ", "negative", C_ADJ_I, C_V5),
    rule!("さない", "す", "negative", C_ADJ_I, C_V5),
    rule!("たない", "つ", "negative", C_ADJ_I, C_V5),
    rule!("なない", "ぬ", "negative", C_ADJ_I, C_V5),
    rule!("ばない", "ぶ", "negative", C_ADJ_I, C_V5),
    rule!("まない", "む", "negative", C_ADJ_I, C_V5),
    rule!("らない", "る", "negative", C_ADJ_I, C_V5),
    rule!("しない", "する", "negative", C_ADJ_I, C_VS),
    rule!("こない", "くる", "negative", C_ADJ_I, C_VK),
    rule!("来ない", "来る", "negative", C_ADJ_I, C_VK),
    rule!("ず", "る", "negative", 0, C_V1 | C_IRU),
    rule!("わず", "う", "negative", 0, C_V5),
    rule!("かず", "く", "negative", 0, C_V5),
    rule!("がず", "ぐ", "negative", 0, C_V5),
    rule!("さず", "す", "negative", 0, C_V5),
    rule!("たず", "つ", "negative", 0, C_V5),
    rule!("なず", "ぬ", "negative", 0, C_V5),
    rule!("ばず", "ぶ", "negative", 0, C_V5),
    rule!("まず", "む", "negative", 0, C_V5),
    rule!("らず", "る", "negative", 0, C_V5),
    rule!("せず", "する", "negative", 0, C_VS),
    rule!("こず", "くる", "negative", 0, C_VK),
    rule!("来ず", "来る", "negative", 0, C_VK),

    // --- Passive / Potential (-られる / -れる / -える) ---
    rule!("られる", "る", "passive/potential", C_V1, C_V1),
    rule!("れる", "る", "potential (ra-nuki)", C_V1, C_V1),
    rule!("われる", "う", "passive", C_V1, C_V5),
    rule!("かれる", "く", "passive", C_V1, C_V5),
    rule!("がれる", "ぐ", "passive", C_V1, C_V5),
    rule!("される", "す", "passive", C_V1, C_V5),
    rule!("たれる", "つ", "passive", C_V1, C_V5),
    rule!("なれる", "ぬ", "passive", C_V1, C_V5),
    rule!("ばれる", "ぶ", "passive", C_V1, C_V5),
    rule!("まれる", "む", "passive", C_V1, C_V5),
    rule!("られる", "る", "passive", C_V1, C_V5),
    rule!("される", "する", "passive", C_V1, C_VS),
    rule!("こられる", "くる", "passive/potential", C_V1, C_VK),
    rule!("来られる", "来る", "passive/potential", C_V1, C_VK),
    rule!("これる", "くる", "potential", C_V1, C_VK),
    rule!("来れる", "来る", "potential", C_V1, C_VK),
    rule!("できる", "する", "potential", C_V1, C_VS),

    rule!("える", "う", "potential", C_V1, C_V5),
    rule!("ける", "く", "potential", C_V1, C_V5),
    rule!("げる", "ぐ", "potential", C_V1, C_V5),
    rule!("せる", "す", "potential", C_V1, C_V5),
    rule!("てる", "つ", "potential", C_V1, C_V5),
    rule!("ねる", "ぬ", "potential", C_V1, C_V5),
    rule!("べる", "ぶ", "potential", C_V1, C_V5),
    rule!("める", "む", "potential", C_V1, C_V5),
    rule!("れる", "る", "potential", C_V1, C_V5),

    // --- Causative & Causative-Passive (-させる / -せる / -させられる / -される) ---
    rule!("させられる", "る", "causative-passive", C_V1, C_V1),
    rule!("わせられる", "う", "causative-passive", C_V1, C_V5),
    rule!("かせられる", "く", "causative-passive", C_V1, C_V5),
    rule!("がせられる", "ぐ", "causative-passive", C_V1, C_V5),
    rule!("させられる", "す", "causative-passive", C_V1, C_V5),
    rule!("たせられる", "つ", "causative-passive", C_V1, C_V5),
    rule!("なせられる", "ぬ", "causative-passive", C_V1, C_V5),
    rule!("ばせられる", "ぶ", "causative-passive", C_V1, C_V5),
    rule!("ませられる", "む", "causative-passive", C_V1, C_V5),
    rule!("らせられる", "る", "causative-passive", C_V1, C_V5),
    rule!("واされる", "う", "causative-passive", C_V1, C_V5),
    rule!("わされる", "う", "causative-passive", C_V1, C_V5),
    rule!("かされる", "く", "causative-passive", C_V1, C_V5),
    rule!("がされる", "ぐ", "causative-passive", C_V1, C_V5),
    rule!("たされる", "つ", "causative-passive", C_V1, C_V5),
    rule!("なされる", "ぬ", "causative-passive", C_V1, C_V5),
    rule!("ばされる", "ぶ", "causative-passive", C_V1, C_V5),
    rule!("まされる", "む", "causative-passive", C_V1, C_V5),
    rule!("らされる", "る", "causative-passive", C_V1, C_V5),
    rule!("させられる", "する", "causative-passive", C_V1, C_VS),
    rule!("こさせられる", "くる", "causative-passive", C_V1, C_VK),
    rule!("来させられる", "来る", "causative-passive", C_V1, C_VK),

    rule!("させる", "る", "causative", C_V1, C_V1),
    rule!("わせる", "う", "causative", C_V1, C_V5),
    rule!("かせる", "く", "causative", C_V1, C_V5),
    rule!("がせる", "ぐ", "causative", C_V1, C_V5),
    rule!("させる", "す", "causative", C_V1, C_V5),
    rule!("たせる", "つ", "causative", C_V1, C_V5),
    rule!("なせる", "ぬ", "causative", C_V1, C_V5),
    rule!("ばせる", "ぶ", "causative", C_V1, C_V5),
    rule!("ませる", "む", "causative", C_V1, C_V5),
    rule!("らせる", "る", "causative", C_V1, C_V5),
    rule!("させる", "する", "causative", C_V1, C_VS),
    rule!("こさせる", "くる", "causative", C_V1, C_VK),
    rule!("来させる", "来る", "causative", C_V1, C_VK),

    // --- Desiderative (-たい) ---
    rule!("たい", "る", "want to (-tai)", C_ADJ_I, C_V1 | C_IRU),
    rule!("いたい", "う", "want to (-tai)", C_ADJ_I, C_V5),
    rule!("きたい", "く", "want to (-tai)", C_ADJ_I, C_V5),
    rule!("ぎたい", "ぐ", "want to (-tai)", C_ADJ_I, C_V5),
    rule!("したい", "す", "want to (-tai)", C_ADJ_I, C_V5),
    rule!("ちたい", "つ", "want to (-tai)", C_ADJ_I, C_V5),
    rule!("にたい", "ぬ", "want to (-tai)", C_ADJ_I, C_V5),
    rule!("びたい", "ぶ", "want to (-tai)", C_ADJ_I, C_V5),
    rule!("みたい", "む", "want to (-tai)", C_ADJ_I, C_V5),
    rule!("りたい", "る", "want to (-tai)", C_ADJ_I, C_V5),
    rule!("したい", "する", "want to (-tai)", C_ADJ_I, C_VS),
    rule!("きたい", "くる", "want to (-tai)", C_ADJ_I, C_VK),
    rule!("来たい", "来る", "want to (-tai)", C_ADJ_I, C_VK),

    // --- Volitional (-よう / -おう) ---
    rule!("よう", "る", "volitional", 0, C_V1 | C_IRU),
    rule!("おう", "う", "volitional", 0, C_V5),
    rule!("こう", "く", "volitional", 0, C_V5),
    rule!("ごう", "ぐ", "volitional", 0, C_V5),
    rule!("そう", "す", "volitional", 0, C_V5),
    rule!("とう", "つ", "volitional", 0, C_V5),
    rule!("のう", "ぬ", "volitional", 0, C_V5),
    rule!("ぼう", "ぶ", "volitional", 0, C_V5),
    rule!("もう", "む", "volitional", 0, C_V5),
    rule!("ろう", "る", "volitional", 0, C_V5),
    rule!("しよう", "する", "volitional", 0, C_VS),
    rule!("こよう", "くる", "volitional", 0, C_VK),
    rule!("来よう", "来る", "volitional", 0, C_VK),

    // --- Conditional (-えば / -たら) ---
    rule!("れば", "る", "conditional (-ba)", 0, C_V1 | C_V5 | C_IRU),
    rule!("えば", "う", "conditional (-ba)", 0, C_V5),
    rule!("けば", "く", "conditional (-ba)", 0, C_V5),
    rule!("げば", "ぐ", "conditional (-ba)", 0, C_V5),
    rule!("せば", "す", "conditional (-ba)", 0, C_V5),
    rule!("てば", "つ", "conditional (-ba)", 0, C_V5),
    rule!("ねば", "ぬ", "conditional (-ba)", 0, C_V5),
    rule!("べば", "ぶ", "conditional (-ba)", 0, C_V5),
    rule!("めば", "む", "conditional (-ba)", 0, C_V5),
    rule!("すれば", "する", "conditional (-ba)", 0, C_VS),
    rule!("くれば", "くる", "conditional (-ba)", 0, C_VK),
    rule!("来れば", "来る", "conditional (-ba)", 0, C_VK),
    rule!("たら", "た", "conditional (-tara)", 0, C_TA),
    rule!("だら", "だ", "conditional (-tara)", 0, C_TA),

    // --- Imperative ---
    rule!("ろ", "る", "imperative", 0, C_V1 | C_IRU),
    rule!("よ", "る", "imperative", 0, C_V1),
    rule!("え", "う", "imperative", 0, C_V5),
    rule!("け", "く", "imperative", 0, C_V5),
    rule!("げ", "ぐ", "imperative", 0, C_V5),
    rule!("せ", "す", "imperative", 0, C_V5),
    rule!("て", "つ", "imperative", 0, C_V5),
    rule!("ね", "ぬ", "imperative", 0, C_V5),
    rule!("べ", "ぶ", "imperative", 0, C_V5),
    rule!("め", "む", "imperative", 0, C_V5),
    rule!("れ", "る", "imperative", 0, C_V5),
    rule!("しろ", "する", "imperative", 0, C_VS),
    rule!("せよ", "する", "imperative", 0, C_VS),
    rule!("こい", "くる", "imperative", 0, C_VK),
    rule!("来い", "来る", "imperative", 0, C_VK),

    // --- Auxiliary compounds (-すぎる / -やすい / -にくい) ---
    rule!("すぎる", "る", "too much (-sugiru)", C_V1, C_V1 | C_IRU),
    rule!("いすぎる", "う", "too much (-sugiru)", C_V1, C_V5),
    rule!("きすぎる", "く", "too much (-sugiru)", C_V1, C_V5),
    rule!("ぎすぎる", "ぐ", "too much (-sugiru)", C_V1, C_V5),
    rule!("しすぎる", "す", "too much (-sugiru)", C_V1, C_V5),
    rule!("ちすぎる", "つ", "too much (-sugiru)", C_V1, C_V5),
    rule!("にすぎる", "ぬ", "too much (-sugiru)", C_V1, C_V5),
    rule!("びすぎる", "ぶ", "too much (-sugiru)", C_V1, C_V5),
    rule!("みすぎる", "む", "too much (-sugiru)", C_V1, C_V5),
    rule!("りすぎる", "る", "too much (-sugiru)", C_V1, C_V5),
    rule!("しすぎる", "する", "too much (-sugiru)", C_V1, C_VS),
    rule!("きすぎる", "くる", "too much (-sugiru)", C_V1, C_VK),
    rule!("来すぎる", "来る", "too much (-sugiru)", C_V1, C_VK),
    rule!("すぎる", "い", "too much (-sugiru)", C_V1, C_ADJ_I),
    rule!("やすい", "る", "easy to (-yasui)", C_ADJ_I, C_V1),
    rule!("にくい", "る", "hard to (-nikui)", C_ADJ_I, C_V1),

    // --- I-Adjectives (形容詞) ---
    rule!("くない", "い", "negative", C_ADJ_I, C_ADJ_I),
    rule!("かった", "い", "past", C_TA, C_ADJ_I),
    rule!("くなかった", "い", "past negative", C_TA, C_ADJ_I),
    rule!("くて", "い", "te-form", C_TE, C_ADJ_I),
    rule!("くなくて", "い", "negative te-form", C_TE, C_ADJ_I),
    rule!("ければ", "い", "conditional (-ba)", 0, C_ADJ_I),
    rule!("く", "い", "adverbial", 0, C_ADJ_I),
    rule!("さ", "い", "nominal (-sa)", 0, C_ADJ_I),
    rule!("そう", "い", "seeming (-sou)", 0, C_ADJ_I),
    rule!("良", "いい", "kanji stem", C_ADJ_I, C_ADJ_I),
    rule!("よ", "いい", "irregular stem", C_ADJ_I, C_ADJ_I),

    // --- Suru noun compounds (e.g. 勉強している -> 勉強する -> 勉強) ---
    rule!("する", "", "suru verb", C_VS, C_N),
];

/// Map a Yomitan dictionary entry's `rules` and `definition_tags` strings to its grammar condition bitmask.
fn entry_condition_mask(rules: &str, def_tags: &str) -> u32 {
    let mut mask = 0u32;
    for tok in rules.split_whitespace().chain(def_tags.split_whitespace()) {
        match tok {
            "v1" | "v1-s" => mask |= C_V1,
            s if s.starts_with("v5") => mask |= C_V5,
            "vs" | "vs-i" | "vs-s" | "vs-c" => mask |= C_VS | C_N,
            "vk" => mask |= C_VK,
            "vz" => mask |= C_VZ | C_V1,
            "adj-i" | "adj-ix" => mask |= C_ADJ_I,
            "n" => mask |= C_N,
            _ => {}
        }
    }
    mask
}

#[derive(Debug, Clone)]
pub struct DeinflectCandidate {
    pub text: String,
    pub reasons: Vec<String>,
    pub conditions: u32,
}

/// Generate all valid deinflected base forms for `source` using Yomitan's condition-checked rule graph.
pub fn deinflect(source: &str) -> Vec<DeinflectCandidate> {
    let mut results = vec![DeinflectCandidate {
        text: source.to_string(),
        reasons: Vec::new(),
        conditions: 0,
    }];
    let mut seen: HashMap<(String, u32), usize> = HashMap::new();
    seen.insert((source.to_string(), 0), 0);

    let mut i = 0;
    while i < results.len() && results.len() < 96 {
        let cur = results[i].clone();
        i += 1;
        if cur.reasons.len() >= 5 {
            continue;
        }
        for rule in TRANSFORM_RULES {
            // If `cur` already has active conditions (`cur.conditions != 0`), the rule's `conditions_in`
            // must intersect with `cur.conditions`.
            if cur.conditions != 0 && (cur.conditions & rule.conditions_in) == 0 {
                continue;
            }
            if !cur.text.ends_with(rule.inflected_suffix) {
                continue;
            }
            let stem_len = cur.text.len() - rule.inflected_suffix.len();
            let mut next_text = String::with_capacity(stem_len + rule.deinflected_suffix.len());
            next_text.push_str(&cur.text[..stem_len]);
            next_text.push_str(rule.deinflected_suffix);
            if next_text.is_empty() {
                continue;
            }
            let key = (next_text.clone(), rule.conditions_out);
            if seen.contains_key(&key) {
                continue;
            }
            let mut next_reasons = cur.reasons.clone();
            if next_reasons.last().map(|s| s.as_str()) != Some(rule.reason) {
                next_reasons.push(rule.reason.to_string());
            }
            seen.insert(key, results.len());
            results.push(DeinflectCandidate {
                text: next_text,
                reasons: next_reasons,
                conditions: rule.conditions_out,
            });
        }
    }
    results
}

/// Convert Katakana to Hiragana for phonetic fallback matching.
pub fn katakana_to_hiragana(s: &str) -> String {
    s.chars()
        .map(|c| {
            let u = c as u32;
            if (0x30A1..=0x30F6).contains(&u) {
                char::from_u32(u - 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

fn is_kana(c: char) -> bool {
    let u = c as u32;
    (0x3040..=0x309F).contains(&u) || (0x30A0..=0x30FF).contains(&u)
}

pub fn is_kanji(c: char) -> bool {
    let u = c as u32;
    (0x4E00..=0x9FFF).contains(&u) || (0x3400..=0x4DBF).contains(&u) || c == '々'
}

/// Yomitan's `distributeFurigana`: aligns `expression` and `reading` into segments so okurigana
/// don't get redundant furigana above them.
pub fn distribute_furigana(expression: &str, reading: &str) -> Vec<FuriganaSegment> {
    if reading.is_empty() || expression == reading {
        return vec![FuriganaSegment {
            text: expression.to_string(),
            reading: None,
        }];
    }
    let expr_chars: Vec<char> = expression.chars().collect();
    let read_chars: Vec<char> = reading.chars().collect();
    if expr_chars.iter().all(|c| is_kana(*c)) {
        return vec![FuriganaSegment {
            text: expression.to_string(),
            reading: None,
        }];
    }

    let expr_hira: Vec<char> = katakana_to_hiragana(expression).chars().collect();
    let read_hira: Vec<char> = katakana_to_hiragana(reading).chars().collect();

    // Match leading kana
    let mut lead = 0;
    while lead < expr_chars.len()
        && lead < read_chars.len()
        && is_kana(expr_chars[lead])
        && expr_hira[lead] == read_hira[lead]
    {
        lead += 1;
    }

    // Match trailing kana
    let mut trail = 0;
    while trail < (expr_chars.len() - lead)
        && trail < (read_chars.len() - lead)
        && is_kana(expr_chars[expr_chars.len() - 1 - trail])
        && expr_hira[expr_chars.len() - 1 - trail] == read_hira[read_chars.len() - 1 - trail]
    {
        trail += 1;
    }

    let mut out = Vec::new();
    if lead > 0 {
        out.push(FuriganaSegment {
            text: expr_chars[..lead].iter().collect(),
            reading: None,
        });
    }
    let mid_expr_end = expr_chars.len() - trail;
    let mid_read_end = read_chars.len() - trail;
    if mid_expr_end > lead {
        let mid_text: String = expr_chars[lead..mid_expr_end].iter().collect();
        let mid_read: String = read_chars[lead..mid_read_end].iter().collect();
        out.push(FuriganaSegment {
            text: mid_text,
            reading: if mid_read.is_empty() { None } else { Some(mid_read) },
        });
    }
    if trail > 0 {
        out.push(FuriganaSegment {
            text: expr_chars[mid_expr_end..].iter().collect(),
            reading: None,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Longest-Prefix Dictionary Lookup
// ---------------------------------------------------------------------------

pub fn lookup_at(conn: &Connection, text: &str, char_offset: usize) -> Result<LookupResult> {
    let chars: Vec<char> = text.chars().collect();
    if char_offset >= chars.len() {
        return Ok(LookupResult {
            matched_text: String::new(),
            match_len: 0,
            terms: Vec::new(),
            kanji: Vec::new(),
        });
    }

    let rest: Vec<char> = chars[char_offset..].iter().take(18).copied().collect();
    if rest.is_empty() || rest[0].is_whitespace() {
        return Ok(LookupResult {
            matched_text: String::new(),
            match_len: 0,
            terms: Vec::new(),
            kanji: Vec::new(),
        });
    }

    struct PrefixCandidates {
        sub: String,
        len: usize,
        candidates: Vec<DeinflectCandidate>,
    }

    let mut prefixes: Vec<PrefixCandidates> = Vec::new();
    let mut unique_texts: Vec<String> = Vec::new();
    let mut seen_texts: HashSet<String> = HashSet::new();

    for len in (1..=rest.len()).rev() {
        let sub: String = rest[..len].iter().collect();
        if sub.chars().any(|c| {
            matches!(
                c,
                '。' | '、' | '！' | '？' | '「' | '」' | '（' | '）' | '\n' | '\r'
            )
        }) {
            continue;
        }
        let hira_sub = katakana_to_hiragana(&sub);
        let mut candidates = deinflect(&sub);
        if hira_sub != sub {
            for hc in deinflect(&hira_sub) {
                if !candidates
                    .iter()
                    .any(|c| c.text == hc.text && c.conditions == hc.conditions)
                {
                    candidates.push(hc);
                }
            }
        }
        for c in &candidates {
            if seen_texts.insert(c.text.clone()) {
                unique_texts.push(c.text.clone());
            }
        }
        prefixes.push(PrefixCandidates {
            sub,
            len,
            candidates,
        });
    }

    #[derive(Clone)]
    struct RawTermRow {
        id: i64,
        expr: String,
        read: String,
        def_tags: String,
        rules: String,
        score: i64,
        gloss_json: String,
        term_tags: String,
        dict_title: String,
    }

    let mut fetched_rows_by_id: HashMap<i64, RawTermRow> = HashMap::new();
    for chunk in unique_texts.chunks(64) {
        let placeholders = std::iter::repeat("?")
            .take(chunk.len())
            .collect::<Vec<_>>()
            .join(", ");
        for col in ["t.expression", "t.reading"] {
            let sql = format!(
                "SELECT t.id, t.expression, t.reading, t.definition_tags, t.rules, t.score, t.glossary_json, t.term_tags, d.title
                 FROM terms t
                 JOIN dictionaries d ON d.id = t.dict_id
                 WHERE {col} IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params_from_iter(chunk.iter()), |r| {
                Ok(RawTermRow {
                    id: r.get(0)?,
                    expr: r.get(1)?,
                    read: r.get(2)?,
                    def_tags: r.get(3)?,
                    rules: r.get(4)?,
                    score: r.get(5)?,
                    gloss_json: r.get(6)?,
                    term_tags: r.get(7)?,
                    dict_title: r.get(8)?,
                })
            })?;
            for row in rows.flatten() {
                fetched_rows_by_id.entry(row.id).or_insert(row);
            }
        }
    }

    let mut all_rows: Vec<RawTermRow> = fetched_rows_by_id.into_values().collect();
    all_rows.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));

    let mut rows_by_key: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, row) in all_rows.iter().enumerate() {
        let expr_list = rows_by_key.entry(row.expr.clone()).or_default();
        if expr_list.len() < 24 {
            expr_list.push(idx);
        }
        if !row.read.is_empty() && row.read != row.expr {
            let read_list = rows_by_key.entry(row.read.clone()).or_default();
            if read_list.len() < 24 {
                read_list.push(idx);
            }
        }
    }

    let mut collected_terms: Vec<TermEntry> = Vec::new();
    let mut best_match_text = String::new();
    let mut best_match_len = 0usize;
    let mut seen_entries: HashSet<(String, String, String)> = HashSet::new();

    for pref in &prefixes {
        for cand in &pref.candidates {
            let Some(indices) = rows_by_key.get(&cand.text) else {
                continue;
            };
            for &row_idx in indices {
                let row = &all_rows[row_idx];
                if cand.conditions != 0 {
                    let entry_mask = entry_condition_mask(&row.rules, &row.def_tags);
                    if entry_mask != 0 && (entry_mask & cand.conditions) == 0 {
                        continue;
                    }
                    if entry_mask == 0 && (!row.rules.is_empty() || !row.def_tags.is_empty()) {
                        continue;
                    }
                }

                let eff_reading = if row.read.is_empty() {
                    row.expr.clone()
                } else {
                    row.read.clone()
                };
                let dedup_key = (row.expr.clone(), eff_reading.clone(), row.dict_title.clone());
                if !seen_entries.insert(dedup_key) {
                    continue;
                }

                let glossaries = extract_glossaries(&row.gloss_json);
                if glossaries.is_empty() {
                    continue;
                }

                if pref.len > best_match_len {
                    best_match_len = pref.len;
                    best_match_text = pref.sub.clone();
                }

                let furigana = distribute_furigana(&row.expr, &eff_reading);
                let mut reasons = cand.reasons.clone();
                reasons.reverse();

                collected_terms.push(TermEntry {
                    expression: row.expr.clone(),
                    reading: eff_reading,
                    furigana,
                    matched_text: pref.sub.clone(),
                    match_len: pref.len,
                    deinflection_reasons: reasons,
                    definition_tags: row.def_tags.split_whitespace().map(String::from).collect(),
                    term_tags: row.term_tags.split_whitespace().map(String::from).collect(),
                    rules: row.rules.split_whitespace().map(String::from).collect(),
                    glossaries,
                    dictionary_title: row.dict_title.clone(),
                    score: row.score,
                    frequencies: Vec::new(),
                    pitches: Vec::new(),
                    word_status: None,
                    lookup_count: 0,
                });
            }
        }
    }

    // Sort: longest match first, fewer deinflection steps second, higher score third
    collected_terms.sort_by(|a, b| {
        b.match_len
            .cmp(&a.match_len)
            .then_with(|| a.deinflection_reasons.len().cmp(&b.deinflection_reasons.len()))
            .then_with(|| b.score.cmp(&a.score))
    });
    collected_terms.truncate(20);

    // Single batched `term_meta` query for the top terms (`C-07`)
    let mut unique_exprs: Vec<String> = Vec::new();
    for t in &collected_terms {
        if !unique_exprs.contains(&t.expression) {
            unique_exprs.push(t.expression.clone());
        }
    }
    if !unique_exprs.is_empty() {
        let placeholders = std::iter::repeat("?")
            .take(unique_exprs.len())
            .collect::<Vec<_>>()
            .join(", ");
        let meta_sql = format!(
            "SELECT expression, mode, data_json FROM term_meta WHERE expression IN ({placeholders})"
        );
        if let Ok(mut meta_stmt) = conn.prepare(&meta_sql) {
            if let Ok(meta_rows) = meta_stmt.query_map(params_from_iter(unique_exprs.iter()), |mr| {
                Ok((
                    mr.get::<_, String>(0)?,
                    mr.get::<_, String>(1)?,
                    mr.get::<_, String>(2)?,
                ))
            }) {
                let mut meta_by_expr: HashMap<String, Vec<(String, String)>> = HashMap::new();
                for (expr, mode, data_str) in meta_rows.flatten() {
                    let list = meta_by_expr.entry(expr).or_default();
                    if list.len() < 24 {
                        list.push((mode, data_str));
                    }
                }
                for t in &mut collected_terms {
                    if let Some(items) = meta_by_expr.get(&t.expression) {
                        for (mode, data_str) in items {
                            if mode == "freq" {
                                if let Some(f) = parse_freq_meta(data_str) {
                                    if !t.frequencies.contains(&f) {
                                        t.frequencies.push(f);
                                    }
                                }
                            } else if mode == "pitch" {
                                for p in parse_pitch_meta(data_str, &t.reading) {
                                    if !t
                                        .pitches
                                        .iter()
                                        .any(|x: &PitchAccent| x.downstep == p.downstep && x.reading == p.reading)
                                    {
                                        t.pitches.push(p);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Collect Kanji entries for any kanji in the matched text or top term expressions via a single batched query
    let mut kanji_chars: Vec<char> = Vec::new();
    for ch in best_match_text.chars() {
        if is_kanji(ch) && !kanji_chars.contains(&ch) {
            kanji_chars.push(ch);
        }
    }
    if kanji_chars.is_empty() && is_kanji(rest[0]) {
        kanji_chars.push(rest[0]);
    }
    for t in collected_terms.iter().take(3) {
        for ch in t.expression.chars() {
            if is_kanji(ch) && !kanji_chars.contains(&ch) {
                kanji_chars.push(ch);
            }
        }
    }

    let mut kanji_entries = Vec::new();
    if !kanji_chars.is_empty() {
        let kanji_strs: Vec<String> = kanji_chars.iter().map(|c| c.to_string()).collect();
        let placeholders = std::iter::repeat("?")
            .take(kanji_strs.len())
            .collect::<Vec<_>>()
            .join(", ");
        let k_sql = format!(
            "SELECT k.character, k.onyomi, k.kunyomi, k.tags, k.meanings_json, k.stats_json, d.title
             FROM kanji k
             JOIN dictionaries d ON d.id = k.dict_id
             WHERE k.character IN ({placeholders})
             ORDER BY k.id ASC"
        );
        if let Ok(mut k_stmt) = conn.prepare(&k_sql) {
            if let Ok(rows) = k_stmt.query_map(params_from_iter(kanji_strs.iter()), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            }) {
                let mut first_by_char: HashMap<String, KanjiEntry> = HashMap::new();
                for (character, onyomi, kunyomi, tags, meanings_json, stats_json, dict_title) in
                    rows.flatten()
                {
                    first_by_char.entry(character.clone()).or_insert_with(|| {
                        let meanings: Vec<String> =
                            serde_json::from_str(&meanings_json).unwrap_or_default();
                        let stats: HashMap<String, String> =
                            serde_json::from_str(&stats_json).unwrap_or_default();
                        KanjiEntry {
                            character,
                            onyomi: onyomi.split_whitespace().map(String::from).collect(),
                            kunyomi: kunyomi.split_whitespace().map(String::from).collect(),
                            meanings,
                            tags: tags.split_whitespace().map(String::from).collect(),
                            stats,
                            dictionary_title: dict_title,
                        }
                    });
                }
                for ch_s in &kanji_strs {
                    if let Some(entry) = first_by_char.remove(ch_s) {
                        kanji_entries.push(entry);
                    }
                }
            }
        }
    }

    if best_match_len == 0 && !kanji_entries.is_empty() {
        best_match_len = 1;
        best_match_text = rest[0].to_string();
    }

    Ok(LookupResult {
        matched_text: best_match_text,
        match_len: best_match_len,
        terms: collected_terms,
        kanji: kanji_entries,
    })
}

/// Extract human-readable definitions from Yomitan v3 glossary JSON (handles plain strings, text objects, and `structured-content`).
fn extract_glossaries(gloss_json: &str) -> Vec<String> {
    let Ok(val) = serde_json::from_str::<Value>(gloss_json) else {
        return Vec::new();
    };
    let Some(arr) = val.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        if let Some(s) = item.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        } else if let Some(obj) = item.as_object() {
            let kind = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if kind == "text" {
                if let Some(t) = obj.get("text").and_then(|v| v.as_str()) {
                    let trimmed = t.trim();
                    if !trimmed.is_empty() {
                        out.push(trimmed.to_string());
                    }
                }
            } else if kind == "structured-content" {
                if let Some(content) = obj.get("content") {
                    let mut buf = String::new();
                    flatten_structured_content(content, &mut buf);
                    let cleaned = buf
                        .lines()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ");
                    if !cleaned.is_empty() {
                        out.push(cleaned);
                    }
                }
            }
        }
    }
    out
}

fn flatten_structured_content(v: &Value, out: &mut String) {
    match v {
        Value::String(s) => {
            out.push_str(s);
        }
        Value::Array(arr) => {
            for item in arr {
                flatten_structured_content(item, out);
            }
        }
        Value::Object(obj) => {
            let tag = obj.get("tag").and_then(|t| t.as_str()).unwrap_or("");
            // Skip ruby rt annotations when flattening plain text so furigana isn't interleaved mid-word
            if tag == "rt" || tag == "rp" {
                return;
            }
            if matches!(tag, "li" | "div" | "p" | "br") && !out.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }
            if let Some(c) = obj.get("content") {
                flatten_structured_content(c, out);
            }
        }
        _ => {}
    }
}

fn parse_freq_meta(data_str: &str) -> Option<String> {
    let v: Value = serde_json::from_str(data_str).ok()?;
    if let Some(n) = v.as_i64() {
        return Some(format!("#{n}"));
    }
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    if let Some(obj) = v.as_object() {
        if let Some(disp) = obj.get("displayValue").and_then(|x| x.as_str()) {
            return Some(disp.to_string());
        }
        if let Some(val) = obj.get("value").and_then(|x| x.as_i64()) {
            return Some(format!("#{val}"));
        }
        if let Some(freq) = obj.get("frequency") {
            return parse_freq_meta(&freq.to_string());
        }
    }
    None
}

fn parse_pitch_meta(data_str: &str, default_reading: &str) -> Vec<PitchAccent> {
    let Ok(v) = serde_json::from_str::<Value>(data_str) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let reading = v
        .get("reading")
        .and_then(|r| r.as_str())
        .unwrap_or(default_reading)
        .to_string();
    if let Some(pitches) = v.get("pitches").and_then(|p| p.as_array()) {
        for p in pitches {
            if let Some(pos) = p.get("position").and_then(|x| x.as_i64()) {
                out.push(PitchAccent {
                    reading: reading.clone(),
                    downstep: pos,
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Streaming Yomitan v3 Archive Importer (`C-09` & `H-06`)
// ---------------------------------------------------------------------------

fn stream_yomitan_bank_rows<F>(
    entry: &crate::subtitles::ZipCentralEntry<'_>,
    max_uncompressed_bytes: u64,
    on_row: F,
) -> Result<()>
where
    F: FnMut(Value) -> Result<()>,
{
    struct BankArrayVisitor<'a, F> {
        on_row: F,
        row_err: &'a mut Option<anyhow::Error>,
    }

    impl<'de, 'a, F> serde::de::Visitor<'de> for BankArrayVisitor<'a, F>
    where
        F: FnMut(Value) -> Result<()>,
    {
        type Value = ();

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a JSON array of Yomitan bank entries")
        }

        fn visit_seq<A>(mut self, mut seq: A) -> std::result::Result<(), A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            while let Some(row) = seq.next_element::<Value>()? {
                if let Err(e) = (self.on_row)(row) {
                    *self.row_err = Some(e);
                    return Err(serde::de::Error::custom("bank row insert failed"));
                }
            }
            Ok(())
        }
    }

    fn run_stream<R: Read, F: FnMut(Value) -> Result<()>>(
        entry_name: &str,
        reader: R,
        max_uncompressed_bytes: u64,
        on_row: F,
    ) -> Result<()> {
        let mut limited = reader.take(max_uncompressed_bytes.saturating_add(1));
        let mut row_err: Option<anyhow::Error> = None;
        let parse_res = {
            let buf_reader = BufReader::with_capacity(64 * 1024, &mut limited);
            let mut de = serde_json::Deserializer::from_reader(buf_reader);
            serde::Deserializer::deserialize_seq(
                &mut de,
                BankArrayVisitor {
                    on_row,
                    row_err: &mut row_err,
                },
            )
        };
        if limited.limit() == 0 {
            bail!(
                "Yomitan bank '{}' exceeds maximum allowed uncompressed size ({max_uncompressed_bytes} bytes)",
                entry_name
            );
        }
        if let Some(err) = row_err {
            return Err(err);
        }
        parse_res.with_context(|| format!("Invalid JSON in Yomitan bank '{entry_name}'"))
    }

    match entry.method {
        0 => run_stream(
            &entry.name,
            Cursor::new(entry.comp_slice),
            max_uncompressed_bytes,
            on_row,
        ),
        8 => run_stream(
            &entry.name,
            DeflateDecoder::new(entry.comp_slice),
            max_uncompressed_bytes,
            on_row,
        ),
        other => bail!(
            "Unsupported ZIP compression method {other} for '{}'",
            entry.name
        ),
    }
}

pub fn import_yomitan_zip_bytes(conn: &mut Connection, zip_bytes: &[u8]) -> Result<DictionaryInfo> {
    const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;
    const MAX_BANK_BYTES: u64 = 64 * 1024 * 1024;

    let entries = crate::subtitles::iter_zip_entries(zip_bytes)?;
    let index_entry = entries
        .iter()
        .find(|e| {
            let base = e.name.rsplit(['/', '\\']).next().unwrap_or(&e.name);
            base == "index.json"
        })
        .ok_or_else(|| anyhow!("Not a valid Yomitan dictionary (missing index.json)"))?;

    let index_bytes = index_entry.decompress(MAX_INDEX_BYTES)?;
    let index_json: Value = serde_json::from_slice(&index_bytes)?;
    let title = index_json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Imported Yomitan Dictionary")
        .to_string();
    let revision = index_json
        .get("revision")
        .and_then(|v| {
            v.as_str()
                .map(String::from)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
        })
        .unwrap_or_else(|| "1".to_string());
    let format = index_json
        .get("format")
        .or_else(|| index_json.get("version"))
        .and_then(|v| v.as_i64())
        .unwrap_or(3);

    let tx = conn.transaction()?;
    tx.execute("DELETE FROM dictionaries WHERE title = ?1", [&title])?;
    let imported_at = now();
    tx.execute(
        "INSERT INTO dictionaries (title, revision, format, imported_at) VALUES (?1, ?2, ?3, ?4)",
        params![title, revision, format, imported_at],
    )?;
    let dict_id = tx.last_insert_rowid();

    let mut term_count = 0i64;
    let mut kanji_count = 0i64;
    let mut meta_count = 0i64;

    {
        let mut term_stmt = tx.prepare(
            "INSERT INTO terms (dict_id, expression, reading, definition_tags, rules, score, glossary_json, sequence, term_tags)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        let mut meta_stmt = tx.prepare(
            "INSERT INTO term_meta (dict_id, expression, mode, data_json)
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        let mut kanji_stmt = tx.prepare(
            "INSERT INTO kanji (dict_id, character, onyomi, kunyomi, tags, meanings_json, stats_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        for entry in &entries {
            if entry.name.ends_with('/') || entry.name.ends_with('\\') {
                continue;
            }
            let base_name = entry.name.rsplit(['/', '\\']).next().unwrap_or(&entry.name);
            if !base_name.ends_with(".json") {
                continue;
            }

            if base_name.starts_with("term_bank_") {
                stream_yomitan_bank_rows(entry, MAX_BANK_BYTES, |row| {
                    let Some(arr) = row.as_array() else {
                        return Ok(());
                    };
                    if arr.len() < 6 {
                        return Ok(());
                    }
                    let expr = arr[0].as_str().unwrap_or("");
                    if expr.is_empty() {
                        return Ok(());
                    }
                    let reading = arr[1].as_str().unwrap_or("");
                    let def_tags = arr[2].as_str().unwrap_or("");
                    let rules = arr[3].as_str().unwrap_or("");
                    let score = arr[4].as_i64().unwrap_or(0);
                    let glossary_json =
                        serde_json::to_string(&arr[5]).unwrap_or_else(|_| "[]".into());
                    let seq = arr.get(6).and_then(|v| v.as_i64()).unwrap_or(0);
                    let term_tags = arr.get(7).and_then(|v| v.as_str()).unwrap_or("");
                    term_stmt.execute(params![
                        dict_id,
                        expr,
                        reading,
                        def_tags,
                        rules,
                        score,
                        glossary_json,
                        seq,
                        term_tags
                    ])?;
                    term_count += 1;
                    Ok(())
                })?;
            } else if base_name.starts_with("term_meta_bank_") {
                stream_yomitan_bank_rows(entry, MAX_BANK_BYTES, |row| {
                    let Some(arr) = row.as_array() else {
                        return Ok(());
                    };
                    if arr.len() < 3 {
                        return Ok(());
                    }
                    let expr = arr[0].as_str().unwrap_or("");
                    let mode = arr[1].as_str().unwrap_or("");
                    if expr.is_empty() || mode.is_empty() {
                        return Ok(());
                    }
                    let data_json =
                        serde_json::to_string(&arr[2]).unwrap_or_else(|_| "{}".into());
                    meta_stmt.execute(params![dict_id, expr, mode, data_json])?;
                    meta_count += 1;
                    Ok(())
                })?;
            } else if base_name.starts_with("kanji_bank_") {
                stream_yomitan_bank_rows(entry, MAX_BANK_BYTES, |row| {
                    let Some(arr) = row.as_array() else {
                        return Ok(());
                    };
                    if arr.len() < 5 {
                        return Ok(());
                    }
                    let character = arr[0].as_str().unwrap_or("");
                    if character.is_empty() {
                        return Ok(());
                    }
                    let onyomi = arr[1].as_str().unwrap_or("");
                    let kunyomi = arr[2].as_str().unwrap_or("");
                    let tags = arr[3].as_str().unwrap_or("");
                    let meanings_json =
                        serde_json::to_string(&arr[4]).unwrap_or_else(|_| "[]".into());
                    let stats_json = arr
                        .get(5)
                        .and_then(|v| serde_json::to_string(v).ok())
                        .unwrap_or_else(|| "{}".into());
                    kanji_stmt.execute(params![
                        dict_id,
                        character,
                        onyomi,
                        kunyomi,
                        tags,
                        meanings_json,
                        stats_json
                    ])?;
                    kanji_count += 1;
                    Ok(())
                })?;
            }
        }
    }

    tx.execute(
        "UPDATE dictionaries SET term_count = ?2, kanji_count = ?3, meta_count = ?4 WHERE id = ?1",
        params![dict_id, term_count, kanji_count, meta_count],
    )?;
    tx.commit()?;

    // Clear cached read-only connections so readers immediately observe newly imported tables/pages cleanly
    if let Some(pool) = DICT_READ_POOL.get() {
        if let Ok(mut map) = pool.lock() {
            map.clear();
        }
    }

    Ok(DictionaryInfo {
        id: dict_id,
        title,
        revision,
        term_count,
        kanji_count,
        meta_count,
        imported_at,
    })
}

pub fn list_dictionaries(conn: &Connection) -> Result<Vec<DictionaryInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, revision, term_count, kanji_count, meta_count, imported_at
         FROM dictionaries ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(DictionaryInfo {
                id: r.get(0)?,
                title: r.get(1)?,
                revision: r.get(2)?,
                term_count: r.get(3)?,
                kanji_count: r.get(4)?,
                meta_count: r.get(5)?,
                imported_at: r.get(6)?,
            })
        })?
        .flatten()
        .collect();
    Ok(rows)
}

pub fn remove_dictionary(conn: &Connection, dict_id: i64) -> Result<()> {
    conn.execute("DELETE FROM dictionaries WHERE id = ?1", [dict_id])?;
    if let Some(pool) = DICT_READ_POOL.get() {
        if let Ok(mut map) = pool.lock() {
            map.clear();
        }
    }
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
    Ok(())
}

fn dict_download_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!(
                "Kura/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/wantingCat/kura)"
            ))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// 1-click download of full Yomitan JMdict (English) + KanjiDic dictionaries from GitHub releases.
pub async fn download_recommended_dictionaries(
    app: &AppHandle,
    data_dir: &Path,
) -> Result<Vec<DictionaryInfo>> {
    const MAX_DICT_ZIP_BYTES: usize = 128 * 1024 * 1024;
    let client = dict_download_client();

    let _ = app.emit(
        "dictionary-progress",
        json!({ "stage": "discovering", "message": "Finding latest Yomitan JMdict & KanjiDic releases…" }),
    );

    // Discover latest asset URLs from yomidevs/jmdict-yomitan releases
    let mut zip_urls: Vec<(&str, String)> = Vec::new();
    if let Ok(resp) = client
        .get("https://api.github.com/repos/yomidevs/jmdict-yomitan/releases/latest")
        .send()
        .await
    {
        if let Ok(rel) = resp.json::<Value>().await {
            if let Some(assets) = rel.get("assets").and_then(|a| a.as_array()) {
                for a in assets {
                    let name = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let url = a
                        .get("browser_download_url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("");
                    if name == "JMdict_english.zip" && !url.is_empty() {
                        zip_urls.push(("JMdict (English)", url.to_string()));
                    } else if name == "KANJIDIC_english.zip" && !url.is_empty() {
                        zip_urls.push(("KANJIDIC (English)", url.to_string()));
                    }
                }
            }
        }
    }

    if zip_urls.is_empty() {
        bail!("Could not reach GitHub releases to download JMdict. You can also import any Yomitan .zip dictionary directly from disk.");
    }

    let mut imported = Vec::new();
    for (label, url) in zip_urls {
        let _ = app.emit(
            "dictionary-progress",
            json!({ "stage": "downloading", "message": format!("Downloading {label}…") }),
        );
        let mut resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to download {label}: {}", e.without_url()))?;
        if !resp.status().is_success() {
            bail!("Failed to download {label} (HTTP {})", resp.status());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = resp.chunk().await? {
            if bytes.len().saturating_add(chunk.len()) > MAX_DICT_ZIP_BYTES {
                bail!("{label} archive exceeds maximum allowed download size (128 MiB)");
            }
            bytes.extend_from_slice(&chunk);
        }

        let _ = app.emit(
            "dictionary-progress",
            json!({ "stage": "importing", "message": format!("Indexing {label} into SQLite…") }),
        );
        let dir = data_dir.to_path_buf();
        let info = tauri::async_runtime::spawn_blocking(move || -> Result<DictionaryInfo> {
            let mut conn = open_dict_db(&dir)?;
            import_yomitan_zip_bytes(&mut conn, &bytes)
        })
        .await??;
        imported.push(info);
    }

    let _ = app.emit(
        "dictionary-progress",
        json!({ "stage": "done", "message": "Dictionaries ready!" }),
    );
    Ok(imported)
}

/// Seed a lightweight built-in starter anime/core vocabulary & kanji pack when `dictionary.db` is first created
/// so pause-and-hover works immediately out-of-the-box even before downloading the full 200k-entry JMdict!
pub fn ensure_starter_dictionary(conn: &Connection) -> Result<()> {
    let has_any: bool = conn
        .query_row("SELECT EXISTS(SELECT 1 FROM dictionaries)", [], |r| r.get(0))
        .unwrap_or(false);
    if has_any {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO dictionaries (title, revision, format, imported_at) VALUES ('Kura Built-in Core (Starter)', '1.0', 3, ?1)",
        [now()],
    )?;
    let dict_id = tx.last_insert_rowid();

    let starter_terms: &[(&str, &str, &str, &str, i64, &[&str], i64)] = &[
        ("食べる", "たべる", "v1 vt", "v1", 500, &["to eat", "to live on (e.g. a salary)"], 2),
        ("飲む", "のむ", "v5m vt", "v5", 480, &["to drink", "to swallow", "to take (medicine)"], 1),
        ("見る", "みる", "v1 vt", "v1", 520, &["to see", "to look", "to watch", "to view", "to observe"], 1),
        ("行く", "いく", "v5k-s vi", "v5", 510, &["to go", "to move (towards)", "to head (towards)"], 0),
        ("来る", "くる", "vk vi", "vk", 510, &["to come (spatially or temporally)", "to approach", "to arrive"], 1),
        ("する", "する", "vs-i", "vs", 550, &["to do", "to carry out", "to perform"], 0),
        ("言う", "いう", "v5u", "v5", 520, &["to say", "to utter", "to declare", "to name"], 0),
        ("思う", "おもう", "v5u vt", "v5", 500, &["to think", "to consider", "to believe", "to feel"], 2),
        ("知る", "しる", "v5r vt", "v5", 490, &["to know", "to be aware (of)", "to understand"], 0),
        ("分かる", "わかる", "v5r vi", "v5", 495, &["to understand", "to comprehend", "to grasp"], 2),
        ("聞く", "きく", "v5k vt", "v5", 485, &["to hear", "to listen (e.g. to music)", "to ask", "to enquire"], 0),
        ("話す", "はなす", "v5s vt", "v5", 480, &["to talk", "to speak", "to converse", "to tell"], 2),
        ("読む", "よむ", "v5m vt", "v5", 475, &["to read", "to recite", "to predict"], 1),
        ("書く", "かく", "v5k vt", "v5", 470, &["to write", "to compose", "to pen", "to draw"], 1),
        ("待つ", "まつ", "v5t vt vi", "v5", 470, &["to wait", "to await", "to look forward to"], 1),
        ("持つ", "もつ", "v5t vt", "v5", 470, &["to hold (in one's hand)", "to take", "to carry", "to possess"], 1),
        ("帰る", "かえる", "v5r vi", "v5", 465, &["to return", "to come home", "to go home", "to go back"], 1),
        ("出る", "でる", "v1 vi", "v1", 480, &["to leave", "to exit", "to go out", "to appear"], 1),
        ("入る", "はいる", "v5r vi", "v5", 480, &["to enter", "to come in", "to go in", "to join"], 1),
        ("作る", "つくる", "v5r vt", "v5", 475, &["to make", "to produce", "to manufacture", "to build"], 2),
        ("使う", "つかう", "v5u vt", "v5", 475, &["to use (a tool, method, etc.)", "to make use of", "to spend"], 0),
        ("会う", "あう", "v5u vi", "v5", 475, &["to meet", "to encounter", "to see"], 1),
        ("死ぬ", "しぬ", "v5n vi", "v5", 460, &["to die", "to pass away"], 0),
        ("遊ぶ", "あそぶ", "v5b vi", "v5", 450, &["to play (games, sports)", "to hang out", "to have a good time"], 0),
        ("泳ぐ", "およぐ", "v5g vi", "v5", 440, &["to swim"], 2),
        ("守る", "まもる", "v5r vt", "v5", 465, &["to protect", "to guard", "to defend", "to keep (a promise)"], 2),
        ("戦う", "たたかう", "v5u vi", "v5", 465, &["to fight", "to battle", "to combat", "to struggle"], 0),
        ("勝つ", "かつ", "v5t vi", "v5", 460, &["to win", "to gain victory"], 1),
        ("負ける", "まける", "v1 vi", "v1", 460, &["to lose", "to be defeated", "to give in"], 0),
        ("助ける", "たすける", "v1 vt", "v1", 465, &["to save", "to rescue", "to help", "to assist"], 3),
        ("逃げる", "にげる", "v1 vi", "v1", 460, &["to escape", "to run away"], 2),
        ("信じる", "しんじる", "v1 vt", "v1", 465, &["to believe", "to trust", "to confide in"], 3),
        ("忘れる", "わすれる", "v1 vt", "v1", 460, &["to forget", "to leave carelessly"], 0),
        ("覚える", "おぼえる", "v1 vt", "v1", 460, &["to memorize", "to learn", "to remember", "to feel"], 3),
        ("考える", "かんがえる", "v1 vt", "v1", 470, &["to think (about)", "to consider", "to ponder"], 4),
        ("変わる", "かわる", "v5r vi", "v5", 460, &["to change", "to be transformed", "to vary"], 0),
        ("変える", "かえる", "v1 vt", "v1", 460, &["to change", "to alter", "to transform"], 0),
        ("始まる", "はじまる", "v5r vi", "v5", 460, &["to begin", "to start", "to commence"], 0),
        ("終わる", "おわる", "v5r vi", "v5", 460, &["to end", "to come to an end", "to finish"], 0),
        ("生きる", "いきる", "v1 vi", "v1", 465, &["to live", "to exist"], 2),
        ("勉強", "べんきょう", "n vs", "vs", 450, &["study", "diligence", "learning"], 0),
        ("約束", "やくそく", "n vs", "vs", 460, &["promise", "agreement", "appointment", "pact"], 0),
        ("魔法", "まほう", "n", "", 455, &["magic", "witchcraft", "sorcery", "spell"], 0),
        ("世界", "せかい", "n", "", 480, &["the world", "society", "the universe", "sphere"], 1),
        ("時間", "じかん", "n", "", 480, &["time", "hour", "period", "class"], 0),
        ("人間", "にんげん", "n", "", 475, &["human being", "person", "mankind"], 0),
        ("友達", "ともだち", "n", "", 470, &["friend", "companion"], 0),
        ("仲間", "なかま", "n", "", 470, &["companion", "fellow", "friend", "comrade", "ally"], 3),
        ("本当", "ほんとう", "n adj-na", "", 490, &["truth", "reality", "actuality", "fact", "really"], 0),
        ("大丈夫", "だいじょうぶ", "adj-na n", "", 490, &["safe", "secure", "all right", "OK", "okay"], 3),
        ("好き", "すき", "adj-na n", "", 485, &["liked", "well-liked", "favourite", "in love (with)"], 2),
        ("嫌い", "きらい", "adj-na n", "", 470, &["disliked", "hated", "disagreeable"], 0),
        ("高い", "たかい", "adj-i", "adj-i", 475, &["high", "tall", "expensive", "high-priced"], 2),
        ("強い", "つよい", "adj-i", "adj-i", 475, &["strong", "potent", "competent", "resilient"], 2),
        ("早い", "はやい", "adj-i", "adj-i", 470, &["fast", "quick", "early", "soon"], 2),
        ("面白い", "おもしろい", "adj-i", "adj-i", 470, &["interesting", "fascinating", "funny", "amusing"], 4),
        ("可愛い", "かわいい", "adj-i", "adj-i", 475, &["cute", "adorable", "charming", "lovely"], 3),
        ("怖い", "こわい", "adj-i", "adj-i", 465, &["scary", "frightening", "eerie", "dreadful"], 2),
        ("嬉しい", "うれしい", "adj-i", "adj-i", 465, &["happy", "glad", "pleased", "delighted"], 3),
        ("悲しい", "かなしい", "adj-i", "adj-i", 460, &["sad", "miserable", "unhappy", "sorrowful"], 0),
        ("優しい", "やさしい", "adj-i", "adj-i", 465, &["tender", "kind", "gentle", "graceful", "affectionate"], 0),
        ("新しい", "あたらしい", "adj-i", "adj-i", 470, &["new", "novel", "fresh", "recent"], 4),
        ("良い", "いい", "adj-i", "adj-i", 500, &["good", "excellent", "fine", "nice", "pleasant"], 1),
        ("悪い", "わるい", "adj-i", "adj-i", 475, &["bad", "poor", "undesirable", "at fault", "sorry"], 2),
        ("今日", "きょう", "n", "", 490, &["today", "this day"], 1),
        ("明日", "あした", "n", "", 485, &["tomorrow"], 3),
        ("昨日", "きのう", "n", "", 480, &["yesterday"], 2),
        ("今", "いま", "n", "", 500, &["now", "the present time", "just now", "soon"], 1),
        ("私", "わたし", "pn", "", 500, &["I", "me"], 0),
        ("俺", "おれ", "pn", "", 490, &["I", "me (masculine, informal)"], 0),
        ("僕", "ぼく", "pn", "", 490, &["I", "me (mainly used by males)"], 1),
        ("君", "きみ", "pn", "", 485, &["you", "buddy", "pal", "monarch"], 0),
        ("お前", "おまえ", "pn", "", 480, &["you (informal/rough)"], 0),
        ("彼", "かれ", "pn", "", 475, &["he", "him", "boyfriend"], 1),
        ("彼女", "かのじょ", "pn", "", 475, &["she", "her", "girlfriend"], 1),
        ("誰", "だれ", "pn", "", 485, &["who"], 1),
        ("何", "なに", "pn", "", 500, &["what", "how many"], 1),
        ("どうして", "どうして", "adv", "", 475, &["why", "for what reason", "how"], 1),
        ("ありがとう", "ありがとう", "int", "", 500, &["thank you", "thanks"], 2),
        ("ごめん", "ごめん", "int", "", 480, &["I'm sorry", "my apologies", "excuse me"], 0),
    ];

    {
        let mut term_stmt = tx.prepare(
            "INSERT INTO terms (dict_id, expression, reading, definition_tags, rules, score, glossary_json, sequence, term_tags)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 'core')",
        )?;
        let mut meta_stmt = tx.prepare(
            "INSERT INTO term_meta (dict_id, expression, mode, data_json) VALUES (?1, ?2, 'pitch', ?3)",
        )?;
        for (expr, read, def_tags, rules, score, glosses, pitch) in starter_terms {
            let gloss_json = serde_json::to_string(glosses)?;
            term_stmt.execute(params![dict_id, expr, read, def_tags, rules, score, gloss_json])?;
            let pitch_json = json!({
                "reading": read,
                "pitches": [{ "position": pitch }]
            });
            meta_stmt.execute(params![dict_id, expr, pitch_json.to_string()])?;
        }
    }

    let starter_kanji: &[(&str, &str, &str, &[&str], &str, &str)] = &[
        ("食", "ショク ジキ", "た.べる く.う", &["eat", "food"], "7", "N5"),
        ("飲", "イン オン", "の.む", &["drink", "smoke", "take"], "12", "N4"),
        ("見", "ケン", "み.る み.える み.せる", &["see", "hopes", "chances", "idea", "opinion", "look at"], "7", "N5"),
        ("行", "コウ ギョウ アン", "い.く ゆ.く おこな.う", &["going", "journey", "carry out", "conduct", "act"], "6", "N5"),
        ("来", "ライ タイ", "く.る きた.る", &["come", "due", "next", "cause", "become"], "7", "N5"),
        ("言", "ゲン ゴン", "い.う こと", &["say", "word"], "7", "N5"),
        ("思", "シ", "おも.う", &["think"], "9", "N4"),
        ("知", "チ", "し.る", &["know", "wisdom"], "8", "N4"),
        ("分", "ブン フン ブ", "わ.ける わ.かる", &["part", "minute of time", "segment", "understand"], "4", "N5"),
        ("聞", "ブン モン", "き.く き.こえる", &["hear", "ask", "listen"], "14", "N5"),
        ("話", "ワ", "はな.す はなし", &["tale", "talk"], "13", "N5"),
        ("読", "ドク トク トウ", "よ.む", &["read"], "14", "N5"),
        ("書", "ショ", "か.く", &["write"], "10", "N5"),
        ("高", "コウ", "たか.い たか.まる", &["tall", "high", "expensive"], "10", "N5"),
        ("強", "キョウ ゴウ", "つよ.い つよ.まる", &["strong"], "11", "N4"),
        ("世", "セイ セ ソウ", "よ", &["generation", "world", "society", "public"], "5", "N4"),
        ("界", "カイ", "", &["world", "boundary"], "9", "N4"),
        ("時", "ジ", "とき", &["time", "hour"], "10", "N5"),
        ("間", "カン ケン", "あいだ ま", &["interval", "space"], "12", "N5"),
        ("人", "ジン ニン", "ひと", &["person"], "2", "N5"),
    ];

    {
        let mut kanji_stmt = tx.prepare(
            "INSERT INTO kanji (dict_id, character, onyomi, kunyomi, tags, meanings_json, stats_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for (ch, on, kun, meanings, strokes, jlpt) in starter_kanji {
            let meanings_json = serde_json::to_string(meanings)?;
            let stats_json = json!({ "strokes": strokes, "jlpt": jlpt }).to_string();
            kanji_stmt.execute(params![dict_id, ch, on, kun, jlpt, meanings_json, stats_json])?;
        }
    }

    tx.execute(
        "UPDATE dictionaries SET term_count = ?2, kanji_count = ?3, meta_count = ?2 WHERE id = ?1",
        params![dict_id, starter_terms.len() as i64, starter_kanji.len() as i64],
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deinflects_multi_step_japanese_conjugations() {
        // Causative-passive past: 食べさせられた -> 食べる
        let cands = deinflect("食べさせられた");
        let base = cands.iter().find(|c| c.text == "食べる").expect("should deinflect 食べさせられた to 食べる");
        assert_ne!(base.conditions & C_V1, 0);

        // Godan negative past: 読まなかった -> 読む
        let cands = deinflect("読まなかった");
        let base = cands.iter().find(|c| c.text == "読む").expect("should deinflect 読まなかった to 読む");
        assert_ne!(base.conditions & C_V5, 0);

        // Godan progressive colloquial: 泳いでる -> 泳ぐ
        let cands = deinflect("泳いでる");
        assert!(cands.iter().any(|c| c.text == "泳ぐ"));

        // I-adjective negative past: 高くなかった -> 高い
        let cands = deinflect("高くなかった");
        let base = cands.iter().find(|c| c.text == "高い").expect("should deinflect 高くなかった to 高い");
        assert_ne!(base.conditions & C_ADJ_I, 0);

        // Suru progressive: 勉強している -> 勉強する & 勉強
        let cands = deinflect("勉強している");
        assert!(cands.iter().any(|c| c.text == "勉強する"));
        assert!(cands.iter().any(|c| c.text == "勉強"));
    }

    #[test]
    fn distributes_furigana_cleanly() {
        let segs = distribute_furigana("食べる", "たべる");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "食");
        assert_eq!(segs[0].reading.as_deref(), Some("た"));
        assert_eq!(segs[1].text, "べる");
        assert_eq!(segs[1].reading, None);
    }

    #[test]
    fn starter_dictionary_lookup_finds_conjugated_words_and_kanji() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(DICT_SCHEMA).unwrap();
        ensure_starter_dictionary(&conn).unwrap();

        // Look up at offset 2 ("食べさせられた") in "昨日食べさせられた"
        let res = lookup_at(&conn, "昨日食べさせられた", 2).unwrap();
        assert_eq!(res.matched_text, "食べさせられた");
        assert_eq!(res.match_len, 7);
        assert!(!res.terms.is_empty());
        assert_eq!(res.terms[0].expression, "食べる");
        assert!(!res.kanji.is_empty());
        assert_eq!(res.kanji[0].character, "食");
    }
}

