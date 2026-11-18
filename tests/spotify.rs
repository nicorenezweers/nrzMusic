//! Spotify mit einem Ersatz-Server (Anmeldung unter `/api/token`, Web-API unter `/v1`,
//! über `Spotify::with_endpoints`) und einer Ersatz-Suche bei YouTube Music.
//! Dazu die Einstellungen samt alter Datenbank-Zeilen.

mod common;

use common::*;
use nrzmusic::spotify::SongSearch;
use nrzmusic_types::{ServerEvent, YtArtistRef, YtTrack};
use rocket::http::Status;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const CLIENT_ID: &str = "testclient";

// ---------------------------------------------------------------- Ersatz-Spotify

#[derive(Default)]
struct MockSpotify {
    /// Das Access-Token, das die API gerade annimmt
    valid_access: Mutex<String>,
    /// Refresh-Token widerrufen
    revoked: AtomicBool,
    /// `GET /me` → 403 (Konto nicht in der App eingetragen)
    me_forbidden: AtomicBool,
    issued: AtomicUsize,
    refreshes: AtomicUsize,
    /// "METHOD /pfad?query" aller Anfragen
    log: Mutex<Vec<String>>,
}

fn sp_track(id: &str, name: &str, artist: &str, isrc: Option<&str>) -> Value {
    json!({
        "id": id,
        "name": name,
        "type": "track",
        "is_local": false,
        "explicit": false,
        "duration_ms": 200000,
        "artists": [{ "id": format!("ar-{artist}"), "name": artist, "type": "artist" }],
        "album": {
            "id": "al1",
            "name": "Album",
            "images": [
                { "url": "https://i.scdn.co/image/a640", "width": 640, "height": 640 },
                { "url": "https://i.scdn.co/image/a300", "width": 300, "height": 300 }
            ]
        },
        "external_ids": match isrc { Some(i) => json!({ "isrc": i }), None => json!({}) },
    })
}

fn page(items: Vec<Value>) -> Value {
    let total = items.len();
    json!({ "href": "x", "items": items, "limit": 50, "next": null, "offset": 0, "previous": null, "total": total })
}

/// Fremde Playlists: eine von Spotify (Titel gesperrt), eine kollaborative (lesbar).
fn foreign_playlist(id: &str, owner: &str, collaborative: bool) -> Value {
    json!({
        "id": id,
        "name": format!("Fremd {id}"),
        "type": "playlist",
        "collaborative": collaborative,
        "owner": { "id": owner, "display_name": owner },
        "images": [],
        "items": { "href": "x", "total": 5 },
    })
}

fn playlist_meta(with_items: bool) -> Value {
    let mut v = json!({
        "id": "pl1",
        "name": "Roadtrip",
        "type": "playlist",
        "collaborative": false,
        "owner": { "id": "nico", "display_name": "Nico" },
        "images": [{ "url": "https://mosaic.scdn.co/640/x", "width": 640, "height": 640 }],
    });
    if with_items {
        v["items"] = page(vec![
            json!({ "added_at": "2026-10-02T10:00:00Z", "item": sp_track("t1", "Eins", "Anna", Some("DEAB12300001")) }),
            json!({ "added_at": "2026-10-03T10:00:00Z", "item": sp_track("t3", "Drei", "Anna", None) }),
        ]);
    } else {
        v["items"] = json!({ "href": "x", "total": 2 });
        v["tracks"] = json!({ "href": "x", "total": 2 });
    }
    v
}

impl MockSpotify {
    fn handle(&self, req: MockRequest) -> MockResponse {
        self.log
            .lock()
            .unwrap()
            .push(format!("{} {}?{}", req.method, req.path, req.query));
        if req.path == "/api/token" {
            return self.token(&req);
        }
        let Some(path) = req.path.strip_prefix("/v1") else {
            return MockResponse::json(
                404,
                json!({ "error": { "status": 404, "message": "nope" } }),
            );
        };
        let auth = req.header("authorization").unwrap_or("").to_string();
        let valid = format!("Bearer {}", self.valid_access.lock().unwrap());
        if auth != valid {
            return MockResponse::json(
                401,
                json!({ "error": { "status": 401, "message": "The access token expired" } }),
            );
        }
        if path == "/me" && self.me_forbidden.load(Ordering::SeqCst) {
            return MockResponse::json(
                403,
                json!({ "error": { "status": 403, "message": "User not registered in the Developer Dashboard" } }),
            );
        }
        match path {
            "/me" => MockResponse::json(
                200,
                json!({
                    "id": "nico",
                    "display_name": "Nico",
                    "images": [{ "url": "https://i.scdn.co/image/me", "width": 300, "height": 300 }],
                    "product": "premium",
                    "type": "user"
                }),
            ),
            "/me/tracks" => MockResponse::json(
                200,
                page(vec![
                    json!({ "added_at": "2026-10-01T10:00:00Z", "track": sp_track("t1", "Eins", "Anna", Some("DEAB12300001")) }),
                    json!({ "added_at": "2026-09-01T10:00:00Z", "track": sp_track("t2", "Zwei", "Ben", None) }),
                ]),
            ),
            "/me/playlists" => MockResponse::json(
                200,
                page(vec![
                    playlist_meta(false),
                    foreign_playlist("pl2", "spotify", false),
                    foreign_playlist("pl3", "freund", true),
                ]),
            ),
            "/playlists/pl1" => MockResponse::json(200, playlist_meta(true)),
            "/playlists/pl2" => MockResponse::json(200, foreign_playlist("pl2", "spotify", false)),
            p if p.starts_with("/playlists/pl2/") => MockResponse::json(
                403,
                json!({ "error": { "status": 403, "message": "Forbidden" } }),
            ),
            p if p.starts_with("/playlists/pl1/") => {
                MockResponse::json(200, playlist_meta(true)["items"].clone())
            }
            _ => MockResponse::json(
                404,
                json!({ "error": { "status": 404, "message": "Not found" } }),
            ),
        }
    }

    fn new_access(&self) -> String {
        let n = self.issued.fetch_add(1, Ordering::SeqCst) + 1;
        let token = format!("acc-{n}");
        *self.valid_access.lock().unwrap() = token.clone();
        token
    }

    fn token(&self, req: &MockRequest) -> MockResponse {
        let invalid = |msg: &str| {
            MockResponse::json(
                400,
                json!({ "error": "invalid_grant", "error_description": msg }),
            )
        };
        if req.param("client_id").as_deref() != Some(CLIENT_ID) {
            return MockResponse::json(400, json!({ "error": "invalid_client" }));
        }
        match req.param("grant_type").as_deref() {
            Some("authorization_code") => {
                if req.param("code").as_deref() != Some("good-code")
                    || req.param("code_verifier").unwrap_or_default().len() < 43
                {
                    return invalid("Invalid authorization code");
                }
                let access = self.new_access();
                MockResponse::json(
                    200,
                    json!({
                        "access_token": access,
                        "token_type": "Bearer",
                        "expires_in": 3600,
                        "refresh_token": "ref-1",
                        "scope": "user-library-read playlist-read-private"
                    }),
                )
            }
            Some("refresh_token") => {
                if self.revoked.load(Ordering::SeqCst)
                    || req.param("refresh_token").as_deref() != Some("ref-1")
                {
                    return invalid("Refresh token revoked");
                }
                self.refreshes.fetch_add(1, Ordering::SeqCst);
                let access = self.new_access();
                MockResponse::json(
                    200,
                    json!({ "access_token": access, "token_type": "Bearer", "expires_in": 3600, "scope": "user-library-read" }),
                )
            }
            _ => MockResponse::json(400, json!({ "error": "unsupported_grant_type" })),
        }
    }
}

// ---------------------------------------------------------------- Ersatz-Suche

#[derive(Default)]
struct StubSearch {
    calls: Mutex<Vec<String>>,
    results: HashMap<String, Vec<YtTrack>>,
}

#[rocket::async_trait]
impl SongSearch for StubSearch {
    async fn songs(&self, query: &str) -> Result<Vec<YtTrack>, String> {
        self.calls.lock().unwrap().push(query.to_string());
        Ok(self.results.get(query).cloned().unwrap_or_default())
    }
}

fn yt(id: &str, title: &str, artist: &str) -> YtTrack {
    YtTrack {
        video_id: id.into(),
        title: title.into(),
        artists: vec![YtArtistRef {
            name: artist.into(),
            id: None,
        }],
        album: None,
        duration_ms: Some(200_000),
        thumbnail: Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
        explicit: false,
        is_video: false,
    }
}

struct Setup {
    env: Env,
    mock: Arc<MockSpotify>,
    search: Arc<StubSearch>,
}

async fn setup() -> Setup {
    setup_with(|f| f).await
}

async fn setup_with(
    adjust: impl FnOnce(rocket::figment::Figment) -> rocket::figment::Figment,
) -> Setup {
    let mock = Arc::new(MockSpotify::default());
    let m = mock.clone();
    let handler: Handler = Arc::new(move |req| m.handle(req));
    let base = mock_server(handler).await;
    let mut results = HashMap::new();
    // ISRC findet "Eins"; "Ben Zwei" und "Anna Drei" finden nichts Passendes
    results.insert(
        "DEAB12300001".to_string(),
        vec![
            yt("ytOther0001", "Etwas ganz anderes", "Niemand"),
            yt("ytEins00001", "Eins", "Anna"),
        ],
    );
    results.insert(
        "Ben Zwei".to_string(),
        vec![yt("ytNope00001", "Völlig fremd", "Jemand")],
    );
    let search = Arc::new(StubSearch {
        calls: Mutex::new(Vec::new()),
        results,
    });
    let services = nrzmusic::Services {
        spotify: Some(nrzmusic_spotify::Spotify::with_endpoints(
            &base,
            &format!("{base}/v1"),
        )),
        song_search: Some(search.clone()),
    };
    let env = env_with_services(adjust, services).await;
    Setup { env, mock, search }
}

async fn set_client_id(env: &Env, id: &str) {
    let mut s = env.get("/api/v1/settings").await;
    s["spotify_client_id"] = json!(id);
    let (status, v) = env.call("PUT", "/api/v1/settings", Some(s)).await;
    assert_eq!(status, Status::Ok, "{v}");
}

/// `state` aus der Anmelde-Adresse.
fn state_of(url: &str) -> String {
    url.split_once('?')
        .unwrap()
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix("state="))
        .unwrap()
        .to_string()
}

async fn login_url(env: &Env) -> String {
    let (status, login) = env.call("POST", "/api/v1/spotify/login", None).await;
    assert_eq!(status, Status::Ok, "{login}");
    login["authorize_url"].as_str().unwrap().to_string()
}

/// Verbinden über die eingefügte Adresse.
async fn connect(env: &Env) -> Value {
    let state = state_of(&login_url(env).await);
    let (status, v) = env
        .call(
            "POST",
            "/api/v1/spotify/callback",
            Some(json!({ "url": format!("http://127.0.0.1:8350/api/v1/spotify/callback?code=good-code&state={state}") })),
        )
        .await;
    assert_eq!(status, Status::Ok, "{v}");
    v
}

fn pool(env: &Env) -> sqlx::SqlitePool {
    env.client
        .rocket()
        .state::<sqlx::SqlitePool>()
        .unwrap()
        .clone()
}

fn sp_json(id: &str, name: &str, artist: &str, isrc: Option<&str>) -> Value {
    json!({
        "id": id, "name": name, "artists": [artist], "album": "Album", "duration_ms": 200000,
        "image": null, "isrc": isrc, "explicit": false, "added_at": null, "youtube": null
    })
}

// ---------------------------------------------------------------- Tests

#[rocket::async_test]
async fn login_with_pasted_callback() {
    let Setup { env, mock, .. } = setup().await;

    // Ohne Client-ID: nicht eingerichtet
    let st = env.get("/api/v1/spotify/status").await;
    assert_eq!(st["configured"], false);
    assert_eq!(st["connected"], false);
    let redirect = st["redirect_uri"].as_str().unwrap();
    assert!(
        redirect.starts_with("http://127.0.0.1:") && redirect.ends_with("/api/v1/spotify/callback"),
        "{redirect}"
    );
    for (method, uri) in [
        ("POST", "/api/v1/spotify/login"),
        ("GET", "/api/v1/spotify/liked"),
        ("GET", "/api/v1/spotify/playlists"),
    ] {
        let (status, err) = env.call(method, uri, None).await;
        assert_eq!(status, Status::ServiceUnavailable, "{uri}: {err}");
        assert!(err["error"].as_str().unwrap().contains("Client-ID"));
    }

    set_client_id(&env, CLIENT_ID).await;
    let st = env.get("/api/v1/spotify/status").await;
    assert_eq!(st["configured"], true);
    assert_eq!(st["client_id"], CLIENT_ID);
    let (status, _) = env.call("GET", "/api/v1/spotify/liked", None).await;
    assert_eq!(status, Status::Unauthorized);

    // Anmelde-Adresse beim (Ersatz-)Spotify, mit PKCE und state
    let url = login_url(&env).await;
    assert!(url.contains("/authorize?"), "{url}");
    assert!(url.contains(&format!("client_id={CLIENT_ID}")), "{url}");
    assert!(url.contains("code_challenge_method=S256"), "{url}");
    assert!(url.contains("response_type=code"), "{url}");
    let state = state_of(&url);

    // Unbekannter state, Fehler von Spotify, Adresse ohne Code
    let mut rx = env.library().events().subscribe();
    for (pasted, needle) in [
        (
            "http://127.0.0.1:8350/api/v1/spotify/callback?code=good-code&state=falsch".to_string(),
            "abgelaufen",
        ),
        (
            format!(
                "http://127.0.0.1:8350/api/v1/spotify/callback?error=access_denied&state={state}"
            ),
            "abgelehnt",
        ),
        (
            "http://127.0.0.1:8350/api/v1/spotify/callback".to_string(),
            "code",
        ),
        ("irgendwas".to_string(), "code"),
    ] {
        let (status, err) = env
            .call(
                "POST",
                "/api/v1/spotify/callback",
                Some(json!({ "url": pasted })),
            )
            .await;
        assert_eq!(status, Status::BadRequest, "{pasted}: {err}");
        assert!(err["error"].as_str().unwrap().contains(needle), "{err}");
    }
    // Der abgelehnte state ist verbraucht
    let (status, _) = env
        .call(
            "POST",
            "/api/v1/spotify/callback",
            Some(json!({ "url": format!("?code=good-code&state={state}") })),
        )
        .await;
    assert_eq!(status, Status::BadRequest);

    // Falscher Code: Spotify nimmt ihn nicht an
    let state = state_of(&login_url(&env).await);
    let (status, err) = env
        .call(
            "POST",
            "/api/v1/spotify/callback",
            Some(json!({ "url": format!("code=bad-code&state={state}") })),
        )
        .await;
    assert_eq!(status, Status::BadRequest, "{err}");

    // Konto nicht in der Spotify-App eingetragen: /me antwortet 403 → nicht verbunden
    mock.me_forbidden.store(true, Ordering::SeqCst);
    let state = state_of(&login_url(&env).await);
    let (status, err) = env
        .call(
            "POST",
            "/api/v1/spotify/callback",
            Some(json!({ "url": format!("code=good-code&state={state}") })),
        )
        .await;
    assert_eq!(status, Status::Forbidden, "{err}");
    assert!(
        err["error"].as_str().unwrap().contains("User Management"),
        "{err}"
    );
    assert_eq!(env.get("/api/v1/spotify/status").await["connected"], false);
    mock.me_forbidden.store(false, Ordering::SeqCst);

    // Richtig
    let st = connect(&env).await;
    assert_eq!(st["connected"], true, "{st}");
    assert_eq!(st["user"]["id"], "nico");
    assert_eq!(st["user"]["display_name"], "Nico");
    let text = st.to_string();
    assert!(
        !text.contains("acc-") && !text.contains("ref-1"),
        "keine Tokens: {text}"
    );
    assert!(std::iter::from_fn(|| rx.try_recv().ok().map(|(_, e)| e)).any(|e| e == ServerEvent::SpotifyChanged));

    // Benutzer kommt aus dem Zwischenspeicher
    let before = mock.log.lock().unwrap().len();
    assert_eq!(
        env.get("/api/v1/spotify/status").await["user"]["id"],
        "nico"
    );
    assert_eq!(mock.log.lock().unwrap().len(), before);

    // Trennen
    assert_eq!(
        env.call("DELETE", "/api/v1/spotify/session", None).await.0,
        Status::NoContent
    );
    let st = env.get("/api/v1/spotify/status").await;
    assert_eq!(st["connected"], false);
    assert!(st["user"].is_null());
    assert_eq!(
        env.call("GET", "/api/v1/spotify/playlists", None).await.0,
        Status::Unauthorized
    );
}

#[rocket::async_test]
async fn redirect_callback() {
    let Setup { env, .. } = setup().await;
    set_client_id(&env, CLIENT_ID).await;

    let state = state_of(&login_url(&env).await);
    let res = env
        .client
        .get(format!(
            "/api/v1/spotify/callback?error=access_denied&state={state}"
        ))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::SeeOther);
    let location = res.headers().get_one("Location").unwrap().to_string();
    assert!(location.starts_with("/spotify?error="), "{location}");
    assert!(location.contains("abgelehnt"), "{location}");

    let res = env
        .client
        .get("/api/v1/spotify/callback?code=good-code&state=unbekannt")
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::SeeOther);
    assert!(res
        .headers()
        .get_one("Location")
        .unwrap()
        .starts_with("/spotify?error="));

    let state = state_of(&login_url(&env).await);
    let res = env
        .client
        .get(format!(
            "/api/v1/spotify/callback?code=good-code&state={state}&iss=https%3A%2F%2Faccounts.spotify.com"
        ))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::SeeOther);
    assert_eq!(res.headers().get_one("Location"), Some("/spotify"));
    assert_eq!(env.get("/api/v1/spotify/status").await["connected"], true);
}

#[rocket::async_test]
async fn tokens_refresh_and_revocation() {
    let Setup { env, mock, .. } = setup().await;
    set_client_id(&env, CLIENT_ID).await;
    connect(&env).await;
    let pool = pool(&env);

    // Läuft bald ab → vorher erneuert
    sqlx::query("UPDATE spotify_auth SET expires_at = strftime('%s','now') + 30")
        .execute(&pool)
        .await
        .unwrap();
    let (status, liked) = env.call("GET", "/api/v1/spotify/liked", None).await;
    assert_eq!(status, Status::Ok, "{liked}");
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 1);
    let (refresh, expires): (String, i64) =
        sqlx::query_as("SELECT refresh_token, expires_at FROM spotify_auth")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(refresh, "ref-1", "altes Refresh-Token bleibt");
    assert!(expires > nrzmusic::db::now() + 3000);

    // Spotify lehnt das Token ab (401) → einmal erneuern, dann klappt es
    *mock.valid_access.lock().unwrap() = "anderes".into();
    let (status, v) = env.call("GET", "/api/v1/spotify/playlists", None).await;
    assert_eq!(status, Status::Ok, "{v}");
    assert_eq!(mock.refreshes.load(Ordering::SeqCst), 2);

    // Widerrufen: Erneuern scheitert → 401, getrennt, Ereignis
    let mut rx = env.library().events().subscribe();
    mock.revoked.store(true, Ordering::SeqCst);
    *mock.valid_access.lock().unwrap() = "abgelaufen".into();
    let (status, err) = env.call("GET", "/api/v1/spotify/liked", None).await;
    assert_eq!(status, Status::Unauthorized, "{err}");
    assert!(err["error"].as_str().unwrap().contains("neu verbinden"));
    assert!(std::iter::from_fn(|| rx.try_recv().ok().map(|(_, e)| e)).any(|e| e == ServerEvent::SpotifyChanged));
    let st = env.get("/api/v1/spotify/status").await;
    assert_eq!(st["connected"], false);
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM spotify_auth")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    assert_eq!(
        env.call("GET", "/api/v1/spotify/liked", None).await.0,
        Status::Unauthorized
    );

    // Neue Client-ID → alte Anmeldung verworfen
    mock.revoked.store(false, Ordering::SeqCst);
    connect(&env).await;
    set_client_id(&env, "otherclient").await;
    assert_eq!(env.get("/api/v1/spotify/status").await["connected"], false);
}

#[rocket::async_test]
async fn liked_playlists_and_matching() {
    let Setup { env, mock, search } = setup().await;
    set_client_id(&env, CLIENT_ID).await;
    connect(&env).await;

    // Abgleich: ISRC findet "Eins", "Zwei" findet nichts – beides wird gemerkt
    let body = json!({ "tracks": [
        sp_json("t1", "Eins", "Anna", Some("DEAB12300001")),
        sp_json("t2", "Zwei", "Ben", None),
        sp_json("t1", "Eins", "Anna", Some("DEAB12300001")),
    ] });
    let (status, matched) = env
        .call("POST", "/api/v1/spotify/match", Some(body.clone()))
        .await;
    assert_eq!(status, Status::Ok, "{matched}");
    assert_eq!(matched.as_array().unwrap().len(), 3);
    assert_eq!(matched[0]["youtube"]["video_id"], "ytEins00001");
    assert!(matched[1]["youtube"].is_null());
    assert_eq!(matched[2]["youtube"]["video_id"], "ytEins00001");
    let calls = search.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec!["DEAB12300001".to_string(), "Ben Zwei".to_string()]
    );

    // Zweites Mal: alles aus dem Zwischenspeicher
    let (_, again) = env.call("POST", "/api/v1/spotify/match", Some(body)).await;
    assert_eq!(again, matched);
    assert_eq!(search.calls.lock().unwrap().len(), 2);

    // Lieblingssongs mit Treffern aus dem Zwischenspeicher; limit höchstens 50
    let liked = env.get("/api/v1/spotify/liked?limit=500&offset=0").await;
    assert_eq!(liked["total"], 2, "{liked}");
    assert_eq!(liked["items"][0]["id"], "t1");
    assert_eq!(liked["items"][0]["name"], "Eins");
    assert_eq!(liked["items"][0]["artists"], json!(["Anna"]));
    assert_eq!(liked["items"][0]["isrc"], "DEAB12300001");
    assert_eq!(liked["items"][0]["youtube"]["video_id"], "ytEins00001");
    assert!(liked["items"][1]["youtube"].is_null());
    assert!(mock
        .log
        .lock()
        .unwrap()
        .iter()
        .any(|l| l.starts_with("GET /v1/me/tracks") && l.contains("limit=50")));

    let lists = env.get("/api/v1/spotify/playlists").await;
    assert_eq!(lists[0]["id"], "pl1", "{lists}");
    assert_eq!(lists[0]["name"], "Roadtrip");
    assert_eq!(lists[0]["track_count"], 2);
    // Titel lesbar: eigene und kollaborative ja, fremde nein
    let readable: Vec<(Value, Value)> = lists
        .as_array()
        .unwrap()
        .iter()
        .map(|l| (l["id"].clone(), l["readable"].clone()))
        .collect();
    assert_eq!(
        readable,
        vec![
            (json!("pl1"), json!(true)),
            (json!("pl2"), json!(false)),
            (json!("pl3"), json!(true)),
        ]
    );
    // Gesperrte Titel: 403 mit der Erklärung des Spotify-Clients
    let (status, err) = env.call("GET", "/api/v1/spotify/playlists/pl2", None).await;
    assert_eq!(status, Status::Forbidden, "{err}");
    assert!(
        err["error"].as_str().unwrap().contains("kollaborative"),
        "{err}"
    );
    let list = env.get("/api/v1/spotify/playlists/pl1").await;
    assert_eq!(list["summary"]["name"], "Roadtrip", "{list}");
    assert_eq!(list["tracks"][0]["youtube"]["video_id"], "ytEins00001");
    assert_eq!(list["tracks"][1]["id"], "t3");
    assert!(list["tracks"][1]["youtube"].is_null());
    assert_eq!(
        env.call("GET", "/api/v1/spotify/playlists/..%2Fme", None)
            .await
            .0,
        Status::NotFound
    );
    assert_eq!(
        env.call("GET", "/api/v1/spotify/playlists/gibtsnicht", None)
            .await
            .0,
        Status::NotFound
    );

    // YouTube aus: Abgleich 503, Lieblingssongs gehen weiter
    let mut s = env.get("/api/v1/settings").await;
    s["youtube_enabled"] = json!(false);
    env.call("PUT", "/api/v1/settings", Some(s)).await;
    let (status, _) = env
        .call(
            "POST",
            "/api/v1/spotify/match",
            Some(json!({ "tracks": [sp_json("t9", "Neun", "X", None)] })),
        )
        .await;
    assert_eq!(status, Status::ServiceUnavailable);
    assert_eq!(
        env.call("GET", "/api/v1/spotify/liked", None).await.0,
        Status::Ok
    );
}

#[rocket::async_test]
async fn import_creates_playlist() {
    let Setup { env, .. } = setup().await;
    set_client_id(&env, CLIENT_ID).await;

    // Nicht verbunden → 401; ungültige Quelle → 400
    let (status, _) = env
        .call(
            "POST",
            "/api/v1/spotify/import",
            Some(json!({ "source": "liked" })),
        )
        .await;
    assert_eq!(status, Status::Unauthorized);
    connect(&env).await;
    let (status, _) = env
        .call(
            "POST",
            "/api/v1/spotify/import",
            Some(json!({ "source": "../x" })),
        )
        .await;
    assert_eq!(status, Status::BadRequest);
    // Herunterladen verlangt, aber yt-dlp fehlt (Tests) → 503
    let (status, _) = env
        .call(
            "POST",
            "/api/v1/spotify/import",
            Some(json!({ "source": "liked", "download": true })),
        )
        .await;
    assert_eq!(status, Status::ServiceUnavailable);

    // Playlist, deren Titel Spotify nicht herausgibt: sofort 403 (und der nächste Import geht)
    let (status, err) = env
        .call(
            "POST",
            "/api/v1/spotify/import",
            Some(json!({ "source": "pl2", "create_playlist": true })),
        )
        .await;
    assert_eq!(status, Status::Forbidden, "{err}");
    assert!(
        err["error"].as_str().unwrap().contains("kollaborative"),
        "{err}"
    );

    for (source, name) in [
        ("liked", "Spotify – Lieblingssongs"),
        (
            "https://open.spotify.com/playlist/pl1?si=abc",
            "Spotify – Roadtrip",
        ),
    ] {
        let mut rx = env.library().events().subscribe();
        let (status, _) = env
            .call(
                "POST",
                "/api/v1/spotify/import",
                Some(json!({ "source": source, "create_playlist": true })),
            )
            .await;
        assert_eq!(status, Status::Accepted);
        let mut events = Vec::new();
        let finished = wait_for(10, || {
            while let Ok((_, ev)) = rx.try_recv() {
                events.push(ev);
            }
            let done = events.iter().find_map(|e| match e {
                ServerEvent::SpotifyProgress { task, done, total } if task == "import" => {
                    Some((*done, *total))
                }
                _ => None,
            });
            async move { done }
        })
        .await;
        assert_eq!(finished, (1, 2), "{source}");
        assert!(events.iter().any(|e| matches!(
            e,
            ServerEvent::SpotifyProgress { task, total: 2, .. } if task == "match"
        )));
        // PlaylistsChanged kommt direkt nach dem letzten Fortschritt
        wait_for(5, || {
            while let Ok((_, ev)) = rx.try_recv() {
                events.push(ev);
            }
            let ok = events.contains(&ServerEvent::PlaylistsChanged);
            async move { ok.then_some(()) }
        })
        .await;

        let lists = env.get("/api/v1/playlists").await;
        let pl = lists
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == name)
            .unwrap_or_else(|| panic!("{name} fehlt: {lists}"))
            .clone();
        assert_eq!(pl["item_count"], 1);
        let full = env.get(format!("/api/v1/playlists/{}", pl["id"])).await;
        assert_eq!(full["items"][0]["source"], "youtube");
        assert_eq!(full["items"][0]["video_id"], "ytEins00001");
    }
}

// ---------------------------------------------------------------- Einstellungen

#[rocket::async_test]
async fn settings_round_trip_and_old_rows() {
    let env = env().await;
    let s = env.get("/api/v1/settings").await;
    assert_eq!(
        s,
        json!({
            "youtube_enabled": true,
            "youtube_language": "de",
            "youtube_region": "CH",
            "downloads_enabled": true,
            "download_format": "mp3",
            "ytdlp_auto_update": true,
            "spotify_client_id": "",
            "spotify_redirect_uri": "",
            "registration_open": true
        })
    );

    let new = json!({
        "youtube_enabled": true,
        "youtube_language": "en",
        "youtube_region": "us",
        "downloads_enabled": false,
        "download_format": " OPUS ",
        "ytdlp_auto_update": false,
        "spotify_client_id": " abc123 ",
        "spotify_redirect_uri": "https://music.example.com/api/v1/spotify/callback"
    });
    let (status, saved) = env.call("PUT", "/api/v1/settings", Some(new)).await;
    assert_eq!(status, Status::Ok, "{saved}");
    assert_eq!(saved["download_format"], "opus");
    assert_eq!(saved["spotify_client_id"], "abc123");
    assert_eq!(saved["downloads_enabled"], false);
    let st = env.get("/api/v1/spotify/status").await;
    assert_eq!(st["configured"], true);
    assert_eq!(
        st["redirect_uri"],
        "https://music.example.com/api/v1/spotify/callback"
    );

    // Ungültiges wird abgewiesen
    for (key, value) in [
        ("download_format", json!("wav")),
        ("spotify_client_id", json!("a b")),
        ("spotify_redirect_uri", json!("ftp://x")),
        ("spotify_redirect_uri", json!("https://a b")),
    ] {
        let mut bad = saved.clone();
        bad[key] = value.clone();
        let (status, _) = env.call("PUT", "/api/v1/settings", Some(bad)).await;
        assert_eq!(status, Status::BadRequest, "{key} = {value}");
    }

    // Gespeichert: ein neuer Server mit derselben Datenbank kennt alles
    let rocket = nrzmusic::build_with(figment(env.data.path(), env.music.path()))
        .await
        .unwrap();
    let c2 = rocket::local::asynchronous::Client::tracked(rocket)
        .await
        .unwrap();
    assert_eq!(call(&c2, "GET", "/api/v1/settings", None).await.1, saved);

    // Alte Datenbank (nur die Schlüssel von 1.0): neue Felder nehmen den Standard
    let pool = env
        .client
        .rocket()
        .state::<sqlx::SqlitePool>()
        .unwrap()
        .clone();
    sqlx::query("DELETE FROM settings")
        .execute(&pool)
        .await
        .unwrap();
    for (k, v) in [
        ("youtube_enabled", "false"),
        ("youtube_language", "fr"),
        ("youtube_region", "FR"),
    ] {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?)")
            .bind(k)
            .bind(v)
            .execute(&pool)
            .await
            .unwrap();
    }
    let rocket = nrzmusic::build_with(figment(env.data.path(), env.music.path()))
        .await
        .unwrap();
    let c3 = rocket::local::asynchronous::Client::tracked(rocket)
        .await
        .unwrap();
    assert_eq!(
        call(&c3, "GET", "/api/v1/settings", None).await.1,
        json!({
            "youtube_enabled": false,
            "youtube_language": "fr",
            "youtube_region": "FR",
            "downloads_enabled": true,
            "download_format": "mp3",
            "ytdlp_auto_update": true,
            "spotify_client_id": "",
            "spotify_redirect_uri": "",
            "registration_open": true
        })
    );
}

#[rocket::async_test]
async fn spotify_client_id_from_environment() {
    let env = env_with(|f| f.merge(("spotify_client_id", "envclient"))).await;
    let s = env.get("/api/v1/settings").await;
    assert_eq!(s["spotify_client_id"], "envclient");
    assert_eq!(env.get("/api/v1/spotify/status").await["configured"], true);

    // Unverändert zurückgeschrieben: die Voreinstellung wird nicht festgeschrieben
    let (status, _) = env.call("PUT", "/api/v1/settings", Some(s.clone())).await;
    assert_eq!(status, Status::Ok);
    let pool = env
        .client
        .rocket()
        .state::<sqlx::SqlitePool>()
        .unwrap()
        .clone();
    let saved: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'spotify_client_id'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(saved, "");

    // Eine eigene ID gewinnt
    let mut own = s;
    own["spotify_client_id"] = json!("mine42");
    let (_, v) = env.call("PUT", "/api/v1/settings", Some(own)).await;
    assert_eq!(v["spotify_client_id"], "mine42");
    assert_eq!(
        env.get("/api/v1/spotify/status").await["client_id"],
        "mine42"
    );
}
