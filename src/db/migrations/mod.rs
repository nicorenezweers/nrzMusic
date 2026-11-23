//! Schema-Migrationen, als Rust-Module ins Programm eingebettet. sqlx führt sie der Reihe nach
//! aus und merkt sich in `_sqlx_migrations`, was schon gelaufen ist – samt Prüfsumme (SHA-384)
//! des SQL-Textes. Darum: Eine angewendete Migration nie mehr ändern (auch keine Kommentare),
//! sondern eine neue anhängen – nächste Nummer, neues Modul `mNNNN_<name>.rs`, Eintrag in
//! [`MIGRATIONS`] und Prüfsumme im Test unten.
//!
//! Bis 1.8 lagen die Migrationen als `.sql`-Dateien in `migrations/`. Nummern, Beschreibungen
//! und Texte sind Byte für Byte übernommen; bestehende Datenbanken laufen ohne Weiteres weiter.

use sqlx::migrate::{Migration, MigrationType, Migrator};
use sqlx::SqlSafeStr;
use std::borrow::Cow;

mod m0001_init;
mod m0002_downloads_spotify;
mod m0003_accounts;
mod m0004_feed_stats;
mod m0005_shares;
mod m0006_presence;

/// Version, Beschreibung (wie sqlx sie aus dem alten Dateinamen bildete) und SQL.
const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "init", m0001_init::SQL),
    (2, "downloads spotify", m0002_downloads_spotify::SQL),
    (3, "accounts", m0003_accounts::SQL),
    (4, "feed stats", m0004_feed_stats::SQL),
    (5, "shares", m0005_shares::SQL),
    (6, "presence", m0006_presence::SQL),
];

/// Alle Migrationen, bereit für [`Migrator::run`].
pub fn migrator() -> Migrator {
    Migrator::with_migrations(
        MIGRATIONS
            .iter()
            .map(|&(version, description, sql)| {
                Migration::new(
                    version,
                    Cow::Borrowed(description),
                    MigrationType::Simple,
                    sql.into_sql_str(),
                    false,
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    /// SHA-384 der angewendeten Migrationen (identisch mit den früheren `.sql`-Dateien).
    /// Schlägt der Test fehl, wurde eine Migration verändert – bestehende Datenbanken würden
    /// den Start verweigern.
    const CHECKSUMS: &[(i64, &str)] = &[
        (1, "fc4efa6c38b822f5ea4038507dd628b4c49a86a68cf09e18f0239b79cec67fd458ba571e8de0fb63ecd51c9d941e4976"),
        (2, "3a61519fc634d73246a79b9ce92156944f2ff23b73bdae4a7e2d6730a1babfd9ed0ec66f74b80d03c8850c5cbcbadde3"),
        (3, "37f78a9b876d207944f7e127c9ffeb922fe6e67ca44212a3e7e163f861931e29a43e7d153a0aaa46a6c09fa6e8921404"),
        (4, "98d9709e9320cd813a5905d50bd0c98014374c720fbfc3dcabdba6f295a940d06cfeef4921aa642a71f0bc2942bd5807"),
        (5, "e94a9e11233496a0917e52e931aa2bf5de7d5445ce232d0b6ad7502f7c962efcc0624586dc4560387f882eefa2042d1e"),
        (6, "e711fe2e0fedc6878406ccf8dd51ec2418e68afd47d9473d52150d25178a77a0e10372b70893076b17ae7263591b46a3"),
    ];

    #[test]
    fn applied_migrations_unchanged() {
        let migrator = super::migrator();
        let found: Vec<(i64, String)> = migrator
            .iter()
            .map(|m| (m.version, hex::encode(&m.checksum)))
            .collect();
        let expected: Vec<(i64, String)> = CHECKSUMS
            .iter()
            .map(|&(v, sum)| (v, sum.to_string()))
            .collect();
        assert_eq!(found, expected);
    }

    #[test]
    fn versions_ascending_without_gaps() {
        for (i, m) in super::migrator().iter().enumerate() {
            assert_eq!(m.version, i as i64 + 1);
            assert!(!m.no_tx);
        }
    }
}
