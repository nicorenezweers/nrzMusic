//! Konten: Registrieren, Anmelden, Abmelden, Rechte und getrennte Daten je Konto.

mod common;

use common::{call, env_with, figment};
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use serde_json::json;

async fn client_for(env: &common::Env) -> Client {
    let rocket = nrzmusic::build_with(
        figment(env.data.path(), env.music.path()).merge(("auth", true)),
    )
    .await
    .unwrap();
    Client::tracked(rocket).await.unwrap()
}

#[rocket::async_test]
async fn register_login_and_separate_data() {
    // Daten aus der Zeit ohne Konten (user_id 0) übernimmt das erste Konto
    let env = env_with(|f| f.merge(("auth", true))).await;
    let c = &env.client;

    // Ohne Anmeldung: 401 auf der API, Status und Oberfläche bleiben offen
    let (s, _) = call(c, "GET", "/api/v1/playlists", None).await;
    assert_eq!(s, Status::Unauthorized);
    let (s, _) = call(c, "POST", "/api/v1/playlists", Some(json!({"name": "x"}))).await;
    assert_eq!(s, Status::Unauthorized);
    let (s, st) = call(c, "GET", "/api/v1/auth/status", None).await;
    assert_eq!(s, Status::Ok);
    assert_eq!(st["enabled"], true);
    assert_eq!(st["setup"], true);
    assert!(st["user"].is_null());
    let (s, _) = call(c, "GET", "/api/v1/health", None).await;
    assert_eq!(s, Status::Ok);

    // Ungültige Eingaben
    let (s, _) = call(c, "POST", "/api/v1/auth/register", Some(json!({"username": "a", "password": "langespasswort"}))).await;
    assert_eq!(s, Status::BadRequest);
    let (s, _) = call(c, "POST", "/api/v1/auth/register", Some(json!({"username": "nico", "password": "kurz"}))).await;
    assert_eq!(s, Status::BadRequest);

    // Erstes Konto wird Admin
    let (s, r) = call(c, "POST", "/api/v1/auth/register", Some(json!({"username": "nico", "password": "geheim-12345", "display_name": "Nico"}))).await;
    assert_eq!(s, Status::Created, "{r}");
    assert_eq!(r["user"]["is_admin"], true);
    assert!(r["token"].as_str().unwrap().len() >= 64);
    let (_, st) = call(c, "GET", "/api/v1/auth/status", None).await;
    assert_eq!(st["user"]["username"], "nico");
    assert_eq!(st["setup"], false);

    let (s, _) = call(c, "POST", "/api/v1/playlists", Some(json!({"name": "Nicos Liste"}))).await;
    assert_eq!(s, Status::Created);

    // Zweites Konto (eigener Client = eigenes Cookie): kein Admin, sieht Nicos Liste nicht
    let c2 = client_for(&env).await;
    let (s, r) = call(&c2, "POST", "/api/v1/auth/register", Some(json!({"username": "lea", "password": "geheim-67890"}))).await;
    assert_eq!(s, Status::Created, "{r}");
    assert_eq!(r["user"]["is_admin"], false);
    let (_, lists) = call(&c2, "GET", "/api/v1/playlists", None).await;
    assert_eq!(lists.as_array().unwrap().len(), 0);
    let (_, mine) = call(c, "GET", "/api/v1/playlists", None).await;
    let id = mine[0]["id"].as_i64().unwrap();
    let (s, _) = call(&c2, "GET", format!("/api/v1/playlists/{id}"), None).await;
    assert_eq!(s, Status::NotFound);
    let (s, _) = call(&c2, "DELETE", format!("/api/v1/playlists/{id}"), None).await;
    assert_eq!(s, Status::NotFound);

    // Name schon vergeben (Gross/Klein egal)
    let (s, _) = call(&c2, "POST", "/api/v1/auth/register", Some(json!({"username": "NICO", "password": "geheim-67890"}))).await;
    assert_eq!(s, Status::Conflict);

    // Nur Admins: Einstellungen, Kontenliste
    let (s, _) = call(&c2, "GET", "/api/v1/users", None).await;
    assert_eq!(s, Status::Forbidden);
    let (_, settings) = call(c, "GET", "/api/v1/settings", None).await;
    let (s, _) = call(&c2, "PUT", "/api/v1/settings", Some(settings.clone())).await;
    assert_eq!(s, Status::Forbidden);
    let (s, users) = call(c, "GET", "/api/v1/users", None).await;
    assert_eq!(s, Status::Ok);
    assert_eq!(users.as_array().unwrap().len(), 2);

    // Registrierung schliessen
    let mut closed = settings.clone();
    closed["registration_open"] = json!(false);
    let (s, _) = call(c, "PUT", "/api/v1/settings", Some(closed)).await;
    assert_eq!(s, Status::Ok);
    let c3 = client_for(&env).await;
    let (s, _) = call(&c3, "POST", "/api/v1/auth/register", Some(json!({"username": "max", "password": "geheim-00000"}))).await;
    assert_eq!(s, Status::Forbidden);

    // Abmelden, falsches und richtiges Passwort
    let (s, _) = call(&c2, "POST", "/api/v1/auth/logout", None).await;
    assert_eq!(s, Status::NoContent);
    let (s, _) = call(&c2, "GET", "/api/v1/playlists", None).await;
    assert_eq!(s, Status::Unauthorized);
    let (s, _) = call(&c2, "POST", "/api/v1/auth/login", Some(json!({"username": "lea", "password": "falsch-falsch"}))).await;
    assert_eq!(s, Status::Unauthorized);
    let (s, r) = call(&c2, "POST", "/api/v1/auth/login", Some(json!({"username": "Lea", "password": "geheim-67890"}))).await;
    assert_eq!(s, Status::Ok, "{r}");

    // Bearer-Token statt Cookie (Android-App)
    let token = r["token"].as_str().unwrap().to_string();
    let c4 = client_for(&env).await;
    let res = c4
        .get("/api/v1/auth/status")
        .header(rocket::http::Header::new("Authorization", format!("Bearer {token}")))
        .dispatch()
        .await;
    let v: serde_json::Value = serde_json::from_str(&res.into_string().await.unwrap()).unwrap();
    assert_eq!(v["user"]["username"], "lea");

    // Passwort ändern meldet andere Geräte ab
    let (s, _) = call(&c2, "PUT", "/api/v1/auth/password", Some(json!({"current": "geheim-67890", "new": "neues-passwort"}))).await;
    assert_eq!(s, Status::NoContent);
    let res = c4
        .get("/api/v1/playlists")
        .header(rocket::http::Header::new("Authorization", format!("Bearer {token}")))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok, "das eigene Gerät bleibt angemeldet");

    // Der letzte Admin kann seine Rechte nicht abgeben; Admin löscht Lea
    let me = users[0]["id"].as_i64().unwrap();
    let lea = users[1]["id"].as_i64().unwrap();
    let (s, _) = call(c, "PATCH", format!("/api/v1/users/{me}"), Some(json!({"is_admin": false}))).await;
    assert_eq!(s, Status::Conflict);
    let (s, _) = call(c, "DELETE", format!("/api/v1/users/{lea}"), None).await;
    assert_eq!(s, Status::NoContent);
    let (s, _) = call(&c2, "GET", "/api/v1/playlists", None).await;
    assert_eq!(s, Status::Unauthorized);
}

#[rocket::async_test]
async fn first_account_takes_over_old_data() {
    // Erst ohne Konten (wie 1.1) Favorit und Playlist anlegen …
    let env = common::env().await;
    let (s, _) = env
        .call("POST", "/api/v1/playlists", Some(json!({"name": "Alt"})))
        .await;
    assert_eq!(s, Status::Created);
    // … dann mit Konten: das erste Konto bekommt sie
    let c = client_for(&env).await;
    let (s, _) = call(&c, "POST", "/api/v1/auth/register", Some(json!({"username": "nico", "password": "geheim-12345"}))).await;
    assert_eq!(s, Status::Created);
    let (_, lists) = call(&c, "GET", "/api/v1/playlists", None).await;
    assert_eq!(lists[0]["name"], "Alt");
}

#[rocket::async_test]
async fn login_is_throttled() {
    let env = env_with(|f| f.merge(("auth", true))).await;
    let c = &env.client;
    let (s, _) = call(c, "POST", "/api/v1/auth/register", Some(json!({"username": "nico", "password": "geheim-12345"}))).await;
    assert_eq!(s, Status::Created);
    let mut last = Status::Ok;
    for _ in 0..12 {
        let (s, _) = call(c, "POST", "/api/v1/auth/login", Some(json!({"username": "nico", "password": "falsch-falsch"}))).await;
        last = s;
    }
    assert_eq!(last, Status::TooManyRequests);
}
