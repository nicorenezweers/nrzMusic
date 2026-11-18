//! Streaming mit Range, Download, Cover in Grössen und Upload.

mod common;

use common::*;
use rocket::http::{Header, Status};

async fn one_track() -> (Env, Vec<u8>, i64) {
    let env = env().await;
    let path = put_tagged_mp3(
        &env,
        "Künstler/Album/01 Bär.mp3",
        20,
        Tags {
            title: Some("Bär"),
            artist: Some("Künstler"),
            album: Some("Album"),
            picture: Some(png(400, 200)),
            ..Tags::default()
        },
    );
    let bytes = std::fs::read(path).unwrap();
    env.scan().await;
    let id = track_by_path(&env, "Künstler/Album/01 Bär.mp3").await["id"]
        .as_i64()
        .unwrap();
    (env, bytes, id)
}

#[rocket::async_test]
async fn stream_ranges() {
    let (env, bytes, id) = one_track().await;
    let size = bytes.len();
    let uri = format!("/api/v1/tracks/{id}/stream");

    // Ohne Range: alles
    let res = env.client.get(uri.clone()).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(res.headers().get_one("Accept-Ranges"), Some("bytes"));
    assert_eq!(res.headers().get_one("Content-Type"), Some("audio/mpeg"));
    // Content-Length setzt erst hyper (siehe Smoke-Test mit curl); hier zählt die Grösse des Bodys
    assert_eq!(res.into_bytes().await.unwrap(), bytes);

    // Safaris Probe
    let res = env
        .client
        .get(uri.clone())
        .header(Header::new("Range", "bytes=0-1"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::PartialContent);
    assert_eq!(
        res.headers().get_one("Content-Range"),
        Some(format!("bytes 0-1/{size}").as_str())
    );
    assert_eq!(res.into_bytes().await.unwrap(), &bytes[..2]);

    // Offenes Ende und Mitte
    for (range, start, end) in [
        ("bytes=100-".to_string(), 100, size - 1),
        ("bytes=10-19".to_string(), 10, 19),
        ("bytes=-7".to_string(), size - 7, size - 1),
        (format!("bytes=5-{}", size * 2), 5, size - 1),
    ] {
        let res = env
            .client
            .get(uri.clone())
            .header(Header::new("Range", range.clone()))
            .dispatch()
            .await;
        assert_eq!(res.status(), Status::PartialContent, "{range}");
        assert_eq!(
            res.headers().get_one("Content-Range"),
            Some(format!("bytes {start}-{end}/{size}").as_str())
        );
        assert_eq!(
            res.into_bytes().await.unwrap(),
            &bytes[start..=end],
            "{range}"
        );
    }

    // Unerfüllbar
    for range in [
        format!("bytes={size}-"),
        "bytes=9-3".into(),
        "bytes=x-y".into(),
    ] {
        let res = env
            .client
            .get(uri.clone())
            .header(Header::new("Range", range.clone()))
            .dispatch()
            .await;
        assert_eq!(res.status(), Status::RangeNotSatisfiable, "{range}");
        assert_eq!(
            res.headers().get_one("Content-Range"),
            Some(format!("bytes */{size}").as_str())
        );
    }

    // HEAD: Grösse ohne Body
    let res = env.client.head(uri.clone()).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(res.headers().get_one("Accept-Ranges"), Some("bytes"));

    // Download als Anhang mit UTF-8-Namen
    let res = env
        .client
        .get(format!("/api/v1/tracks/{id}/download"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let cd = res
        .headers()
        .get_one("Content-Disposition")
        .unwrap()
        .to_string();
    assert!(
        cd.starts_with("attachment; filename=\"01 B_r.mp3\""),
        "{cd}"
    );
    assert!(cd.contains("filename*=UTF-8''01%20B%C3%A4r.mp3"), "{cd}");
    assert_eq!(res.into_bytes().await.unwrap(), bytes);

    // Datei inzwischen weg → 404; unbekannter Titel → 404
    std::fs::remove_file(env.music_path("Künstler/Album/01 Bär.mp3")).unwrap();
    assert_eq!(
        env.client.get(uri).dispatch().await.status(),
        Status::NotFound
    );
    assert_eq!(
        env.client
            .get("/api/v1/tracks/4711/stream")
            .dispatch()
            .await
            .status(),
        Status::NotFound
    );
}

#[rocket::async_test]
async fn covers_in_sizes() {
    let (env, _, id) = one_track().await;
    let track = env.get(format!("/api/v1/tracks/{id}")).await;
    let cover = track["cover"].as_str().unwrap().to_string();

    // Original
    let res = env
        .client
        .get(format!("/api/v1/covers/{cover}"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(res.headers().get_one("Content-Type"), Some("image/png"));
    assert!(res
        .headers()
        .get_one("Cache-Control")
        .unwrap()
        .contains("immutable"));
    let original = res.into_bytes().await.unwrap();
    assert_eq!(original, png(400, 200));

    // Verkleinert: JPEG, Seitenverhältnis bleibt, Zwischenspeicher liegt in data/covers
    let res = env
        .client
        .get(format!("/api/v1/covers/{cover}?size=96"))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    assert_eq!(res.headers().get_one("Content-Type"), Some("image/jpeg"));
    assert!(res
        .headers()
        .get_one("Cache-Control")
        .unwrap()
        .contains("max-age=31536000"));
    let img = image::load_from_memory(&res.into_bytes().await.unwrap()).unwrap();
    assert_eq!((img.width(), img.height()), (96, 48));
    assert!(env
        .data
        .path()
        .join(format!("covers/{cover}-96.jpg"))
        .is_file());

    // Grösser als das Original: nicht hochrechnen
    let res = env
        .client
        .get(format!("/api/v1/covers/{cover}?size=600"))
        .dispatch()
        .await;
    let img = image::load_from_memory(&res.into_bytes().await.unwrap()).unwrap();
    assert_eq!((img.width(), img.height()), (400, 200));

    for bad in ["0000000000000000", "../../nrzmusic.db", "zz"] {
        assert_eq!(
            env.client
                .get(format!("/api/v1/covers/{bad}?size=96"))
                .dispatch()
                .await
                .status(),
            Status::NotFound,
            "{bad}"
        );
    }
}

#[rocket::async_test]
async fn upload_sanitizes_and_never_overwrites() {
    let env = env().await;
    env.put_file("Album/01 Song.mp3", &mp3(3));
    env.scan().await;

    let (ct, body) = multipart(&[
        ("Album/01 Song.mp3", mp3(8)),
        ("Neu/Ordner/02 Weiter.wav", wav(100)),
        ("../evil.mp3", mp3(2)),
        ("/etc/evil.mp3", mp3(2)),
        (".hidden.mp3", mp3(2)),
        ("Album/.git/x.mp3", mp3(2)),
        ("readme.txt", b"text".to_vec()),
        ("leer.mp3", Vec::new()),
    ]);
    let res = env
        .client
        .post("/api/v1/upload")
        .header(ct)
        .body(body)
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let result: serde_json::Value = res.into_json().await.unwrap();
    let imported: Vec<&str> = result["imported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        imported,
        ["Album/01 Song (2).mp3", "Neu/Ordner/02 Weiter.wav"]
    );
    let skipped: Vec<&str> = result["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        skipped,
        [
            "../evil.mp3",
            "/etc/evil.mp3",
            ".hidden.mp3",
            "Album/.git/x.mp3",
            "readme.txt",
            "leer.mp3"
        ]
    );
    assert!(result["skipped"][0]["reason"].as_str().unwrap().len() > 3);

    // Original unverändert, nichts ausserhalb des Musikordners, keine Reste
    assert_eq!(
        std::fs::read(env.music_path("Album/01 Song.mp3")).unwrap(),
        mp3(3)
    );
    assert_eq!(
        std::fs::read(env.music_path("Album/01 Song (2).mp3")).unwrap(),
        mp3(8)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(env.music_path("Album/01 Song (2).mp3"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644);
    }
    assert!(!env.music.path().parent().unwrap().join("evil.mp3").exists());
    assert!(!env.music_path("evil.mp3").exists());
    let leftovers: Vec<_> = std::fs::read_dir(env.music_path("Album"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(leftovers.is_empty());

    // Sofort in der Bibliothek
    let t = track_by_path(&env, "Neu/Ordner/02 Weiter.wav").await;
    assert_eq!(t["title"], "Weiter");
    assert_eq!(t["album"], "Ordner");
    assert_eq!(env.get("/api/v1/tracks").await["total"], 3);

    // Gleicher Name noch einmal → (3)
    let (ct, body) = multipart(&[("Album/01 Song.mp3", mp3(4))]);
    let res = env
        .client
        .post("/api/v1/upload")
        .header(ct)
        .body(body)
        .dispatch()
        .await;
    let result: serde_json::Value = res.into_json().await.unwrap();
    assert_eq!(result["imported"][0]["path"], "Album/01 Song (3).mp3");
}
