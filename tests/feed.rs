//! Hörverhalten, Statistik, Feed (ohne YouTube) und Einstellungen je Konto.

mod common;

use common::*;
use rocket::http::Status;
use serde_json::{json, Value};

fn yt(id: &str, title: &str) -> Value {
    json!({
        "source": "youtube",
        "video_id": id,
        "title": title,
        "artists": [{ "name": "YT Artist", "id": "UC123" }],
        "album": null,
        "duration_ms": 200000,
        "thumbnail": null,
    })
}

#[rocket::async_test]
async fn listens_stats_feed_and_prefs() {
    let env = env_with(|f| f.merge(("youtube", false))).await;
    for (i, name) in ["Eins", "Zwei", "Drei"].iter().enumerate() {
        put_tagged_mp3(&env, &format!("A/{i}.mp3"), 10 + i, Tags { title: Some(name), ..Tags::default() });
    }
    env.scan().await;
    let a = track_by_path(&env, "A/0.mp3").await["id"].as_i64().unwrap();
    let b = track_by_path(&env, "A/1.mp3").await["id"].as_i64().unwrap();

    // Ganz gehört, früh übersprungen, YouTube halb
    for body in [
        json!({ "track": { "source": "local", "id": a }, "listened_ms": 180000, "duration_ms": 180000, "completed": true }),
        json!({ "track": { "source": "local", "id": b }, "listened_ms": 4000, "duration_ms": 180000, "skipped": true }),
        json!({ "track": yt("dQw4w9WgXcQ", "Video"), "listened_ms": 100000, "duration_ms": 200000, "source": "feed" }),
    ] {
        let (status, v) = env.call("POST", "/api/v1/listens", Some(body)).await;
        assert_eq!(status, Status::NoContent, "{v}");
    }
    let (status, _) = env
        .call("POST", "/api/v1/listens", Some(json!({ "track": { "source": "local", "id": 99999 }, "listened_ms": 1000 })))
        .await;
    assert_eq!(status, Status::NotFound);

    let s = env.get("/api/v1/stats?days=7&tz=-60").await;
    assert_eq!(s["total_ms"], 284000);
    assert_eq!(s["plays"], 2);
    assert_eq!(s["skips"], 1);
    assert_eq!(s["tracks"], 3);
    assert_eq!(s["bucket"], "day");
    assert_eq!(s["timeline"].as_array().unwrap().len(), 7);
    assert_eq!(s["by_hour"].as_array().unwrap().len(), 24);
    assert_eq!(s["top_tracks"][0]["track"]["id"], a);
    assert_eq!(s["top_artists"][0]["name"], "YT Artist");
    let all = env.get("/api/v1/stats?days=0").await;
    assert_eq!(all["bucket"], "month");
    assert!(!all["timeline"].as_array().unwrap().is_empty());

    // Feed ohne YouTube: Titel aus der Bibliothek, ohne Ausgeschlossene
    let (status, page) = env
        .call("POST", "/api/v1/feed", Some(json!({ "exclude": [format!("local:{a}")], "count": 10 })))
        .await;
    assert_eq!(status, Status::Ok, "{page}");
    let keys: Vec<String> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| format!("local:{}", i["track"]["id"]))
        .collect();
    assert!(!keys.is_empty());
    assert!(!keys.contains(&format!("local:{a}")));

    // Nicht interessiert → kommt nicht mehr
    let (status, _) = env
        .call("POST", "/api/v1/feed/dislike", Some(json!({ "track": { "source": "local", "id": b } })))
        .await;
    assert_eq!(status, Status::NoContent);
    let (_, page) = env.call("POST", "/api/v1/feed", Some(json!({ "count": 10 }))).await;
    assert!(page["items"].as_array().unwrap().iter().all(|i| i["track"]["id"] != b));

    // Einstellungen je Konto
    assert_eq!(env.get("/api/v1/prefs").await["auto_download_likes"], false);
    let (status, p) = env.call("PUT", "/api/v1/prefs", Some(json!({ "auto_download_likes": true }))).await;
    assert_eq!(status, Status::Ok);
    assert_eq!(p["auto_download_likes"], true);
}
