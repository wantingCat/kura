//! Persisting provider data into the local cache.

use crate::db::now;
use crate::providers::{AlMedia, EpisodeMeta};
use anyhow::Result;
use rusqlite::{params, Connection};

pub fn upsert_media(conn: &Connection, m: &AlMedia) -> Result<()> {
    let synonyms = serde_json::to_string(&m.synonyms)?;
    let genres = serde_json::to_string(&m.genres)?;
    let tags: Vec<_> = m.tags.iter().filter(|t| !t.is_media_spoiler.unwrap_or(false)).map(|t| &t.name).collect();
    let tags = serde_json::to_string(&tags)?;
    let studios: Vec<String> = m.studios.as_ref().map(|s| s.nodes.iter().map(|n| n.name.clone()).collect()).unwrap_or_default();
    let studios = serde_json::to_string(&studios)?;
    let cover = m.cover_image.as_ref();

    conn.execute(
        "INSERT INTO media (anilist_id, id_mal, title_romaji, title_english, title_native, synonyms, format, status,
            episodes, duration, season, season_year, start_date, end_date, description, genres, tags, studios,
            cover_url, cover_color, banner_url, average_score, next_airing_episode, next_airing_at, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)
         ON CONFLICT(anilist_id) DO UPDATE SET
            id_mal = excluded.id_mal, title_romaji = excluded.title_romaji, title_english = excluded.title_english,
            title_native = excluded.title_native, synonyms = excluded.synonyms, format = excluded.format,
            status = excluded.status, episodes = excluded.episodes, duration = excluded.duration,
            season = excluded.season, season_year = excluded.season_year, start_date = excluded.start_date,
            end_date = excluded.end_date, description = excluded.description, genres = excluded.genres,
            tags = excluded.tags, studios = excluded.studios,
            cover_path = CASE
                WHEN media.cover_path IS NOT NULL AND (media.cover_url IS NULL OR media.cover_url IS excluded.cover_url) THEN media.cover_path
                WHEN media.cover_path IS NOT NULL AND NOT (media.cover_path LIKE '%images%covers%' OR media.cover_path LIKE '%images/covers%') THEN media.cover_path
                ELSE NULL
            END,
            banner_path = CASE
                WHEN media.banner_path IS NOT NULL AND (media.banner_url IS NULL OR media.banner_url IS excluded.banner_url) THEN media.banner_path
                WHEN media.banner_path IS NOT NULL AND NOT (media.banner_path LIKE '%images%banners%' OR media.banner_path LIKE '%images/banners%') THEN media.banner_path
                ELSE NULL
            END,
            banner_url = CASE
                WHEN media.banner_url LIKE '%thetvdb.com%' OR media.banner_url LIKE '%fanart.tv%' THEN media.banner_url
                ELSE COALESCE(excluded.banner_url, media.banner_url)
            END,
            cover_url = excluded.cover_url, cover_color = excluded.cover_color,
            average_score = excluded.average_score, next_airing_episode = excluded.next_airing_episode,
            next_airing_at = excluded.next_airing_at, fetched_at = excluded.fetched_at",
        params![
            m.id,
            m.id_mal,
            m.title.romaji,
            m.title.english,
            m.title.native,
            synonyms,
            m.format,
            m.status,
            m.episodes,
            m.duration,
            m.season,
            m.season_year,
            m.start_date.as_ref().and_then(|d| d.to_string_opt()),
            m.end_date.as_ref().and_then(|d| d.to_string_opt()),
            m.description,
            genres,
            tags,
            studios,
            cover.and_then(|c| c.extra_large.clone().or(c.large.clone())),
            cover.and_then(|c| c.color.clone()),
            m.banner_image,
            m.average_score,
            m.next_airing_episode.as_ref().map(|a| a.episode),
            m.next_airing_episode.as_ref().map(|a| a.airing_at),
            now(),
        ],
    )?;

    if let Some(rel) = &m.relations {
        conn.execute("DELETE FROM media_relations WHERE anilist_id = ?1", [m.id])?;
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO media_relations
             (anilist_id, related_id, relation_type, title, format, media_type, status, episodes, season_year, cover_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for e in &rel.edges {
            let n = &e.node;
            let title = n.title.english.clone().or(n.title.romaji.clone());
            stmt.execute(params![
                m.id,
                n.id,
                e.relation_type,
                title,
                n.format,
                n.media_type,
                n.status,
                n.episodes,
                n.season_year,
                n.cover_image.as_ref().and_then(|c| c.large.clone()),
            ])?;
        }
    }
    Ok(())
}

/// Replace the episode list of a media entry. Keeps already-downloaded thumbnails when the URL is unchanged.
pub fn replace_episodes(conn: &mut Connection, anilist_id: i64, eps: &[EpisodeMeta], source: &str) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut old_thumbs = std::collections::HashMap::new();
        {
            let mut stmt = tx.prepare("SELECT ep_key, thumb_url, thumb_path FROM episodes WHERE anilist_id = ?1")?;
            let rows = stmt.query_map([anilist_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?))
            })?;
            for row in rows {
                let (k, u, p) = row?;
                old_thumbs.insert(k, (u, p));
            }
        }
        tx.execute("DELETE FROM episodes WHERE anilist_id = ?1", [anilist_id])?;
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO episodes (anilist_id, ep_key, number, is_special, title_en, title_ja, title_romaji,
                overview, air_date, runtime, thumb_url, thumb_path, tvdb_season, tvdb_episode, absolute_number, filler, recap)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        )?;
        for e in eps {
            let thumb_path = old_thumbs
                .get(&e.ep_key)
                .filter(|(u, _)| *u == e.thumb_url)
                .and_then(|(_, p)| p.clone());
            stmt.execute(params![
                anilist_id,
                e.ep_key,
                e.number,
                e.is_special as i64,
                e.title_en,
                e.title_ja,
                e.title_romaji,
                e.overview,
                e.air_date,
                e.runtime,
                e.thumb_url,
                thumb_path,
                e.tvdb_season,
                e.tvdb_episode,
                e.absolute_number,
                e.filler as i64,
                e.recap as i64,
            ])?;
        }
        tx.execute(
            "UPDATE media SET episodes_fetched_at = ?2, episodes_source = ?3 WHERE anilist_id = ?1",
            params![anilist_id, now(), source],
        )?;
    }
    tx.commit()?;
    Ok(())
}
