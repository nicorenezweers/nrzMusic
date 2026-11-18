//! Download-Aufträge (yt-dlp): ein Auftrag je YouTube-Video in `download_jobs`.
//! Ein erneuter Wunsch für ein fehlgeschlagenes, abgebrochenes oder gelöschtes Video setzt
//! denselben Auftrag zurück, statt einen zweiten anzulegen.

use crate::db::now;
use crate::error::AppResult;
use nrzmusic_types::{DownloadJob, DownloadStatus, YtTrack};
use sqlx::SqlitePool;

/// Fertige Aufträge erscheinen so lange in der Liste (Sekunden).
pub const LIST_FINISHED_FOR: i64 = 7 * 24 * 60 * 60;

const COLUMNS: &str =
    "id, video_id, track, status, progress, track_id, error, created_at, finished_at";

#[derive(sqlx::FromRow)]
struct JobRow {
    id: i64,
    video_id: String,
    track: String,
    status: String,
    progress: f64,
    track_id: Option<i64>,
    error: Option<String>,
    created_at: i64,
    finished_at: Option<i64>,
}

pub fn status_str(s: DownloadStatus) -> &'static str {
    match s {
        DownloadStatus::Queued => "queued",
        DownloadStatus::Running => "running",
        DownloadStatus::Done => "done",
        DownloadStatus::Failed => "failed",
        DownloadStatus::Canceled => "canceled",
    }
}

fn parse_status(s: &str) -> DownloadStatus {
    match s {
        "queued" => DownloadStatus::Queued,
        "running" => DownloadStatus::Running,
        "done" => DownloadStatus::Done,
        "canceled" => DownloadStatus::Canceled,
        _ => DownloadStatus::Failed,
    }
}

impl From<JobRow> for DownloadJob {
    fn from(r: JobRow) -> Self {
        let track = serde_json::from_str::<YtTrack>(&r.track).unwrap_or_else(|e| {
            tracing::warn!(video = %r.video_id, "Gespeicherter Download-Titel unlesbar: {e}");
            YtTrack {
                video_id: r.video_id.clone(),
                title: r.video_id.clone(),
                artists: Vec::new(),
                album: None,
                duration_ms: None,
                thumbnail: None,
                explicit: false,
                is_video: false,
            }
        });
        DownloadJob {
            id: r.id,
            track,
            status: parse_status(&r.status),
            progress: r.progress.clamp(0.0, 1.0) as f32,
            track_id: r.track_id,
            error: r.error,
            created_at: r.created_at,
            finished_at: r.finished_at,
        }
    }
}

pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<Option<DownloadJob>> {
    let sql = format!("SELECT {COLUMNS} FROM download_jobs WHERE id = ?");
    let row: Option<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(DownloadJob::from))
}

pub async fn by_video(pool: &SqlitePool, video_id: &str) -> AppResult<Option<DownloadJob>> {
    let sql = format!("SELECT {COLUMNS} FROM download_jobs WHERE video_id = ?");
    let row: Option<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(video_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(DownloadJob::from))
}

/// Offene Aufträge und die fertigen der letzten 7 Tage (ohne ausgeblendete), neueste zuerst.
pub async fn list(pool: &SqlitePool) -> AppResult<Vec<DownloadJob>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM download_jobs \
         WHERE hidden = 0 AND (status IN ('queued', 'running') OR COALESCE(finished_at, created_at) >= ?) \
         ORDER BY created_at DESC, id DESC"
    );
    let rows: Vec<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(now() - LIST_FINISHED_FOR)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(DownloadJob::from).collect())
}

/// Neuen Auftrag anlegen oder einen bestehenden (fertig, fehlgeschlagen, abgebrochen) neu
/// einreihen – mit den aktuellen Metadaten.
pub async fn enqueue(pool: &SqlitePool, track: &YtTrack) -> AppResult<DownloadJob> {
    let data = serde_json::to_string(track).unwrap_or_default();
    let sql = format!(
        "INSERT INTO download_jobs (video_id, track, status, progress, created_at) \
         VALUES (?, ?, 'queued', 0, ?) \
         ON CONFLICT (video_id) DO UPDATE SET \
            track = excluded.track, status = 'queued', progress = 0, track_id = NULL, \
            error = NULL, hidden = 0, created_at = excluded.created_at, started_at = NULL, \
            finished_at = NULL \
         RETURNING {COLUMNS}"
    );
    let row: JobRow = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(&track.video_id)
        .bind(data)
        .bind(now())
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Einen bestehenden Auftrag wieder in der Liste zeigen (z. B. erneut gewünscht, aber schon da).
pub async fn unhide(pool: &SqlitePool, id: i64) -> AppResult<()> {
    sqlx::query("UPDATE download_jobs SET hidden = 0 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Nächsten wartenden Auftrag übernehmen (atomar: nur ein Arbeiter bekommt ihn).
pub async fn claim_next(pool: &SqlitePool) -> AppResult<Option<DownloadJob>> {
    let sql = format!(
        "UPDATE download_jobs SET status = 'running', progress = 0, error = NULL, started_at = ? \
         WHERE id = (SELECT id FROM download_jobs WHERE status = 'queued' \
                     ORDER BY created_at, id LIMIT 1) \
           AND status = 'queued' \
         RETURNING {COLUMNS}"
    );
    let row: Option<JobRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(now())
        .fetch_optional(pool)
        .await?;
    Ok(row.map(DownloadJob::from))
}

/// Fortschritt eines laufenden Auftrags speichern.
pub async fn set_progress(pool: &SqlitePool, id: i64, progress: f32) -> AppResult<()> {
    sqlx::query("UPDATE download_jobs SET progress = ? WHERE id = ? AND status = 'running'")
        .bind(f64::from(progress))
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Erfolgreich beendet; liefert den Zeitpunkt.
pub async fn finish_done(pool: &SqlitePool, id: i64, track_id: i64) -> AppResult<i64> {
    let at = now();
    sqlx::query(
        "UPDATE download_jobs SET status = 'done', progress = 1, track_id = ?, error = NULL, \
         finished_at = ? WHERE id = ?",
    )
    .bind(track_id)
    .bind(at)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(at)
}

/// Fehlgeschlagen – nur, wenn der Auftrag noch läuft (ein Abbruch bleibt ein Abbruch).
pub async fn finish_failed(pool: &SqlitePool, id: i64, error: &str) -> AppResult<bool> {
    let res = sqlx::query(
        "UPDATE download_jobs SET status = 'failed', error = ?, finished_at = ? \
         WHERE id = ? AND status = 'running'",
    )
    .bind(error)
    .bind(now())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Wartenden oder laufenden Auftrag abbrechen. `true`, wenn er offen war.
pub async fn cancel(pool: &SqlitePool, id: i64) -> AppResult<bool> {
    let res = sqlx::query(
        "UPDATE download_jobs SET status = 'canceled', finished_at = ?, error = NULL \
         WHERE id = ? AND status IN ('queued', 'running')",
    )
    .bind(now())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Läuft der Auftrag noch (nicht abgebrochen)?
pub async fn is_running(pool: &SqlitePool, id: i64) -> AppResult<bool> {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM download_jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(status.as_deref() == Some("running"))
}

/// Fertigen Auftrag aus der Liste nehmen. Erfolgreiche bleiben ausgeblendet erhalten
/// (für `/downloads/map`), alle anderen werden gelöscht.
pub async fn remove_finished(pool: &SqlitePool, id: i64) -> AppResult<bool> {
    let mut tx = pool.begin().await?;
    let hidden =
        sqlx::query("UPDATE download_jobs SET hidden = 1 WHERE id = ? AND status = 'done'")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    let deleted =
        sqlx::query("DELETE FROM download_jobs WHERE id = ? AND status IN ('failed', 'canceled')")
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    tx.commit().await?;
    Ok(hidden + deleted > 0)
}

/// Alle fertigen Aufträge (done, failed, canceled) aus der Liste nehmen.
pub async fn clear_finished(pool: &SqlitePool) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE download_jobs SET hidden = 1 WHERE status = 'done'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM download_jobs WHERE status IN ('failed', 'canceled')")
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Nach einem Neustart: was als "running" stehen geblieben ist, kommt zurück in die Warteschlange.
pub async fn requeue_running(pool: &SqlitePool) -> AppResult<u64> {
    let res = sqlx::query(
        "UPDATE download_jobs SET status = 'queued', progress = 0, started_at = NULL \
         WHERE status = 'running'",
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Erfolgreiche Downloads: (Video-ID, Titel-ID, Pfad des Titels) – nur, wo der Titel noch in
/// der Bibliothek steht.
pub async fn done_with_paths(pool: &SqlitePool) -> AppResult<Vec<(String, i64, String)>> {
    Ok(sqlx::query_as(
        "SELECT d.video_id, t.id, t.path FROM download_jobs d \
         JOIN tracks t ON t.id = d.track_id \
         WHERE d.status = 'done' ORDER BY d.finished_at DESC",
    )
    .fetch_all(pool)
    .await?)
}
