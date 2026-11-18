//! Konten und Sitzungen.

use crate::db::now;
use crate::error::{AppError, AppResult};
use nrzmusic_types::Account;
use sqlx::{AssertSqlSafe, SqlitePool};

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    username: String,
    display_name: String,
    is_admin: i64,
    created_at: i64,
    last_login: Option<i64>,
}

impl From<Row> for Account {
    fn from(r: Row) -> Self {
        Account {
            id: r.id,
            username: r.username,
            display_name: r.display_name,
            is_admin: r.is_admin != 0,
            created_at: r.created_at,
            last_login: r.last_login,
        }
    }
}

const COLS: &str = "id, username, display_name, is_admin, created_at, last_login";

pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<Option<Account>> {
    let row: Option<Row> = sqlx::query_as(AssertSqlSafe(format!("SELECT {COLS} FROM users WHERE id = ?")))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(Into::into))
}

pub async fn list(pool: &SqlitePool) -> AppResult<Vec<Account>> {
    let rows: Vec<Row> = sqlx::query_as(AssertSqlSafe(format!("SELECT {COLS} FROM users ORDER BY id")))
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Konto und Passwort-Hash zum Benutzernamen (Gross/Klein egal).
pub async fn by_name(pool: &SqlitePool, username: &str) -> AppResult<Option<(Account, String)>> {
    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(pool)
            .await?;
    let Some((id, hash)) = row else {
        return Ok(None);
    };
    Ok(get(pool, id).await?.map(|a| (a, hash)))
}

pub async fn password_hash(pool: &SqlitePool, id: i64) -> AppResult<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Gibt es ein Konto, mit dem man sich anmelden kann?
pub async fn any_with_password(pool: &SqlitePool) -> AppResult<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE password_hash != ''")
        .fetch_one(pool)
        .await?;
    Ok(n > 0)
}

pub async fn count_admins(pool: &SqlitePool) -> AppResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE is_admin = 1 AND password_hash != ''")
            .fetch_one(pool)
            .await?,
    )
}

/// Neues Konto. 409, wenn der Name vergeben ist.
pub async fn create(
    pool: &SqlitePool,
    username: &str,
    display_name: &str,
    password_hash: &str,
    is_admin: bool,
) -> AppResult<Account> {
    let res = sqlx::query_scalar::<_, i64>(
        "INSERT INTO users (username, display_name, password_hash, is_admin, created_at) \
         VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(username)
    .bind(display_name)
    .bind(password_hash)
    .bind(is_admin as i64)
    .bind(now())
    .fetch_one(pool)
    .await;
    let id = match res {
        Ok(id) => id,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(AppError::Conflict(
                "Dieser Benutzername ist schon vergeben.".into(),
            ))
        }
        Err(e) => return Err(e.into()),
    };
    get(pool, id).await?.ok_or(AppError::NotFound)
}

pub async fn set_password(pool: &SqlitePool, id: i64, hash: &str) -> AppResult<()> {
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(hash)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_admin(pool: &SqlitePool, id: i64, admin: bool) -> AppResult<()> {
    sqlx::query("UPDATE users SET is_admin = ? WHERE id = ?")
        .bind(admin as i64)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_display_name(pool: &SqlitePool, id: i64, name: &str) -> AppResult<()> {
    sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn touch_login(pool: &SqlitePool, id: i64) -> AppResult<()> {
    sqlx::query("UPDATE users SET last_login = ? WHERE id = ?")
        .bind(now())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Tabellen mit Daten je Konto.
const OWNED: &[&str] = &["favorites", "history", "playlists", "spotify_auth"];

/// Daten von `from` (0 = niemandem zugeordnet) an `to` übergeben. Doppelte Favoriten,
/// Verlaufseinträge und eine zweite Spotify-Anmeldung bleiben beim Ziel.
pub async fn transfer(pool: &SqlitePool, from: i64, to: i64) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    for table in OWNED {
        sqlx::query(AssertSqlSafe(format!(
            "UPDATE OR IGNORE {table} SET user_id = ? WHERE user_id = ?"
        )))
        .bind(to)
        .bind(from)
        .execute(&mut *tx)
        .await?;
        sqlx::query(AssertSqlSafe(format!("DELETE FROM {table} WHERE user_id = ?")))
            .bind(from)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE download_jobs SET user_id = ? WHERE user_id = ?")
        .bind(to)
        .bind(from)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Konto samt Playlists, Favoriten, Verlauf, Spotify-Anmeldung und Sitzungen löschen.
pub async fn delete(pool: &SqlitePool, id: i64) -> AppResult<bool> {
    let mut tx = pool.begin().await?;
    for table in OWNED {
        sqlx::query(AssertSqlSafe(format!("DELETE FROM {table} WHERE user_id = ?")))
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    let res = sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

// ------------------------------------------------------------ Sitzungen

pub async fn create_session(
    pool: &SqlitePool,
    token_hash: &str,
    user_id: i64,
    lifetime: i64,
    user_agent: &str,
) -> AppResult<()> {
    let at = now();
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at, last_seen, user_agent) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(at)
    .bind(at + lifetime)
    .bind(at)
    .bind(user_agent.chars().take(200).collect::<String>())
    .execute(pool)
    .await?;
    // Abgelaufene wegräumen
    sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
        .bind(at)
        .execute(pool)
        .await?;
    Ok(())
}

/// Konto zur Sitzung; verlängert sie (rollierend), höchstens einmal pro Stunde geschrieben.
pub async fn session_user(
    pool: &SqlitePool,
    token_hash: &str,
    lifetime: i64,
) -> AppResult<Option<Account>> {
    let at = now();
    let row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT user_id, last_seen FROM sessions WHERE token_hash = ? AND expires_at > ?",
    )
    .bind(token_hash)
    .bind(at)
    .fetch_optional(pool)
    .await?;
    let Some((user_id, last_seen)) = row else {
        return Ok(None);
    };
    if at - last_seen > 3600 {
        sqlx::query("UPDATE sessions SET last_seen = ?, expires_at = ? WHERE token_hash = ?")
            .bind(at)
            .bind(at + lifetime)
            .bind(token_hash)
            .execute(pool)
            .await?;
    }
    get(pool, user_id).await
}

pub async fn delete_session(pool: &SqlitePool, token_hash: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(token_hash)
        .execute(pool)
        .await?;
    Ok(())
}

/// Alle Sitzungen eines Kontos ausser `keep` (z. B. nach einem neuen Passwort).
pub async fn delete_sessions_except(
    pool: &SqlitePool,
    user_id: i64,
    keep: Option<&str>,
) -> AppResult<()> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash != ?")
        .bind(user_id)
        .bind(keep.unwrap_or(""))
        .execute(pool)
        .await?;
    Ok(())
}
