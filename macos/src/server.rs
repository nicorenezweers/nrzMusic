//! Lokaler Modus: der nrzMusic-Server im selben Prozess, nur auf 127.0.0.1, ohne Anmeldung
//! (ein Nutzer), Daten in `~/Library/Application Support/nrzMusic`, Musik in `~/Music/nrzMusic`.
//! Läuft auf einem eigenen Thread mit eigener Tokio-Laufzeit.

use crate::settings;
use rocket::figment::providers::{Format, Serialized, Toml};
use rocket::figment::Figment;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Bevorzugter Port – nicht der des Servers (8350), damit beide nebeneinander laufen.
const PREFERRED_PORT: u16 = 18350;

impl std::fmt::Debug for LocalServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalServer").field("port", &self.port).finish()
    }
}

pub struct LocalServer {
    pub port: u16,
    shutdown: Option<rocket::Shutdown>,
}

impl LocalServer {
    pub fn url(&self) -> String {
        local_url(self.port)
    }

    /// Sauber beenden (Datenbank schliessen) und höchstens `wait` darauf warten.
    pub fn stop(&mut self, wait: Duration) {
        if let Some(s) = self.shutdown.take() {
            s.notify();
            let start = Instant::now();
            while start.elapsed() < wait && port_open(self.port) {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// GUI-Apps bekommen nur einen kurzen PATH – yt-dlp, ffmpeg und Deno liegen meist bei Homebrew.
/// Muss vor dem Start weiterer Threads laufen (`set_var`).
pub fn prepare_env() {
    let mut path = std::env::var("PATH").unwrap_or_default();
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"] {
        if !path.split(':').any(|p| p == dir) {
            path = if path.is_empty() { dir.to_string() } else { format!("{dir}:{path}") };
        }
    }
    let deno = settings::home().join(".deno/bin");
    if deno.is_dir() {
        path = format!("{path}:{}", deno.display());
    }
    // SAFETY: Aufruf in `main`, bevor weitere Threads laufen.
    unsafe { std::env::set_var("PATH", path) };
}

fn figment(port: u16) -> Figment {
    let data = settings::data_dir();
    Figment::from(rocket::Config::default())
        .merge(Toml::string(include_str!("../../Rocket.toml")).nested())
        .select(rocket::Config::DEFAULT_PROFILE)
        .merge(Serialized::global("address", "127.0.0.1"))
        .merge(Serialized::global("port", port))
        .merge(Serialized::global("data_dir", data.join("data")))
        .merge(Serialized::global("music_dir", settings::music_dir()))
        // Ein Nutzer auf diesem Mac, nur von hier erreichbar
        .merge(Serialized::global("auth", false))
        .merge(Serialized::global("public_url", ""))
        // Beenden übernimmt die App (Ctrl-C und Signale gehören ihr)
        .merge(Serialized::global(
            "shutdown",
            serde_json::json!({ "ctrlc": false, "signals": [], "grace": 2, "mercy": 2 }),
        ))
}

/// Port für den lokalen Server – schon beim Start der App gewählt, damit das Skript der Seiten
/// weiss, welche Adresse die eigene ist (`pages::window_script`).
pub fn pick_port() -> u16 {
    free_port(PREFERRED_PORT).unwrap_or(PREFERRED_PORT)
}

pub fn local_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Server auf `port` starten und warten, bis er Anfragen annimmt.
pub fn start(port: u16) -> Result<LocalServer, String> {
    let figment = figment(port);
    let (tx, rx) = std::sync::mpsc::channel::<Result<rocket::Shutdown, String>>();
    std::thread::Builder::new()
        .name("nrzmusic-server".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(format!("Tokio: {e}")));
                    return;
                }
            };
            rt.block_on(async move {
                let rocket = match nrzmusic::build_with(figment).await {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let rocket = match rocket.ignite().await {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let _ = tx.send(Ok(rocket.shutdown()));
                if let Err(e) = rocket.launch().await {
                    tracing::error!("Server beendet: {e}");
                }
            });
        })
        .map_err(|e| e.to_string())?;
    let shutdown = rx.recv().map_err(|_| "Server-Thread beendet".to_string())??;
    let start = Instant::now();
    while !port_open(port) {
        if start.elapsed() > Duration::from_secs(30) {
            return Err("Der Server antwortet nicht".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    tracing::info!("Lokaler Server auf 127.0.0.1:{port}");
    Ok(LocalServer { port, shutdown: Some(shutdown) })
}

fn port_open(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(200)).is_ok()
}

fn free_port(preferred: u16) -> Option<u16> {
    (preferred..preferred + 50)
        .chain([0])
        .find_map(|p| TcpListener::bind(("127.0.0.1", p)).ok().and_then(|l| l.local_addr().ok()).map(|a| a.port()))
}

/// Antwortet unter `base` ein nrzMusic-Server? (`GET /api/v1/health` → `{"ok": true}`)
pub fn check_remote(candidates: &[String]) -> Result<String, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .user_agent(concat!("nrzMusic-macOS/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| e.to_string())?;
        let mut last = "Keine Adresse".to_string();
        for base in candidates {
            match client.get(format!("{base}/api/v1/health")).send().await {
                Ok(r) if r.status().is_success() => match r.text().await {
                    Ok(body) if body.contains("\"ok\"") => return Ok(base.clone()),
                    _ => last = format!("{base} ist kein nrzMusic-Server"),
                },
                Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => {
                    // Proxy mit eigener Anmeldung davor: trotzdem verbinden, die Seite fragt selbst
                    return Ok(base.clone());
                }
                Ok(r) => last = format!("{base} antwortet mit {}", r.status()),
                Err(e) => last = format!("{base} nicht erreichbar ({})", short_error(&e)),
            }
        }
        Err(last)
    })
}

fn short_error(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        "Zeitüberschreitung"
    } else if e.is_connect() {
        "keine Verbindung"
    } else {
        "Fehler"
    }
}
