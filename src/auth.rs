//! Konten: Anmeldung mit Benutzername und Passwort (Argon2id), Sitzungen als zufälliges
//! Token im Cookie `nrzmusic_session` (HttpOnly, SameSite=Lax; in der Datenbank nur sein
//! SHA-256) oder als `Authorization: Bearer <token>` (Android-App, Skripte).
//!
//! - [`Gate`] (Fairing) lässt unter `/api/v1/` ohne Sitzung nur `health` und `auth/*` durch;
//!   alles andere wird auf `GET /api/v1/auth/required` (401) umgeleitet. Die Oberfläche und
//!   ihre Dateien bleiben offen – sie zeigt dann die Anmeldung.
//! - [`CurrentUser`] / [`Admin`] sind die Request-Guards der Routen.
//! - Mit `auth = false` gibt es genau einen lokalen Nutzer (Admin) ohne Anmeldung – so wie
//!   bis 1.1 hinter dem umbrelOS-Login.

use crate::db::users;
use crate::error::{AppError, AppResult};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use nrzmusic_types::Account;
use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::uri::Origin;
use rocket::http::{Cookie, CookieJar, Method, SameSite, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::{Data, Request};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

pub const COOKIE: &str = "nrzmusic_session";
/// Sitzungen gelten 180 Tage ab der letzten Nutzung.
pub const SESSION_LIFETIME: i64 = 180 * 24 * 60 * 60;
/// Fehlversuche je IP-Adresse in [`FAIL_WINDOW`], danach 429.
const MAX_FAILS: u32 = 10;
const FAIL_WINDOW: Duration = Duration::from_secs(15 * 60);
pub const MIN_PASSWORD: usize = 8;
const LOCAL_USER: &str = "lokal";

#[derive(Clone)]
pub struct Auth {
    inner: Arc<Inner>,
}

struct Inner {
    pool: SqlitePool,
    enabled: bool,
    /// Host der öffentlichen HTTPS-Adresse (dort sind Cookies "Secure")
    public_https_host: Option<String>,
    /// Der lokale Nutzer bei `auth = false`
    owner: RwLock<Option<Account>>,
    fails: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

/// Zufälliges Sitzungs-Token (32 Byte, hex).
fn new_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn hash_password(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Config(format!("Passwort-Hash: {e}")))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    if hash.is_empty() {
        return false;
    }
    PasswordHash::new(hash)
        .map(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
        .unwrap_or(false)
}

/// Benutzername: 2–32 Zeichen, Buchstaben, Ziffern, `.`, `_`, `-`.
pub fn clean_username(name: &str) -> AppResult<String> {
    let name = name.trim();
    let ok = (2..=32).contains(&name.chars().count())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(name.to_string())
    } else {
        Err(AppError::bad(
            "Der Benutzername braucht 2 bis 32 Zeichen: Buchstaben, Ziffern, Punkt, _ oder -.",
        ))
    }
}

pub fn check_password(password: &str) -> AppResult<()> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(AppError::bad(format!(
            "Das Passwort braucht mindestens {MIN_PASSWORD} Zeichen."
        )));
    }
    if password.len() > 1024 {
        return Err(AppError::bad("Das Passwort ist zu lang."));
    }
    Ok(())
}

pub fn clean_display_name(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect()
}

impl Auth {
    pub fn new(pool: SqlitePool, enabled: bool, public_url: &str) -> Self {
        Auth {
            inner: Arc::new(Inner {
                pool,
                enabled,
                public_https_host: public_url
                    .trim()
                    .strip_prefix("https://")
                    .map(|r| r.split(['/', '?', '#']).next().unwrap_or("").to_ascii_lowercase())
                    .filter(|h| !h.is_empty()),
                owner: RwLock::new(None),
                fails: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn enabled(&self) -> bool {
        self.inner.enabled
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    /// Beim Start: ohne Konten den lokalen Nutzer anlegen und ihm alle Daten geben.
    pub async fn init(&self) -> AppResult<()> {
        if self.inner.enabled {
            return Ok(());
        }
        let pool = self.pool();
        let owner = match users::by_name(pool, LOCAL_USER).await? {
            Some((a, _)) => a,
            None => users::create(pool, LOCAL_USER, "", "", true).await?,
        };
        users::transfer(pool, 0, owner.id).await?;
        *self.inner.owner.write().unwrap() = Some(owner);
        Ok(())
    }

    /// Konto zur Anfrage (Cookie oder Bearer-Token), bzw. der lokale Nutzer.
    async fn resolve(&self, req: &Request<'_>) -> Option<Account> {
        if !self.inner.enabled {
            return self.inner.owner.read().unwrap().clone();
        }
        let token = token_from(req)?;
        match users::session_user(self.pool(), &token_hash(&token), SESSION_LIFETIME).await {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("Sitzung prüfen: {e}");
                None
            }
        }
    }

    /// Kam die Anfrage über HTTPS? (`X-Forwarded-Proto` des Proxys oder der Host der
    /// öffentlichen HTTPS-Adresse.) Nur dann wird das Cookie "Secure" – über
    /// `http://umbrel.local:8350` im Heimnetz würde der Browser es sonst verwerfen.
    pub fn is_https(&self, forwarded_proto: Option<&str>, host: Option<&str>) -> bool {
        if forwarded_proto.is_some_and(|p| p.trim().eq_ignore_ascii_case("https")) {
            return true;
        }
        match (&self.inner.public_https_host, host) {
            (Some(public), Some(h)) => h.trim().eq_ignore_ascii_case(public),
            _ => false,
        }
    }

    /// Neue Sitzung anlegen und als Cookie setzen. Liefert das Token (für die App).
    pub async fn start_session(
        &self,
        user: &Account,
        cookies: &CookieJar<'_>,
        user_agent: &str,
        https: bool,
    ) -> AppResult<String> {
        let token = new_token();
        users::create_session(
            self.pool(),
            &token_hash(&token),
            user.id,
            SESSION_LIFETIME,
            user_agent,
        )
        .await?;
        users::touch_login(self.pool(), user.id).await?;
        let cookie = Cookie::build((COOKIE, token.clone()))
            .path("/")
            .http_only(true)
            .same_site(SameSite::Lax)
            .secure(https)
            .max_age(rocket::time::Duration::seconds(SESSION_LIFETIME));
        cookies.add(cookie);
        Ok(token)
    }

    pub async fn end_session(&self, req_token: Option<String>, cookies: &CookieJar<'_>) {
        if let Some(t) = req_token {
            let _ = users::delete_session(self.pool(), &token_hash(&t)).await;
        }
        cookies.remove(Cookie::build(COOKIE).path("/"));
    }

    /// Zu viele Fehlversuche von dieser Adresse?
    pub fn check_throttle(&self, ip: Option<IpAddr>) -> AppResult<()> {
        let ip = throttle_key(ip);
        let mut map = self.inner.fails.lock().unwrap();
        map.retain(|_, (_, at)| at.elapsed() < FAIL_WINDOW);
        match map.get(&ip) {
            Some((n, _)) if *n >= MAX_FAILS => Err(AppError::TooMany(
                "Zu viele Fehlversuche – bitte in ein paar Minuten noch einmal.".into(),
            )),
            _ => Ok(()),
        }
    }

    pub fn record_fail(&self, ip: Option<IpAddr>) {
        let ip = throttle_key(ip);
        let mut map = self.inner.fails.lock().unwrap();
        let e = map.entry(ip).or_insert((0, Instant::now()));
        e.0 += 1;
    }

    pub fn clear_fails(&self, ip: Option<IpAddr>) {
        self.inner.fails.lock().unwrap().remove(&throttle_key(ip));
    }
}

/// Unbekannte Adresse: alle zusammen zählen (lieber zu streng als gar nicht).
fn throttle_key(ip: Option<IpAddr>) -> IpAddr {
    ip.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
}

/// Token aus `Authorization: Bearer …` oder dem Cookie.
pub fn token_from(req: &Request<'_>) -> Option<String> {
    if let Some(h) = req.headers().get_one("Authorization") {
        if let Some(t) = h.strip_prefix("Bearer ").map(str::trim) {
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    req.cookies()
        .get(COOKIE)
        .map(|c| c.value().to_string())
        .filter(|v| !v.is_empty())
}

/// Ergebnis der Prüfung, je Anfrage einmal (`local_cache`).
struct Resolved(Option<Account>);

async fn resolved<'a>(req: &'a Request<'_>) -> &'a Option<Account> {
    &req.local_cache_async(async {
        match req.rocket().state::<Auth>() {
            Some(auth) => Resolved(auth.resolve(req).await),
            None => Resolved(None),
        }
    })
    .await
    .0
}

/// Angemeldetes Konto.
#[derive(Clone, Debug)]
pub struct CurrentUser(pub Account);

impl CurrentUser {
    pub fn id(&self) -> i64 {
        self.0.id
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for CurrentUser {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        match resolved(req).await {
            Some(a) => Outcome::Success(CurrentUser(a.clone())),
            None => Outcome::Error((Status::Unauthorized, ())),
        }
    }
}

/// Angemeldetes Konto mit Admin-Rechten (sonst 403).
#[derive(Clone, Debug)]
pub struct Admin(pub Account);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Admin {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        match resolved(req).await {
            Some(a) if a.is_admin => Outcome::Success(Admin(a.clone())),
            Some(_) => Outcome::Error((Status::Forbidden, ())),
            None => Outcome::Error((Status::Unauthorized, ())),
        }
    }
}

/// Konto, falls angemeldet (für `GET /auth/status`).
pub struct MaybeUser(pub Option<Account>);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for MaybeUser {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        Outcome::Success(MaybeUser(resolved(req).await.clone()))
    }
}

/// Wer sich nicht angemeldet hat, kommt an der API nur an diesen Pfaden vorbei. Der
/// Spotify-Rückruf (GET) gehört dazu: Er ordnet die Anmeldung über ihren `state` zu;
/// ebenso geteilte Links (`/public/…`, nur mit gültigem Token) und Cover.
fn is_public(method: Method, path: &str) -> bool {
    path == "/api/v1/health"
        || path.starts_with("/api/v1/auth/")
        // Geteilte Links und ihre Cover (Cover-IDs sind Inhalts-Hashes, nicht zu erraten)
        || (method == Method::Get
            && (path.starts_with("/api/v1/public/") || path.starts_with("/api/v1/covers/")))
        || (method == Method::Get && path == "/api/v1/spotify/callback")
}

pub struct Gate;

#[rocket::async_trait]
impl Fairing for Gate {
    fn info(&self) -> Info {
        Info {
            name: "Anmeldung",
            kind: Kind::Request,
        }
    }

    async fn on_request(&self, req: &mut Request<'_>, _: &mut Data<'_>) {
        let path = req.uri().path().as_str().to_string();
        if !path.starts_with("/api/") || is_public(req.method(), &path) {
            return;
        }
        if resolved(req).await.is_none() {
            req.set_method(Method::Get);
            req.set_uri(Origin::parse("/api/v1/auth/required").expect("feste Adresse"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_and_names() {
        let h = hash_password("geheim-12345").unwrap();
        assert!(verify_password("geheim-12345", &h));
        assert!(!verify_password("falsch", &h));
        assert!(!verify_password("x", ""));
        assert!(clean_username("nico.r_z-1").is_ok());
        assert!(clean_username("a").is_err());
        assert!(clean_username("mit leer").is_err());
        assert!(check_password("1234567").is_err());
        assert_eq!(token_hash("a").len(), 64);
    }

    #[tokio::test]
    async fn https_detection() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let a = Auth::new(pool.clone(), true, "https://music.zweers.ch/");
        assert!(a.is_https(None, Some("music.zweers.ch")));
        assert!(a.is_https(Some("https"), Some("umbrel.local:8350")));
        assert!(!a.is_https(None, Some("umbrel.local:8350")));
        let b = Auth::new(pool, true, "");
        assert!(!b.is_https(None, Some("music.zweers.ch")));
    }
}
