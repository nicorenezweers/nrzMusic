//! Migration 2 (nrzMusic 1.1): Downloads mit yt-dlp und Spotify.
//! Angewendet heisst unveränderlich – siehe [`super`].

pub const SQL: &str = r#"-- nrzMusic 1.1 – Downloads mit yt-dlp und Spotify.
-- Einstellungen brauchen keine Änderung (Schlüssel/Wert, neue Schlüssel kommen einfach dazu).

-- ============================================================ Downloads (yt-dlp)

-- Ein Auftrag je YouTube-Video. Ein erneuter Wunsch (fehlgeschlagen, abgebrochen, Datei
-- gelöscht) setzt denselben Auftrag zurück auf "queued".
CREATE TABLE download_jobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    video_id    TEXT    NOT NULL UNIQUE,
    track       TEXT    NOT NULL,                     -- YtTrack als JSON
    status      TEXT    NOT NULL DEFAULT 'queued'
                CHECK (status IN ('queued', 'running', 'done', 'failed', 'canceled')),
    progress    REAL    NOT NULL DEFAULT 0,           -- 0.0 – 1.0
    track_id    INTEGER,                              -- tracks.id nach dem Download (ohne FK: die Datei kann verschwinden)
    error       TEXT,
    hidden      INTEGER NOT NULL DEFAULT 0,           -- aus der Liste entfernt, bleibt für /downloads/map
    created_at  INTEGER NOT NULL,
    started_at  INTEGER,
    finished_at INTEGER
) STRICT;
CREATE INDEX download_jobs_status ON download_jobs (status, created_at);
CREATE INDEX download_jobs_track ON download_jobs (track_id);

-- ============================================================ Spotify

-- Höchstens eine Anmeldung. Die Tokens verlassen den Server nie.
CREATE TABLE spotify_auth (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    access_token  TEXT    NOT NULL,
    refresh_token TEXT    NOT NULL,
    expires_at    INTEGER NOT NULL,                   -- Unix-Sekunden
    scope         TEXT    NOT NULL DEFAULT '',
    user          TEXT,                               -- SpotifyUser als JSON (zwischengespeichert)
    updated_at    INTEGER NOT NULL
) STRICT;

-- Abgleich Spotify-Titel → YouTube Music. `video_id` NULL = nichts gefunden
-- (wird nach 7 Tagen neu gesucht).
CREATE TABLE spotify_matches (
    spotify_id TEXT    PRIMARY KEY NOT NULL,
    video_id   TEXT,
    data       TEXT,                                  -- YtTrack als JSON
    score      REAL    NOT NULL DEFAULT 0,
    matched_at INTEGER NOT NULL
) STRICT;
"#;
