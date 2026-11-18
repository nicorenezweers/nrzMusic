//! Scanner, Tags, Fallbacks und die Bibliotheks-API (Titel, Alben, Künstler, Suche, Status).

mod common;

use common::*;
use serde_json::Value;

/// Ein kleiner Musikordner mit getaggten, ungetaggten und zu ignorierenden Dateien.
async fn sample() -> Env {
    let env = env().await;
    put_tagged_mp3(
        &env,
        "Band/Platte/01 - Erster.mp3",
        40,
        Tags {
            title: Some("Der Erste"),
            artist: Some("Die Band"),
            album: Some("Platte"),
            track: Some(1),
            year: Some(2001),
            genre: Some("Rock"),
            picture: Some(png(64, 48)),
            ..Tags::default()
        },
    );
    put_tagged_mp3(
        &env,
        "Band/Platte/02 - Zweiter.mp3",
        30,
        Tags {
            title: Some("Der Zweite"),
            artist: Some("Die Band feat. Gast"),
            album: Some("Platte"),
            album_artist: Some("Die Band"),
            track: Some(2),
            year: Some(2001),
            ..Tags::default()
        },
    );
    // Ohne Tags: Titel aus dem Dateinamen, Album aus dem Ordner, Cover aus folder.jpg
    env.put_file("Lose Sachen/03. Ohne Tags.wav", &wav(300));
    env.put_file("Lose Sachen/Folder.jpg", &jpeg(20, 20));
    // Direkt im Musikordner: kein Album
    env.put_file("einzeln.wav", &wav(200));
    // Wird ignoriert
    env.put_file(".versteckt.mp3", &mp3(5));
    env.put_file(".versteckter Ordner/x.mp3", &mp3(5));
    env.put_file("halb.mp3.part", &mp3(5));
    env.put_file("download.flac.crdownload", &mp3(5));
    env.put_file("notizen.txt", b"hallo");
    env
}

#[rocket::async_test]
async fn scan_reads_tags_and_fallbacks() {
    let env = sample().await;
    let changes = env.scan().await;
    assert_eq!((changes.added, changes.updated, changes.removed), (4, 0, 0));

    let page = env.get("/api/v1/tracks").await;
    assert_eq!(page["total"], 4);
    let paths: Vec<&str> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap())
        .collect();
    assert!(!paths
        .iter()
        .any(|p| p.contains("versteckt") || p.contains("halb") || p.contains("download")));

    let first = track_by_path(&env, "Band/Platte/01 - Erster.mp3").await;
    assert_eq!(first["title"], "Der Erste");
    assert_eq!(first["artist"], "Die Band");
    assert_eq!(first["album"], "Platte");
    assert_eq!(first["album_artist"], "Die Band");
    assert_eq!(first["track_no"], 1);
    assert_eq!(first["year"], 2001);
    assert_eq!(first["genre"], "Rock");
    assert_eq!(first["format"], "mp3");
    assert!(first["duration_ms"].as_u64().unwrap() > 900, "{first}");
    assert!(first["cover"].as_str().unwrap().len() == 16);
    assert_eq!(first["album_id"].as_str().unwrap().len(), 16);

    let second = track_by_path(&env, "Band/Platte/02 - Zweiter.mp3").await;
    assert_eq!(second["album_id"], first["album_id"]);
    assert_eq!(second["artist_id"], first["artist_id"]);
    assert_eq!(second["cover"], Value::Null);

    let untagged = track_by_path(&env, "Lose Sachen/03. Ohne Tags.wav").await;
    assert_eq!(untagged["title"], "Ohne Tags");
    assert_eq!(untagged["track_no"], 3);
    assert_eq!(untagged["album"], "Lose Sachen");
    assert_eq!(untagged["artist"], "");
    assert_eq!(untagged["album_artist"], "");
    assert_eq!(untagged["format"], "wav");
    assert!(
        untagged["cover"].is_string(),
        "folder.jpg als Cover: {untagged}"
    );
    assert!((250..=350).contains(&untagged["duration_ms"].as_u64().unwrap()));

    let loose = track_by_path(&env, "einzeln.wav").await;
    assert_eq!(loose["title"], "einzeln");
    assert_eq!(loose["album"], "");

    // Alben: lose Titel ohne Album zählen nicht
    let albums = env.get("/api/v1/albums").await;
    let albums = albums.as_array().unwrap();
    assert_eq!(albums.len(), 2, "{albums:?}");
    let platte = albums.iter().find(|a| a["title"] == "Platte").unwrap();
    assert_eq!(platte["track_count"], 2);
    assert_eq!(platte["artist"], "Die Band");
    assert_eq!(platte["year"], 2001);
    assert_eq!(platte["cover"], first["cover"]);

    let detail = env
        .get(format!("/api/v1/albums/{}", platte["id"].as_str().unwrap()))
        .await;
    let titles: Vec<&str> = detail["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Der Erste", "Der Zweite"]);

    // Künstler: nach Album-Künstler; der unbekannte (leere) zuletzt
    let artists = env.get("/api/v1/artists").await;
    let names: Vec<&str> = artists
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Die Band", ""]);
    let band = env
        .get(format!(
            "/api/v1/artists/{}",
            first["artist_id"].as_str().unwrap()
        ))
        .await;
    assert_eq!(band["artist"]["album_count"], 1);
    assert_eq!(band["artist"]["track_count"], 2);
    assert_eq!(band["albums"].as_array().unwrap().len(), 1);
    assert_eq!(band["tracks"].as_array().unwrap().len(), 2);

    let status = env.get("/api/v1/status").await;
    assert_eq!(status["tracks"], 4);
    assert_eq!(status["albums"], 2);
    assert_eq!(status["artists"], 2);
    assert_eq!(status["scanning"], false);
    assert_eq!(status["watching"], false);
    assert!(status["last_scan"].as_i64().unwrap() > 0);
    assert!(status["total_size"].as_u64().unwrap() > 0);

    assert_eq!(
        env.call("GET", "/api/v1/tracks/99999", None).await.0.code,
        404
    );
    assert_eq!(
        env.call("GET", "/api/v1/albums/nope", None).await.0.code,
        404
    );
}

#[rocket::async_test]
async fn rescan_keeps_ids_updates_and_removes() {
    let env = sample().await;
    env.scan().await;
    let before = track_by_path(&env, "Band/Platte/02 - Zweiter.mp3").await;
    let untagged = track_by_path(&env, "Lose Sachen/03. Ohne Tags.wav").await;

    // Nichts geändert: nichts neu gelesen, IDs bleiben
    let changes = env.scan().await;
    assert_eq!((changes.added, changes.updated, changes.removed), (0, 0, 0));
    assert_eq!(
        track_by_path(&env, "Band/Platte/02 - Zweiter.mp3").await["id"],
        before["id"]
    );

    // Datei ersetzt (andere Grösse): aktualisiert, gleiche ID
    env.put_file("Lose Sachen/03. Ohne Tags.wav", &wav(900));
    // Datei gelöscht, neue dazu
    std::fs::remove_file(env.music_path("einzeln.wav")).unwrap();
    env.put_file("Neu/Neues Lied.mp3", &mp3(10));
    let changes = env.scan().await;
    assert_eq!((changes.added, changes.updated, changes.removed), (1, 1, 1));
    let after = track_by_path(&env, "Lose Sachen/03. Ohne Tags.wav").await;
    assert_eq!(after["id"], untagged["id"]);
    assert!(after["duration_ms"].as_u64().unwrap() > 800);
    assert_eq!(env.get("/api/v1/tracks").await["total"], 4);

    // Einzelne Datei (wie die Überwachung): umbenennen behält die ID
    let lib = env.library();
    let from = env.music_path("Neu/Neues Lied.mp3");
    let to = env.music_path("Neu/Umbenannt.mp3");
    let id = track_by_path(&env, "Neu/Neues Lied.mp3").await["id"].clone();
    std::fs::rename(&from, &to).unwrap();
    let from_c = lib.root().join("Neu/Neues Lied.mp3");
    let to_c = lib.root().join("Neu/Umbenannt.mp3");
    assert_eq!(lib.moved(&from_c, &to_c).await.unwrap(), 1);
    lib.scan_paths(vec![(from_c, false), (to_c, true)], Default::default())
        .await
        .unwrap();
    let renamed = track_by_path(&env, "Neu/Umbenannt.mp3").await;
    assert_eq!(renamed["id"], id);
    assert_eq!(renamed["title"], "Umbenannt");

    // Ganzer Ordner gelöscht
    std::fs::remove_dir_all(env.music_path("Band")).unwrap();
    let c = lib
        .scan_paths(vec![(lib.root().join("Band"), false)], Default::default())
        .await
        .unwrap();
    assert_eq!(c.removed, 2);
    assert_eq!(env.get("/api/v1/tracks").await["total"], 2);
}

#[rocket::async_test]
async fn missing_music_dir_deletes_nothing() {
    let env = sample().await;
    env.scan().await;
    let music = env.music.path().to_path_buf();
    let moved = env.data.path().join("weg");
    std::fs::rename(&music, &moved).unwrap();
    assert!(env.library().full_scan().await.is_err());
    std::fs::rename(&moved, &music).unwrap();
    assert_eq!(env.get("/api/v1/tracks").await["total"], 4);
}

#[rocket::async_test]
async fn tracks_paging_sorting_and_search() {
    let env = sample().await;
    env.scan().await;

    let page = env.get("/api/v1/tracks?sort=title&limit=2&offset=1").await;
    assert_eq!(page["total"], 4);
    assert_eq!(page["offset"], 1);
    assert_eq!(page["limit"], 2);
    let titles: Vec<&str> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Der Zweite", "einzeln"]);

    let desc = env.get("/api/v1/tracks?sort=duration&order=desc").await;
    let durations: Vec<u64> = desc["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["duration_ms"].as_u64().unwrap())
        .collect();
    assert!(durations.windows(2).all(|w| w[0] >= w[1]), "{durations:?}");

    // Suche: Gross/Klein egal, über Titel, Künstler und Album, jedes Wort muss passen
    assert_eq!(env.get("/api/v1/tracks?q=ERSTE").await["total"], 1);
    assert_eq!(
        env.get("/api/v1/tracks?q=platte%20zweite").await["total"],
        1
    );
    assert_eq!(env.get("/api/v1/tracks?q=band").await["total"], 2);
    assert_eq!(env.get("/api/v1/tracks?q=lose").await["total"], 1);
    assert_eq!(env.get("/api/v1/tracks?q=100%25").await["total"], 0);

    assert_eq!(
        env.call("GET", "/api/v1/tracks?sort=bogus", None)
            .await
            .0
            .code,
        400
    );
    assert_eq!(
        env.call("GET", "/api/v1/tracks?order=up", None)
            .await
            .0
            .code,
        400
    );

    let search = env.get("/api/v1/search?q=band").await;
    assert_eq!(search["tracks"].as_array().unwrap().len(), 2);
    assert_eq!(search["albums"].as_array().unwrap().len(), 1);
    assert_eq!(search["artists"].as_array().unwrap().len(), 1);
    let empty = env.get("/api/v1/search?q=").await;
    assert_eq!(empty["tracks"].as_array().unwrap().len(), 0);

    let albums = env.get("/api/v1/albums?sort=year").await;
    assert_eq!(albums[0]["title"], "Platte");
    let albums = env.get("/api/v1/albums?q=lose").await;
    assert_eq!(albums.as_array().unwrap().len(), 1);

    let home = env.get("/api/v1/home").await;
    assert_eq!(home["recently_added"].as_array().unwrap().len(), 2);
    assert_eq!(home["random_albums"].as_array().unwrap().len(), 2);
    assert_eq!(home["most_played"].as_array().unwrap().len(), 0);
}

#[rocket::async_test]
async fn rescan_route_runs_in_background() {
    let env = sample().await;
    let (status, _) = env.call("POST", "/api/v1/library/rescan", None).await;
    assert_eq!(status.code, 202);
    for _ in 0..200 {
        let s = env.get("/api/v1/status").await;
        if s["tracks"] == 4 && s["scanning"] == false {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("Scan im Hintergrund nicht fertig geworden");
}
