//! Persisting provider data into the local cache.

use crate::db::now;
use crate::providers::{AlCharacters, AlMedia, AlSeasonalMedia, AlStaff, EpisodeMeta};
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
    if m.characters.is_some() || m.staff.is_some() {
        let _ = upsert_credits(
            conn,
            m.id,
            m.characters.as_ref(),
            m.staff.as_ref(),
        );
    }
    if let Some(entry) = &m.media_list_entry {
        let _ = conn.execute_batch(crate::db::SCHEMA_V5);
        let eff_prog = if entry.status == "COMPLETED" {
            entry.progress.max(m.episodes.unwrap_or(1))
        } else {
            entry.progress
        };
        let _ = conn.execute(
            "INSERT INTO anilist_entries (anilist_id, entry_id, status, progress, score, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(anilist_id) DO UPDATE SET
               entry_id = excluded.entry_id,
               status = excluded.status,
               progress = MAX(anilist_entries.progress, excluded.progress),
               score = COALESCE(excluded.score, anilist_entries.score),
               updated_at = excluded.updated_at",
            params![
                m.id,
                entry.id,
                entry.status,
                eff_prog,
                entry.score,
                entry.updated_at.unwrap_or_else(now)
            ],
        );
        let _ = crate::anilist_sync::apply_remote_progress_to_local(conn, m.id, eff_prog);
    } else {
        crate::anilist_sync::reconcile_media_entry(conn, m.id, m.episodes);
    }
    Ok(())
}

/// Seed basic metadata from an AniList list/seasonal entry (`fetched_at = 0` so opening the detail page still fetches full relations/credits).
pub fn upsert_seasonal_media(conn: &Connection, m: &AlSeasonalMedia) -> Result<()> {
    let genres = serde_json::to_string(&m.genres)?;
    let studios: Vec<String> = m
        .studios
        .as_ref()
        .map(|s| s.nodes.iter().map(|n| n.name.clone()).collect())
        .unwrap_or_default();
    let studios = serde_json::to_string(&studios)?;
    let cover = m.cover_image.as_ref();

    conn.execute(
        "INSERT INTO media (anilist_id, title_romaji, title_english, title_native, synonyms, format, status,
            episodes, duration, season, season_year, start_date, genres, tags, studios,
            cover_url, cover_color, banner_url, average_score, next_airing_episode, next_airing_at, fetched_at)
         VALUES (?1, ?2, ?3, ?4, '[]', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, '[]', ?13, ?14, ?15, ?16, ?17, ?18, ?19, 0)
         ON CONFLICT(anilist_id) DO UPDATE SET
            title_romaji = COALESCE(excluded.title_romaji, media.title_romaji),
            title_english = COALESCE(excluded.title_english, media.title_english),
            title_native = COALESCE(excluded.title_native, media.title_native),
            format = COALESCE(excluded.format, media.format),
            status = COALESCE(excluded.status, media.status),
            episodes = COALESCE(excluded.episodes, media.episodes),
            duration = COALESCE(excluded.duration, media.duration),
            season = COALESCE(excluded.season, media.season),
            season_year = COALESCE(excluded.season_year, media.season_year),
            start_date = COALESCE(excluded.start_date, media.start_date),
            genres = CASE WHEN media.genres IS NULL OR media.genres = '[]' THEN excluded.genres ELSE media.genres END,
            studios = CASE WHEN media.studios IS NULL OR media.studios = '[]' THEN excluded.studios ELSE media.studios END,
            cover_url = COALESCE(media.cover_url, excluded.cover_url),
            cover_color = COALESCE(media.cover_color, excluded.cover_color),
            banner_url = COALESCE(media.banner_url, excluded.banner_url),
            average_score = COALESCE(excluded.average_score, media.average_score),
            next_airing_episode = excluded.next_airing_episode,
            next_airing_at = excluded.next_airing_at",
        params![
            m.id,
            m.title.romaji,
            m.title.english,
            m.title.native,
            m.format,
            m.status,
            m.episodes,
            m.duration,
            m.season,
            m.season_year,
            m.start_date.as_ref().and_then(|d| d.to_string_opt()),
            genres,
            studios,
            cover.and_then(|c| c.extra_large.clone().or(c.large.clone())),
            cover.and_then(|c| c.color.clone()),
            m.banner_image,
            m.average_score,
            m.next_airing_episode.as_ref().map(|a| a.episode),
            m.next_airing_episode.as_ref().map(|a| a.airing_at),
        ],
    )?;
    Ok(())
}

pub fn upsert_credits(
    conn: &Connection,
    anilist_id: i64,
    characters: Option<&AlCharacters>,
    staff: Option<&AlStaff>,
) -> Result<()> {
    let _ = conn.execute_batch(crate::db::SCHEMA_V6);
    if let Some(chars) = characters {
        conn.execute("DELETE FROM media_characters WHERE anilist_id = ?1", [anilist_id])?;
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO media_characters
             (anilist_id, character_id, role, name_full, name_native, image_url,
              va_id, va_name_full, va_name_native, va_image_url, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for (idx, edge) in chars.edges.iter().enumerate() {
            let Some(name_full) = edge.node.name.full.as_deref().filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            let va = edge.voice_actors.first();
            stmt.execute(params![
                anilist_id,
                edge.node.id,
                edge.role.as_deref().unwrap_or("SUPPORTING"),
                name_full,
                edge.node.name.native,
                edge.node.image.as_ref().and_then(|i| i.large.clone()),
                va.map(|v| v.id),
                va.and_then(|v| v.name.full.clone()),
                va.and_then(|v| v.name.native.clone()),
                va.and_then(|v| v.image.as_ref().and_then(|i| i.large.clone())),
                idx as i64,
            ])?;
        }
    }
    if let Some(stf) = staff {
        conn.execute("DELETE FROM media_staff WHERE anilist_id = ?1", [anilist_id])?;
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO media_staff
             (anilist_id, staff_id, role, name_full, name_native, image_url, sort_order)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for (idx, edge) in stf.edges.iter().enumerate() {
            let Some(name_full) = edge.node.name.full.as_deref().filter(|s| !s.trim().is_empty()) else {
                continue;
            };
            let role = edge.role.as_deref().unwrap_or("Staff").trim();
            stmt.execute(params![
                anilist_id,
                edge.node.id,
                role,
                name_full,
                edge.node.name.native,
                edge.node.image.as_ref().and_then(|i| i.large.clone()),
                idx as i64,
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
