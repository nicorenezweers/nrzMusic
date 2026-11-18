//! Konten (`/api/v1/auth/*`, `/api/v1/users/*`): registrieren, anmelden, abmelden, Passwort
//! und Name ändern, Konto löschen; Admins verwalten alle Konten.

use crate::auth::{
    check_password, clean_display_name, clean_username, hash_password, token_from, token_hash,
    verify_password, Admin, Auth, CurrentUser, MaybeUser,
};
use crate::db::settings::SettingsStore;
use crate::db::users;
use crate::error::{AppError, AppResult};
use nrzmusic_types::{
    Account, AccountUpdate, AuthStatus, LoginRequest, PasswordChange, RegisterRequest, UserUpdate,
};
use rocket::http::{CookieJar, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::serde::json::Json;
use rocket::{delete, get, patch, post, put, routes, Request, Route, State};
use std::net::IpAddr;

pub fn routes() -> Vec<Route> {
    routes![
        required,
        status,
        register,
        login,
        logout,
        password,
        update_account,
        delete_account,
        list_users,
        update_user,
        delete_user
    ]
}

/// Adresse des Aufrufers und sein User-Agent. Hinter einem Proxy zählt der letzte Eintrag in
/// `X-Forwarded-For` (den hat der eigene Proxy gesetzt – frühere kann der Aufrufer fälschen),
/// sonst Rockets `client_ip` (`X-Real-IP` bzw. die Verbindung).
pub struct Client {
    ip: Option<IpAddr>,
    agent: String,
    token: Option<String>,
    forwarded_proto: Option<String>,
    host: Option<String>,
}

impl Client {
    fn https(&self, auth: &Auth) -> bool {
        auth.is_https(self.forwarded_proto.as_deref(), self.host.as_deref())
    }
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Client {
    type Error = ();
    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, ()> {
        let forwarded = req
            .headers()
            .get_one("X-Forwarded-For")
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse().ok());
        Outcome::Success(Client {
            ip: forwarded.or_else(|| req.client_ip()),
            agent: req
                .headers()
                .get_one("User-Agent")
                .unwrap_or_default()
                .to_string(),
            token: token_from(req),
            forwarded_proto: req.headers().get_one("X-Forwarded-Proto").map(str::to_string),
            host: req.headers().get_one("Host").map(str::to_string),
        })
    }
}

/// Ziel des [`crate::auth::Gate`] für Anfragen ohne Sitzung.
#[get("/auth/required")]
fn required() -> AppError {
    AppError::Unauthorized("Bitte anmelden.".into())
}

#[get("/auth/status")]
async fn status(
    me: MaybeUser,
    auth: &State<Auth>,
    settings: &State<SettingsStore>,
) -> AppResult<Json<AuthStatus>> {
    let setup = auth.enabled() && !users::any_with_password(auth.pool()).await?;
    Ok(Json(AuthStatus {
        enabled: auth.enabled(),
        user: me.0,
        registration_open: setup || settings.get().registration_open,
        setup,
    }))
}

/// Antwort auf Anmeldung/Registrierung: das Konto und das Token (für Apps, die kein Cookie
/// nutzen wollen; der Browser nimmt das Cookie).
#[derive(serde::Serialize)]
pub struct Session {
    user: Account,
    token: String,
}

#[post("/auth/register", data = "<body>")]
async fn register(
    body: Json<RegisterRequest>,
    client: Client,
    auth: &State<Auth>,
    settings: &State<SettingsStore>,
    cookies: &CookieJar<'_>,
) -> AppResult<(Status, Json<Session>)> {
    if !auth.enabled() {
        return Err(AppError::bad("Konten sind ausgeschaltet."));
    }
    auth.check_throttle(client.ip)?;
    let pool = auth.pool();
    let first = !users::any_with_password(pool).await?;
    if !first && !settings.get().registration_open {
        return Err(AppError::Forbidden(
            "Neue Konten sind gerade geschlossen – frag den Admin.".into(),
        ));
    }
    let username = clean_username(&body.username)?;
    check_password(&body.password)?;
    let hash = hash_password(&body.password)?;
    // Mehrere Registrierungen auf einmal würden sonst alle zum Admin
    let user = users::create(
        pool,
        &username,
        &clean_display_name(&body.display_name),
        &hash,
        first,
    )
    .await
    .inspect_err(|_| auth.record_fail(client.ip))?;
    if first {
        // Das erste Konto übernimmt alles aus der Zeit ohne Konten
        users::transfer(pool, 0, user.id).await?;
        for old in users::list(pool).await? {
            if old.id != user.id && users::password_hash(pool, old.id).await?.unwrap_or_default().is_empty() {
                users::transfer(pool, old.id, user.id).await?;
                users::delete(pool, old.id).await?;
            }
        }
        tracing::info!(user = %user.username, "Erstes Konto (Admin) angelegt");
    } else {
        tracing::info!(user = %user.username, "Konto angelegt");
    }
    let token = auth
        .start_session(&user, cookies, &client.agent, client.https(auth))
        .await?;
    Ok((Status::Created, Json(Session { user, token })))
}

#[post("/auth/login", data = "<body>")]
async fn login(
    body: Json<LoginRequest>,
    client: Client,
    auth: &State<Auth>,
    cookies: &CookieJar<'_>,
) -> AppResult<Json<Session>> {
    if !auth.enabled() {
        return Err(AppError::bad("Konten sind ausgeschaltet."));
    }
    auth.check_throttle(client.ip)?;
    let found = users::by_name(auth.pool(), body.username.trim()).await?;
    let ok = match &found {
        Some((_, hash)) => verify_password(&body.password, hash),
        None => {
            // Gleich lange rechnen wie mit Konto (kein Hinweis, ob es den Namen gibt)
            let _ = hash_password(&body.password);
            false
        }
    };
    let Some((user, _)) = found.filter(|_| ok) else {
        auth.record_fail(client.ip);
        return Err(AppError::Unauthorized(
            "Benutzername oder Passwort stimmt nicht.".into(),
        ));
    };
    auth.clear_fails(client.ip);
    let token = auth
        .start_session(&user, cookies, &client.agent, client.https(auth))
        .await?;
    Ok(Json(Session { user, token }))
}

#[post("/auth/logout")]
async fn logout(client: Client, auth: &State<Auth>, cookies: &CookieJar<'_>) -> Status {
    auth.end_session(client.token, cookies).await;
    Status::NoContent
}

#[put("/auth/password", data = "<body>")]
async fn password(
    body: Json<PasswordChange>,
    me: CurrentUser,
    client: Client,
    auth: &State<Auth>,
) -> AppResult<Status> {
    if !auth.enabled() {
        return Err(AppError::bad("Konten sind ausgeschaltet."));
    }
    auth.check_throttle(client.ip)?;
    let hash = users::password_hash(auth.pool(), me.id())
        .await?
        .unwrap_or_default();
    if !verify_password(&body.current, &hash) {
        auth.record_fail(client.ip);
        return Err(AppError::bad("Das aktuelle Passwort stimmt nicht."));
    }
    check_password(&body.new)?;
    users::set_password(auth.pool(), me.id(), &hash_password(&body.new)?).await?;
    // Andere Geräte abmelden, dieses bleibt angemeldet
    let keep = client.token.as_deref().map(token_hash);
    users::delete_sessions_except(auth.pool(), me.id(), keep.as_deref()).await?;
    Ok(Status::NoContent)
}

#[patch("/auth/account", data = "<body>")]
async fn update_account(
    body: Json<AccountUpdate>,
    me: CurrentUser,
    auth: &State<Auth>,
) -> AppResult<Json<Account>> {
    if let Some(name) = &body.display_name {
        users::set_display_name(auth.pool(), me.id(), &clean_display_name(name)).await?;
    }
    Ok(Json(
        users::get(auth.pool(), me.id())
            .await?
            .ok_or(AppError::NotFound)?,
    ))
}

#[derive(serde::Deserialize)]
pub struct DeleteAccount {
    password: String,
}

/// Eigenes Konto löschen (mit Passwort). Der letzte Admin kann sich nicht löschen.
#[delete("/auth/account", data = "<body>")]
async fn delete_account(
    body: Json<DeleteAccount>,
    me: CurrentUser,
    client: Client,
    auth: &State<Auth>,
    cookies: &CookieJar<'_>,
) -> AppResult<Status> {
    if !auth.enabled() {
        return Err(AppError::bad("Konten sind ausgeschaltet."));
    }
    auth.check_throttle(client.ip)?;
    let hash = users::password_hash(auth.pool(), me.id())
        .await?
        .unwrap_or_default();
    if !verify_password(&body.password, &hash) {
        auth.record_fail(client.ip);
        return Err(AppError::bad("Das Passwort stimmt nicht."));
    }
    if me.0.is_admin && users::count_admins(auth.pool()).await? <= 1 {
        return Err(AppError::Conflict(
            "Du bist der einzige Admin – erst einem anderen Konto Admin-Rechte geben.".into(),
        ));
    }
    users::delete(auth.pool(), me.id()).await?;
    auth.end_session(client.token, cookies).await;
    tracing::info!(user = %me.0.username, "Konto gelöscht");
    Ok(Status::NoContent)
}

#[get("/users")]
async fn list_users(_admin: Admin, auth: &State<Auth>) -> AppResult<Json<Vec<Account>>> {
    Ok(Json(users::list(auth.pool()).await?))
}

#[patch("/users/<id>", data = "<body>")]
async fn update_user(
    id: i64,
    body: Json<UserUpdate>,
    admin: Admin,
    auth: &State<Auth>,
) -> AppResult<Json<Account>> {
    let pool = auth.pool();
    let user = users::get(pool, id).await?.ok_or(AppError::NotFound)?;
    if let Some(is_admin) = body.is_admin {
        if !is_admin && user.is_admin && users::count_admins(pool).await? <= 1 {
            return Err(AppError::Conflict(
                "Es muss mindestens einen Admin geben.".into(),
            ));
        }
        users::set_admin(pool, id, is_admin).await?;
    }
    if let Some(pw) = &body.password {
        check_password(pw)?;
        users::set_password(pool, id, &hash_password(pw)?).await?;
        users::delete_sessions_except(pool, id, None).await?;
    }
    tracing::info!(admin = %admin.0.username, user = %user.username, "Konto geändert");
    Ok(Json(users::get(pool, id).await?.ok_or(AppError::NotFound)?))
}

#[delete("/users/<id>")]
async fn delete_user(id: i64, admin: Admin, auth: &State<Auth>) -> AppResult<Status> {
    if id == admin.0.id {
        return Err(AppError::bad(
            "Das eigene Konto unter „Konto“ löschen, nicht hier.",
        ));
    }
    if !users::delete(auth.pool(), id).await? {
        return Err(AppError::NotFound);
    }
    tracing::info!(admin = %admin.0.username, id, "Konto gelöscht");
    Ok(Status::NoContent)
}
