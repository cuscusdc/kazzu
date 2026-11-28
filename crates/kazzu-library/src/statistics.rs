//! Aggregates listening history in the existing SQLite database.

use rusqlite::params;

use crate::{Library, Result};

#[derive(Debug)]
pub struct ListeningStats {
    pub from: i64,
    pub until: i64,
    pub listened_ms: u64,
    pub plays: u64,
    pub songs: Vec<ListeningRank>,
    pub artists: Vec<ListeningRank>,
    pub albums: Vec<ListeningRank>,
    pub genres: Vec<ListeningRank>,
}

#[derive(Debug)]
pub struct ListeningRank {
    pub title: String,
    pub subtitle: String,
    pub plays: u64,
    pub listened_ms: u64,
}

const WINDOW: &str = "
    WITH listens AS (
        SELECT 'local:' || t.id AS song, t.title,
            COALESCE(a.name, '') AS artist, COALESCE(al.title, '') AS album,
            COALESCE(al.artist, a.name, '') AS album_artist, COALESCE(TRIM(t.genre), '') AS genre,
            h.listened_ms
        FROM play_history h JOIN tracks t ON t.id = h.track_id
        LEFT JOIN track_artists ta ON ta.track_id = t.id AND ta.position = 0
        LEFT JOIN artists a ON a.id = ta.artist_id
        LEFT JOIN albums al ON al.id = t.album_id
        WHERE ?1 <= h.played_at AND h.played_at < ?2
        UNION ALL
        SELECT provider || ':' || remote_id, title, artist, album, artist, '', listened_ms
        FROM remote_play_history WHERE ?1 <= played_at AND played_at < ?2
    )";

impl Library {
    /// Half-open range using UTC Unix seconds. The app calculates the
    /// boundaries in the user's local time zone.
    pub fn listening_stats(&self, from: i64, until: i64) -> Result<ListeningStats> {
        let (listened_ms, plays) = self.conn.query_row(
            "SELECT COALESCE(SUM(listened_ms), 0), COUNT(*) FROM (
                SELECT listened_ms FROM play_history WHERE ?1 <= played_at AND played_at < ?2
                UNION ALL
                SELECT listened_ms FROM remote_play_history WHERE ?1 <= played_at AND played_at < ?2
            )",
            params![from, until],
            |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)),
        )?;
        let songs = self.listening_ranks(from, until, "
            SELECT MIN(title), MIN(artist), COUNT(*), SUM(listened_ms) FROM listens GROUP BY song
            ORDER BY COUNT(*) DESC, SUM(listened_ms) DESC, MIN(title) COLLATE NOCASE, song LIMIT 20")?;
        let artists = self.listening_ranks(
            from,
            until,
            "
            SELECT MIN(artist), '', COUNT(*), SUM(listened_ms) FROM listens WHERE artist <> ''
            GROUP BY artist COLLATE NOCASE
            ORDER BY COUNT(*) DESC, SUM(listened_ms) DESC, MIN(artist) COLLATE NOCASE LIMIT 20",
        )?;
        let albums = self.listening_ranks(from, until, "
            SELECT MIN(album), MIN(album_artist), COUNT(*), SUM(listened_ms) FROM listens WHERE album <> ''
            GROUP BY album COLLATE NOCASE, album_artist COLLATE NOCASE
            ORDER BY COUNT(*) DESC, SUM(listened_ms) DESC, MIN(album) COLLATE NOCASE LIMIT 20")?;
        let genres = self.listening_ranks(
            from,
            until,
            "
            SELECT MIN(genre), '', COUNT(*), SUM(listened_ms) FROM listens WHERE genre <> ''
            GROUP BY genre COLLATE NOCASE
            ORDER BY SUM(listened_ms) DESC, COUNT(*) DESC, MIN(genre) COLLATE NOCASE LIMIT 20",
        )?;

        Ok(ListeningStats {
            from,
            until,
            listened_ms,
            plays,
            songs,
            artists,
            albums,
            genres,
        })
    }

    fn listening_ranks(&self, from: i64, until: i64, sql: &str) -> Result<Vec<ListeningRank>> {
        let query = format!("{WINDOW} {sql}");
        let mut statement = self.conn.prepare(&query)?;
        let ranks = statement
            .query_map(params![from, until], |row| {
                Ok(ListeningRank {
                    title: row.get(0)?,
                    subtitle: row.get(1)?,
                    plays: row.get::<_, i64>(2)? as u64,
                    listened_ms: row.get::<_, i64>(3)? as u64,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ranks)
    }
}
