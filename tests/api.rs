//! Playlists, Favoriten, Verlauf (lokal und YouTube gemischt), Einstellungen, YouTube-Schalter,
//! Ereignisse und die Auslieferung der Oberfläche.

mod common;

use common::*;
use rocket::http::{Accept, Status};
use serde_json::{json, Value};

fn yt(id: &str, title: &str) -> Value {
    json!({
        "source": "youtube",
        "video_id": id,
        "title": title,
        "artists": [{ "name": "YT Artist", "id": "UC123" }],
        "album": null,
        "duration_ms": 200000,
        "thumbnail": format!("https://i.ytimg.com/vi/{id}/hq.jpg"),
    })
}

async fn two_local() -> (Env, i64, i64) {
    let env = env().await;
    put_tagged_mp3(
        &env,
        "A/1.mp3",
        10,
        Tags {
            title: Some("Eins"),
            picture: Some(png(10, 10)),
            ..Tags::default()
        },
    );
    put_tagged_mp3(
        &env,
        "A/2.mp3",
        12,
        Tags {
            title: Some("Zwei"),
            ..Tags::default()
        },
    );
    env.scan().await;
    let a = track_by_path(&env, "A/1.mp3").await["id"].as_i64().unwrap();
    let b = track_by_path(&env, "A/2.mp3").await["id"].as_i64().unwrap();
    (env, a, b)
}

fn keys(playlist: &Value) -> Vec<String> {
    playlist["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| match i["source"].as_str().unwrap() {
            "local" => format!("local:{}", i["id"]),
            _ => format!("yt:{}", i["video_id"].as_str().unwrap()),
        })
        .collect()
}

#[rocket::async_test]
async fn playlists_with_mixed_items() {
    let (env, a, b) = two_local().await;
    let (status, created) = env
        .call(
            "POST",
            "/api/v1/playlists",
            Some(json!({
                "name": "  Gemischt ",
                "items": [
                    { "source": "local", "id": a },
                    yt("dQw4w9WgXcQ", "Video"),
                    { "source": "local", "id": 99999 },
                ],
            })),
        )
        .await;
    assert_eq!(status, Status::Created, "{created}");
    assert_eq!(created["name"], "Gemischt");
    assert_eq!(created["item_count"], 2);
    let id = created["id"].as_i64().unwrap();
    let artwork = created["artwork"].as_array().unwrap();
    assert_eq!(artwork.len(), 2);
    assert!(artwork[0].as_str().unwrap().starts_with("/api/v1/covers/"));
    assert!(artwork[0].as_str().unwrap().ends_with("?size=300"));
    assert_eq!(artwork[1], "https://i.ytimg.com/vi/dQw4w9WgXcQ/hq.jpg");

    // Anhängen, Reihenfolge, Entfernen
    let (status, s) = env
        .call(
            "POST",
            &format!("/api/v1/playlists/{id}/items"),
            Some(json!({ "items": [{ "source": "local", "id": b }, yt("abcdefghijk", "Zweites Video")] })),
        )
        .await;
    assert_eq!(status, Status::Ok, "{s}");
    assert_eq!(s["item_count"], 4);
    let p = env.get(format!("/api/v1/playlists/{id}")).await;
    assert_eq!(
        keys(&p),
        [
            format!("local:{a}"),
            "yt:dQw4w9WgXcQ".into(),
            format!("local:{b}"),
            "yt:abcdefghijk".into()
        ]
    );
    assert_eq!(p["items"][1]["title"], "Video");
    assert_eq!(p["items"][1]["artists"][0]["name"], "YT Artist");
    assert!(p["summary"]["duration_ms"].as_u64().unwrap() > 400000);

    let (status, _) = env
        .call(
            "PUT",
            &format!("/api/v1/playlists/{id}/order"),
            Some(json!({ "order": [3, 2, 1, 0] })),
        )
        .await;
    assert_eq!(status, Status::Ok);
    let p = env.get(format!("/api/v1/playlists/{id}")).await;
    assert_eq!(
        keys(&p),
        [
            "yt:abcdefghijk".into(),
            format!("local:{b}"),
            "yt:dQw4w9WgXcQ".into(),
            format!("local:{a}")
        ]
    );
    for bad in [
        json!({ "order": [0, 1, 2] }),
        json!({ "order": [0, 0, 1, 2] }),
        json!({ "order": [0, 1, 2, 9] }),
    ] {
        let (status, err) = env
            .call("PUT", &format!("/api/v1/playlists/{id}/order"), Some(bad))
            .await;
        assert_eq!(status, Status::BadRequest);
        assert!(err["error"].is_string());
    }

    let (status, s) = env
        .call("DELETE", &format!("/api/v1/playlists/{id}/items/0"), None)
        .await;
    assert_eq!(status, Status::Ok);
    assert_eq!(s["item_count"], 3);
    assert_eq!(
        env.call("DELETE", &format!("/api/v1/playlists/{id}/items/3"), None)
            .await
            .0,
        Status::NotFound
    );

    // Lokale Datei gelöscht: Eintrag fällt weg, Positionen bleiben stimmig
    std::fs::remove_file(env.music_path("A/2.mp3")).unwrap();
    env.scan().await;
    let p = env.get(format!("/api/v1/playlists/{id}")).await;
    assert_eq!(
        keys(&p),
        ["yt:dQw4w9WgXcQ".to_string(), format!("local:{a}")]
    );
    assert_eq!(p["summary"]["item_count"], 2);
    let (_, s) = env
        .call("DELETE", &format!("/api/v1/playlists/{id}/items/1"), None)
        .await;
    assert_eq!(s["item_count"], 1);
    let p = env.get(format!("/api/v1/playlists/{id}")).await;
    assert_eq!(keys(&p), ["yt:dQw4w9WgXcQ"]);

    // Umbenennen, Liste, Löschen
    let (status, s) = env
        .call(
            "PATCH",
            &format!("/api/v1/playlists/{id}"),
            Some(json!({ "name": "Neu", "description": "Text" })),
        )
        .await;
    assert_eq!(status, Status::Ok);
    assert_eq!(
        (s["name"].as_str(), s["description"].as_str()),
        (Some("Neu"), Some("Text"))
    );
    let (status, _) = env
        .call(
            "PATCH",
            &format!("/api/v1/playlists/{id}"),
            Some(json!({ "name": " " })),
        )
        .await;
    assert_eq!(status, Status::BadRequest);
    let list = env.get("/api/v1/playlists").await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(
        env.call("DELETE", &format!("/api/v1/playlists/{id}"), None)
            .await
            .0,
        Status::NoContent
    );
    assert_eq!(
        env.call("GET", &format!("/api/v1/playlists/{id}"), None)
            .await
            .0,
        Status::NotFound
    );
    let (status, _) = env
        .call("POST", "/api/v1/playlists", Some(json!({ "name": "" })))
        .await;
    assert_eq!(status, Status::BadRequest);
}

#[rocket::async_test]
async fn favorites_and_history() {
    let (env, a, b) = two_local().await;
    let put = |body: Value| env.call("PUT", "/api/v1/favorites", Some(body));
    assert_eq!(
        put(json!({ "source": "local", "id": a })).await.0,
        Status::NoContent
    );
    assert_eq!(put(yt("dQw4w9WgXcQ", "Video")).await.0, Status::NoContent);
    assert_eq!(
        put(json!({ "source": "local", "id": b })).await.0,
        Status::NoContent
    );
    // Doppelt setzen schadet nicht, Unbekanntes → 404
    assert_eq!(
        put(json!({ "source": "local", "id": a })).await.0,
        Status::NoContent
    );
    assert_eq!(
        put(json!({ "source": "local", "id": 4711 })).await.0,
        Status::NotFound
    );

    let favs = env.get("/api/v1/favorites").await;
    let sources: Vec<&str> = favs
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["source"].as_str().unwrap())
        .collect();
    assert_eq!(sources, ["local", "youtube", "local"]);
    assert_eq!(favs[0]["id"], b);
    let keys = env.get("/api/v1/favorites/keys").await;
    assert_eq!(
        keys,
        json!([format!("local:{b}"), "yt:dQw4w9WgXcQ", format!("local:{a}")])
    );

    assert_eq!(
        env.call("DELETE", "/api/v1/favorites/yt:dQw4w9WgXcQ", None)
            .await
            .0,
        Status::NoContent
    );
    assert_eq!(
        env.get("/api/v1/favorites").await.as_array().unwrap().len(),
        2
    );

    // Verwaister lokaler Favorit verschwindet aus der Antwort
    std::fs::remove_file(env.music_path("A/2.mp3")).unwrap();
    env.scan().await;
    assert_eq!(
        env.get("/api/v1/favorites/keys").await,
        json!([format!("local:{a}")])
    );

    // Verlauf: neueste zuerst, ohne Doppelte; lokale Titel zählen Wiedergaben
    for r in [
        json!({ "source": "local", "id": a }),
        yt("abcdefghijk", "YT"),
        json!({ "source": "local", "id": a }),
    ] {
        assert_eq!(
            env.call("POST", "/api/v1/history", Some(r)).await.0,
            Status::NoContent
        );
    }
    let history = env.get("/api/v1/history?limit=10").await;
    assert_eq!(history.as_array().unwrap().len(), 2);
    assert_eq!(history[0]["source"], "local");
    assert_eq!(history[0]["play_count"], 2);
    assert!(history[0]["last_played"].as_i64().unwrap() > 0);
    assert_eq!(history[1]["video_id"], "abcdefghijk");
    assert_eq!(
        env.get("/api/v1/history?limit=1")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let home = env.get("/api/v1/home").await;
    assert_eq!(home["most_played"][0]["id"], a);
    assert_eq!(home["recently_played"].as_array().unwrap().len(), 2);
    let sorted = env.get("/api/v1/tracks?sort=played").await;
    assert_eq!(sorted["items"][0]["id"], a);
}

/// Die YouTube-Felder der Einstellungen (die übrigen prüft tests/spotify.rs).
fn youtube_part(s: &Value) -> Value {
    json!({
        "youtube_enabled": s["youtube_enabled"],
        "youtube_language": s["youtube_language"],
        "youtube_region": s["youtube_region"],
    })
}

#[rocket::async_test]
async fn settings_and_youtube_switch() {
    let env = env().await;
    let s = env.get("/api/v1/settings").await;
    assert_eq!(
        youtube_part(&s),
        json!({ "youtube_enabled": true, "youtube_language": "de", "youtube_region": "CH" })
    );
    assert_eq!(env.get("/api/v1/status").await["youtube_enabled"], true);

    // Leere Suche braucht YouTube nicht
    assert_eq!(env.get("/api/v1/yt/search?q=").await["songs"], json!([]));
    assert_eq!(
        env.call("GET", "/api/v1/yt/search?q=x&filter=nope", None)
            .await
            .0,
        Status::BadRequest
    );
    let (status, err) = env
        .call("POST", "/api/v1/yt/import", Some(json!({ "url": " " })))
        .await;
    assert_eq!(status, Status::BadRequest, "{err}");

    let (status, s) = env
        .call(
            "PUT",
            "/api/v1/settings",
            Some(json!({ "youtube_enabled": false, "youtube_language": "en", "youtube_region": "us" })),
        )
        .await;
    assert_eq!(status, Status::Ok);
    assert_eq!(
        youtube_part(&s),
        json!({ "youtube_enabled": false, "youtube_language": "en", "youtube_region": "US" })
    );
    assert_eq!(env.get("/api/v1/settings").await, s);
    assert_eq!(env.get("/api/v1/status").await["youtube_enabled"], false);
    for uri in [
        "/api/v1/yt/search?q=x",
        "/api/v1/yt/suggest?q=x",
        "/api/v1/yt/home",
        "/api/v1/yt/radio/abc",
    ] {
        let (status, err) = env.call("GET", uri, None).await;
        assert_eq!(status, Status::ServiceUnavailable, "{uri}");
        assert!(err["error"].is_string());
    }
    let (status, _) = env
        .call(
            "PUT",
            "/api/v1/settings",
            Some(
                json!({ "youtube_enabled": true, "youtube_language": "", "youtube_region": "CH" }),
            ),
        )
        .await;
    assert_eq!(status, Status::BadRequest);

    // Gespeichert: ein neuer Server mit derselben Datenbank kennt die Einstellungen
    let rocket = nrzmusic::build_with(figment(env.data.path(), env.music.path()))
        .await
        .unwrap();
    let c2 = rocket::local::asynchronous::Client::tracked(rocket)
        .await
        .unwrap();
    assert_eq!(
        call(&c2, "GET", "/api/v1/settings", None).await.1["youtube_language"],
        "en"
    );

    // Umgebung schaltet YouTube ab, egal was gespeichert ist
    let off = env_with(|f| f.merge(("youtube", false))).await;
    assert_eq!(off.get("/api/v1/settings").await["youtube_enabled"], false);
    assert_eq!(
        off.call("GET", "/api/v1/yt/home", None).await.0,
        Status::ServiceUnavailable
    );
}

#[rocket::async_test]
async fn events_stream_sends_json() {
    use rocket::tokio::io::AsyncReadExt;
    let env = env().await;
    let mut res = env.client.get("/api/v1/events").dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(res.content_type().unwrap().to_string(), "text/event-stream");
    assert_eq!(res.headers().get_one("X-Accel-Buffering"), Some("no"));
    let lib = env.library().clone();
    rocket::tokio::spawn(async move {
        rocket::tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        lib.events()
            .send(nrzmusic_types::ServerEvent::PlaylistsChanged);
    });
    let mut text = String::new();
    let mut buf = [0u8; 256];
    let read = async {
        while !text.contains("\n\n") || !text.contains("data:") {
            let n = res.read(&mut buf).await.unwrap();
            assert!(n > 0);
            text.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    };
    rocket::tokio::time::timeout(std::time::Duration::from_secs(5), read)
        .await
        .expect("kein Ereignis");
    assert!(
        text.contains("data:{\"type\":\"playlists_changed\"}"),
        "{text}"
    );
}

#[rocket::async_test]
async fn spa_static_and_errors() {
    let env = env().await;

    // Seiten der App (ohne eingebaute Oberfläche: Hinweis-Seite); kein YouTube-IFrame mehr
    for uri in ["/", "/library/albums", "/playlists/3"] {
        let res = env.client.get(uri).dispatch().await;
        assert!(matches!(res.status().code, 200 | 503), "{uri}");
        assert_eq!(res.content_type(), Some(rocket::http::ContentType::HTML));
        let csp = res.headers().get_one("Content-Security-Policy").unwrap();
        assert!(csp.contains("frame-src 'self';"), "nur eigene Frames (Widget-Vorschau)");
        assert!(csp.contains("script-src 'self' 'wasm-unsafe-eval' 'unsafe-eval';"));
        assert!(!csp.contains("www.youtube.com"));
        assert!(csp.contains("https://*.ytimg.com"));
        assert!(csp.contains("frame-ancestors 'self'"));
        assert_eq!(
            res.headers().get_one("Referrer-Policy"),
            Some("same-origin")
        );
    }

    // Unbekannte API-Pfade: JSON-404
    let res = env
        .client
        .get("/api/v1/gibtsnicht")
        .header(Accept::HTML)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::NotFound);
    let body: Value = res.into_json().await.unwrap();
    assert!(body["error"].is_string());
    // Kaputtes JSON → 4xx mit JSON
    let res = env
        .client
        .post("/api/v1/playlists")
        .header(rocket::http::ContentType::JSON)
        .body("{kaputt")
        .dispatch()
        .await;
    assert!(res.status().code >= 400 && res.status().code < 500);
    assert!(res.into_json::<Value>().await.unwrap()["error"].is_string());

    // Statische Dateien
    let res = env
        .client
        .get("/static/fonts/inter-latin.woff2")
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    assert!(res
        .headers()
        .get_one("Cache-Control")
        .unwrap()
        .contains("immutable"));
    assert_eq!(
        env.client
            .get("/static/fonts/fehlt.woff2")
            .dispatch()
            .await
            .status(),
        Status::NotFound
    );

    let (status, health) = env.call("GET", "/api/v1/health", None).await;
    assert_eq!(status, Status::Ok);
    assert_eq!(health, json!({ "ok": true }));
}
