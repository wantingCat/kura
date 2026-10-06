//! Metadata orchestration: caching rules, franchise walking and file → episode resolution.

use crate::db::{self, now, MediaLite};
use crate::parser::{Parsed, SpecialKind};
use crate::providers::{EpisodeMeta, Providers};
use crate::store;
use anyhow::Result;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard};

pub struct AppState {
    db: Mutex<Connection>,
    pub providers: Providers,
    pub data_dir: PathBuf,
    pub scanning: AtomicBool,
    pub rescan_requested: AtomicBool,
}

impl AppState {
    pub fn new(conn: Connection, data_dir: PathBuf) -> Self {
        Self {
            db: Mutex::new(conn),
            providers: Providers::new(),
            data_dir,
            scanning: AtomicBool::new(false),
            rescan_requested: AtomicBool::new(false),
        }
    }

    pub fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }
}

const DAY: i64 = 86_400;
const SERIES_FORMATS: &[&str] = &["TV", "TV_SHORT", "ONA"];

fn is_airing(m: &MediaLite) -> bool {
    matches!(m.status.as_deref(), Some("RELEASING") | Some("NOT_YET_RELEASED"))
}

/// Make sure a media entry is cached and reasonably fresh. Works offline if already cached.
pub async fn ensure_media(st: &AppState, id: i64) -> Result<()> {
    let existing = db::media_lite(&st.db(), id)?;
    let stale = match &existing {
        None => true,
        Some(m) => {
            let max_age = if is_airing(m) { DAY } else { 14 * DAY };
            now() - m.fetched_at > max_age
        }
    };
    if !stale {
        return Ok(());
    }
    match st.providers.anilist_media(id).await {
        Ok(m) => store::upsert_media(&st.db(), &m),
        // Offline / API error: fall back to whatever we have.
        Err(e) if existing.is_some() => {
            eprintln!("[kura] refresh of media {id} failed, using cache: {e}");
            Ok(())
        }
        Err(e) => Err(e),
    }
}

pub async fn ensure_episodes(st: &AppState, id: i64) -> Result<()> {
    let Some(m) = db::media_lite(&st.db(), id)? else { return Ok(()) };
    let stale = match m.episodes_fetched_at {
        None => true,
        Some(t) => {
            let max_age = if is_airing(&m) { DAY / 2 } else { 30 * DAY };
            now() - t > max_age
        }
    };
    if !stale {
        return Ok(());
    }
    if let Err(e) = fetch_episodes(st, &m).await {
        eprintln!("[kura] episode fetch for {id} failed: {e}");
    }
    Ok(())
}

pub async fn fetch_episodes(st: &AppState, m: &MediaLite) -> Result<()> {
    let mut source = Vec::new();
    let mut eps: Vec<EpisodeMeta> = match st.providers.anizip_episodes(m.id).await {
        Ok(e) if !e.is_empty() => {
            source.push("anizip");
            e
        }
        Ok(_) => Vec::new(),
        Err(e) => {
            eprintln!("[kura] ani.zip failed for {}: {e}", m.id);
            Vec::new()
        }
    };

    let regular_titled = eps.iter().filter(|e| !e.is_special && (e.title_en.is_some() || e.title_ja.is_some())).count();
    if regular_titled == 0 {
        if let Some(mal) = m.id_mal {
            match st.providers.jikan_episodes(mal).await {
                Ok(jk) if !jk.is_empty() => {
                    source.push("jikan");
                    let mut by_key: HashMap<String, usize> =
                        eps.iter().enumerate().map(|(i, e)| (e.ep_key.clone(), i)).collect();
                    for j in jk {
                        if let Some(&i) = by_key.get(&j.ep_key) {
                            let e = &mut eps[i];
                            e.title_en = e.title_en.take().or(j.title_en);
                            e.title_ja = e.title_ja.take().or(j.title_ja);
                            e.title_romaji = e.title_romaji.take().or(j.title_romaji);
                            e.air_date = e.air_date.take().or(j.air_date);
                            e.filler = j.filler;
                            e.recap = j.recap;
                        } else {
                            by_key.insert(j.ep_key.clone(), eps.len());
                            eps.push(j);
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("[kura] Jikan failed for MAL {mal}: {e}"),
            }
        }
    }

    // Fill gaps so every expected episode has a row.
    let expected = m
        .episodes
        .or(m.next_airing_episode.map(|n| n - 1))
        .unwrap_or(0)
        .max(if m.format.as_deref() == Some("MOVIE") { 1 } else { 0 });
    let present: HashSet<String> = eps.iter().map(|e| e.ep_key.clone()).collect();
    for n in 1..=expected {
        if !present.contains(&n.to_string()) {
            eps.push(EpisodeMeta { ep_key: n.to_string(), number: n, ..Default::default() });
        }
    }
    if source.is_empty() {
        source.push("placeholder");
    }
    store::replace_episodes(&mut st.db(), m.id, &eps, &source.join("+"))
}

/// The ordered chain of main-series entries (TV / ONA) this entry belongs to.
pub async fn franchise_chain(st: &AppState, base: i64) -> Result<Vec<i64>> {
    let pick = |rels: &[db::RelationLite], rel: &str, seen: &HashSet<i64>| -> Option<i64> {
        rels.iter()
            .filter(|r| r.relation_type == rel)
            .filter(|r| r.media_type.as_deref().unwrap_or("ANIME") == "ANIME")
            .filter(|r| r.format.as_deref().map_or(false, |f| SERIES_FORMATS.contains(&f)))
            .map(|r| r.related_id)
            .find(|id| !seen.contains(id))
    };

    let mut seen = HashSet::from([base]);
    let mut root = base;
    for _ in 0..12 {
        ensure_media(st, root).await?;
        let rels = db::relations(&st.db(), root)?;
        match pick(&rels, "PREQUEL", &seen) {
            Some(p) => {
                seen.insert(p);
                root = p;
            }
            None => break,
        }
    }

    let mut chain = vec![root];
    let mut seen = HashSet::from([root]);
    let mut cur = root;
    for _ in 0..20 {
        ensure_media(st, cur).await?;
        let rels = db::relations(&st.db(), cur)?;
        match pick(&rels, "SEQUEL", &seen) {
            Some(n) => {
                seen.insert(n);
                chain.push(n);
                cur = n;
            }
            None => break,
        }
    }
    if !chain.contains(&base) {
        chain.insert(0, base);
    }
    Ok(chain)
}

fn episode_count(conn: &Connection, id: i64) -> Option<i64> {
    let m = db::media_lite(conn, id).ok()??;
    m.episodes.or(m.next_airing_episode.map(|n| n - 1)).filter(|n| *n > 0)
}

/// Roll an episode number over sequel entries ("episode 30" of a 24-episode entry → next entry, ep 6).
async fn rollover(st: &AppState, chain: &[i64], start_idx: usize, ep: i64) -> (i64, i64) {
    let mut n = ep;
    let mut idx = start_idx;
    loop {
        let id = chain[idx];
        let count = episode_count(&st.db(), id);
        match count {
            Some(c) if n > c && idx + 1 < chain.len() => {
                n -= c;
                idx += 1;
            }
            _ => return (id, n),
        }
    }
}

pub struct FileHints<'a> {
    pub season: Option<u32>,
    pub special_folder: bool,
    pub parsed: &'a Parsed,
}

/// Map a local file to (anilist_id, ep_key) given the group's base match.
pub async fn resolve_file(st: &AppState, base: i64, h: FileHints<'_>) -> Result<Option<(i64, String)>> {
    let p = h.parsed;
    ensure_media(st, base).await?;
    ensure_episodes(st, base).await?;
    let Some(lite) = db::media_lite(&st.db(), base)? else { return Ok(None) };
    let base_format = lite.format.clone().unwrap_or_default();

    // --- Movies -----------------------------------------------------------
    if p.special == Some(SpecialKind::Movie) && base_format != "MOVIE" {
        let rels = db::relations(&st.db(), base)?;
        let movies: Vec<_> = rels.iter().filter(|r| r.format.as_deref() == Some("MOVIE")).collect();
        if movies.len() == 1 {
            return Ok(Some((movies[0].related_id, "1".into())));
        }
        return Ok(None);
    }
    if base_format == "MOVIE" || (p.episode.is_none() && p.special.is_none() && !h.special_folder) {
        return Ok(Some((base, "1".into())));
    }

    // --- Specials / OVAs ---------------------------------------------------
    if p.special.is_some() || h.special_folder {
        let n = p.special_number.or(p.episode).map(|n| n as i64);
        let idx = db::episode_index(&st.db(), base)?;
        if let Some(n) = n {
            let key = format!("S{n}");
            if idx.iter().any(|(k, ..)| *k == key) {
                return Ok(Some((base, key)));
            }
        }
        // A separate AniList entry for the OVA/special?
        if matches!(base_format.as_str(), "OVA" | "SPECIAL" | "ONA") {
            return Ok(Some((base, n.unwrap_or(1).to_string())));
        }
        let wanted = match p.special {
            Some(SpecialKind::Ova) => Some("OVA"),
            Some(SpecialKind::Ona) => Some("ONA"),
            Some(SpecialKind::Special) => Some("SPECIAL"),
            _ => None,
        };
        let rels = db::relations(&st.db(), base)?;
        let all: Vec<_> = rels
            .iter()
            .filter(|r| r.media_type.as_deref().unwrap_or("ANIME") == "ANIME")
            .filter(|r| matches!(r.format.as_deref(), Some("OVA" | "SPECIAL" | "ONA")))
            .filter(|r| r.relation_type != "CHARACTER")
            .collect();
        let preferred: Vec<_> = all.iter().filter(|r| wanted.map_or(false, |w| r.format.as_deref() == Some(w))).collect();
        let pool: Vec<_> = if preferred.is_empty() { all.iter().collect() } else { preferred };
        if pool.len() == 1 {
            let num = n.unwrap_or(1);
            if pool[0].episodes.map_or(true, |e| num <= e) {
                return Ok(Some((pool[0].related_id, num.to_string())));
            }
        }
        return Ok(n.map(|n| (base, format!("S{n}"))));
    }

    // --- Regular episodes -------------------------------------------------
    let Some(ep) = p.episode.map(|e| e as i64) else { return Ok(None) };
    let base_eps = episode_count(&st.db(), base);
    let season = h.season.map(|s| s as i64);

    if season.unwrap_or(1) <= 1 && base_eps.map_or(true, |c| ep <= c) {
        return Ok(Some((base, ep.to_string())));
    }
    if !SERIES_FORMATS.contains(&base_format.as_str()) {
        return Ok(Some((base, ep.to_string())));
    }

    let chain = franchise_chain(st, base).await?;
    for id in &chain {
        ensure_episodes(st, *id).await?;
    }

    if let Some(s) = season {
        // TVDB-style season/episode numbering, as used by most release groups.
        for id in &chain {
            let idx = db::episode_index(&st.db(), *id)?;
            if let Some((k, ..)) = idx.iter().find(|(_, ts, te, _, sp)| !*sp && *ts == Some(s) && *te == Some(ep)) {
                return Ok(Some((*id, k.clone())));
            }
        }
        // Fallback: Nth main-series entry, with rollover for split cours.
        let s_idx = (s as usize).saturating_sub(1);
        if s_idx < chain.len() {
            let (id, n) = rollover(st, &chain, s_idx, ep).await;
            return Ok(Some((id, n.to_string())));
        }
    } else {
        // Absolute numbering across the franchise.
        for id in &chain {
            let idx = db::episode_index(&st.db(), *id)?;
            if let Some((k, ..)) = idx.iter().find(|(_, _, _, abs, sp)| !*sp && *abs == Some(ep)) {
                if *id != base || base_eps.map_or(true, |c| ep > c) {
                    return Ok(Some((*id, k.clone())));
                }
            }
        }
    }

    let start = chain.iter().position(|i| *i == base).unwrap_or(0);
    let (id, n) = rollover(st, &chain, start, ep).await;
    Ok(Some((id, n.to_string())))
}
