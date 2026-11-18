//! Fehler, die vor einem Handler entstehen (Route fehlt, JSON kaputt, Upload zu gross).
//! Die API bekommt JSON (`{"error": "…"}`), alles andere eine kleine Seite.

use crate::error::error_response;
use rocket::http::Status;
use rocket::response::{self, Responder};
use rocket::{catch, catchers, Catcher, Request};

pub fn all() -> Vec<Catcher> {
    catchers![default]
}

pub struct Reply(Status, &'static str);

impl<'r> Responder<'r, 'static> for Reply {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        error_response(req, self.0, self.1)
    }
}

fn message(status: Status) -> &'static str {
    match status.code {
        400 => "Die Anfrage ist ungültig.",
        401 => "Bitte anmelden.",
        403 => "Dafür fehlt die Berechtigung.",
        404 => "Nicht gefunden.",
        405 => "Diese Methode gibt es hier nicht.",
        413 => "Die Datei ist zu gross.",
        415 => "Falsches Format der Anfrage.",
        422 => "Die Eingaben sind unvollständig oder ungültig.",
        503 => "Der Dienst ist gerade nicht verfügbar.",
        _ => "Interner Fehler – Details stehen im Server-Protokoll.",
    }
}

#[catch(default)]
fn default(status: Status, _req: &Request<'_>) -> Reply {
    Reply(status, message(status))
}
