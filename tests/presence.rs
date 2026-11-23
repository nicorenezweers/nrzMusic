//! Jetzt läuft: nur eingeschaltet; Geräte melden, die App liest den Stand; öffentlich über
//! Widget, JSON und MCP mit geheimer Adresse; neue Adresse macht die alte ungültig.

mod common;

use common::*;
use rocket::http::{ContentType, Status};
use rocket::local::asynchronous::Client;
use serde_json::json;

fn report(device: &str, name: &str, playing: bool, pos: u64) -> serde_json::Value {
    json!({
        "device_id": device, "device_name": name, "device_kind": "computer",
        "track": { "source": "youtube", "video_id": "abc", "title": "Song", "artists": [{"name": "Band", "id": null}],
                   "album": null, "duration_ms": 200000, "thumbnail": "https://i.ytimg.com/x.jpg" },
        "position_ms": pos, "duration_ms": 200000, "playing": playing
    })
}

#[rocket::async_test]
async fn now_playing_sync_widget_and_mcp() {
    let env = env_with(|f| f.merge(("auth", true))).await;
    let c = &env.client;
    let (s, _) = call(c, "POST", "/api/v1/auth/register", Some(json!({"username": "nico", "password": "geheim-12345", "display_name": "Nico"}))).await;
    assert_eq!(s, Status::Created);

    // Aus: Meldungen werden abgelehnt
    let (_, st) = call(c, "GET", "/api/v1/presence", None).await;
    assert_eq!(st["enabled"], false);
    let (s, _) = call(c, "POST", "/api/v1/now-playing", Some(report("mac", "MacBook", true, 1000))).await;
    assert_eq!(s, Status::Forbidden);

    // Sync an, noch nicht öffentlich
    let (_, st) = call(c, "PUT", "/api/v1/presence", Some(json!({"enabled": true, "public": false}))).await;
    assert_eq!(st["enabled"], true);
    assert!(st["widget_path"].is_null());
    let (s, _) = call(c, "POST", "/api/v1/now-playing", Some(report("mac", "MacBook", true, 30_000))).await;
    assert_eq!(s, Status::NoContent);
    let (_, now) = call(c, "GET", "/api/v1/now-playing", None).await;
    assert_eq!(now["device_name"], "MacBook");
    assert_eq!(now["playing"], true);

    // Ein pausiertes Gerät überschreibt das spielende nicht, ein spielendes schon
    let (s, _) = call(c, "POST", "/api/v1/now-playing", Some(report("phone", "iPhone", false, 5_000))).await;
    assert_eq!(s, Status::NoContent);
    let (_, now) = call(c, "GET", "/api/v1/now-playing", None).await;
    assert_eq!(now["device_name"], "MacBook");
    call(c, "POST", "/api/v1/now-playing", Some(report("phone", "iPhone", true, 5_000))).await;
    let (_, now) = call(c, "GET", "/api/v1/now-playing", None).await;
    assert_eq!(now["device_name"], "iPhone");

    // Öffentlich
    let (_, st) = call(c, "PUT", "/api/v1/presence", Some(json!({"enabled": true, "public": true}))).await;
    let widget = st["widget_path"].as_str().unwrap().to_string();
    let api = st["api_path"].as_str().unwrap().to_string();
    let mcp = st["mcp_path"].as_str().unwrap().to_string();

    let anon = Client::tracked(nrzmusic::build_with(figment(env.data.path(), env.music.path()).merge(("auth", true))).await.unwrap())
        .await
        .unwrap();
    let (s, v) = call(&anon, "GET", api.clone(), None).await;
    assert_eq!(s, Status::Ok, "{v}");
    assert_eq!(v["active"], true);
    assert_eq!(v["title"], "Song");
    assert_eq!(v["artist"], "Band");
    assert_eq!(v["user"], "Nico");
    assert_eq!(v["device_name"], "iPhone");
    assert!(v["position_ms"].as_u64().unwrap() >= 5_000);

    let res = anon.get(widget.clone()).dispatch().await;
    assert_eq!(res.status(), Status::Ok);
    let csp = res.headers().get_one("Content-Security-Policy").unwrap().to_string();
    assert!(csp.contains("frame-ancestors *"), "{csp}");
    let html = res.into_string().await.unwrap();
    assert!(html.contains(&api.rsplit('/').next().unwrap().to_string()));

    // MCP
    let rpc = |body: serde_json::Value| {
        let anon = &anon;
        let mcp = mcp.clone();
        async move {
            let res = anon.post(mcp).header(ContentType::JSON).body(body.to_string()).dispatch().await;
            let status = res.status();
            let text = res.into_string().await.unwrap_or_default();
            (status, serde_json::from_str::<serde_json::Value>(&text).unwrap_or(serde_json::Value::Null))
        }
    };
    let (s, v) = rpc(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}})).await;
    assert_eq!(s, Status::Ok);
    assert_eq!(v["result"]["serverInfo"]["name"], "nrzMusic");
    let (s, _) = rpc(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
    assert_eq!(s, Status::Accepted);
    let (_, v) = rpc(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).await;
    assert_eq!(v["result"]["tools"][0]["name"], "now_playing");
    let (_, v) = rpc(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "now_playing", "arguments": {}}})).await;
    let text = v["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("Song") && text.contains("iPhone"), "{text}");
    assert_eq!(v["result"]["structuredContent"]["title"], "Song");

    // Neue Adresse: alte tot
    let (_, st) = call(c, "POST", "/api/v1/presence/rotate", None).await;
    assert_ne!(st["api_path"].as_str().unwrap(), api);
    let (s, _) = call(&anon, "GET", api.clone(), None).await;
    assert_eq!(s, Status::NotFound);
    assert_eq!(anon.get(widget).dispatch().await.status(), Status::NotFound);

    // Ausschalten: nichts mehr da
    call(c, "PUT", "/api/v1/presence", Some(json!({"enabled": false, "public": true}))).await;
    let (_, now) = call(c, "GET", "/api/v1/now-playing", None).await;
    assert!(now.is_null());
    let (s, _) = call(&anon, "GET", st["api_path"].as_str().unwrap().to_string(), None).await;
    assert_eq!(s, Status::NotFound);
}
