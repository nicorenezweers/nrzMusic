-- nrzMusic 1.6 – Feed und Statistik.
-- Jede Wiedergabe mit gehörter Zeit, Überspringen und Ende; daraus lernt der Feed, was
-- jemand mag, und daraus entsteht die Statistik. Künstler und Genre werden mitgeschrieben,
-- damit die Statistik ohne Nachschlagen auskommt (auch wenn eine Datei später fehlt).

CREATE TABLE listens (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL,
    key         TEXT    NOT NULL,                 -- local:<id> oder yt:<videoId>
    title       TEXT    NOT NULL DEFAULT '',
    artist      TEXT    NOT NULL DEFAULT '',      -- Hauptkünstler
    artist_id   TEXT,                             -- YouTube-Kanal ("UC…"), falls bekannt
    genre       TEXT,                             -- aus den Tags (lokale Titel)
    image       TEXT,
    started_at  INTEGER NOT NULL,
    listened_ms INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    skipped     INTEGER NOT NULL DEFAULT 0,
    completed   INTEGER NOT NULL DEFAULT 0,
    source      TEXT    NOT NULL DEFAULT ''
) STRICT;
CREATE INDEX listens_user_time ON listens (user_id, started_at);
CREATE INDEX listens_user_key ON listens (user_id, key);

-- "Nicht interessiert" im Feed
CREATE TABLE dislikes (
    user_id INTEGER NOT NULL,
    key     TEXT    NOT NULL,
    artist  TEXT    NOT NULL DEFAULT '',
    at      INTEGER NOT NULL,
    PRIMARY KEY (user_id, key)
) STRICT;

-- Genre je Künstler (für YouTube-Titel, die keins mitbringen); NULL = nichts gefunden
CREATE TABLE artist_genres (
    artist       TEXT    PRIMARY KEY NOT NULL COLLATE NOCASE,
    genre        TEXT,
    looked_up_at INTEGER NOT NULL
) STRICT;

-- Einstellungen je Konto (Schlüssel/Wert)
CREATE TABLE user_prefs (
    user_id INTEGER NOT NULL,
    key     TEXT    NOT NULL,
    value   TEXT    NOT NULL,
    PRIMARY KEY (user_id, key)
) STRICT;
