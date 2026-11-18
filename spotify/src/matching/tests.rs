use super::*;
use nrzmusic_types::{YtAlbumRef, YtArtistRef};

fn sp(name: &str, artists: &[&str], album: &str, secs: u64) -> SpTrack {
    SpTrack {
        id: "sp".into(),
        name: name.into(),
        artists: artists.iter().map(|a| a.to_string()).collect(),
        album: album.into(),
        duration_ms: secs * 1000,
        image: None,
        isrc: None,
        explicit: false,
        added_at: None,
        youtube: None,
    }
}

fn song(title: &str, artists: &[&str], album: Option<&str>, secs: Option<u64>) -> YtTrack {
    YtTrack {
        video_id: "vid".into(),
        title: title.into(),
        artists: artists
            .iter()
            .map(|a| YtArtistRef {
                name: a.to_string(),
                id: None,
            })
            .collect(),
        album: album.map(|a| YtAlbumRef {
            title: a.into(),
            id: None,
        }),
        duration_ms: secs.map(|s| s * 1000),
        thumbnail: None,
        explicit: false,
        is_video: false,
    }
}

fn video(title: &str, artists: &[&str], secs: Option<u64>) -> YtTrack {
    YtTrack {
        is_video: true,
        ..song(title, artists, None, secs)
    }
}

#[track_caller]
fn assert_match(track: &SpTrack, cand: &YtTrack) -> f64 {
    let s = match_score(track, cand);
    assert!(
        s >= MATCH_THRESHOLD,
        "erwartet Treffer: {:?} ↔ {:?}: {s:.3}",
        track.name,
        cand.title
    );
    s
}

#[track_caller]
fn assert_no_match(track: &SpTrack, cand: &YtTrack) -> f64 {
    let s = match_score(track, cand);
    assert!(
        s < MATCH_THRESHOLD,
        "erwartet kein Treffer: {:?} ↔ {:?}: {s:.3}",
        track.name,
        cand.title
    );
    s
}

// ---------------------------------------------------------------- Bausteine

#[test]
fn normalize_folds_accents_and_punctuation() {
    assert_eq!(normalize("Beyoncé"), "beyonce");
    assert_eq!(normalize("Mädchen aus Ostberlin"), "madchen aus ostberlin");
    assert_eq!(normalize("Ça plane pour moi"), "ca plane pour moi");
    assert_eq!(normalize("Straße"), "strasse");
    assert_eq!(normalize("Sigur Rós – Hoppípolla"), "sigur ros hoppipolla");
    assert_eq!(normalize("Don't Stop Me Now"), "dont stop me now");
    assert_eq!(normalize("Don’t"), "dont");
    assert_eq!(normalize("Simon & Garfunkel"), "simon and garfunkel");
    assert_eq!(normalize("Mr. Brightside!!"), "mr brightside");
    assert_eq!(normalize("Ke$ha"), "kesha");
    // zerlegte Schreibweise (e + kombinierender Akut)
    assert_eq!(normalize("Beyonce\u{301}"), "beyonce");
    assert_eq!(normalize("Ølstykke Æble Œuvre"), "olstykke aeble oeuvre");
    // nicht-lateinische Schrift bleibt erhalten
    assert_eq!(normalize("Мумий Тролль"), "мумий тролль");
    assert_eq!(normalize("  "), "");
}

#[test]
fn phrases_match_whole_words_only() {
    assert!(has_phrase("song live at wembley", "live at"));
    assert!(has_phrase("live", "live"));
    assert!(!has_phrase("alive", "live"));
    assert!(!has_phrase("lively song", "live"));
    assert!(has_phrase("a b c", "c"));
    assert!(!has_phrase("abc", ""));
}

#[test]
fn similarity_basics() {
    assert_eq!(lev_sim("abc", "abc"), 1.0);
    assert_eq!(lev_sim("", ""), 1.0);
    assert!((lev_sim("kitten", "sitting") - (1.0 - 3.0 / 7.0)).abs() < 1e-9);
    assert_eq!(token_dice("a b", "b a"), 1.0);
    assert!(token_dice("madchen aus ostberlin", "maedchen aus ostberlin") > 0.9);
    assert_eq!(token_dice("love story", "hello"), 0.0);
    assert_eq!(text_sim("hey jude", "heyjude"), 0.97);
    assert_eq!(text_sim("", "x"), 0.0);
}

#[test]
fn title_noise_is_removed() {
    let t = |s: &str| parse_title(s, &[]).full();
    assert_eq!(
        t("Bohemian Rhapsody - Remastered 2011"),
        "bohemian rhapsody"
    );
    assert_eq!(t("Don't Stop Me Now - 2011 Remaster"), "dont stop me now");
    assert_eq!(
        t("Get Lucky (feat. Pharrell Williams & Nile Rodgers) - Radio Edit"),
        "get lucky"
    );
    assert_eq!(t("Stay (with Justin Bieber)"), "stay");
    assert_eq!(
        t("Lose Yourself - From \"8 Mile\" Soundtrack"),
        "lose yourself"
    );
    assert_eq!(
        t("Let It Go - From \"Frozen\"/Soundtrack Version"),
        "let it go"
    );
    assert_eq!(t("Wonderwall - Remastered"), "wonderwall");
    assert_eq!(t("Heroes - 2017 Remaster"), "heroes");
    assert_eq!(t("Help! - Mono Version"), "help");
    assert_eq!(t("Song feat. Somebody"), "song");
    assert_eq!(t("Song ft. Somebody (Official Video) [HD]"), "song");
    assert_eq!(t("Bad Guy (Official Music Video)"), "bad guy");
    assert_eq!(t("Bad Guy Official Video"), "bad guy");
    assert_eq!(t("Bad Guy (Lyrics)"), "bad guy");
    // Versionen und sonstige Zusätze bleiben im ganzen Titel
    assert_eq!(t("Song - Live at Wembley 1986"), "song live at wembley");
    assert_eq!(t("Love Story (Taylor's Version)"), "love story taylors");
    assert_eq!(t("Song, Pt. 2"), "song pt 2");
    // Nur Klammer
    assert_eq!(t("[Untitled]"), "untitled");
}

#[test]
fn title_tags() {
    let tags = |s: &str| parse_title(s, &[]).tags;
    assert_eq!(
        tags("Live Forever"),
        0,
        "Live im Haupttitel ist kein Live-Titel"
    );
    assert_eq!(tags("Live and Let Die"), 0);
    assert_ne!(tags("Live Forever (Live at Knebworth)"), 0);
    assert_ne!(tags("Song - Live"), 0);
    assert_ne!(tags("Song Live at Wembley"), 0);
    assert_ne!(tags("Song (Karaoke Version)"), 0);
    assert_ne!(tags("Song Karaoke"), 0);
    assert_ne!(tags("Song (In the Style of Queen)"), 0);
    assert_ne!(tags("Song [8D AUDIO]"), 0);
    assert_ne!(tags("Song (Slowed + Reverb)"), 0);
    assert_ne!(tags("song sped up"), 0);
    assert_ne!(tags("Song - Kygo Remix"), 0);
    assert_ne!(tags("Song (Acoustic)"), 0);
    assert_ne!(tags("Song (Instrumental)"), 0);
    assert_eq!(tags("Cover Me"), 0, "Cover im Haupttitel ist kein Cover");
    assert_ne!(tags("Song (Cover)"), 0);
    assert_eq!(tags("Song - Remastered 2011"), 0);
    assert_eq!(tags("Song (feat. X)"), 0);
}

#[test]
fn artist_prefix_of_videos_is_stripped() {
    let artists = vec!["queen".to_string()];
    assert_eq!(
        parse_title(
            "Queen - Bohemian Rhapsody (Official Video Remastered)",
            &artists
        )
        .full(),
        "bohemian rhapsody"
    );
    assert_eq!(
        parse_title("Bohemian Rhapsody | Queen", &artists).full(),
        "bohemian rhapsody"
    );
    // Ohne passenden Künstler bleibt der Zusatz
    assert_eq!(parse_title("Intro - Outro", &artists).full(), "intro outro");
}

#[test]
fn artist_keys_and_similarity() {
    assert_eq!(artist_keys("The Beatles"), vec!["beatles"]);
    assert_eq!(
        artist_keys("Simon & Garfunkel"),
        vec!["simon and garfunkel", "simon", "garfunkel"]
    );
    assert!(artist_keys("Daft Punk - Topic").contains(&"daft punk".to_string()));
    assert!(artist_keys("QueenVEVO").contains(&"queen".to_string()));
    assert!(artist_keys("Macklemore & Ryan Lewis").contains(&"ryan lewis".to_string()));
    assert_eq!(artist_pair_sim("beyonce", "beyonce"), 1.0);
    assert_eq!(artist_pair_sim("jay z", "jayz"), 1.0);
    assert!(artist_pair_sim("pnk", "pink") >= ARTIST_MIN_SIM);
    assert_eq!(artist_pair_sim("adele", "abba"), 0.0);
    assert_eq!(
        artist_pair_sim("macklemore", "macklemore and ryan lewis"),
        0.85
    );
}

#[test]
fn duration_curve() {
    assert_eq!(duration_factor(200_000, Some(201_000), false), 1.0);
    assert_eq!(duration_factor(200_000, Some(203_000), false), 1.0);
    assert!(duration_factor(200_000, Some(210_000), false) > 0.85);
    assert!(duration_factor(200_000, Some(215_000), false) >= 0.75);
    assert!(duration_factor(200_000, Some(216_000), false) < 0.56);
    assert!(duration_factor(200_000, Some(260_000), false) < 0.31);
    assert_eq!(duration_factor(200_000, Some(500_000), false), 0.25);
    assert_eq!(duration_factor(200_000, None, false), UNKNOWN_DURATION);
    assert_eq!(duration_factor(0, Some(1), false), UNKNOWN_DURATION);
    // Videos: halbe Abweichung zählt (Intro/Outro)
    assert!(duration_factor(200_000, Some(230_000), true) >= 0.75);
}

// ---------------------------------------------------------------- Suchanfrage

#[test]
fn query_without_noise() {
    let q = |name: &str, artists: &[&str]| match_query(&sp(name, artists, "", 200));
    assert_eq!(
        q("Bohemian Rhapsody - Remastered 2011", &["Queen"]),
        "Queen Bohemian Rhapsody"
    );
    assert_eq!(
        q(
            "Get Lucky (feat. Pharrell Williams & Nile Rodgers) - Radio Edit",
            &["Daft Punk", "Pharrell Williams"]
        ),
        "Daft Punk Get Lucky"
    );
    assert_eq!(
        q(
            "Stay (with Justin Bieber)",
            &["The Kid LAROI", "Justin Bieber"]
        ),
        "The Kid LAROI Stay"
    );
    assert_eq!(q("Song ft. Somebody", &["A"]), "A Song");
    assert_eq!(
        q("Lose Yourself - From \"8 Mile\" Soundtrack", &["Eminem"]),
        "Eminem Lose Yourself"
    );
    // Versionen bleiben
    assert_eq!(q("Wonderwall - Live", &["Oasis"]), "Oasis Wonderwall Live");
    assert_eq!(
        q("Faded - Kygo Remix", &["Alan Walker"]),
        "Alan Walker Faded Kygo Remix"
    );
    assert_eq!(
        q("Love Story (Taylor's Version)", &["Taylor Swift"]),
        "Taylor Swift Love Story Taylor's Version"
    );
    assert_eq!(
        q(
            "(I Can't Get No) Satisfaction - Mono Version",
            &["The Rolling Stones"]
        ),
        "The Rolling Stones I Can't Get No Satisfaction"
    );
    // Ohne Künstler, leere Künstler
    assert_eq!(q("Song", &[]), "Song");
    assert_eq!(q("Song", &["", "B"]), "B Song");
    assert_eq!(q("  Spaces   everywhere ", &[" A "]), "A Spaces everywhere");
}

// ---------------------------------------------------------------- Abgleich

#[test]
fn identical_song_scores_high() {
    let t = sp("Bohemian Rhapsody", &["Queen"], "A Night At The Opera", 354);
    let s = assert_match(
        &t,
        &song(
            "Bohemian Rhapsody",
            &["Queen"],
            Some("A Night At The Opera"),
            Some(355),
        ),
    );
    assert!(s > 0.95, "{s}");
}

#[test]
fn remaster_suffix_matches_plain_title() {
    let t = sp(
        "Bohemian Rhapsody - Remastered 2011",
        &["Queen"],
        "A Night At The Opera (2011 Remaster)",
        354,
    );
    let s = assert_match(
        &t,
        &song(
            "Bohemian Rhapsody",
            &["Queen"],
            Some("A Night at the Opera"),
            Some(355),
        ),
    );
    assert!(s > 0.95, "{s}");
    let t = sp(
        "Heroes - 2017 Remaster",
        &["David Bowie"],
        "\"Heroes\" (2017 Remaster)",
        371,
    );
    assert_match(&t, &song("\"Heroes\"", &["David Bowie"], None, Some(372)));
}

#[test]
fn feat_in_title_vs_artist_list() {
    // Spotify: feat. im Titel, YouTube: als zweiter Künstler
    let t = sp(
        "Get Lucky (feat. Pharrell Williams & Nile Rodgers) - Radio Edit",
        &["Daft Punk", "Pharrell Williams", "Nile Rodgers"],
        "Get Lucky",
        248,
    );
    assert_match(
        &t,
        &song(
            "Get Lucky",
            &["Daft Punk", "Pharrell Williams", "Nile Rodgers"],
            None,
            Some(249),
        ),
    );
    // umgekehrt: YouTube hat feat. im Titel, Spotify nur in der Künstlerliste
    let t = sp(
        "Stay",
        &["The Kid LAROI", "Justin Bieber"],
        "F*CK LOVE 3",
        141,
    );
    assert_match(
        &t,
        &song(
            "Stay (feat. Justin Bieber)",
            &["The Kid LAROI"],
            None,
            Some(142),
        ),
    );
    // YouTube nennt nur den Gast
    assert_match(&t, &song("STAY", &["Justin Bieber"], None, Some(141)));
}

#[test]
fn spelling_variants_match() {
    let t = sp(
        "Don't Stop Me Now - Remastered 2011",
        &["Queen"],
        "Jazz",
        209,
    );
    assert_match(&t, &song("Don’t Stop Me Now", &["Queen"], None, Some(210)));
    let t = sp("Mr. Brightside", &["The Killers"], "Hot Fuss", 222);
    assert_match(&t, &song("Mr Brightside", &["Killers"], None, Some(223)));
    let t = sp(
        "Sweet Child O' Mine",
        &["Guns N' Roses"],
        "Appetite For Destruction",
        356,
    );
    assert_match(
        &t,
        &song("Sweet Child of Mine", &["Guns N Roses"], None, Some(356)),
    );
    let t = sp("Paint It, Black", &["The Rolling Stones"], "Aftermath", 202);
    assert_match(
        &t,
        &song("Paint It Black", &["The Rolling Stones"], None, Some(203)),
    );
    let t = sp("Hey Jude", &["The Beatles"], "1", 431);
    assert_match(&t, &song("HeyJude", &["Beatles"], None, Some(431)));
}

#[test]
fn umlauts_and_accents_match() {
    let t = sp("Mädchen aus Ostberlin", &["Die Ärzte"], "Debil", 160);
    let s = assert_match(
        &t,
        &song("Maedchen aus Ostberlin", &["Die Aerzte"], None, Some(161)),
    );
    assert!(s > 0.75, "{s}");
    assert_match(
        &t,
        &song("Madchen aus Ostberlin", &["Die Arzte"], None, Some(160)),
    );
    let t = sp("Ça plane pour moi", &["Plastic Bertrand"], "AN1", 182);
    assert_match(
        &t,
        &song("Ca Plane Pour Moi", &["Plastic Bertrand"], None, Some(181)),
    );
    let t = sp("Hoppípolla", &["Sigur Rós"], "Takk...", 268);
    assert_match(&t, &song("Hoppipolla", &["Sigur Ros"], None, Some(268)));
    let t = sp("Halo", &["Beyoncé"], "I AM...SASHA FIERCE", 261);
    assert_match(&t, &song("Halo", &["Beyonce"], None, Some(262)));
    let t = sp("Мой рок-н-ролл", &["Би-2"], "Иномарки", 260);
    assert_match(&t, &song("Мой рок-н-ролл", &["Би-2"], None, Some(260)));
}

#[test]
fn live_karaoke_cover_decoys_are_rejected() {
    let t = sp(
        "Wonderwall - Remastered",
        &["Oasis"],
        "(What's The Story) Morning Glory?",
        258,
    );
    let good = assert_match(&t, &song("Wonderwall", &["Oasis"], None, Some(259)));
    let live = assert_no_match(
        &t,
        &song(
            "Wonderwall (Live at Knebworth)",
            &["Oasis"],
            None,
            Some(262),
        ),
    );
    let live_album = assert_no_match(
        &t,
        &song(
            "Wonderwall",
            &["Oasis"],
            Some("Familiar to Millions (Live)"),
            Some(260),
        ),
    );
    let karaoke = assert_no_match(
        &t,
        &song(
            "Wonderwall (Karaoke Version)",
            &["Ameritz Karaoke"],
            None,
            Some(258),
        ),
    );
    let style = assert_no_match(
        &t,
        &video(
            "Wonderwall (In the Style of Oasis) [Karaoke Version]",
            &["Sing2Music"],
            Some(258),
        ),
    );
    let cover = assert_no_match(
        &t,
        &video(
            "Wonderwall - Oasis (Cover by Jane Doe)",
            &["Jane Doe"],
            Some(250),
        ),
    );
    let other_artist = assert_no_match(&t, &song("Wonderwall", &["Ryan Adams"], None, Some(255)));
    for bad in [live, live_album, karaoke, style, cover, other_artist] {
        assert!(bad < good);
    }
}

#[test]
fn edits_and_versions_are_rejected() {
    let t = sp("Blinding Lights", &["The Weeknd"], "After Hours", 200);
    assert_match(
        &t,
        &song(
            "Blinding Lights",
            &["The Weeknd"],
            Some("After Hours"),
            Some(200),
        ),
    );
    for decoy in [
        song(
            "Blinding Lights (Sped Up)",
            &["The Weeknd"],
            None,
            Some(160),
        ),
        song(
            "Blinding Lights - sped up",
            &["Speed Radio"],
            None,
            Some(160),
        ),
        song(
            "Blinding Lights (Slowed + Reverb)",
            &["The Weeknd"],
            None,
            Some(230),
        ),
        video(
            "The Weeknd - Blinding Lights [8D AUDIO]",
            &["8D Tunes"],
            Some(201),
        ),
        song(
            "Blinding Lights (Instrumental)",
            &["The Weeknd"],
            None,
            Some(200),
        ),
        song(
            "Blinding Lights (Chromatics Remix)",
            &["The Weeknd"],
            None,
            Some(301),
        ),
        song(
            "Blinding Lights (Acoustic)",
            &["The Weeknd"],
            None,
            Some(198),
        ),
        song("Blinding Lights", &["Nightcore Hits"], None, Some(170)),
        video("Blinding Lights 1 Hour", &["The Weeknd"], Some(3600)),
    ] {
        assert_no_match(&t, &decoy);
    }
}

#[test]
fn version_on_spotify_prefers_same_version() {
    let t = sp("Wonderwall - Live", &["Oasis"], "Familiar to Millions", 270);
    let live = assert_match(&t, &song("Wonderwall (Live)", &["Oasis"], None, Some(271)));
    // Studiofassung ist schlechter, aber kein Ausschluss
    let studio = match_score(&t, &song("Wonderwall", &["Oasis"], None, Some(258)));
    assert!(live > studio, "{live} > {studio}");

    let t = sp(
        "Faded - Kygo Remix",
        &["Alan Walker", "Kygo"],
        "Faded (Remixes)",
        290,
    );
    let remix = assert_match(
        &t,
        &song("Faded (Kygo Remix)", &["Alan Walker"], None, Some(290)),
    );
    let original = match_score(&t, &song("Faded", &["Alan Walker"], None, Some(212)));
    assert!(remix > original);
    assert!(original < MATCH_THRESHOLD);

    // Spotify-Titel heisst selbst "Live …" – kein Abzug
    let t = sp(
        "Live Forever - Remastered",
        &["Oasis"],
        "Definitely Maybe",
        276,
    );
    assert_match(&t, &song("Live Forever", &["Oasis"], None, Some(277)));
}

#[test]
fn duration_mismatch() {
    let t = sp("Shape of You", &["Ed Sheeran"], "÷", 233);
    let exact = assert_match(&t, &song("Shape of You", &["Ed Sheeran"], None, Some(234)));
    let close = assert_match(&t, &song("Shape of You", &["Ed Sheeran"], None, Some(241)));
    let off = assert_no_match(&t, &song("Shape of You", &["Ed Sheeran"], None, Some(263)));
    let far = assert_no_match(&t, &song("Shape of You", &["Ed Sheeran"], None, Some(420)));
    assert!(
        exact > close && close > off && off > far,
        "{exact} {close} {off} {far}"
    );
    // Unbekannte Dauer: neutral, knapp hinter genau passender
    let unknown = assert_match(&t, &song("Shape of You", &["Ed Sheeran"], None, None));
    assert!(unknown < exact && unknown > close, "{unknown}");
}

#[test]
fn music_videos_count_slightly_less() {
    let t = sp("Shape of You", &["Ed Sheeran"], "÷", 233);
    let audio = assert_match(
        &t,
        &song(
            "Shape of You",
            &["Ed Sheeran"],
            Some("÷ (Deluxe)"),
            Some(234),
        ),
    );
    let clip = assert_match(
        &t,
        &video(
            "Ed Sheeran - Shape of You (Official Music Video)",
            &["Ed Sheeran"],
            Some(263),
        ),
    );
    assert!(audio > clip, "{audio} > {clip}");
    // Video eines Fremdkanals, Künstler nur im Titel
    let channel = assert_match(
        &t,
        &video(
            "Ed Sheeran - Shape of You [Official Video]",
            &["Atlantic Records"],
            Some(263),
        ),
    );
    assert!(channel < clip);
}

#[test]
fn different_songs_of_same_artist_do_not_match() {
    let t = sp("Help! - Remastered 2009", &["The Beatles"], "Help!", 139);
    assert_no_match(
        &t,
        &song("Hello, Goodbye", &["The Beatles"], None, Some(208)),
    );
    assert_no_match(
        &t,
        &song("Hello Goodbye", &["The Beatles"], None, Some(139)),
    );
    let t = sp("Love Story", &["Taylor Swift"], "Fearless", 235);
    assert_no_match(&t, &song("Love Song", &["Taylor Swift"], None, Some(235)));
    let t = sp(
        "Yesterday - Remastered 2009",
        &["The Beatles"],
        "Help!",
        125,
    );
    assert_no_match(&t, &song("Let It Be", &["The Beatles"], None, Some(125)));
    let t = sp(
        "Symphony No. 5",
        &["Ludwig van Beethoven"],
        "Symphonies",
        420,
    );
    assert_no_match(
        &t,
        &song("Symphony No. 9", &["Ludwig van Beethoven"], None, Some(420)),
    );
}

#[test]
fn same_title_other_artist_does_not_match() {
    let t = sp("Hello", &["Adele"], "25", 295);
    assert_no_match(&t, &song("Hello", &["Lionel Richie"], None, Some(251)));
    assert_no_match(&t, &song("Hello", &["Lionel Richie"], None, Some(295)));
    assert_match(&t, &song("Hello", &["Adele"], Some("25"), Some(296)));
}

#[test]
fn reissue_versions_prefer_exact_version() {
    let t = sp(
        "Love Story (Taylor's Version)",
        &["Taylor Swift"],
        "Fearless (Taylor's Version)",
        236,
    );
    let tv = assert_match(
        &t,
        &song(
            "Love Story (Taylor's Version)",
            &["Taylor Swift"],
            None,
            Some(236),
        ),
    );
    let old = match_score(&t, &song("Love Story", &["Taylor Swift"], None, Some(235)));
    assert!(tv > old, "{tv} > {old}");
}

#[test]
fn best_match_picks_highest_above_threshold() {
    let t = sp(
        "Bohemian Rhapsody - Remastered 2011",
        &["Queen"],
        "A Night At The Opera",
        354,
    );
    let candidates = vec![
        video(
            "Queen – Bohemian Rhapsody (Official Video Remastered)",
            &["Queen Official"],
            Some(359),
        ),
        song("Bohemian Rhapsody (Live Aid)", &["Queen"], None, Some(360)),
        song(
            "Bohemian Rhapsody",
            &["Queen"],
            Some("A Night at the Opera"),
            Some(355),
        ),
        song(
            "Bohemian Rhapsody (Karaoke)",
            &["Karaoke Hits"],
            None,
            Some(354),
        ),
    ];
    let (best, score) = best_match(&t, &candidates).unwrap();
    assert!(!best.is_video && best.album.is_some(), "{best:?}");
    assert!(score > 0.95);
    assert!(best_match(&t, &candidates[1..2]).is_none());
    assert!(best_match(&t, &[]).is_none());
}

#[test]
fn degenerate_input_never_panics() {
    let empty = sp("", &[], "", 0);
    assert_eq!(match_score(&empty, &song("", &[], None, None)), 0.0);
    assert_eq!(
        match_score(&empty, &song("Song", &["A"], None, Some(10))),
        0.0
    );
    let t = sp("((((", &["))"], "[", 1);
    let s = match_score(&t, &song(")))", &["(("], Some("]]"), Some(0)));
    assert!((0.0..=1.0).contains(&s));
    let t = sp("Song", &[], "", 200);
    let s = match_score(&t, &song("Song", &[], None, Some(200)));
    assert!((0.0..=1.0).contains(&s));
    let long = "a ".repeat(500);
    let s = match_score(
        &sp(&long, &["x"], "", 1),
        &song(&long, &["x"], None, Some(1)),
    );
    assert!(s > 0.9);
}

/// Der Schwellwert liegt mit Abstand zwischen echten Treffern und Fallen.
#[test]
fn threshold_has_margin() {
    let same = [
        (
            sp("Mädchen aus Ostberlin", &["Die Ärzte"], "", 160),
            song("Maedchen aus Ostberlin", &["Die Aerzte"], None, Some(161)),
        ),
        (
            sp("Sweet Child O' Mine", &["Guns N' Roses"], "", 356),
            song("Sweet Child of Mine", &["Guns N Roses"], None, Some(356)),
        ),
        (
            sp("Wonderwall - Remastered", &["Oasis"], "", 258),
            video("Oasis - Wonderwall (Official Video)", &["Oasis"], Some(278)),
        ),
        (
            sp("Halo", &["Beyoncé"], "", 261),
            song("Halo", &["Beyonce"], None, Some(262)),
        ),
        (
            sp("Stay", &["The Kid LAROI", "Justin Bieber"], "", 141),
            song("STAY", &["Justin Bieber"], None, Some(141)),
        ),
        (
            sp("Wonderwall", &["Oasis"], "", 258),
            song("Wonderwall", &["Oasis"], None, None),
        ),
    ];
    for (a, b) in &same {
        let s = match_score(a, b);
        assert!(
            s >= MATCH_THRESHOLD + 0.15,
            "{} ↔ {}: {s:.3}",
            a.name,
            b.title
        );
    }
    let different = [
        (
            sp("Help! - Remastered 2009", &["The Beatles"], "", 139),
            song("Hello Goodbye", &["The Beatles"], None, Some(139)),
        ),
        (
            sp("Love Story", &["Taylor Swift"], "", 235),
            song("Love Song", &["Taylor Swift"], None, Some(235)),
        ),
        (
            sp("Hello", &["Adele"], "", 295),
            song("Hello", &["Lionel Richie"], None, Some(295)),
        ),
        (
            sp("Wonderwall", &["Oasis"], "", 258),
            song(
                "Wonderwall",
                &["Oasis"],
                Some("Familiar to Millions (Live)"),
                Some(260),
            ),
        ),
        (
            sp("Wonderwall", &["Oasis"], "", 258),
            song(
                "Wonderwall (Live at Knebworth)",
                &["Oasis"],
                None,
                Some(262),
            ),
        ),
        (
            sp("Faded - Kygo Remix", &["Alan Walker", "Kygo"], "", 290),
            song("Faded", &["Alan Walker"], None, Some(212)),
        ),
        (
            sp("Wonderwall", &["Oasis"], "", 258),
            song(
                "Wonderwall (Karaoke Version)",
                &["Ameritz Karaoke"],
                None,
                Some(258),
            ),
        ),
    ];
    for (a, b) in &different {
        let s = match_score(a, b);
        assert!(
            s <= MATCH_THRESHOLD - 0.09,
            "{} ↔ {}: {s:.3}",
            a.name,
            b.title
        );
    }
}
