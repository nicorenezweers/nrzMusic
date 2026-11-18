-- nrzMusic 1.2 – Konten. Playlists, Favoriten, Verlauf und die Spotify-Anmeldung gehören
-- jetzt einem Konto; die Bibliothek (Dateien im Musikordner) und die Downloads teilen sich
-- alle. `user_id = 0` heisst "noch niemandem zugeordnet" (Daten aus 1.1) – das erste
-- Konto (Admin) übernimmt sie.

CREATE TABLE users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    username      TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    display_name  TEXT    NOT NULL DEFAULT '',
    password_hash TEXT    NOT NULL,                   -- Argon2id (PHC); leer = keine Anmeldung möglich
    is_admin      INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    last_login    INTEGER
) STRICT;

-- Sitzungen: nur der SHA-256 des Tokens liegt hier, das Token selbst im Cookie.
CREATE TABLE sessions (
    token_hash TEXT    PRIMARY KEY NOT NULL,
    user_id    INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL,
    user_agent TEXT    NOT NULL DEFAULT ''
) STRICT;
CREATE INDEX sessions_user ON sessions (user_id);

-- Favoriten je Konto
CREATE TABLE favorites_new (
    user_id  INTEGER NOT NULL DEFAULT 0,
    key      TEXT    NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, key)
) STRICT;
INSERT INTO favorites_new (user_id, key, added_at) SELECT 0, key, added_at FROM favorites;
DROP TABLE favorites;
ALTER TABLE favorites_new RENAME TO favorites;
CREATE INDEX favorites_added ON favorites (user_id, added_at);

-- Verlauf je Konto
CREATE TABLE history_new (
    user_id    INTEGER NOT NULL DEFAULT 0,
    key        TEXT    NOT NULL,
    played_at  INTEGER NOT NULL,
    play_count INTEGER NOT NULL DEFAULT 1,
    seq        INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, key)
) STRICT;
INSERT INTO history_new (user_id, key, played_at, play_count, seq)
    SELECT 0, key, played_at, play_count, seq FROM history;
DROP TABLE history;
ALTER TABLE history_new RENAME TO history;
CREATE INDEX history_seq ON history (user_id, seq);

-- Playlists je Konto
ALTER TABLE playlists ADD COLUMN user_id INTEGER NOT NULL DEFAULT 0;
CREATE INDEX playlists_user ON playlists (user_id, updated_at);

-- Spotify-Anmeldung je Konto
CREATE TABLE spotify_auth_new (
    user_id       INTEGER PRIMARY KEY NOT NULL,
    access_token  TEXT    NOT NULL,
    refresh_token TEXT    NOT NULL,
    expires_at    INTEGER NOT NULL,
    scope         TEXT    NOT NULL DEFAULT '',
    user          TEXT,
    updated_at    INTEGER NOT NULL
) STRICT;
INSERT INTO spotify_auth_new (user_id, access_token, refresh_token, expires_at, scope, user, updated_at)
    SELECT 0, access_token, refresh_token, expires_at, scope, user, updated_at FROM spotify_auth;
DROP TABLE spotify_auth;
ALTER TABLE spotify_auth_new RENAME TO spotify_auth;

-- Wer einen Download angestossen hat (für die Anzeige; die Datei gehört allen)
ALTER TABLE download_jobs ADD COLUMN user_id INTEGER NOT NULL DEFAULT 0;

-- Aus Spotify übernommene Playlists merken sich ihre Herkunft ("liked" oder die
-- Playlist-ID) – ein erneuter Import aktualisiert sie statt eine zweite anzulegen.
ALTER TABLE playlists ADD COLUMN spotify_id TEXT;
