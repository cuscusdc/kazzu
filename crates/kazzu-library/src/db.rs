//! Abertura do banco SQLite e migrações versionadas (`PRAGMA user_version`).

use rusqlite::Connection;

use crate::error::{Error, Result};

/// Cada item é uma migração; o índice + 1 é a versão resultante.
/// Nunca edite uma migração já publicada: acrescente uma nova.
///
/// ATENÇÃO (downgrade impossível): o banco v2 NÃO abre em app antigo
/// (`Error::SchemaTooNew`). Faça backup de `library.sqlite` antes de atualizar.
const MIGRATIONS: &[&str] = &[
    // v1 — esquema inicial
    r#"
    CREATE TABLE music_folders (
        path TEXT PRIMARY KEY
    );

    CREATE TABLE artists (
        id   INTEGER PRIMARY KEY,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE
    );

    CREATE TABLE albums (
        id        INTEGER PRIMARY KEY,
        title     TEXT NOT NULL COLLATE NOCASE,
        artist    TEXT NOT NULL COLLATE NOCASE,
        year      INTEGER,
        -- NULL: arte ainda não processada; '': processada, sem arte; senão chave do cache.
        art_key   TEXT,
        accent    INTEGER,
        UNIQUE (title, artist)
    );

    CREATE TABLE tracks (
        id          INTEGER PRIMARY KEY,
        path        TEXT NOT NULL UNIQUE,
        mtime       INTEGER NOT NULL,
        size        INTEGER NOT NULL,
        title       TEXT NOT NULL,
        album_id    INTEGER REFERENCES albums(id) ON DELETE SET NULL,
        year        INTEGER,
        track_no    INTEGER,
        disc_no     INTEGER,
        genre       TEXT,
        duration_ms INTEGER NOT NULL,
        sample_rate INTEGER,
        bit_depth   INTEGER,
        bitrate     INTEGER,
        channels    INTEGER,
        codec       TEXT NOT NULL,
        has_picture INTEGER NOT NULL DEFAULT 0,
        added_at    INTEGER NOT NULL
    );
    CREATE INDEX idx_tracks_album ON tracks(album_id, disc_no, track_no);
    CREATE INDEX idx_tracks_added ON tracks(added_at);

    CREATE TABLE track_artists (
        track_id  INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
        artist_id INTEGER NOT NULL REFERENCES artists(id) ON DELETE CASCADE,
        position  INTEGER NOT NULL,
        PRIMARY KEY (track_id, position)
    );
    CREATE INDEX idx_track_artists_artist ON track_artists(artist_id);

    CREATE TABLE playlists (
        id         INTEGER PRIMARY KEY,
        name       TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );

    CREATE TABLE playlist_items (
        id          INTEGER PRIMARY KEY,
        playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
        track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
        position    INTEGER NOT NULL,
        added_at    INTEGER NOT NULL
    );
    CREATE INDEX idx_playlist_items ON playlist_items(playlist_id, position);

    CREATE TABLE likes (
        track_id INTEGER PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
        liked_at INTEGER NOT NULL
    );

    -- Placeholder para o modo offline (etapa 4): faixas de provedores baixadas.
    CREATE TABLE downloads (
        id         INTEGER PRIMARY KEY,
        provider   TEXT NOT NULL,
        remote_id  TEXT NOT NULL,
        track_id   INTEGER REFERENCES tracks(id) ON DELETE SET NULL,
        file_path  TEXT,
        status     TEXT NOT NULL DEFAULT 'queued',
        bytes      INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        UNIQUE (provider, remote_id)
    );

    -- Histórico de reprodução: SOMENTE local. Nunca é enviado à rede.
    CREATE TABLE play_history (
        id          INTEGER PRIMARY KEY,
        track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
        played_at   INTEGER NOT NULL,
        listened_ms INTEGER NOT NULL
    );
    CREATE INDEX idx_history_time ON play_history(played_at);

    CREATE VIRTUAL TABLE tracks_fts USING fts5(
        title, artists, album,
        tokenize = 'unicode61 remove_diacritics 2'
    );
    "#,
    // v2 — playlists aceitam faixas remotas (por referência) ao lado das locais.
    // Posições continuam compartilhadas; `remote_json` é a `kazzu_core::Track`.
    r#"
    CREATE TABLE playlist_items_v2 (
        id              INTEGER PRIMARY KEY,
        playlist_id     INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
        track_id        INTEGER REFERENCES tracks(id) ON DELETE CASCADE,
        remote_provider TEXT,
        remote_id       TEXT,
        remote_json     TEXT,
        position        INTEGER NOT NULL,
        added_at        INTEGER NOT NULL,
        CHECK (
            (track_id IS NOT NULL AND remote_provider IS NULL
                AND remote_id IS NULL AND remote_json IS NULL)
            OR
            (track_id IS NULL AND remote_provider IS NOT NULL
                AND remote_id IS NOT NULL AND remote_json IS NOT NULL)
        )
    );
    INSERT INTO playlist_items_v2(id, playlist_id, track_id, position, added_at)
        SELECT id, playlist_id, track_id, position, added_at FROM playlist_items;
    DROP TABLE playlist_items;
    ALTER TABLE playlist_items_v2 RENAME TO playlist_items;
    CREATE INDEX idx_playlist_items ON playlist_items(playlist_id, position);
    "#,
    // v3 — Histórico remoto no mesmo SQLite. Guarda somente metadados e tempo de escuta.
    r#"
    CREATE TABLE remote_play_history (
        id          INTEGER PRIMARY KEY,
        provider    TEXT NOT NULL,
        remote_id   TEXT NOT NULL,
        track_json  TEXT NOT NULL,
        title       TEXT NOT NULL,
        artist      TEXT NOT NULL,
        album       TEXT NOT NULL,
        played_at   INTEGER NOT NULL,
        listened_ms INTEGER NOT NULL CHECK(listened_ms >= 0)
    );
    CREATE INDEX idx_remote_history_time ON remote_play_history(played_at);
    CREATE INDEX idx_remote_history_track ON remote_play_history(provider, remote_id);
    "#,

    // v4 — A capa escolhida faz parte da playlist e não do cache.
    r#"
    CREATE TABLE playlist_covers (
        playlist_id INTEGER PRIMARY KEY REFERENCES playlists(id) ON DELETE CASCADE,
        image BLOB NOT NULL CHECK(length(image) <= 1048576)
    );
    "#,

];

pub(crate) const SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// Quanto uma conexão espera por outra que esteja escrevendo (o app usa uma conexão para o scan e outra para a UI).
pub(crate) const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub(crate) fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    // Ignora erro em bancos em memória (WAL não se aplica).
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

pub(crate) fn migrate(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if current > SCHEMA_VERSION {
        return Err(Error::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }
    for (idx, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", idx as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Banco v1 populado sobrevive à migração v2 (itens locais, ordem e posições).
    #[test]
    fn v1_to_v2_preserves_playlist_items() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute_batch(
            "INSERT INTO tracks(id,path,mtime,size,title,duration_ms,codec,added_at)
               VALUES (1,'/a.flac',0,0,'A',1000,'FLAC',0),(2,'/b.flac',0,0,'B',1000,'FLAC',0);
             INSERT INTO playlists(id,name,created_at,updated_at) VALUES (7,'P',0,0);
             INSERT INTO playlist_items(id,playlist_id,track_id,position,added_at)
               VALUES (10,7,2,0,5),(11,7,1,1,6);",
        )
        .unwrap();

        migrate(&mut conn).unwrap();
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
        let rows: Vec<(i64, i64, i64, i64)> = conn
            .prepare(
                "SELECT id, track_id, position, added_at FROM playlist_items ORDER BY position",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![(10, 2, 0, 5), (11, 1, 1, 6)]);
        // O CHECK rejeita item sem nenhuma das duas referências.
        assert!(
            conn.execute(
                "INSERT INTO playlist_items(playlist_id, position, added_at) VALUES (7,2,0)",
                []
            )
            .is_err()
        );
    }
}
