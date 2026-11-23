//! Migration 6 (nrzMusic 1.6): Jetzt läuft – Sync zwischen Geräten, Widget, MCP.
//! Angewendet heisst unveränderlich – siehe [`super`].

pub const SQL: &str = r#"-- nrzMusic 1.6 – Jetzt läuft. Jedes Gerät meldet, was es spielt (nur wenn das Konto es
-- eingeschaltet hat); die anderen Geräte zeigen „Hört auf …“. Mit `public` ist der Stand
-- zusätzlich über eine geheime Adresse lesbar (Widget, JSON, MCP).

CREATE TABLE presence (
    user_id    INTEGER PRIMARY KEY NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    enabled    INTEGER NOT NULL DEFAULT 0,
    public     INTEGER NOT NULL DEFAULT 0,
    token      TEXT    NOT NULL UNIQUE,           -- 16 Zufallsbytes, hex
    state      TEXT,                              -- letzter Stand (NowPlaying als JSON)
    updated_at INTEGER NOT NULL DEFAULT 0         -- Unix-Millisekunden
) STRICT;
"#;
