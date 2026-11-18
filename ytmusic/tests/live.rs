//! Tests gegen das echte YouTube Music (Netzwerk nötig). Laufen nur auf Wunsch:
//! `cargo test -p nrzmusic-ytmusic --test live -- --ignored --nocapture`

use nrzmusic_ytmusic::{playlist_id_from_url, Error, SearchFilter, YtItem, YtMusic, YtTrack};

const DAFT_PUNK: &str = "UCRr1xG_2WIDs18a6cIiCxeA";
const RANDOM_ACCESS_MEMORIES: &str = "MPREb_K8qWMWVqXGi";
/// Community-Playlist mit knapp 500 Titeln (mehrere Fortsetzungen)
const BIG_PLAYLIST: &str = "PLciALUPf8sIJlvH350O8PWh4UbSbOffSJ";

fn line(t: &YtTrack) -> String {
    format!(
        "{} | {} – {} | {:?} ms | album {:?} | video {} | {:?}",
        t.video_id,
        t.artist_line(),
        t.title,
        t.duration_ms,
        t.album.as_ref().map(|a| a.title.as_str()),
        t.is_video,
        t.thumbnail
    )
}

#[tokio::test]
#[ignore]
async fn live_search_mixed() {
    let yt = YtMusic::new();
    let r = yt.search("Daft Punk", None).await.unwrap();
    println!(
        "songs {} videos {} albums {} artists {} playlists {}",
        r.songs.len(),
        r.videos.len(),
        r.albums.len(),
        r.artists.len(),
        r.playlists.len()
    );
    for s in r.songs.iter().take(3) {
        println!("song   {}", line(s));
    }
    for v in r.videos.iter().take(2) {
        println!("video  {}", line(v));
    }
    for a in r.albums.iter().take(2) {
        println!(
            "album  {} | {} | {} {:?}",
            a.browse_id, a.title, a.kind, a.year
        );
    }
    for a in r.artists.iter().take(2) {
        println!("artist {} | {} | {:?}", a.browse_id, a.name, a.subscribers);
    }
    for p in r.playlists.iter().take(2) {
        println!(
            "list   {} | {} | {:?} | {:?}",
            p.playlist_id, p.title, p.author, p.track_count
        );
    }
    assert!(!r.songs.is_empty() && !r.albums.is_empty() && !r.artists.is_empty());
    assert!(r.artists.iter().any(|a| a.browse_id == DAFT_PUNK));
}

#[tokio::test]
#[ignore]
async fn live_search_filtered() {
    let yt = YtMusic::new();
    let songs = yt
        .search("Daft Punk", Some(SearchFilter::Songs))
        .await
        .unwrap();
    println!("songs: {}", songs.songs.len());
    for s in songs.songs.iter().take(3) {
        println!("  {}", line(s));
    }
    assert!(songs.songs.len() > 20, "Fortsetzung geladen");
    assert!(songs.songs.iter().all(|s| s.duration_ms.is_some()));
    assert!(songs.videos.is_empty() && songs.albums.is_empty());

    for f in [
        SearchFilter::Videos,
        SearchFilter::Albums,
        SearchFilter::Artists,
        SearchFilter::Playlists,
    ] {
        let r = yt.search("Daft Punk", Some(f)).await.unwrap();
        let n = [
            r.songs.len(),
            r.videos.len(),
            r.albums.len(),
            r.artists.len(),
            r.playlists.len(),
        ];
        println!("{f:?}: {n:?}");
        assert_eq!(
            n.iter().filter(|&&x| x > 0).count(),
            1,
            "{f:?} liefert nur eine Kategorie"
        );
    }
    let none = yt.search("   ", None).await.unwrap();
    assert!(none.songs.is_empty());
}

#[tokio::test]
#[ignore]
async fn live_suggestions() {
    let yt = YtMusic::new();
    let s = yt.suggestions("daft p").await.unwrap();
    println!("{s:?}");
    assert!(s.iter().any(|x| x.contains("daft punk")));
}

#[tokio::test]
#[ignore]
async fn live_album() {
    let yt = YtMusic::new();
    let a = yt.album(RANDOM_ACCESS_MEMORIES).await.unwrap();
    println!(
        "{} – {} ({:?}, {}) {} Titel, Playlist {:?}",
        a.summary.artists[0].name,
        a.summary.title,
        a.summary.year,
        a.summary.kind,
        a.tracks.len(),
        a.playlist_id
    );
    for t in a.tracks.iter().take(3) {
        println!("  {}", line(t));
    }
    assert_eq!(a.tracks.len(), 13);
    assert!(a
        .playlist_id
        .as_deref()
        .is_some_and(|p| p.starts_with("OLAK5uy_")));
    // Zweiter Aufruf kommt aus dem Zwischenspeicher
    let again = yt.album(RANDOM_ACCESS_MEMORIES).await.unwrap();
    assert_eq!(again, a);
    assert!(matches!(
        yt.album("MPREb_XXXXXXXXXXXXX").await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
#[ignore]
async fn live_playlist_with_continuations() {
    let yt = YtMusic::new();
    let p = yt.playlist(&format!("VL{BIG_PLAYLIST}")).await.unwrap();
    println!(
        "{} von {:?}: {} Titel geladen (angezeigt: {:?})",
        p.summary.title,
        p.summary.author,
        p.tracks.len(),
        p.summary.track_count
    );
    for t in p.tracks.iter().take(2) {
        println!("  {}", line(t));
    }
    assert!(p.tracks.len() > 300, "nur {} Titel", p.tracks.len());
    assert_eq!(p.summary.playlist_id, BIG_PLAYLIST);

    // Album-Playlist und Link
    let url = "https://music.youtube.com/playlist?list=OLAK5uy_kNhM2yaBTOVwrcZJepB1C9P3-n5_Sfy5c";
    let id = playlist_id_from_url(url).unwrap();
    let album = yt.playlist(&id).await.unwrap();
    println!(
        "Album-Playlist: {} ({} Titel)",
        album.summary.title,
        album.tracks.len()
    );
    assert_eq!(album.tracks.len(), 13);

    // Mix ohne eigene Seite
    let mix = yt.playlist("RDAMVMiRHo9cz2wFc").await.unwrap();
    println!("Mix: {} ({} Titel)", mix.summary.title, mix.tracks.len());
    assert!(!mix.tracks.is_empty());

    assert!(matches!(
        yt.playlist("PLxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx").await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
#[ignore]
async fn live_artist() {
    let yt = YtMusic::new();
    let a = yt.artist(DAFT_PUNK).await.unwrap();
    println!(
        "{} | {:?} | top {} | alben {} | singles {} | songs {:?}",
        a.summary.name,
        a.summary.subscribers,
        a.top_songs.len(),
        a.albums.len(),
        a.singles.len(),
        a.songs_playlist_id
    );
    for t in a.top_songs.iter().take(2) {
        println!("  {}", line(t));
    }
    for al in a.albums.iter().take(3) {
        println!("  album {} | {} | {:?}", al.browse_id, al.title, al.year);
    }
    assert_eq!(a.summary.name, "Daft Punk");
    assert!(!a.top_songs.is_empty() && !a.albums.is_empty() && !a.singles.is_empty());
    assert!(a.songs_playlist_id.is_some());
}

#[tokio::test]
#[ignore]
async fn live_radio() {
    let yt = YtMusic::new();
    let tracks = yt.radio("iRHo9cz2wFc").await.unwrap();
    println!("{} Titel", tracks.len());
    for t in tracks.iter().take(3) {
        println!("  {}", line(t));
    }
    assert!(tracks.len() > 10);
    assert!(tracks.iter().all(|t| t.video_id != "iRHo9cz2wFc"));
}

#[tokio::test]
#[ignore]
async fn live_home() {
    let yt = YtMusic::new();
    let shelves = yt.home().await.unwrap();
    for s in &shelves {
        let kinds: Vec<&str> = s
            .items
            .iter()
            .map(|i| match i {
                YtItem::Track(_) => "T",
                YtItem::Album(_) => "A",
                YtItem::Artist(_) => "K",
                YtItem::Playlist(_) => "P",
            })
            .collect();
        println!("{} ({}): {}", s.title, s.items.len(), kinds.join(""));
    }
    assert!(shelves.len() >= 3, "nur {} Regale", shelves.len());
}

#[tokio::test]
#[ignore]
async fn live_locale_switch() {
    let yt = YtMusic::new();
    yt.set_locale("en", "US");
    let r = yt
        .search("Daft Punk", Some(SearchFilter::Playlists))
        .await
        .unwrap();
    println!(
        "{:?}",
        r.playlists
            .iter()
            .take(3)
            .map(|p| (&p.title, &p.track_count))
            .collect::<Vec<_>>()
    );
    let a = yt.artist(DAFT_PUNK).await.unwrap();
    println!(
        "en: {:?}, albums {}, singles {}",
        a.summary.subscribers,
        a.albums.len(),
        a.singles.len()
    );
    assert!(a
        .summary
        .subscribers
        .as_deref()
        .is_some_and(|s| s.contains("subscribers")));
    assert!(!a.albums.is_empty() && !a.singles.is_empty());
}

#[tokio::test]
#[ignore]
async fn live_search_top_results() {
    let yt = YtMusic::new();
    // Top-Treffer ist ein Video bzw. ein Album – er steht in seiner Kategorie vorne
    let r = yt.search("get lucky daft punk", None).await.unwrap();
    let first = r.videos.first().or(r.songs.first()).unwrap();
    println!("get lucky: {}", line(first));
    assert!(first.title.contains("Get Lucky"));
    let r = yt.search("random access memories", None).await.unwrap();
    println!(
        "ram: {:?}",
        r.albums.first().map(|a| (&a.browse_id, &a.title, &a.kind))
    );
    assert_eq!(r.albums[0].browse_id, RANDOM_ACCESS_MEMORIES);

    let nothing = yt
        .search("qqzxqqzxqqzx wvwvwvkkj", Some(SearchFilter::Songs))
        .await
        .unwrap();
    println!("ohne Treffer: {} songs", nothing.songs.len());
}

#[tokio::test]
#[ignore]
async fn live_user_channel_and_small_artist() {
    let yt = YtMusic::new();
    // Hochlader eines Videos (kein Künstler) – Seite lässt sich trotzdem öffnen
    let u = yt.artist("UC-L8sbkj8DxzCbK80MEySXQ").await.unwrap();
    println!(
        "Kanal: {} {:?} top {} alben {}",
        u.summary.name,
        u.summary.thumbnail,
        u.top_songs.len(),
        u.albums.len()
    );
    assert!(!u.summary.name.is_empty());
    assert!(matches!(
        yt.artist("UCxxxxxxxxxxxxxxxxxxxxxx").await,
        Err(Error::NotFound)
    ));
}
