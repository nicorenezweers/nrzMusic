//! Teilen: Links für Titel, Likes und Playlists; ohne Anmeldung lesbar und abspielbar
//! (nur die geteilten Titel), löschbar.

mod common;

use common::*;
use rocket::http::Status;
use rocket::local::asynchronous::Client;
use serde_json::json;

#[rocket::async_test]
async fn share_links_work_without_login() {
    let env = env_with(|f| f.merge(("auth", true))).await;
    put_tagged_mp3(&env, "A/1.mp3", 10, Tags { title: Some("Eins"), ..Tags::default() });
    put_tagged_mp3(&env, "A/2.mp3", 12, Tags { title: Some("Zwei"), ..Tags::default() });
    env.scan().await;
    let c = &env.client;
    let (s, _) = call(c, "POST", "/api/v1/auth/register", Some(json!({"username": "nico", "password": "geheim-12345", "display_name": "Nico"}))).await;
    assert_eq!(s, Status::Created);
    let a = track_by_path(&env, "A/1.mp3").await["id"].as_i64().unwrap();
    let b = track_by_path(&env, "A/2.mp3").await["id"].as_i64().unwrap();

    // Likes und eine Playlist
    let (s, _) = call(c, "PUT", "/api/v1/favorites", Some(json!({ "source": "local", "id": a }))).await;
    assert_eq!(s, Status::NoContent);
    let (_, pl) = call(c, "POST", "/api/v1/playlists", Some(json!({ "name": "Sommer", "items": [{ "source": "local", "id": b }] }))).await;
    let pid = pl["id"].as_i64().unwrap();

    let (s, fav) = call(c, "POST", "/api/v1/shares", Some(json!({ "kind": "favorites" }))).await;
    assert_eq!(s, Status::Ok, "{fav}");
    let (_, again) = call(c, "POST", "/api/v1/shares", Some(json!({ "kind": "favorites" }))).await;
    assert_eq!(again["token"], fav["token"], "derselbe Link");
    let (_, pls) = call(c, "POST", "/api/v1/shares", Some(json!({ "kind": "playlist", "playlist_id": pid }))).await;
    assert_eq!(pls["title"], "Sommer");
    let (_, tr) = call(c, "POST", "/api/v1/shares", Some(json!({ "kind": "track", "track": { "source": "local", "id": b } }))).await;
    assert!(tr["path"].as_str().unwrap().starts_with("/s/"));
    let (s, _) = call(c, "POST", "/api/v1/shares", Some(json!({ "kind": "playlist", "playlist_id": 9999 }))).await;
    assert_eq!(s, Status::NotFound);
    let (_, mine) = call(c, "GET", "/api/v1/shares", None).await;
    assert_eq!(mine.as_array().unwrap().len(), 3);

    // Ohne Anmeldung
    let anon = Client::tracked(nrzmusic::build_with(figment(env.data.path(), env.music.path()).merge(("auth", true))).await.unwrap())
        .await
        .unwrap();
    let ft = fav["token"].as_str().unwrap();
    let (s, v) = call(&anon, "GET", format!("/api/v1/public/share/{ft}"), None).await;
    assert_eq!(s, Status::Ok, "{v}");
    assert_eq!(v["owner"], "Nico");
    assert_eq!(v["tracks"].as_array().unwrap().len(), 1);
    let (_, v) = call(&anon, "GET", format!("/api/v1/public/share/{}", pls["token"].as_str().unwrap()), None).await;
    assert_eq!(v["title"], "Sommer");
    assert_eq!(v["tracks"][0]["id"], b);

    let res = anon.get(format!("/api/v1/public/share/{ft}/stream/local:{a}")).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    // Nicht geteilter Titel, fremde Bereiche, unbekannter Link
    let res = anon.get(format!("/api/v1/public/share/{ft}/stream/local:{b}")).dispatch().await;
    assert_eq!(res.status(), Status::NotFound);
    let (s, _) = call(&anon, "GET", "/api/v1/shares", None).await;
    assert_eq!(s, Status::Unauthorized);
    let (s, _) = call(&anon, "GET", format!("/api/v1/tracks/{a}/stream"), None).await;
    assert_eq!(s, Status::Unauthorized);
    let (s, _) = call(&anon, "GET", format!("/api/v1/public/share/{}", "0".repeat(32)), None).await;
    assert_eq!(s, Status::NotFound);

    // Seite mit Vorschau – ohne eingebaute Oberfläche (`cargo xtask ui`) nur der Hinweis
    let res = anon.get(format!("/s/{ft}")).dispatch().await;
    if nrzmusic::routes::spa::available() {
        assert_eq!(res.status(), Status::Ok);
        let html = res.into_string().await.unwrap();
        assert!(html.contains("og:title"), "{html}");
        assert!(html.contains(r#"<link rel="manifest" href="/static/manifest.webmanifest">"#), "{html}");
    } else {
        assert_eq!(res.status(), Status::ServiceUnavailable);
    }

    // Löschen → Link ungültig
    let (s, _) = call(c, "DELETE", format!("/api/v1/shares/{ft}"), None).await;
    assert_eq!(s, Status::NoContent);
    let (s, _) = call(&anon, "GET", format!("/api/v1/public/share/{ft}"), None).await;
    assert_eq!(s, Status::NotFound);
}
