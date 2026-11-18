//! Migration 5 (nrzMusic 1.7): Teilen.
//! Angewendet heisst unveränderlich – siehe [`super`].

pub const SQL: &str = r#"-- nrzMusic 1.7 – Teilen. Ein Link (`/s/<token>`) zeigt einen Titel, die Likes oder eine
-- Playlist eines Kontos – ohne Anmeldung, immer im aktuellen Stand. Die Titel daraus lassen
-- sich über den Link auch abspielen (nur diese). Löschen macht den Link ungültig.

CREATE TABLE shares (
    token      TEXT    PRIMARY KEY NOT NULL,      -- 16 Zufallsbytes, hex
    user_id    INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL,                  -- track | favorites | playlist
    target     TEXT    NOT NULL DEFAULT '',       -- Titel-Schlüssel bzw. Playlist-ID
    created_at INTEGER NOT NULL,
    views      INTEGER NOT NULL DEFAULT 0,
    UNIQUE (user_id, kind, target)
) STRICT;
CREATE INDEX shares_user ON shares (user_id, created_at);
"#;
