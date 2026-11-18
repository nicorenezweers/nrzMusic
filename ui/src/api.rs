//! Zugriff auf die JSON-API unter `/api/v1`. Fehler kommen als `{"error": "…"}`.

use gloo_net::http::{Request, RequestBuilder, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub const BASE: &str = "/api/v1";

#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl ApiError {
    /// YouTube Music ist auf dem Server ausgeschaltet.
    pub fn youtube_off(&self) -> bool {
        self.status == 503
    }

    /// Funktion auf dem Server ausgeschaltet bzw. nicht eingerichtet (Downloads, Spotify).
    pub fn unavailable(&self) -> bool {
        self.status == 503
    }

    /// Nicht (mehr) angemeldet (Spotify).
    pub fn unauthorized(&self) -> bool {
        self.status == 401
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

fn network(e: impl std::fmt::Display) -> ApiError {
    ApiError { status: 0, message: crate::i18n::trf("Keine Verbindung zum Server ({0}).", &[&e.to_string()]) }
}

async fn finish<T: DeserializeOwned>(res: Response) -> ApiResult<T> {
    let status = res.status();
    if status == 204 || status == 202 {
        return serde_json::from_value(Value::Null).map_err(|e| ApiError { status, message: e.to_string() });
    }
    let text = res.text().await.map_err(network)?;
    if (200..300).contains(&status) {
        let value: Value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).map_err(|e| ApiError { status, message: e.to_string() })?
        };
        return serde_json::from_value(value).map_err(|e| ApiError {
            status,
            message: crate::i18n::trf("Unerwartete Antwort: {0}", &[&e.to_string()]),
        });
    }
    let raw = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"].as_str().map(str::to_string));
    // Sitzung abgelaufen (nicht: Spotify nicht verbunden) → Anmeldeseite
    if status == 401 && raw.as_deref() == Some("Bitte anmelden.") {
        crate::account::session_lost();
    }
    let message = serde_json::from_str::<Value>(&text)
        .ok()
        // Meldungen des Servers sind deutsch – gibt es sie im Katalog, erscheinen sie übersetzt
        .and_then(|v| v["error"].as_str().map(crate::i18n::tr))
        .unwrap_or_else(|| match status {
            404 => crate::i18n::tr("Nicht gefunden."),
            502 => crate::i18n::tr("YouTube Music antwortet gerade nicht wie erwartet."),
            503 => crate::i18n::tr("YouTube Music ist ausgeschaltet."),
            _ => crate::i18n::trf("Fehler {0}", &[&status]),
        });
    Err(ApiError { status, message })
}

fn builder(method: &str, path: &str) -> RequestBuilder {
    let url = if path.starts_with('/') && !path.starts_with(BASE) { format!("{BASE}{path}") } else { path.to_string() };
    let b = match method {
        "POST" => Request::post(&url),
        "PUT" => Request::put(&url),
        "PATCH" => Request::patch(&url),
        "DELETE" => Request::delete(&url),
        _ => Request::get(&url),
    };
    b.header("Accept", "application/json")
}

pub async fn get<T: DeserializeOwned>(path: &str) -> ApiResult<T> {
    finish(builder("GET", path).send().await.map_err(network)?).await
}

pub async fn send<T: DeserializeOwned, B: Serialize>(method: &str, path: &str, body: &B) -> ApiResult<T> {
    let req = builder(method, path).json(body).map_err(network)?;
    finish(req.send().await.map_err(network)?).await
}

pub async fn post<T: DeserializeOwned, B: Serialize>(path: &str, body: &B) -> ApiResult<T> {
    send("POST", path, body).await
}

pub async fn put<T: DeserializeOwned, B: Serialize>(path: &str, body: &B) -> ApiResult<T> {
    send("PUT", path, body).await
}

pub async fn patch<T: DeserializeOwned, B: Serialize>(path: &str, body: &B) -> ApiResult<T> {
    send("PATCH", path, body).await
}

/// POST ohne Inhalt (Aktionen wie „neu einlesen“).
pub async fn action(path: &str) -> ApiResult<()> {
    let _: Value = finish(builder("POST", path).send().await.map_err(network)?).await?;
    Ok(())
}

/// POST ohne Inhalt mit Antwort (z. B. Spotify-Anmeldung, yt-dlp aktualisieren).
pub async fn action_json<T: DeserializeOwned>(path: &str) -> ApiResult<T> {
    finish(builder("POST", path).send().await.map_err(network)?).await
}

/// DELETE; manche Routen antworten mit dem geänderten Objekt, das hier nicht gebraucht wird.
pub async fn delete(path: &str) -> ApiResult<()> {
    let _: Value = finish(builder("DELETE", path).send().await.map_err(network)?).await?;
    Ok(())
}

/// DELETE mit Antwort (z. B. Playlist nach dem Entfernen eines Eintrags).
pub async fn delete_json<T: DeserializeOwned>(path: &str) -> ApiResult<T> {
    finish(builder("DELETE", path).send().await.map_err(network)?).await
}

/// URL-Kodierung für Pfad- und Suchteile.
pub fn enc(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}
