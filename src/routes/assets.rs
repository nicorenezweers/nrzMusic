//! Statische Dateien aus `static/` (Schrift, Icons), beim Übersetzen eingebettet (build.rs) –
//! nrzMusic bleibt ein einzelnes Programm und braucht kein Internet für seine Oberfläche.

use rocket::http::{ContentType, Header};
use rocket::response::{self, Responder, Response};
use rocket::{get, routes, Request, Route};
use std::io::Cursor;

include!(concat!(env!("OUT_DIR"), "/static.rs"));

pub fn routes() -> Vec<Route> {
    routes![asset, favicon]
}

pub struct Asset {
    path: &'static str,
    body: &'static [u8],
}

fn lookup(path: &str) -> Option<Asset> {
    STATIC_FILES
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(path, body)| Asset { path, body })
}

impl<'r> Responder<'r, 'static> for Asset {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let ext = self.path.rsplit('.').next().unwrap_or("");
        // Die Android-App (scripts/android.sh legt sie als static/nrzMusic.apk ab)
        if ext == "apk" {
            return Response::build()
                .header(ContentType::new("application", "vnd.android.package-archive"))
                .header(Header::new("Content-Disposition", "attachment; filename=\"nrzMusic.apk\""))
                .header(Header::new("Cache-Control", "no-cache"))
                .sized_body(self.body.len(), Cursor::new(self.body))
                .ok();
        }
        let content_type = match ext {
            // Webapp-Manifest (Home-Bildschirm auf iPhone/Android)
            "webmanifest" => ContentType::new("application", "manifest+json"),
            _ => ContentType::from_extension(ext).unwrap_or(ContentType::Binary),
        };
        // Schriften ändern sich nie; Icons dürfen nach einem Update neu geladen werden
        let cache = if self.path.starts_with("fonts/") {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=3600"
        };
        Response::build()
            .header(content_type)
            .header(Header::new("Cache-Control", cache))
            .sized_body(self.body.len(), Cursor::new(self.body))
            .ok()
    }
}

#[get("/static/<path..>")]
fn asset(path: std::path::PathBuf) -> Option<Asset> {
    lookup(&path.to_str()?.replace('\\', "/"))
}

/// Browser fragen ohne Link nach /favicon.ico.
#[get("/favicon.ico")]
fn favicon() -> Option<Asset> {
    lookup("favicon-32.png").or_else(|| lookup("icon.png"))
}
