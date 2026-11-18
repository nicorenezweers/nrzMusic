//! Downloads mit einem Ersatz-yt-dlp: ein kleines Shell-Skript, das Fortschrittszeilen
//! ausgibt und eine gültige MP3 an den verlangten Ort kopiert – oder wie yt-dlp scheitert
//! (Video-IDs mit "FAIL…"), oder lange braucht ("SLOW…", zum Abbrechen).
//! ffmpeg spielt für den Ersatz keine Rolle.
#![cfg(unix)]

mod common;

use common::*;
use lofty::prelude::*;
use nrzmusic_types::ServerEvent;
use rocket::http::Status;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const VERSION: &str = "2026.10.26";

/// Ersatz-yt-dlp schreiben; liefert (Skript, Protokoll der Aufrufe).
fn fake_ytdlp(dir: &Path) -> (PathBuf, PathBuf) {
    let samples = dir.join("samples");
    std::fs::create_dir_all(&samples).unwrap();
    std::fs::write(samples.join("sample.mp3"), mp3(40)).unwrap();
    let log = dir.join("calls.log");
    let script = dir.join("fake-yt-dlp");
    let body = r#"#!/bin/sh
# Ersatz für yt-dlp (Tests von nrzMusic)
SAMPLES="__SAMPLES__"
LOG="__LOG__"
case "$1" in
  --version) echo "__VERSION__"; exit 0 ;;
  -U) echo "Latest version: stable@__VERSION__ from yt-dlp/yt-dlp"; echo "yt-dlp is up to date (stable@__VERSION__ from yt-dlp/yt-dlp)"; exit 0 ;;
esac
echo "$*" >> "$LOG"
fmt=mp3; out=.; url=
while [ $# -gt 0 ]; do
  case "$1" in
    --audio-format) fmt="$2"; shift ;;
    -P) out="$2"; shift ;;
    --) shift; url="$1" ;;
  esac
  shift
done
id="${url##*v=}"
case "$id" in
  FAIL*)
    echo "WARNING: [youtube] $id: some warning" >&2
    echo "ERROR: [youtube] $id: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies." >&2
    exit 1 ;;
esac
echo "[youtube] Extracting URL: $url"
echo "[nrz] 0 1000 NA"
echo "[nrz] 500 NA 1000"
case "$id" in SLOW*) sleep 30 ;; esac
echo "[nrz] 1000 1000 NA"
echo "[nrz-pp] started ExtractAudio"
if [ ! -f "$SAMPLES/sample.$fmt" ]; then
  echo "ERROR: Postprocessing: no sample for $fmt" >&2
  exit 1
fi
mkdir -p "$out"
cp "$SAMPLES/sample.$fmt" "$out/$id.$fmt"
echo "[nrz-pp] finished ExtractAudio"
exit 0
"#
    .replace("__SAMPLES__", samples.to_str().unwrap())
    .replace("__LOG__", log.to_str().unwrap())
    .replace("__VERSION__", VERSION);
    std::fs::write(&script, body).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script, log)
}

struct Setup {
    env: Env,
    _tools: tempfile::TempDir,
    log: PathBuf,
}

async fn setup() -> Setup {
    let tools = tempfile::tempdir().unwrap();
    let (script, log) = fake_ytdlp(tools.path());
    let env = env_with(|f| {
        f.merge(("ytdlp", script.to_str().unwrap()))
            .merge(("ffmpeg", "/nonexistent/ffmpeg"))
            .merge(("download_workers", 2))
    })
    .await;
    Setup {
        env,
        _tools: tools,
        log,
    }
}

/// Bild-Server mit einem 320×180-JPEG (wird zum Quadrat zugeschnitten).
async fn cover_server() -> String {
    let jpg = jpeg(320, 180);
    let handler: Handler = Arc::new(move |req: MockRequest| {
        if req.path.ends_with("cover.jpg") {
            MockResponse::bytes("image/jpeg", jpg.clone())
        } else {
            MockResponse::json(404, json!({}))
        }
    });
    mock_server(handler).await
}

fn yt(id: &str, title: &str, artists: &[&str], album: Option<&str>, thumb: Option<&str>) -> Value {
    json!({
        "video_id": id,
        "title": title,
        "artists": artists.iter().map(|a| json!({ "name": a, "id": null })).collect::<Vec<_>>(),
        "album": album.map(|a| json!({ "title": a, "id": "MPREb_x" })),
        "duration_ms": 1000,
        "thumbnail": thumb,
    })
}

async fn job(env: &Env, id: i64) -> Value {
    let list = env.get("/api/v1/downloads").await;
    list.as_array()
        .unwrap()
        .iter()
        .find(|j| j["id"] == id)
        .cloned()
        .unwrap_or(Value::Null)
}

async fn wait_status(env: &Env, id: i64, status: &str) -> Value {
    wait_for(15, || async {
        let j = job(env, id).await;
        (j["status"] == status).then_some(j)
    })
    .await
}

#[rocket::async_test]
async fn download_tags_places_and_scans() {
    let s = setup().await;
    let env = &s.env;
    let covers = cover_server().await;
    let mut rx = env.library().events().subscribe();

    // Werkzeug: das Skript wurde nach data/tools/yt-dlp kopiert
    let (status, tool) = env.call("GET", "/api/v1/downloads/tool", None).await;
    assert_eq!(status, Status::Ok, "{tool}");
    assert_eq!(tool["available"], true, "{tool}");
    assert_eq!(tool["version"], VERSION);
    assert_eq!(tool["ffmpeg"], false);
    assert_eq!(tool["cookies"], false);
    assert!(env.data.path().join("tools/yt-dlp").is_file());

    // Ein Titel mit Album und Cover
    let thumb = format!("{covers}/vi/x/cover.jpg");
    let (status, jobs) = env
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [yt("okAAAAAAAAA", "Song: Eins", &["Anna", "Ben"], Some("Album X"), Some(&thumb))] })),
        )
        .await;
    assert_eq!(status, Status::Ok, "{jobs}");
    let id = jobs[0]["id"].as_i64().unwrap();
    assert_eq!(jobs[0]["status"], "queued");
    assert_eq!(jobs[0]["progress"], 0.0);

    let done = wait_status(env, id, "done").await;
    assert_eq!(done["progress"], 1.0);
    assert!(done["error"].is_null());
    assert!(done["finished_at"].is_i64());
    let track_id = done["track_id"].as_i64().unwrap();

    let track = env.get(format!("/api/v1/tracks/{track_id}")).await;
    assert_eq!(track["path"], "YouTube/Anna/Album X/Song_ Eins.mp3");
    assert_eq!(track["title"], "Song: Eins");
    assert_eq!(track["artist"], "Anna & Ben");
    assert_eq!(track["album"], "Album X");
    assert_eq!(track["album_artist"], "Anna");
    assert!(track["cover"].is_string(), "{track}");

    // Tags in der Datei: Kommentar mit der Adresse, quadratisches Cover
    let file =
        lofty::read_from_path(env.music_path("YouTube/Anna/Album X/Song_ Eins.mp3")).unwrap();
    let tag = file.primary_tag().unwrap();
    assert_eq!(
        tag.comment().as_deref(),
        Some("https://music.youtube.com/watch?v=okAAAAAAAAA")
    );
    let pic = tag.pictures().first().unwrap();
    assert_eq!(pic.pic_type(), lofty::picture::PictureType::CoverFront);
    let img = image::load_from_memory(pic.data()).unwrap();
    assert_eq!((img.width(), img.height()), (180, 180));

    // Ereignisse: angelegt, laufend mit Zwischenstand, fertig
    let mut seen = Vec::new();
    while let Ok((_, ev)) = rx.try_recv() {
        if let ServerEvent::DownloadUpdate(j) = ev {
            seen.push((j.status, j.progress));
        }
    }
    use nrzmusic_types::DownloadStatus::*;
    assert_eq!(seen.first().map(|s| s.0), Some(Queued), "{seen:?}");
    assert!(
        seen.iter()
            .any(|(s, p)| *s == Running && *p > 0.0 && *p < 1.0),
        "{seen:?}"
    );
    assert_eq!(seen.last(), Some(&(Done, 1.0)), "{seen:?}");

    // Aufruf von yt-dlp
    let calls = std::fs::read_to_string(&s.log).unwrap();
    for part in [
        "--no-playlist",
        "--newline",
        "--no-colors",
        "-x --audio-format mp3 --audio-quality 0",
        "--progress-template",
        "-- https://music.youtube.com/watch?v=okAAAAAAAAA",
    ] {
        assert!(calls.contains(part), "{part} fehlt in {calls}");
    }
    assert!(!calls.contains("--cookies"));
    assert!(
        !calls.contains("--ffmpeg-location"),
        "ffmpeg fehlt – keine Angabe"
    );

    // Doppelt angefragt: derselbe Auftrag, kein neuer Download
    let (_, again) = env
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [
                yt("okAAAAAAAAA", "Song: Eins", &["Anna"], None, None),
                yt("okAAAAAAAAA", "Song: Eins", &["Anna"], None, None),
            ] })),
        )
        .await;
    assert_eq!(again.as_array().unwrap().len(), 1, "{again}");
    assert_eq!(again[0]["id"], id);
    assert_eq!(again[0]["status"], "done");
    assert_eq!(std::fs::read_to_string(&s.log).unwrap().lines().count(), 1);

    let map = env.get("/api/v1/downloads/map").await;
    assert_eq!(
        map,
        json!([{ "video_id": "okAAAAAAAAA", "track_id": track_id }])
    );

    // Ohne Album → "Singles"; ein vorhandener Name wird nie überschrieben
    env.put_file("YouTube/Anna/Singles/Zwei.mp3", b"nicht anfassen");
    let (_, jobs) = env
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [yt("okBBBBBBBBB", "Zwei", &["Anna"], None, None)] })),
        )
        .await;
    let id2 = jobs[0]["id"].as_i64().unwrap();
    let done2 = wait_status(env, id2, "done").await;
    let t2 = env
        .get(format!("/api/v1/tracks/{}", done2["track_id"]))
        .await;
    assert_eq!(t2["path"], "YouTube/Anna/Singles/Zwei (2).mp3");
    assert_eq!(
        t2["album"], "Singles",
        "Ordnername als Album (Scanner-Fallback)"
    );
    assert!(t2["cover"].is_null());
    assert_eq!(
        std::fs::read(env.music_path("YouTube/Anna/Singles/Zwei.mp3")).unwrap(),
        b"nicht anfassen"
    );

    // Datei gelöscht → nicht mehr in der Karte, erneuter Wunsch lädt sie wieder
    std::fs::remove_file(env.music_path("YouTube/Anna/Album X/Song_ Eins.mp3")).unwrap();
    let map = env.get("/api/v1/downloads/map").await;
    assert_eq!(map.as_array().unwrap().len(), 1, "{map}");
    assert_eq!(map[0]["video_id"], "okBBBBBBBBB");
    let (_, again) = env
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [yt("okAAAAAAAAA", "Song: Eins", &["Anna", "Ben"], Some("Album X"), None)] })),
        )
        .await;
    assert_eq!(again[0]["id"], id, "derselbe Auftrag, neu eingereiht");
    assert_eq!(again[0]["status"], "queued");
    let redone = wait_status(env, id, "done").await;
    let t3 = env
        .get(format!("/api/v1/tracks/{}", redone["track_id"]))
        .await;
    assert_eq!(t3["path"], "YouTube/Anna/Album X/Song_ Eins.mp3");

    // Fertige aus der Liste nehmen: die Karte bleibt, die Datei auch
    assert_eq!(
        env.call("DELETE", format!("/api/v1/downloads/{id}"), None)
            .await
            .0,
        Status::NoContent
    );
    assert!(job(env, id).await.is_null());
    assert_eq!(
        env.get("/api/v1/downloads/map")
            .await
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(env
        .music_path("YouTube/Anna/Album X/Song_ Eins.mp3")
        .is_file());
    assert_eq!(
        env.call("DELETE", "/api/v1/downloads?finished=true", None)
            .await
            .0,
        Status::NoContent
    );
    assert_eq!(env.get("/api/v1/downloads").await, json!([]));
    assert_eq!(
        env.call("DELETE", "/api/v1/downloads", None).await.0,
        Status::BadRequest
    );
    // Wieder angefragt: schon da – der Auftrag erscheint wieder in der Liste
    let (_, again) = env
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [yt("okAAAAAAAAA", "Song: Eins", &["Anna"], None, None)] })),
        )
        .await;
    assert_eq!(again[0]["status"], "done");
    assert_eq!(
        env.get("/api/v1/downloads").await.as_array().unwrap().len(),
        1
    );

    // Selbst-Aktualisierung
    let (status, info) = env
        .call("POST", "/api/v1/downloads/tool/update", None)
        .await;
    assert_eq!(status, Status::Ok, "{info}");
    assert!(info["message"]
        .as_str()
        .unwrap()
        .contains("yt-dlp is up to date"));
    assert!(info["last_update"].is_i64());
}

#[rocket::async_test]
async fn failures_cancel_retry_and_validation() {
    let s = setup().await;
    let env = &s.env;

    // Ungültige Video-IDs: 400, nichts angelegt
    for bad in ["short", "../../etc/pa", "--exec=rm-x", "okAAAAAAAAAA"] {
        let (status, err) = env
            .call(
                "POST",
                "/api/v1/downloads",
                Some(json!({ "tracks": [yt("okCCCCCCCCC", "Gut", &["A"], None, None), yt(bad, "X", &["A"], None, None)] })),
            )
            .await;
        assert_eq!(status, Status::BadRequest, "{bad}: {err}");
        assert!(err["error"].as_str().unwrap().contains("Video-ID"));
    }
    assert_eq!(env.get("/api/v1/downloads").await, json!([]));

    // Fehlschlag mit lesbarer Meldung
    let (_, jobs) = env
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [yt("FAILxxxxxxx", "Kaputt", &["A"], None, None)] })),
        )
        .await;
    let fail_id = jobs[0]["id"].as_i64().unwrap();
    let failed = wait_status(env, fail_id, "failed").await;
    assert_eq!(failed["error"], "Sign in to confirm you’re not a bot.");
    assert!(failed["track_id"].is_null());

    // Erneut versuchen: derselbe Auftrag, wieder in der Warteschlange
    let (status, retried) = env
        .call("POST", format!("/api/v1/downloads/{fail_id}/retry"), None)
        .await;
    assert_eq!(status, Status::Ok, "{retried}");
    assert_eq!(retried["id"], fail_id);
    assert!(retried["status"] == "queued" || retried["status"] == "running");
    assert!(retried["error"].is_null());
    wait_status(env, fail_id, "failed").await;
    assert_eq!(
        env.call("POST", "/api/v1/downloads/9999/retry", None)
            .await
            .0,
        Status::NotFound
    );

    // Abbrechen eines laufenden Downloads beendet yt-dlp
    let (_, jobs) = env
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [yt("SLOWxxxxxxx", "Langsam", &["A"], None, None)] })),
        )
        .await;
    let slow = jobs[0]["id"].as_i64().unwrap();
    wait_for(15, || async {
        let j = job(env, slow).await;
        (j["status"] == "running" && j["progress"].as_f64().unwrap_or(0.0) > 0.4).then_some(())
    })
    .await;
    let started = std::time::Instant::now();
    assert_eq!(
        env.call("DELETE", format!("/api/v1/downloads/{slow}"), None)
            .await
            .0,
        Status::NoContent
    );
    let canceled = wait_status(env, slow, "canceled").await;
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(canceled["error"].is_null());
    // Bleibt abgebrochen (kein "failed" vom beendeten Prozess), nichts abgelegt
    rocket::tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert_eq!(job(env, slow).await["status"], "canceled");
    assert!(!env.music_path("YouTube").exists());
    // Kein Prozess mehr (die Gruppe samt `sleep` ist beendet)
    let ps = std::process::Command::new("pgrep")
        .args(["-f", "SLOWxxxxxxx"])
        .output();
    if let Ok(ps) = ps {
        assert!(
            String::from_utf8_lossy(&ps.stdout).trim().is_empty(),
            "yt-dlp läuft noch"
        );
    }

    // Abgebrochenes erneut versuchen geht; Wartendes abbrechen auch
    let (_, retried) = env
        .call("POST", format!("/api/v1/downloads/{slow}/retry"), None)
        .await;
    assert_ne!(retried["status"], "canceled");
    env.call("DELETE", format!("/api/v1/downloads/{slow}"), None)
        .await;
    wait_status(env, slow, "canceled").await;
    assert_eq!(
        env.call("DELETE", "/api/v1/downloads/9999", None).await.0,
        Status::NotFound
    );

    // Format aus den Einstellungen: M4A (der Ersatz hat keine M4A – scheitert, aber der Aufruf stimmt)
    let mut settings = env.get("/api/v1/settings").await;
    settings["download_format"] = json!("m4a");
    let (status, saved) = env.call("PUT", "/api/v1/settings", Some(settings)).await;
    assert_eq!(status, Status::Ok, "{saved}");
    let (_, jobs) = env
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [yt("m4aAAAAAAAA", "Format", &["A"], None, None)] })),
        )
        .await;
    let failed = wait_status(env, jobs[0]["id"].as_i64().unwrap(), "failed").await;
    assert_eq!(failed["error"], "Postprocessing: no sample for m4a");
    let calls = std::fs::read_to_string(&s.log).unwrap();
    let last = calls.lines().last().unwrap();
    assert!(
        last.contains("-f bestaudio[ext=m4a]/bestaudio/best"),
        "{last}"
    );
    assert!(last.contains("--audio-format m4a"), "{last}");
    assert!(!last.contains("--audio-quality"), "{last}");
}

#[rocket::async_test]
async fn cookies_are_passed_as_copy() {
    let s = setup().await;
    let env = &s.env;
    let cookies = env.data.path().join("youtube-cookies.txt");
    std::fs::write(&cookies, "# Netscape HTTP Cookie File\n").unwrap();
    assert_eq!(env.get("/api/v1/downloads/tool").await["cookies"], true);
    let (_, jobs) = env
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [yt("cookieAAAAA", "Keks", &["A"], None, None)] })),
        )
        .await;
    wait_status(env, jobs[0]["id"].as_i64().unwrap(), "done").await;
    let calls = std::fs::read_to_string(&s.log).unwrap();
    assert!(calls.contains("--cookies "), "{calls}");
    assert!(
        !calls.contains(cookies.to_str().unwrap()),
        "Kopie, nicht das Original"
    );
}

#[rocket::async_test]
async fn downloads_switched_off_or_missing() {
    // Umgebung schaltet Downloads ab
    let off = env_with(|f| f.merge(("downloads", false))).await;
    assert_eq!(
        off.get("/api/v1/settings").await["downloads_enabled"],
        false
    );
    for (method, uri) in [
        ("GET", "/api/v1/downloads"),
        ("POST", "/api/v1/downloads/1/retry"),
        ("DELETE", "/api/v1/downloads/1"),
        ("POST", "/api/v1/downloads/tool/update"),
    ] {
        let (status, err) = off.call(method, uri, None).await;
        assert_eq!(status, Status::ServiceUnavailable, "{method} {uri}");
        assert!(err["error"].is_string());
    }
    let (status, _) = off
        .call("POST", "/api/v1/downloads", Some(json!({ "tracks": [] })))
        .await;
    assert_eq!(status, Status::ServiceUnavailable);
    assert_eq!(off.get("/api/v1/downloads/map").await, json!([]));
    let tool = off.get("/api/v1/downloads/tool").await;
    assert_eq!(tool["available"], false);
    assert!(tool["message"].as_str().unwrap().contains("ausgeschaltet"));

    // yt-dlp fehlt
    let missing = env_with(|f| f.merge(("ytdlp", "/nonexistent/yt-dlp"))).await;
    let tool = missing.get("/api/v1/downloads/tool").await;
    assert_eq!(tool["available"], false, "{tool}");
    assert!(tool["message"].as_str().unwrap().contains("nicht gefunden"));
    let (status, err) = missing
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [yt("okAAAAAAAAA", "X", &["A"], None, None)] })),
        )
        .await;
    assert_eq!(status, Status::ServiceUnavailable, "{err}");
    assert!(err["error"].as_str().unwrap().contains("yt-dlp"));
    assert_eq!(missing.get("/api/v1/downloads").await, json!([]));

    // In den Einstellungen ausgeschaltet: Aufträge warten, neue gibt es nicht
    let s = setup().await;
    let mut settings = s.env.get("/api/v1/settings").await;
    settings["downloads_enabled"] = json!(false);
    s.env.call("PUT", "/api/v1/settings", Some(settings)).await;
    let (status, _) = s
        .env
        .call(
            "POST",
            "/api/v1/downloads",
            Some(json!({ "tracks": [yt("okAAAAAAAAA", "X", &["A"], None, None)] })),
        )
        .await;
    assert_eq!(status, Status::ServiceUnavailable);
}

#[rocket::async_test]
async fn interrupted_jobs_are_requeued_on_start() {
    let s = setup().await;
    let env = &s.env;
    let pool = env
        .client
        .rocket()
        .state::<sqlx::SqlitePool>()
        .unwrap()
        .clone();
    // Ein Auftrag, der beim letzten Lauf "running" stehen blieb
    let track = yt("restartAAAA", "Neustart", &["A"], None, None);
    sqlx::query(
        "INSERT INTO download_jobs (video_id, track, status, progress, created_at) \
         VALUES ('restartAAAA', ?, 'running', 0.5, 1)",
    )
    .bind(track.to_string())
    .execute(&pool)
    .await
    .unwrap();
    // Neuer Server mit derselben Datenbank
    let tools = tempfile::tempdir().unwrap();
    let (script, _) = fake_ytdlp(tools.path());
    let rocket = nrzmusic::build_with(
        figment(env.data.path(), env.music.path()).merge(("ytdlp", script.to_str().unwrap())),
    )
    .await
    .unwrap();
    let c2 = rocket::local::asynchronous::Client::tracked(rocket)
        .await
        .unwrap();
    let done = wait_for(15, || async {
        let (_, list) = call(&c2, "GET", "/api/v1/downloads", None).await;
        list.as_array()
            .and_then(|l| l.first().cloned())
            .filter(|j| j["status"] == "done")
    })
    .await;
    assert_eq!(done["track"]["video_id"], "restartAAAA");
}
