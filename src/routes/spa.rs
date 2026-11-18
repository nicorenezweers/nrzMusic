//! Die Oberfläche: eine Dioxus-App (WebAssembly, Quelltext in `ui/`), beim Übersetzen
//! eingebettet (siehe build.rs). Ihre Dateien liegen unter `/assets/`; jeder andere Pfad,
//! der nicht zur API gehört, bekommt die Seite (`index.html` von dx mit [`HEAD`]) – den Rest
//! erledigt der Router der App.

use rocket::http::{ContentType, Header, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::{self, Responder, Response};
use rocket::{get, routes, Request, Route};
use std::borrow::Cow;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::OnceLock;

pub struct SpaFile {
    pub path: &'static str,
    pub body: &'static [u8],
    pub gzip: Option<&'static [u8]>,
}

include!(concat!(env!("OUT_DIR"), "/spa.rs"));

pub fn routes() -> Vec<Route> {
    routes![index, shared, fallback]
}

/// Ist die Oberfläche eingebettet? (Ohne `cargo xtask ui` vor dem Übersetzen nicht.)
pub fn available() -> bool {
    FILES.iter().any(|f| f.path == "index.html")
}

fn find(path: &str) -> Option<&'static SpaFile> {
    FILES.iter().find(|f| f.path == path)
}

/// Was die Seite schon vor dem Laden der Oberfläche braucht. dx erzeugt `index.html` mit
/// seinem Standard-Kopf (Titel aus ui/Dioxus.toml, Zeichensatz, Skripte); dazu kommen
/// Viewport bis in die Ränder (iPhone, `env(safe-area-inset-*)`), dunkler Hintergrund ohne
/// weisses Aufblitzen sowie Manifest, Icons und Meta-Angaben für den Home-Bildschirm
/// (iPhone/iPad und Android).
const HEAD: &str = concat!(
    r#"<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content">"#,
    r#"<meta name="color-scheme" content="dark">"#,
    r##"<meta name="theme-color" content="#121212">"##,
    r#"<meta name="description" content="Deine Musik – Bibliothek, YouTube Music und Spotify an einem Ort.">"#,
    r#"<link rel="manifest" href="/static/manifest.webmanifest">"#,
    r#"<meta name="mobile-web-app-capable" content="yes">"#,
    r#"<meta name="apple-mobile-web-app-capable" content="yes">"#,
    r#"<meta name="apple-mobile-web-app-title" content="nrzMusic">"#,
    r#"<meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">"#,
    r#"<meta name="format-detection" content="telephone=no">"#,
    r#"<link rel="apple-touch-icon" href="/static/apple-touch-icon.png">"#,
    "<style>html,body{background:#121212;color:#fafafa;margin:0}</style>",
);

/// `index.html` von dx mit [`HEAD`]: Sprache an `<html>` (die App stellt sie danach auf die
/// gewählte um), dx' eigener Viewport fällt weg, der Rest kommt gleich nach dem Zeichensatz
/// (der muss in den ersten 1024 Bytes stehen) bzw. nach `<head>`.
fn with_head(html: &str) -> String {
    let mut html = html.replacen("<html>", r#"<html lang="de">"#, 1);
    if let Some(start) = html.find(r#"<meta name="viewport""#) {
        if let Some(len) = html[start..].find('>') {
            html.replace_range(start..=start + len, "");
        }
    }
    let after = |tag: &str| {
        let start = html.find(tag)?;
        Some(start + html[start..].find('>')? + 1)
    };
    match after("<meta charset=").or_else(|| after("<head>")) {
        Some(i) => html.insert_str(i, HEAD),
        None => html = html.replacen("</head>", &format!("{HEAD}</head>"), 1),
    }
    html
}

/// Die Seite der App (einmal zusammengesetzt), sofern die Oberfläche eingebettet ist.
fn page_html() -> Option<&'static str> {
    static PAGE: OnceLock<Option<String>> = OnceLock::new();
    PAGE.get_or_init(|| {
        let file = find("index.html")?;
        Some(with_head(std::str::from_utf8(file.body).ok()?))
    })
    .as_deref()
}

/// Versteht der Browser gzip?
pub struct AcceptsGzip(bool);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for AcceptsGzip {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let gzip = req
            .headers()
            .get("Accept-Encoding")
            .any(|v| v.split(',').any(|e| e.trim().starts_with("gzip")));
        Outcome::Success(AcceptsGzip(gzip))
    }
}

pub enum Reply {
    /// Die Seite der App (bei geteilten Links mit Vorschau), nie zwischengespeichert
    Html(Cow<'static, str>),
    File(&'static SpaFile, bool),
    Missing,
}

impl<'r> Responder<'r, 'static> for Reply {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let (file, gzip) = match self {
            Reply::File(f, g) => (f, g),
            Reply::Html(html) => {
                let body: Cow<'static, [u8]> = match html {
                    Cow::Borrowed(s) => Cow::Borrowed(s.as_bytes()),
                    Cow::Owned(s) => Cow::Owned(s.into_bytes()),
                };
                return Response::build()
                    .header(ContentType::HTML)
                    .header(Header::new("Cache-Control", "no-cache"))
                    .sized_body(body.len(), Cursor::new(body))
                    .ok();
            }
            Reply::Missing => {
                let body = "<!doctype html><meta charset=utf-8><title>nrzMusic</title>\
                    <p style=\"font-family:sans-serif\">Die Oberfläche ist nicht eingebaut – vor dem \
                    Übersetzen <code>cargo xtask ui</code> ausführen. Die API unter /api/v1 läuft.</p>";
                return Response::build()
                    .status(Status::ServiceUnavailable)
                    .header(ContentType::HTML)
                    .sized_body(body.len(), Cursor::new(body))
                    .ok();
            }
        };
        let ext = file.path.rsplit('.').next().unwrap_or("");
        let content_type = ContentType::from_extension(ext).unwrap_or(ContentType::Binary);
        // Dateien unter assets/ tragen eine Prüfsumme im Namen; alles andere nie zwischenspeichern
        let cache = if file.path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        let mut res = Response::build();
        res.header(content_type)
            .header(Header::new("Cache-Control", cache))
            .header(Header::new("Vary", "Accept-Encoding"));
        match (gzip, file.gzip) {
            (true, Some(gz)) => res
                .header(Header::new("Content-Encoding", "gzip"))
                .sized_body(gz.len(), Cursor::new(gz)),
            _ => res.sized_body(file.body.len(), Cursor::new(file.body)),
        };
        res.ok()
    }
}

fn page() -> Reply {
    page_html().map_or(Reply::Missing, |html| Reply::Html(Cow::Borrowed(html)))
}

#[get("/")]
fn index() -> Reply {
    page()
}

/// Dateien der App oder – für alle Seiten der App – index.html. Niedriger Rang, damit API
/// und `/static` zuerst greifen; unbekannte API-Pfade bleiben 404 (JSON).
#[get("/<path..>", rank = 50)]
fn fallback(path: PathBuf, gzip: AcceptsGzip) -> Option<Reply> {
    let path = path.to_str()?.replace('\\', "/");
    if path.starts_with("api/") || path == "api" {
        return None;
    }
    if path == "index.html" {
        return Some(page());
    }
    if let Some(f) = find(&path) {
        return Some(Reply::File(f, gzip.0));
    }
    // Fehlende Datei (z. B. alte Prüfsumme nach einem Update) nicht als Seite beantworten
    if path.starts_with("assets/") || path.starts_with("static/") {
        return None;
    }
    Some(page())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Host der Anfrage (für absolute Adressen in der Link-Vorschau).
pub struct HostHeader(Option<String>, bool);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for HostHeader {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let host = req.headers().get_one("Host").map(str::to_string);
        let https = req.headers().get_one("X-Forwarded-Proto") == Some("https");
        Outcome::Success(HostHeader(host, https))
    }
}

/// Geteilter Link: dieselbe App, aber mit Titel und Cover für die Vorschau in Messengern
/// (Open Graph). Unbekannte Links bekommen die normale Seite – die App zeigt den Fehler.
#[get("/s/<token>")]
async fn shared(
    token: &str,
    host: HostHeader,
    pool: &rocket::State<sqlx::SqlitePool>,
    config: &rocket::State<crate::config::Config>,
) -> Reply {
    let Some(html) = page_html() else { return Reply::Missing };
    let Ok(view) = crate::db::shares::view(pool, token, false).await else { return page() };
    let base = if !config.public_url.trim().is_empty() {
        config.public_url.trim().trim_end_matches('/').to_string()
    } else {
        match &host.0 {
            Some(h) => format!("{}://{h}", if host.1 { "https" } else { "http" }),
            None => String::new(),
        }
    };
    let title = match view.kind {
        nrzmusic_types::ShareKind::Favorites => format!("Likes von {}", view.owner),
        _ if view.title.is_empty() => "nrzMusic".to_string(),
        _ => view.title.clone(),
    };
    let desc = match view.kind {
        nrzmusic_types::ShareKind::Track => format!("{} · geteilt von {} auf nrzMusic", view.description, view.owner),
        _ => format!("{} Titel · geteilt von {} auf nrzMusic", view.tracks.len(), view.owner),
    };
    let mut meta = format!(
        "<meta property=\"og:type\" content=\"music.playlist\"><meta property=\"og:site_name\" content=\"nrzMusic\">\
         <meta property=\"og:title\" content=\"{t}\"><meta property=\"og:description\" content=\"{d}\">\
         <meta name=\"description\" content=\"{d}\"><meta name=\"twitter:card\" content=\"summary\">\
         <meta property=\"og:url\" content=\"{u}\">",
        t = escape(&title),
        d = escape(&desc),
        u = escape(&format!("{base}/s/{}", view.token)),
    );
    if let Some(img) = &view.artwork {
        let abs = if img.starts_with('/') { format!("{base}{img}") } else { img.clone() };
        meta.push_str(&format!("<meta property=\"og:image\" content=\"{}\">", escape(&abs)));
    }
    let body = html.replacen("</head>", &format!("{meta}</head>"), 1);
    Reply::Html(Cow::Owned(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// index.html, wie dx 0.7 sie für das Release erzeugt (Standardvorlage, ohne eigene Datei).
    const DX_PAGE: &str = r#"<!DOCTYPE html>
<html>
    <head>
        <title>nrzMusic</title>
        <meta content="text/html;charset=utf-8" http-equiv="Content-Type">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <meta charset="UTF-8">
    <link rel="preload" as="style" href="/./assets/tailwind-dxh1234.css" crossorigin></head>
    <body>
        <div id="main"></div>
    <script type="module" async src="/./assets/nrzmusic-ui-dxh5678.js"></script>
            </body>
</html>"#;

    #[test]
    fn head_for_the_webapp() {
        let html = with_head(DX_PAGE);
        assert!(html.contains(r#"<html lang="de">"#));
        assert_eq!(html.matches(r#"<meta name="viewport""#).count(), 1);
        assert!(html.contains("viewport-fit=cover"));
        assert!(html.contains(r#"<link rel="manifest" href="/static/manifest.webmanifest">"#));
        assert!(html.contains(r#"<link rel="apple-touch-icon" href="/static/apple-touch-icon.png">"#));
        // Gleich nach dem Zeichensatz, vor allem, was dx einsetzt
        assert!(html.contains(&format!(r#"<meta charset="UTF-8">{HEAD}"#)));
        assert!(html.find("<meta charset=").unwrap() < 1024);
        // dx' Teile bleiben
        assert!(html.contains("<title>nrzMusic</title>"));
        assert!(html.contains("/./assets/nrzmusic-ui-dxh5678.js"));
        assert!(html.contains("/./assets/tailwind-dxh1234.css"));
    }

    #[test]
    fn head_without_dx_viewport() {
        let html = with_head("<html><head><title>x</title></head><body></body></html>");
        assert_eq!(html.matches(r#"<meta name="viewport""#).count(), 1);
        assert!(html.starts_with(r#"<html lang="de"><head><meta name="viewport""#));
    }
}
