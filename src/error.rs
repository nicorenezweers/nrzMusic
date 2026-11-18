//! Ein Fehlertyp für die ganze Anwendung. Als Rocket-`Responder` liefert er für die API
//! (`/api/…` oder `Accept: application/json`) JSON im Format `{"error": "…"}`
//! (`nrzmusic_types::ApiError`), sonst eine kleine HTML-Fehlerseite.
//! Interne Fehler (Datenbank, Dateien) werden protokolliert, aber nicht nach aussen gezeigt.

use nrzmusic_types::ApiError;
use rocket::http::{ContentType, Status};
use rocket::request::Request;
use rocket::response::{self, Responder, Response};
use std::io::Cursor;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Konfiguration: {0}")]
    Config(String),
    #[error("Datenbank: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Datei: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    BadRequest(String),
    #[error("Nicht gefunden.")]
    NotFound,
    #[error("YouTube Music ist ausgeschaltet.")]
    YoutubeOff,
    #[error("{0}")]
    Youtube(#[from] nrzmusic_ytmusic::Error),
    /// Funktion ausgeschaltet oder Werkzeug fehlt (Downloads, Spotify ohne Client-ID) – 503
    #[error("{0}")]
    Unavailable(String),
    /// Dauerhaft nicht verfügbar (z. B. YouTube-Video gelöscht, privat, gesperrt) – 410
    #[error("{0}")]
    Gone(String),
    /// Nicht (mehr) bei Spotify angemeldet – 401
    #[error("{0}")]
    Unauthorized(String),
    /// Läuft schon (z. B. ein Spotify-Import) – 409
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Spotify(#[from] nrzmusic_spotify::Error),
    /// Angemeldet, aber nicht berechtigt (z. B. nur für Admins) – 403
    #[error("{0}")]
    Forbidden(String),
    /// Zu viele Fehlversuche – 429
    #[error("{0}")]
    TooMany(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn bad(msg: impl Into<String>) -> Self {
        AppError::BadRequest(msg.into())
    }

    pub fn status(&self) -> Status {
        match self {
            AppError::BadRequest(_) => Status::BadRequest,
            AppError::NotFound | AppError::Database(sqlx::Error::RowNotFound) => Status::NotFound,
            AppError::YoutubeOff => Status::ServiceUnavailable,
            AppError::Youtube(nrzmusic_ytmusic::Error::NotFound) => Status::NotFound,
            AppError::Youtube(_) => Status::BadGateway,
            AppError::Unavailable(_) => Status::ServiceUnavailable,
            AppError::Gone(_) => Status::Gone,
            AppError::Unauthorized(_) => Status::Unauthorized,
            AppError::Conflict(_) => Status::Conflict,
            AppError::Forbidden(_) => Status::Forbidden,
            AppError::TooMany(_) => Status::TooManyRequests,
            AppError::Spotify(e) => match e {
                nrzmusic_spotify::Error::NotConfigured => Status::ServiceUnavailable,
                nrzmusic_spotify::Error::Unauthorized => Status::Unauthorized,
                // Spotifys Antwort samt (deutscher) Meldung durchreichen: ungültige Anfrage,
                // nicht freigegeben (Entwicklungsmodus), nicht gefunden, zu viele Anfragen
                nrzmusic_spotify::Error::Api { status, .. }
                    if matches!(status, 400 | 403 | 404 | 429) =>
                {
                    Status::from_code(*status).unwrap_or(Status::BadGateway)
                }
                _ => Status::BadGateway,
            },
            AppError::Config(_) | AppError::Database(_) | AppError::Io(_) => {
                Status::InternalServerError
            }
        }
    }

    /// Was der Benutzer sehen darf – interne Details bleiben im Protokoll.
    pub fn public_message(&self) -> String {
        match self {
            AppError::Config(_) | AppError::Database(_) | AppError::Io(_)
                if self.status().code >= 500 =>
            {
                "Interner Fehler – Details stehen im Server-Protokoll.".into()
            }
            AppError::Spotify(nrzmusic_spotify::Error::Unauthorized) => {
                "Die Spotify-Anmeldung ist abgelaufen – bitte neu verbinden.".into()
            }
            // Die Meldungen des Spotify-Clients sind schon für den Benutzer geschrieben
            AppError::Spotify(nrzmusic_spotify::Error::Api {
                status: 400 | 403 | 404 | 429,
                message,
            }) => message.clone(),
            _ if self.status().code == 404 => "Nicht gefunden.".into(),
            _ => self.to_string(),
        }
    }
}

/// Soll die Antwort JSON sein? API-Pfade immer, sonst nach `Accept`.
pub fn wants_json(req: &Request<'_>) -> bool {
    req.uri().path().starts_with("/api/") || req.accept().is_some_and(|a| a.preferred().is_json())
}

/// Antwort für einen Status mit Meldung – genutzt vom Responder und von den Catchern.
pub fn error_response(
    req: &Request<'_>,
    status: Status,
    message: &str,
) -> response::Result<'static> {
    let (content_type, body) = if wants_json(req) {
        let body = ApiError {
            error: message.to_string(),
        };
        (
            ContentType::JSON,
            serde_json::to_string(&body).unwrap_or_else(|_| "{}".into()),
        )
    } else {
        let esc = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        };
        let html = format!(
            "<!doctype html><html lang=de><meta charset=utf-8><title>{code} · nrzMusic</title>\
             <body style=\"background:#171717;color:#e5e5e5;font-family:sans-serif;display:grid;place-items:center;min-height:90vh\">\
             <main><h1>{code} – {title}</h1><p>{message}</p><p><a style=\"color:#e5e5e5\" href=\"/\">Zur Startseite</a></p></main>",
            code = status.code,
            title = esc(status.reason_lossy()),
            message = esc(message),
        );
        (ContentType::HTML, html)
    };
    Response::build()
        .status(status)
        .header(content_type)
        .sized_body(body.len(), Cursor::new(body))
        .ok()
}

impl<'r> Responder<'r, 'static> for AppError {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        let status = self.status();
        match status.code {
            500 => tracing::error!(path = %req.uri(), "{self}"),
            502 => tracing::warn!(path = %req.uri(), "{self}"),
            _ => {}
        }
        error_response(req, status, &self.public_message())
    }
}
