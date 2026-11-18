-- nrzMusic – Grundschema.
-- Die Musik selbst bleibt als Dateien im Musikordner; hier liegen nur Metadaten,
-- Playlists, Favoriten, Verlauf und Einstellungen.
-- Konventionen:
--   * Zeitpunkte sind Unix-Sekunden (UTC) als INTEGER, Dauern Millisekunden.
--   * Titel werden über Schlüssel verknüpft: `local:<tracks.id>` oder `yt:<videoId>`.
--   * STRICT-Tabellen: SQLite prüft die Spaltentypen.

-- ============================================================ Bibliothek

-- Ein Titel je Datei. Die ID bleibt stabil, solange die Datei unter demselben Pfad liegt
-- (Upsert über `path`); AUTOINCREMENT vergibt die ID gelöschter Titel nie neu.
CREATE TABLE tracks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    path         TEXT    NOT NULL UNIQUE,             -- relativ zum Musikordner, mit '/'
    title        TEXT    NOT NULL,
    artist       TEXT    NOT NULL DEFAULT '',
    album        TEXT    NOT NULL DEFAULT '',
    album_artist TEXT    NOT NULL DEFAULT '',
    album_id     TEXT    NOT NULL,                    -- Hash aus Album-Künstler + Album
    artist_id    TEXT    NOT NULL,                    -- Hash aus dem Album-Künstler
    track_no     INTEGER,
    disc_no      INTEGER,
    year         INTEGER,
    genre        TEXT,
    duration_ms  INTEGER NOT NULL DEFAULT 0,
    bitrate_kbps INTEGER,
    format       TEXT    NOT NULL,
    size         INTEGER NOT NULL,
    mtime        INTEGER NOT NULL,                    -- Änderungszeit der Datei (ms)
    cover        TEXT,                                -- Cover-ID (data/covers/<id>.orig)
    search       TEXT    NOT NULL DEFAULT '',         -- Titel, Künstler, Album – klein, ohne Akzente
    added_at     INTEGER NOT NULL,
    play_count   INTEGER NOT NULL DEFAULT 0,
    last_played  INTEGER
) STRICT;
CREATE INDEX tracks_album ON tracks (album_id);
CREATE INDEX tracks_artist ON tracks (artist_id);
CREATE INDEX tracks_added ON tracks (added_at);

-- Metadaten von YouTube-Titeln, auf die Playlists, Favoriten oder Verlauf zeigen (YtTrack als JSON)
CREATE TABLE yt_tracks (
    video_id   TEXT    PRIMARY KEY NOT NULL,
    data       TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- ============================================================ Favoriten, Verlauf

CREATE TABLE favorites (
    key      TEXT    PRIMARY KEY NOT NULL,
    added_at INTEGER NOT NULL
) STRICT;
CREATE INDEX favorites_added ON favorites (added_at);

-- Ein Eintrag je Titel: zuletzt gespielt und wie oft. `seq` ordnet Wiedergaben innerhalb
-- derselben Sekunde (steigt bei jeder Wiedergabe).
CREATE TABLE history (
    key        TEXT    PRIMARY KEY NOT NULL,
    played_at  INTEGER NOT NULL,
    play_count INTEGER NOT NULL DEFAULT 1,
    seq        INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX history_seq ON history (seq);

-- ============================================================ Playlists

CREATE TABLE playlists (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    name                TEXT    NOT NULL,
    description         TEXT    NOT NULL DEFAULT '',
    youtube_playlist_id TEXT,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
) STRICT;

CREATE TABLE playlist_items (
    playlist_id INTEGER NOT NULL REFERENCES playlists (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    key         TEXT    NOT NULL,
    PRIMARY KEY (playlist_id, position)
) STRICT;

-- ============================================================ Einstellungen

CREATE TABLE settings (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
