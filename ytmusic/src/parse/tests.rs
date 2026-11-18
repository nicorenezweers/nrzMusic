//! Offline-Tests gegen echte, einmal aufgezeichnete Antworten (`tests/fixtures/*.json`,
//! hl=de, gl=CH; Tracking-Felder und Kontextmenüs entfernt, lange Listen gekürzt).

use serde_json::{json, Value};

use super::search::{parse_search, parse_search_continuation, parse_suggestions, SearchCollector};
use super::*;
use crate::{Error, SearchFilter};

fn fixture(name: &str) -> Value {
    let text = match name {
        "search_mixed" => include_str!("../../tests/fixtures/search_mixed.json"),
        "search_songs" => include_str!("../../tests/fixtures/search_songs.json"),
        "suggestions" => include_str!("../../tests/fixtures/suggestions.json"),
        "album" => include_str!("../../tests/fixtures/album.json"),
        "playlist" => include_str!("../../tests/fixtures/playlist.json"),
        "playlist_continuation" => include_str!("../../tests/fixtures/playlist_continuation.json"),
        "artist" => include_str!("../../tests/fixtures/artist.json"),
        "next" => include_str!("../../tests/fixtures/next.json"),
        "home" => include_str!("../../tests/fixtures/home.json"),
        _ => panic!("unbekannte Fixture {name}"),
    };
    serde_json::from_str(text).expect("Fixture ist gültiges JSON")
}

const DAFT_PUNK: &str = "UCRr1xG_2WIDs18a6cIiCxeA";

/// YouTube trennt Zahl und Einheit mit geschütztem Leerzeichen
fn nbsp(s: &str) -> String {
    s.replace('\u{a0}', " ")
}

fn is_big_thumb(url: &Option<String>) -> bool {
    url.as_deref().is_some_and(|u| {
        u.contains("=w544-h544") || u.contains("=s5") || u.ends_with("/hqdefault.jpg")
    })
}

#[test]
fn search_mixed_fills_every_category() {
    let resp = fixture("search_mixed");
    let mut c = SearchCollector::new(None);
    let token = parse_search(&resp, &mut c).unwrap();
    assert_eq!(token, None, "gemischte Suche hat keine Fortsetzung");
    let r = c.results;

    // Top-Karte: Künstler zuerst, mit Kanal und "Zuschauer/Monat"
    let top = &r.artists[0];
    assert_eq!(top.name, "Daft Punk");
    assert_eq!(top.browse_id, DAFT_PUNK);
    assert!(
        top.subscribers
            .as_deref()
            .is_some_and(|s| s.starts_with("68,3")),
        "{:?}",
        top.subscribers
    );
    assert!(is_big_thumb(&top.thumbnail), "{:?}", top.thumbnail);
    assert!(r.artists.iter().any(|a| a.name == "The Weeknd"));
    assert!(r.artists.iter().all(|a| a.browse_id.starts_with("UC")));

    // Titel der Karte bekommen den Künstler der Karte und ihre Dauer
    let night = r
        .songs
        .iter()
        .find(|s| s.video_id == "iRHo9cz2wFc")
        .expect("Nightvision");
    assert_eq!(night.title, "Nightvision");
    assert_eq!(night.duration_ms, Some(105_000));
    assert_eq!(night.artists[0].id.as_deref(), Some(DAFT_PUNK));
    assert!(!night.is_video);

    // Songs aus den Einzelzeilen: Künstler mit ID, keine Typangabe als Künstler
    let atw = r
        .songs
        .iter()
        .find(|s| s.title == "Around the World (Radio Edit)")
        .expect("Around the World");
    assert_eq!(atw.artists.len(), 1);
    assert_eq!(atw.artists[0].name, "Daft Punk");
    assert!(r.songs.iter().all(|s| s
        .artists
        .iter()
        .all(|a| a.name != "Titel" && a.name != "Song")));
    let globe = r
        .songs
        .iter()
        .find(|s| s.title.starts_with("AROUND THE GLOBE"))
        .expect("Mehrere Künstler");
    assert_eq!(
        globe
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["Unjaps", "404NR", "June Boii"]
    );

    // Videos
    assert!(r.videos.len() >= 3, "{:?}", r.videos);
    assert!(r.videos.iter().all(|v| v.is_video));
    assert!(r.videos.iter().any(|v| v.title == "Something About Us"));

    // Alben
    let ram = r
        .albums
        .iter()
        .find(|a| a.browse_id == "MPREb_K8qWMWVqXGi")
        .expect("Random Access Memories");
    assert_eq!(ram.title, "Random Access Memories");
    assert_eq!(ram.kind, "Album");
    assert_eq!(ram.year.as_deref(), Some("2013"));
    assert_eq!(ram.artists[0].id.as_deref(), Some(DAFT_PUNK));
    assert!(is_big_thumb(&ram.thumbnail));

    // Playlists: ohne "VL", Autor, Titelanzahl nur wo angegeben
    let presenting = r
        .playlists
        .iter()
        .find(|p| p.title == "Presenting Daft Punk")
        .expect("Presenting");
    assert_eq!(
        presenting.playlist_id,
        "RDCLAK5uy_n20FRYQXNt1p1wS55Nj2r14IouO5weaYU"
    );
    assert_eq!(presenting.author.as_deref(), Some("YouTube Music"));
    assert_eq!(
        presenting.track_count.as_deref().map(nbsp),
        Some("35 Songs".into())
    );
    let community = r
        .playlists
        .iter()
        .find(|p| p.title == "The Black Eyed Peas/Daft Punk")
        .expect("Community");
    assert_eq!(community.author.as_deref(), Some("Pedro Henrique"));
    assert_eq!(community.track_count, None);
    assert!(r
        .playlists
        .iter()
        .all(|p| !p.playlist_id.starts_with("VL") && !p.playlist_id.starts_with("MPSP")));

    // Folgen, Podcasts und Profile tauchen nirgends auf
    assert!(r
        .songs
        .iter()
        .chain(&r.videos)
        .all(|t| !t.title.contains("Omar Hakim")));
    assert!(r.playlists.iter().all(|p| p.title != "Season 2"));
    assert!(r.artists.iter().all(|a| a.name != "dafter"));
}

#[test]
fn search_filtered_songs() {
    let resp = fixture("search_songs");
    let mut c = SearchCollector::new(Some(SearchFilter::Songs));
    let token = parse_search(&resp, &mut c).unwrap();
    assert!(token.is_some(), "gefilterte Suche hat eine Fortsetzung");
    let r = c.results;
    assert_eq!(r.songs.len(), 20);
    assert!(
        r.videos.is_empty()
            && r.albums.is_empty()
            && r.artists.is_empty()
            && r.playlists.is_empty()
    );
    for s in &r.songs {
        assert!(!s.video_id.is_empty());
        assert!(s.duration_ms.is_some(), "Dauer fehlt: {}", s.title);
        assert!(!s.artists.is_empty(), "Künstler fehlt: {}", s.title);
        assert!(s.album.is_some(), "Album fehlt: {}", s.title);
        assert!(!s.is_video);
        assert!(is_big_thumb(&s.thumbnail), "{:?}", s.thumbnail);
    }
    let vq = r
        .songs
        .iter()
        .find(|s| s.title == "Veridis Quo (Edit)")
        .unwrap();
    assert_eq!(vq.duration_ms, Some(165_000));
    assert_eq!(vq.album.as_ref().unwrap().title, "Veridis Quo");
    assert!(vq
        .album
        .as_ref()
        .unwrap()
        .id
        .as_deref()
        .unwrap()
        .starts_with("MPRE"));
    let explicit: Vec<&str> = r
        .songs
        .iter()
        .filter(|s| s.explicit)
        .map(|s| s.video_id.as_str())
        .collect();
    assert_eq!(
        explicit,
        ["pHWj-hRP-cY"],
        "nur \"Touch It / Technologic\" trägt das E-Badge"
    );

    // Fortsetzung (gleicher Aufbau wie musicShelfContinuation)
    let cont = json!({"continuationContents": {"musicShelfContinuation": resp
        .pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/1/musicShelfRenderer")
        .unwrap()
        .clone()}});
    let mut c2 = SearchCollector::new(Some(SearchFilter::Songs));
    let next = parse_search_continuation(&cont, &mut c2);
    assert_eq!(c2.results.songs.len(), 20);
    assert!(next.is_some());
}

#[test]
fn search_filter_drops_other_kinds() {
    // Ignoriert YouTube den Filter und liefert die gemischte Seite, bleibt nur die Kategorie
    let resp = fixture("search_mixed");
    let mut c = SearchCollector::new(Some(SearchFilter::Albums));
    parse_search(&resp, &mut c).unwrap();
    assert!(!c.results.albums.is_empty());
    assert!(c.results.songs.is_empty() && c.results.videos.is_empty());
    assert!(c.results.artists.is_empty() && c.results.playlists.is_empty());
}

#[test]
fn search_without_results_or_garbage() {
    let mut c = SearchCollector::new(None);
    assert_eq!(
        parse_search(&json!({"responseContext": {}}), &mut c).unwrap(),
        None
    );
    assert_eq!(c.results, Default::default());
    assert!(matches!(
        parse_search(&json!({"contents": {"foo": 1}}), &mut c),
        Err(Error::Parse(_))
    ));
    // Kaputte Zeilen werden übersprungen
    let resp = json!({"contents": {"sectionListRenderer": {"contents": [
        {"itemSectionRenderer": {"contents": [{"musicResponsiveListItemRenderer": {"flexColumns": "x"}}]}},
        {"musicShelfRenderer": {"contents": [{"musicResponsiveListItemRenderer": null}, 42]}},
        {"musicCardShelfRenderer": {"title": {"runs": [{"text": "X", "navigationEndpoint": {"browseEndpoint": {}}}]}}}
    ]}}});
    assert_eq!(parse_search(&resp, &mut c).unwrap(), None);
    assert_eq!(c.results, Default::default());
}

#[test]
fn suggestions() {
    let s = parse_suggestions(&fixture("suggestions"));
    assert!(!s.is_empty());
    assert_eq!(s[0], "daft punk");
    assert!(s.iter().all(|x| x.starts_with("daft p")));
    assert!(parse_suggestions(&json!({})).is_empty());
    assert!(parse_suggestions(
        &json!({"contents": [1, {"searchSuggestionsSectionRenderer": {"contents": [{}]}}]})
    )
    .is_empty());
}

#[test]
fn album_page() {
    let a = album::parse_album(&fixture("album"), "MPREb_K8qWMWVqXGi").unwrap();
    assert_eq!(a.summary.title, "Random Access Memories");
    assert_eq!(a.summary.browse_id, "MPREb_K8qWMWVqXGi");
    assert_eq!(a.summary.kind, "Album");
    assert_eq!(a.summary.year.as_deref(), Some("2013"));
    assert_eq!(a.summary.artists.len(), 1);
    assert_eq!(a.summary.artists[0].id.as_deref(), Some(DAFT_PUNK));
    assert!(is_big_thumb(&a.summary.thumbnail));
    assert_eq!(
        a.playlist_id.as_deref(),
        Some("OLAK5uy_kNhM2yaBTOVwrcZJepB1C9P3-n5_Sfy5c")
    );
    assert!(a
        .description
        .as_deref()
        .is_some_and(|d| d.contains("Daft Punk")));

    assert_eq!(a.tracks.len(), 13);
    assert_eq!(a.tracks[0].title, "Give Life Back to Music");
    assert_eq!(a.tracks[0].video_id, "IluRBvnYMoY");
    assert_eq!(a.tracks[0].duration_ms, Some(276_000));
    assert_eq!(a.tracks[2].duration_ms, Some(545_000));
    for t in &a.tracks {
        assert_eq!(
            t.album.as_ref().map(|x| x.title.as_str()),
            Some("Random Access Memories")
        );
        assert_eq!(
            t.album.as_ref().and_then(|x| x.id.as_deref()),
            Some("MPREb_K8qWMWVqXGi")
        );
        assert_eq!(
            t.thumbnail, a.summary.thumbnail,
            "Titel bekommen das Albumcover"
        );
        assert!(!t.artists.is_empty());
        assert!(t.duration_ms.is_some());
    }
    let lucky = a
        .tracks
        .iter()
        .find(|t| t.title.starts_with("Get Lucky"))
        .unwrap();
    assert_eq!(
        lucky
            .artists
            .iter()
            .map(|x| x.name.as_str())
            .collect::<Vec<_>>(),
        ["Daft Punk", "Pharrell Williams", "Nile Rodgers"]
    );

    assert!(matches!(
        album::parse_album(&json!({"responseContext": {}}), "x"),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        album::parse_album(&json!({"contents": {}}), "x"),
        Err(Error::Parse(_))
    ));
}

#[test]
fn playlist_page_and_continuation() {
    let (p, token) =
        playlist::parse_playlist(&fixture("playlist"), "PLciALUPf8sIJlvH350O8PWh4UbSbOffSJ")
            .unwrap();
    assert_eq!(p.summary.playlist_id, "PLciALUPf8sIJlvH350O8PWh4UbSbOffSJ");
    assert_eq!(
        p.summary.title,
        "Rolling Stone Magazine's 500 Greatest Songs Of All Time (2004)"
    );
    assert_eq!(p.summary.author.as_deref(), Some("sweetchrisomine"));
    assert_eq!(
        p.summary.track_count.as_deref().map(nbsp),
        Some("496 Titel".into())
    );
    assert!(p.summary.thumbnail.is_some());
    assert_eq!(p.tracks.len(), 15);
    for t in &p.tracks {
        assert_eq!(t.video_id.len(), 11);
        assert!(t.duration_ms.is_some(), "{}", t.title);
        assert!(!t.artists.is_empty(), "{}", t.title);
        assert!(is_big_thumb(&t.thumbnail), "{:?}", t.thumbnail);
    }
    assert!(token.as_deref().is_some_and(|t| t.len() > 20));

    let (more, next) = playlist::parse_playlist_continuation(&fixture("playlist_continuation"));
    assert_eq!(more.len(), 10);
    assert!(more
        .iter()
        .all(|t| !t.video_id.is_empty() && t.duration_ms.is_some()));
    assert!(next.is_some());
    assert_eq!(
        playlist::parse_playlist_continuation(&json!({"foo": []})),
        (vec![], None)
    );

    assert!(matches!(
        playlist::parse_playlist(&json!({"responseContext": {}}), "PLx"),
        Err(Error::NotFound)
    ));
}

#[test]
fn artist_page() {
    let a = artist::parse_artist(&fixture("artist"), DAFT_PUNK).unwrap();
    assert_eq!(a.summary.name, "Daft Punk");
    assert_eq!(a.summary.browse_id, DAFT_PUNK);
    assert_eq!(
        a.summary.subscribers.as_deref().map(nbsp),
        Some("7,21 Mio. Abonnenten".into())
    );
    let thumb = a.summary.thumbnail.as_deref().unwrap();
    assert!(
        thumb.ends_with("=w544-h544-p-l90-rj"),
        "quadratisch zugeschnitten: {thumb}"
    );
    assert!(a
        .description
        .as_deref()
        .is_some_and(|d| d.contains("French-House")));
    assert_eq!(
        a.songs_playlist_id.as_deref(),
        Some("OLAK5uy_lNVBcjNtiCwq-n95uoxJ-Gd4vsfMkyZNs")
    );

    assert_eq!(a.top_songs.len(), 5);
    assert_eq!(
        a.top_songs[0].title,
        "Instant Crush (feat. Julian Casablancas)"
    );
    assert_eq!(
        a.top_songs[0].album.as_ref().map(|x| x.title.as_str()),
        Some("Random Access Memories")
    );
    assert!(a
        .top_songs
        .iter()
        .all(|t| t.artists.iter().any(|x| x.id.as_deref() == Some(DAFT_PUNK))));

    assert_eq!(a.albums.len(), 4);
    assert!(a
        .albums
        .iter()
        .all(|x| x.browse_id.starts_with("MPRE") && x.kind == "Album" && x.year.is_some()));
    assert_eq!(a.albums[1].title, "Random Access Memories");
    assert_eq!(a.singles.len(), 4);
    assert!(a.singles.iter().all(|x| x.kind == "Single"));
    assert!(a
        .singles
        .iter()
        .all(|x| x.artists[0].id.as_deref() == Some(DAFT_PUNK)));

    assert!(matches!(
        artist::parse_artist(&json!({}), "UCx"),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        artist::parse_artist(&json!({"contents": {}}), "UCx"),
        Err(Error::Parse(_))
    ));
}

#[test]
fn radio_queue() {
    let resp = fixture("next");
    let (tracks, _) = watch::parse_watch(&resp, Some("iRHo9cz2wFc")).unwrap();
    assert_eq!(tracks.len(), 11, "Ausgangstitel fällt weg");
    assert!(tracks.iter().all(|t| t.video_id != "iRHo9cz2wFc"));
    let vq = &tracks[0];
    assert_eq!(vq.title, "Veridis Quo");
    assert_eq!(vq.duration_ms, Some(346_000));
    assert_eq!(vq.artists[0].id.as_deref(), Some(DAFT_PUNK));
    assert_eq!(
        vq.album.as_ref().map(|a| a.title.as_str()),
        Some("Something About Us (Love Theme from Interstella)")
    );
    assert!(tracks
        .iter()
        .all(|t| t.duration_ms.is_some() && is_big_thumb(&t.thumbnail)));
    assert!(tracks.iter().all(|t| t
        .artists
        .iter()
        .all(|a| a.name != "2024" && a.name != "und")));

    let (all, _) = watch::parse_watch(&resp, None).unwrap();
    assert_eq!(all.len(), 12);
    assert!(matches!(
        watch::parse_watch(&json!({}), None),
        Err(Error::NotFound)
    ));
}

#[test]
fn home_shelves() {
    let (shelves, token) = home::parse_home(&fixture("home")).unwrap();
    assert!(token.is_some());
    assert_eq!(shelves.len(), 2, "Tastebuilder fällt weg");
    for shelf in &shelves {
        assert!(!shelf.title.is_empty());
        assert!(!shelf.items.is_empty());
    }
    use nrzmusic_types::YtItem;
    let quick = &shelves[0];
    assert!(quick
        .items
        .iter()
        .all(|i| matches!(i, YtItem::Track(t) if !t.artists.is_empty())));
    if let YtItem::Track(t) = &quick.items[0] {
        assert_eq!(t.title, "Fille à Papa");
        assert!(t.album.is_some());
    }
    let throwback = &shelves[1];
    assert!(throwback.items.iter().all(|i| matches!(i, YtItem::Playlist(p) if p.playlist_id.starts_with("RDCLAK") && p.author.is_none())));

    assert!(matches!(home::parse_home(&json!({})), Err(Error::Parse(_))));
    assert_eq!(home::parse_home_continuation(&json!({})), (vec![], None));
}

#[test]
fn fixtures_never_panic_on_wrong_parser() {
    // Jede Antwort durch jeden Parser – Fehler sind erlaubt, Panik nicht
    for name in [
        "search_mixed",
        "search_songs",
        "suggestions",
        "album",
        "playlist",
        "playlist_continuation",
        "artist",
        "next",
        "home",
    ] {
        let v = fixture(name);
        let _ = album::parse_album(&v, "x");
        let _ = artist::parse_artist(&v, "x");
        let _ = playlist::parse_playlist(&v, "x");
        let _ = playlist::parse_playlist_continuation(&v);
        let _ = watch::parse_watch(&v, None);
        let _ = home::parse_home(&v);
        let _ = parse_suggestions(&v);
        let mut c = SearchCollector::new(None);
        let _ = parse_search(&v, &mut c);
        let _ = parse_search_continuation(&v, &mut c);
    }
}
