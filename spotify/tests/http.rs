//! HTTP-Schicht gegen einen kleinen lokalen Ersatz-Server (kein Spotify-Konto nötig):
//! Token-Tausch, Erneuerung, Seiten, Fehler, `429`-Wiederholung.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nrzmusic_spotify::{Error, Spotify};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const CLIENT_ID: &str = "0123456789abcdef0123456789abcdef";
const REDIRECT: &str = "http://127.0.0.1:8350/api/v1/spotify/callback";
const TOKEN: &str = "BQD-test-access";
const SPOTIFY_API: &str = "https://api.spotify.com/v1";

// ---------------------------------------------------------------- Ersatz-Server

#[derive(Clone, Debug)]
struct Request {
    method: String,
    /// Pfad mit Query, z. B. "/v1/me/tracks?offset=0&limit=50"
    target: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    fn query(&self) -> HashMap<String, String> {
        let q = self.target.split_once('?').map(|(_, q)| q).unwrap_or("");
        url::form_urlencoded::parse(q.as_bytes())
            .into_owned()
            .collect()
    }

    fn form(&self) -> HashMap<String, String> {
        url::form_urlencoded::parse(self.body.as_bytes())
            .into_owned()
            .collect()
    }
}

struct Reply {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: String,
}

fn reply(status: u16, body: impl Into<String>) -> Reply {
    Reply {
        status,
        headers: Vec::new(),
        body: body.into(),
    }
}

impl Reply {
    fn header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.to_string()));
        self
    }
}

struct Mock {
    base: String,
    log: Arc<Mutex<Vec<Request>>>,
}

impl Mock {
    async fn start<F>(handler: F) -> Mock
    where
        F: Fn(&Request) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let server_log = log.clone();
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                let handler = handler.clone();
                let log = server_log.clone();
                tokio::spawn(async move { serve(sock, handler.as_ref(), &log).await });
            }
        });
        Mock { base, log }
    }

    fn client(&self) -> Spotify {
        let sp = Spotify::with_endpoints(&self.base, &format!("{}/v1", self.base));
        sp.configure(CLIENT_ID, REDIRECT);
        sp
    }

    fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }
}

async fn serve(
    mut sock: TcpStream,
    handler: &(dyn Fn(&Request) -> Reply + Send + Sync),
    log: &Mutex<Vec<Request>>,
) {
    let Some(req) = read_request(&mut sock).await else {
        return;
    };
    log.lock().unwrap().push(req.clone());
    let r = handler(&req);
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n",
        r.status,
        r.body.len()
    );
    for (k, v) in &r.headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = sock.write_all(head.as_bytes()).await;
    let _ = sock.write_all(r.body.as_bytes()).await;
    let _ = sock.shutdown().await;
}

async fn read_request(sock: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[header_end..].to_vec();
    while body.len() < len {
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some(Request {
        method,
        target,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

// ---------------------------------------------------------------- Anmeldung

#[tokio::test]
async fn token_exchange_sends_pkce_form() {
    let mock = Mock::start(|_| reply(200, fixture("token_exchange.json"))).await;
    let sp = mock.client();
    let tokens = sp.exchange_code(" AQBcode ", "verifier-123").await.unwrap();
    assert_eq!(tokens.access_token, "BQD-access-token-1");
    assert_eq!(tokens.refresh_token, "AQC-refresh-token-1");
    assert!((tokens.expires_at - (now() + 3600)).abs() <= 5);
    assert!(tokens.scope.contains("user-library-read"));

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.target, "/api/token");
    assert_eq!(
        r.header("content-type"),
        Some("application/x-www-form-urlencoded")
    );
    // PKCE: kein Client-Secret, keine Basic-Anmeldung
    assert_eq!(r.header("authorization"), None);
    let form = r.form();
    assert_eq!(form.len(), 5, "{form:?}");
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "AQBcode");
    assert_eq!(form["redirect_uri"], REDIRECT);
    assert_eq!(form["client_id"], CLIENT_ID);
    assert_eq!(form["code_verifier"], "verifier-123");
}

#[tokio::test]
async fn token_exchange_error_is_reported() {
    let mock = Mock::start(|_| {
        reply(
            400,
            r#"{"error":"invalid_grant","error_description":"Invalid authorization code"}"#,
        )
    })
    .await;
    match mock.client().exchange_code("old", "v").await {
        Err(Error::Api { status, message }) => {
            assert_eq!(status, 400);
            assert_eq!(message, "Invalid authorization code (invalid_grant)");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn refresh_keeps_old_refresh_token() {
    let mock = Mock::start(|_| reply(200, fixture("token_refresh.json"))).await;
    let tokens = mock.client().refresh("AQC-old-refresh").await.unwrap();
    assert_eq!(tokens.access_token, "BQD-access-token-2");
    assert_eq!(tokens.refresh_token, "AQC-old-refresh");
    assert!(tokens.expires_at > now());

    let form = mock.requests()[0].form();
    assert_eq!(form.len(), 3, "{form:?}");
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["refresh_token"], "AQC-old-refresh");
    assert_eq!(form["client_id"], CLIENT_ID);
}

#[tokio::test]
async fn refresh_takes_new_refresh_token() {
    let mock = Mock::start(|_| reply(200, fixture("token_exchange.json"))).await;
    let tokens = mock.client().refresh("AQC-old-refresh").await.unwrap();
    assert_eq!(tokens.refresh_token, "AQC-refresh-token-1");
}

#[tokio::test]
async fn refresh_rejected_is_unauthorized() {
    let mock = Mock::start(|_| reply(400, fixture("error_invalid_grant.json"))).await;
    assert!(matches!(
        mock.client().refresh("revoked").await,
        Err(Error::Unauthorized)
    ));

    let mock = Mock::start(|_| {
        reply(
            401,
            r#"{"error":"invalid_client","error_description":"Invalid client"}"#,
        )
    })
    .await;
    assert!(matches!(
        mock.client().refresh("r").await,
        Err(Error::Unauthorized)
    ));

    // Server-Fehler sind kein Grund, die Anmeldung zu verwerfen
    let mock = Mock::start(|_| reply(503, "").header("Retry-After", "0")).await;
    match mock.client().refresh("r").await {
        Err(Error::Api { status, .. }) => assert_eq!(status, 503),
        other => panic!("{other:?}"),
    }
    assert_eq!(mock.requests().len(), 4, "1 Versuch + 3 Wiederholungen");
}

#[tokio::test]
async fn without_client_id_nothing_is_sent() {
    let mock = Mock::start(|_| reply(200, fixture("token_exchange.json"))).await;
    let sp = Spotify::with_endpoints(&mock.base, &format!("{}/v1", mock.base));
    assert!(matches!(
        sp.exchange_code("c", "v").await,
        Err(Error::NotConfigured)
    ));
    assert!(matches!(sp.refresh("r").await, Err(Error::NotConfigured)));
    assert!(matches!(sp.authorize_url(), Err(Error::NotConfigured)));
    assert!(mock.requests().is_empty());
}

#[tokio::test]
async fn authorize_url_uses_accounts_endpoint() {
    let mock = Mock::start(|_| reply(500, "")).await;
    let (url, pending) = mock.client().authorize_url().unwrap();
    assert!(url.starts_with(&format!("{}/authorize?client_id={CLIENT_ID}&", mock.base)));
    assert!(url.contains(&format!("state={}", pending.state)));
    assert!(
        url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8350%2Fapi%2Fv1%2Fspotify%2Fcallback")
    );
}

// ---------------------------------------------------------------- Web-API

#[tokio::test]
async fn me_sends_bearer_token() {
    let mock = Mock::start(|_| reply(200, fixture("me.json"))).await;
    let user = mock.client().me(TOKEN).await.unwrap();
    assert_eq!(user.id, "nico.r");
    assert_eq!(user.display_name.as_deref(), Some("Nico"));
    let r = &mock.requests()[0];
    assert_eq!(r.method, "GET");
    assert_eq!(r.target, "/v1/me");
    assert_eq!(r.header("authorization"), Some(&*format!("Bearer {TOKEN}")));
}

#[tokio::test]
async fn expired_token_is_unauthorized() {
    let mock = Mock::start(|_| reply(401, fixture("error_unauthorized.json"))).await;
    let sp = mock.client();
    assert!(matches!(sp.me(TOKEN).await, Err(Error::Unauthorized)));
    assert!(matches!(
        sp.liked(TOKEN, 0, 20).await,
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        sp.playlists(TOKEN).await,
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        sp.playlist(TOKEN, "3cEYpjA9oz9GiPac4AsH4n").await,
        Err(Error::Unauthorized)
    ));
    // Leeres Token: gar nicht erst fragen
    let before = mock.requests().len();
    assert!(matches!(sp.me("").await, Err(Error::Unauthorized)));
    assert_eq!(mock.requests().len(), before);
}

#[tokio::test]
async fn me_forbidden_explains_allowlist() {
    let mock = Mock::start(|_| {
        reply(
            403,
            r#"{"error":{"status":403,"message":"Check settings on developer.spotify.com/dashboard, the user may not be registered."}}"#,
        )
    })
    .await;
    match mock.client().me(TOKEN).await {
        Err(Error::Api { status, message }) => {
            assert_eq!(status, 403);
            assert!(message.contains("may not be registered"), "{message}");
            assert!(message.contains("User Management"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn rate_limit_is_retried() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let mock = Mock::start(move |_| {
        if c.fetch_add(1, Ordering::SeqCst) < 2 {
            reply(
                429,
                r#"{"error":{"status":429,"message":"API rate limit exceeded"}}"#,
            )
            .header("Retry-After", "0")
        } else {
            reply(200, fixture("me.json"))
        }
    })
    .await;
    let user = mock.client().me(TOKEN).await.unwrap();
    assert_eq!(user.id, "nico.r");
    assert_eq!(mock.requests().len(), 3);
}

#[tokio::test]
async fn rate_limit_gives_up_after_three_retries() {
    let mock = Mock::start(|_| {
        reply(
            429,
            r#"{"error":{"status":429,"message":"API rate limit exceeded"}}"#,
        )
        .header("Retry-After", "0")
    })
    .await;
    match mock.client().liked(TOKEN, 0, 50).await {
        Err(Error::Api { status, message }) => {
            assert_eq!(status, 429);
            assert_eq!(message, "API rate limit exceeded");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(mock.requests().len(), 4);
}

#[tokio::test]
async fn long_retry_after_is_not_awaited() {
    let mock =
        Mock::start(|_| reply(429, fixture("error_quota.json")).header("Retry-After", "86400"))
            .await;
    let started = std::time::Instant::now();
    match mock.client().me(TOKEN).await {
        Err(Error::Api { status, message }) => {
            assert_eq!(status, 429);
            assert_eq!(message, "Too many requests (QUOTA_EXCEEDED)");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(mock.requests().len(), 1);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn short_retry_after_is_awaited() {
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let mock = Mock::start(move |_| {
        if c.fetch_add(1, Ordering::SeqCst) == 0 {
            reply(429, "").header("Retry-After", "1")
        } else {
            reply(200, fixture("me.json"))
        }
    })
    .await;
    let started = std::time::Instant::now();
    mock.client().me(TOKEN).await.unwrap();
    assert!(started.elapsed() >= std::time::Duration::from_millis(900));
}

#[tokio::test]
async fn liked_page_and_limit_clamp() {
    let mock = Mock::start(|_| reply(200, fixture("saved_tracks.json"))).await;
    let sp = mock.client();
    let page = sp.liked(TOKEN, 0, 500).await.unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.total, 3);
    assert_eq!(page.offset, 0);
    assert_eq!(page.limit, 2);
    assert_eq!(
        page.items[0].added_at.as_deref(),
        Some("2026-10-30T19:12:45Z")
    );
    assert_eq!(page.items[1].youtube, None);

    sp.liked(TOKEN, 7, 0).await.unwrap();
    let reqs = mock.requests();
    assert_eq!(reqs[0].path(), "/v1/me/tracks");
    assert_eq!(reqs[0].query()["limit"], "50");
    assert_eq!(reqs[0].query()["offset"], "0");
    assert_eq!(reqs[1].query()["limit"], "1");
    assert_eq!(reqs[1].query()["offset"], "7");
}

fn liked_pages(mock_base: &Arc<Mutex<String>>, rewrite: bool) -> impl Fn(&Request) -> Reply {
    let base = mock_base.clone();
    move |r: &Request| {
        let name = match r.query().get("offset").map(String::as_str) {
            Some("2") => "saved_tracks_page2.json",
            _ => "saved_tracks.json",
        };
        let body = fixture(name);
        let body = if rewrite {
            body.replace(SPOTIFY_API, &format!("{}/v1", base.lock().unwrap()))
        } else {
            body
        };
        reply(200, body)
    }
}

#[tokio::test]
async fn all_liked_follows_next() {
    let base = Arc::new(Mutex::new(String::new()));
    let mock = Mock::start(liked_pages(&base, true)).await;
    *base.lock().unwrap() = mock.base.clone();

    let tracks = mock.client().all_liked(TOKEN, 1000).await.unwrap();
    let names: Vec<_> = tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Bohemian Rhapsody - Remastered 2011",
            "Get Lucky (feat. Pharrell Williams and Nile Rodgers)",
            "Blinding Lights"
        ]
    );
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].target, "/v1/me/tracks?offset=0&limit=50");
    assert_eq!(reqs[1].target, "/v1/me/tracks?offset=2&limit=2");
    assert!(reqs
        .iter()
        .all(|r| r.header("authorization") == Some(&*format!("Bearer {TOKEN}"))));
}

#[tokio::test]
async fn all_liked_never_sends_token_elsewhere() {
    // `next` zeigt auf api.spotify.com statt auf den Ersatz-Server: Adresse wird aus
    // offset/limit selbst gebaut, das Token bleibt beim konfigurierten Server
    let base = Arc::new(Mutex::new(String::new()));
    let mock = Mock::start(liked_pages(&base, false)).await;
    let tracks = mock.client().all_liked(TOKEN, 1000).await.unwrap();
    assert_eq!(tracks.len(), 3);
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[1].target, "/v1/me/tracks?offset=2&limit=2");
}

#[tokio::test]
async fn all_liked_stops_at_max() {
    let base = Arc::new(Mutex::new(String::new()));
    let mock = Mock::start(liked_pages(&base, true)).await;
    *base.lock().unwrap() = mock.base.clone();
    let sp = mock.client();

    let tracks = sp.all_liked(TOKEN, 1).await.unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(mock.requests().len(), 1);
    assert_eq!(mock.requests()[0].query()["limit"], "1");

    assert!(sp.all_liked(TOKEN, 0).await.unwrap().is_empty());
    assert_eq!(mock.requests().len(), 1);
}

#[tokio::test]
async fn playlists_follow_pages() {
    let base = Arc::new(Mutex::new(String::new()));
    let b = base.clone();
    let mock = Mock::start(move |r| {
        let base = format!("{}/v1", b.lock().unwrap());
        if r.query().get("offset").map(String::as_str) == Some("3") {
            reply(
                200,
                serde_json::json!({
                    "href": format!("{base}/me/playlists?offset=3&limit=3"),
                    "limit": 3, "offset": 3, "total": 4, "next": null, "previous": null,
                    "items": [
                        null,
                        {"id": "5zeNtmTNH8rHTp9H1H5cKe", "name": "Joggen", "collaborative": false,
                         "owner": {"id": "nico.r", "display_name": "Nico"},
                         "images": [{"url": "https://i.scdn.co/image/jog", "height": 640, "width": 640}],
                         "items": {"href": "x", "total": 31}, "type": "playlist"}
                    ]
                })
                .to_string(),
            )
        } else {
            let mut v: serde_json::Value =
                serde_json::from_str(&fixture("playlists.json")).unwrap();
            v["limit"] = 3.into();
            v["total"] = 4.into();
            v["next"] = format!("{base}/me/playlists?offset=3&limit=3").into();
            reply(200, v.to_string())
        }
    })
    .await;
    *base.lock().unwrap() = mock.base.clone();
    let sp = mock.client();

    let lists = sp.playlists(TOKEN).await.unwrap();
    let names: Vec<_> = lists.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Roadtrip",
            "Party (gemeinsam)",
            "Today's Top Hits",
            "Joggen"
        ]
    );
    assert_eq!(lists[3].track_count, 31);
    assert_eq!(
        lists[3].image.as_deref(),
        Some("https://i.scdn.co/image/jog")
    );

    let entries = sp.playlist_entries(TOKEN).await.unwrap();
    let readable: Vec<_> = entries
        .iter()
        .filter(|e| e.items_readable("nico.r"))
        .map(|e| e.summary.name.as_str())
        .collect();
    assert_eq!(readable, ["Roadtrip", "Party (gemeinsam)", "Joggen"]);
    assert_eq!(
        mock.requests()[0].target,
        "/v1/me/playlists?offset=0&limit=50"
    );
}

#[tokio::test]
async fn playlist_skips_episodes_and_follows_items_pages() {
    let base = Arc::new(Mutex::new(String::new()));
    let b = base.clone();
    let mock = Mock::start(move |r| {
        let base = format!("{}/v1", b.lock().unwrap());
        let body = match r.path() {
            "/v1/playlists/3cEYpjA9oz9GiPac4AsH4n" => fixture("playlist.json"),
            "/v1/playlists/3cEYpjA9oz9GiPac4AsH4n/items" => fixture("playlist_items_page2.json"),
            _ => return reply(404, r#"{"error":{"status":404,"message":"Not found."}}"#),
        };
        reply(200, body.replace(SPOTIFY_API, &base))
    })
    .await;
    *base.lock().unwrap() = mock.base.clone();

    let pl = mock
        .client()
        .playlist(
            TOKEN,
            "https://open.spotify.com/playlist/3cEYpjA9oz9GiPac4AsH4n?si=abc",
        )
        .await
        .unwrap();
    assert_eq!(pl.summary.id, "3cEYpjA9oz9GiPac4AsH4n");
    assert_eq!(pl.summary.name, "Roadtrip");
    assert_eq!(pl.summary.owner.as_deref(), Some("Nico"));
    assert_eq!(pl.summary.track_count, 5);
    assert_eq!(
        pl.summary.image.as_deref(),
        Some("https://mosaic.scdn.co/640/ab67616d0000b2731")
    );
    let names: Vec<_> = pl.tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["Time to Pretend", "Mädchen aus Ostberlin"]);
    assert_eq!(pl.tracks[1].artists, ["Die Ärzte"]);

    let targets: Vec<_> = mock.requests().iter().map(|r| r.target.clone()).collect();
    assert_eq!(
        targets,
        [
            "/v1/playlists/3cEYpjA9oz9GiPac4AsH4n",
            "/v1/playlists/3cEYpjA9oz9GiPac4AsH4n/items?offset=4&limit=4"
        ]
    );
}

#[tokio::test]
async fn foreign_playlist_items_are_forbidden() {
    let mock = Mock::start(|r| match r.path() {
        "/v1/playlists/37i9dQZF1DXcBWIGoYBM5M" => reply(
            200,
            r#"{"id":"37i9dQZF1DXcBWIGoYBM5M","name":"Today's Top Hits","collaborative":false,
                "owner":{"id":"spotify","display_name":"Spotify"},"images":[],"type":"playlist"}"#,
        ),
        _ => reply(403, fixture("error_forbidden_playlist.json")),
    })
    .await;
    match mock
        .client()
        .playlist(TOKEN, "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M")
        .await
    {
        Err(Error::Api { status, message }) => {
            assert_eq!(status, 403);
            assert!(message.contains("eigene und kollaborative"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    let targets: Vec<_> = mock.requests().iter().map(|r| r.target.clone()).collect();
    assert_eq!(
        targets,
        [
            "/v1/playlists/37i9dQZF1DXcBWIGoYBM5M",
            "/v1/playlists/37i9dQZF1DXcBWIGoYBM5M/items?offset=0&limit=50"
        ]
    );
}

#[tokio::test]
async fn playlist_falls_back_to_tracks_endpoint() {
    let mock = Mock::start(|r| match r.path() {
        "/v1/playlists/7vX3dLqZ9pXy1Lq1d3nM2k" => reply(
            200,
            r#"{"id":"7vX3dLqZ9pXy1Lq1d3nM2k","name":"Party","collaborative":true,
                "owner":{"id":"friend42","display_name":null},"images":[]}"#,
        ),
        "/v1/playlists/7vX3dLqZ9pXy1Lq1d3nM2k/tracks" => reply(
            200,
            fixture("playlist_items_page2.json").replace("\"offset\": 4", "\"offset\": 0"),
        ),
        _ => reply(
            404,
            r#"{"error":{"status":404,"message":"Service not found"}}"#,
        ),
    })
    .await;
    let pl = mock
        .client()
        .playlist(TOKEN, "7vX3dLqZ9pXy1Lq1d3nM2k")
        .await
        .unwrap();
    assert_eq!(pl.tracks.len(), 1);
    assert_eq!(pl.summary.owner.as_deref(), Some("friend42"));
    // Ohne Gesamtzahl im Playlist-Objekt: Anzahl der geladenen Titel
    assert_eq!(pl.summary.track_count, 1);
    assert_eq!(mock.requests().len(), 3);
}

#[tokio::test]
async fn missing_playlist_explains_editorial_block() {
    let mock = Mock::start(|_| {
        reply(
            404,
            r#"{"error":{"status":404,"message":"Resource not found"}}"#,
        )
    })
    .await;
    let sp = mock.client();
    match sp.playlist(TOKEN, "37i9dQZEVXcJZyENOWUFo7").await {
        Err(Error::Api { status, message }) => {
            assert_eq!(status, 404);
            assert!(message.contains("Resource not found"), "{message}");
            assert!(message.contains("algorithmische"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    // Ungültige ID: keine Anfrage
    let before = mock.requests().len();
    assert!(matches!(
        sp.playlist(TOKEN, "../me").await,
        Err(Error::Api { status: 400, .. })
    ));
    assert_eq!(mock.requests().len(), before);
}

#[tokio::test]
async fn broken_json_is_a_parse_error() {
    let mock = Mock::start(|r| match r.path() {
        "/v1/me" => reply(200, "<html>Wartung</html>"),
        _ => reply(200, r#"{"total": 3}"#),
    })
    .await;
    let sp = mock.client();
    assert!(matches!(sp.me(TOKEN).await, Err(Error::Parse(_))));
    assert!(matches!(sp.liked(TOKEN, 0, 10).await, Err(Error::Parse(_))));
    assert!(matches!(
        sp.all_liked(TOKEN, 10).await,
        Err(Error::Parse(_))
    ));
}

// ---------------------------------------------------------------- Webplayer-Cookie

const SP_DC: &str = "AQB-test-cookie-0123456789_abcdef";

fn web_token_reply() -> Reply {
    let ms = (now() + 3600) * 1000;
    reply(
        200,
        format!(
            r#"{{"clientId":"x","accessToken":"BQC-web","accessTokenExpirationTimestampMs":{ms},"isAnonymous":false}}"#
        ),
    )
}

#[tokio::test]
async fn web_token_sends_cookie_and_totp() {
    let mock = Mock::start(|req| match req.path() {
        "/secretDict.json" => reply(200, r#"{"60":[1,2,3],"61":[44,55,47,42]}"#),
        "/api/token" => web_token_reply(),
        _ => reply(404, ""),
    })
    .await;
    let tokens = mock
        .client()
        .web_token(&format!("sp_t=x; sp_dc={SP_DC}"))
        .await
        .unwrap();
    assert_eq!(tokens.access_token, "BQC-web");
    assert_eq!(tokens.refresh_token, SP_DC);
    assert_eq!(tokens.scope, nrzmusic_spotify::WEB_PLAYER_SCOPE);
    assert!(tokens.expires_at > now());

    let reqs = mock.requests();
    let token_req = reqs.iter().find(|r| r.path() == "/api/token").unwrap();
    assert_eq!(token_req.method, "GET");
    assert_eq!(token_req.header("cookie"), Some(format!("sp_dc={SP_DC}").as_str()));
    let q = token_req.query();
    assert_eq!(q["totpVer"], "61");
    assert_eq!(q["productType"], "web-player");
    assert_eq!(q["totp"].len(), 6);
    assert!(q["totp"].bytes().all(|b| b.is_ascii_digit()));
    assert_eq!(q["totp"], q["totpServer"]);
}

#[tokio::test]
async fn web_token_rejected_cookie_is_unauthorized() {
    let mock = Mock::start(|req| match req.path() {
        "/secretDict.json" => reply(200, r#"{"61":[44,55]}"#),
        _ => reply(401, r#"{"error":{"code":401,"message":"Unauthorized"}}"#),
    })
    .await;
    let err = mock.client().web_token(SP_DC).await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized), "{err:?}");

    let anon = Mock::start(|req| match req.path() {
        "/secretDict.json" => reply(200, r#"{"61":[44,55]}"#),
        _ => reply(200, r#"{"accessToken":"BQC-anon","isAnonymous":true}"#),
    })
    .await;
    let err = anon.client().web_token(SP_DC).await.unwrap_err();
    assert!(matches!(err, Error::Unauthorized), "{err:?}");
}

#[tokio::test]
async fn web_token_reloads_rotated_secret() {
    let lists = Arc::new(AtomicUsize::new(0));
    let seen = lists.clone();
    let mock = Mock::start(move |req| match req.path() {
        "/secretDict.json" => {
            // Erst nur Version 60, beim Neuladen auch 61
            if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                reply(200, r#"{"60":[1,2,3]}"#)
            } else {
                reply(200, r#"{"60":[1,2,3],"61":[44,55,47]}"#)
            }
        }
        "/api/token" if req.query()["totpVer"] == "61" => web_token_reply(),
        "/api/token" => reply(400, r#"{"error":{"code":400,"message":"Unauthorized request"}}"#),
        _ => reply(404, ""),
    })
    .await;
    let client = mock.client();
    let tokens = client.web_token(SP_DC).await.unwrap();
    assert_eq!(tokens.access_token, "BQC-web");
    assert_eq!(lists.load(Ordering::SeqCst), 2);
    // Danach aus dem Zwischenspeicher
    client.web_token(SP_DC).await.unwrap();
    assert_eq!(lists.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn web_token_reports_stale_secret() {
    let mock = Mock::start(|req| match req.path() {
        "/secretDict.json" => reply(200, r#"{"60":[1,2,3]}"#),
        _ => reply(400, r#"{"error":{"code":400,"message":"Unauthorized request"}}"#),
    })
    .await;
    match mock.client().web_token(SP_DC).await.unwrap_err() {
        Error::Api { status, message } => {
            assert_eq!(status, 503);
            assert!(message.contains("Version 60"), "{message}");
        }
        e => panic!("{e:?}"),
    }
}
