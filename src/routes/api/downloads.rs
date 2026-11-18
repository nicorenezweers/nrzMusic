//! Downloads mit yt-dlp (`/api/v1/downloads*`).
//!
//! 503, solange Downloads ausgeschaltet sind (Umgebung oder Einstellungen); Anlegen, erneut
//! Versuchen und Aktualisieren zusätzlich, solange yt-dlp fehlt. `/downloads/map` und
//! `/downloads/tool` antworten immer – der Player spielt geladene Titel lokal, die Einstellungen
//! zeigen, was fehlt.

use crate::downloads::Downloads;
use crate::error::{AppError, AppResult};
use nrzmusic_types::{DownloadJob, DownloadRequest, DownloadedTrack, YtDlpInfo};
use rocket::http::Status;
use rocket::serde::json::Json;
use rocket::{delete, get, post, routes, Route, State};

pub fn routes() -> Vec<Route> {
    routes![list, create, retry, remove, clear, map, tool, update_tool]
}

/// Offene Aufträge und die fertigen der letzten 7 Tage, neueste zuerst.
#[get("/downloads")]
async fn list(downloads: &State<Downloads>) -> AppResult<Json<Vec<DownloadJob>>> {
    downloads.ensure_enabled()?;
    Ok(Json(downloads.list().await?))
}

/// Titel herunterladen; bestehende Aufträge kommen unverändert zurück.
#[post("/downloads", data = "<body>")]
async fn create(
    body: Json<DownloadRequest>,
    downloads: &State<Downloads>,
) -> AppResult<Json<Vec<DownloadJob>>> {
    downloads.ensure_usable().await?;
    Ok(Json(downloads.enqueue(&body.tracks).await?))
}

#[post("/downloads/<id>/retry")]
async fn retry(id: i64, downloads: &State<Downloads>) -> AppResult<Json<DownloadJob>> {
    downloads.ensure_usable().await?;
    Ok(Json(downloads.retry(id).await?))
}

/// Abbrechen (wartend/laufend) oder aus der Liste nehmen (fertig – die Datei bleibt).
#[delete("/downloads/<id>")]
async fn remove(id: i64, downloads: &State<Downloads>) -> AppResult<Status> {
    downloads.ensure_enabled()?;
    downloads.remove(id).await?;
    Ok(Status::NoContent)
}

/// `?finished=true`: fertige, fehlgeschlagene und abgebrochene Aufträge aus der Liste nehmen.
#[delete("/downloads?<finished>")]
async fn clear(finished: Option<bool>, downloads: &State<Downloads>) -> AppResult<Status> {
    downloads.ensure_enabled()?;
    if finished != Some(true) {
        return Err(AppError::bad(
            "Nur DELETE /downloads?finished=true ist möglich.",
        ));
    }
    downloads.clear_finished().await?;
    Ok(Status::NoContent)
}

#[get("/downloads/map")]
async fn map(downloads: &State<Downloads>) -> AppResult<Json<Vec<DownloadedTrack>>> {
    Ok(Json(downloads.map().await?))
}

#[get("/downloads/tool")]
async fn tool(downloads: &State<Downloads>) -> Json<YtDlpInfo> {
    downloads.wait_ready().await;
    Json(downloads.info())
}

/// `yt-dlp -U` auf der eigenen Kopie in `data/tools/`.
#[post("/downloads/tool/update")]
async fn update_tool(_admin: crate::auth::Admin, downloads: &State<Downloads>) -> AppResult<Json<YtDlpInfo>> {
    downloads.ensure_usable().await?;
    Ok(Json(downloads.update_tool().await))
}
