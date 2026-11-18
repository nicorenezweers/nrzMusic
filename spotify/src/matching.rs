//! Abgleich eines Spotify-Titels mit Treffern von YouTube Music. Reine Funktionen – die
//! Suche selbst macht der Server (`match_query` → YouTube-Music-Suche → `match_score` für
//! jeden Treffer → bester Treffer ab [`MATCH_THRESHOLD`]).
//!
//! Der Wert setzt sich zusammen aus
//! - **Titel**: normalisiert (Kleinbuchstaben, Akzente/Umlaute gefaltet, Klammerzusätze wie
//!   "feat. …", "Remastered 2011", "Radio Edit", "Official Video" entfernt), verglichen über
//!   Editierdistanz und (unscharfe) Wort-Überdeckung,
//! - **Künstler**: irgendein Spotify-Künstler ↔ irgendein YouTube-Künstler (oder der
//!   Künstler steht im Titel eines Videos),
//! - **Dauer**: bis 3 s Abweichung ideal, über 15 s starker Abzug, fehlende Dauer neutral,
//! - **Versionen**: "Live", "Karaoke", "Cover", "Instrumental", "Remix", "Sped Up", "8D" …
//!   beim YouTube-Treffer, aber nicht bei Spotify, drücken den Wert stark,
//! - Musikvideos zählen etwas weniger als Audio-Titel.

use crate::{SpTrack, YtTrack};

/// Ab diesem Wert gilt ein Treffer als derselbe Titel.
pub const MATCH_THRESHOLD: f64 = 0.6;

/// Musikvideos (statt Audio-Titel) zählen etwas weniger.
const VIDEO_FACTOR: f64 = 0.95;
/// Unbekannte Dauer: neutral, aber knapp hinter einem Treffer mit passender Dauer.
const UNKNOWN_DURATION: f64 = 0.95;
/// Spotify nennt eine Version (z. B. "Live"), der Treffer nicht.
const MISSING_TAG_FACTOR: f64 = 0.85;
/// Der reine Kern des Titels (ohne jeden Zusatz) zählt etwas weniger als der ganze Titel.
const BARE_TITLE_FACTOR: f64 = 0.9;
/// Ab dieser Ähnlichkeit gelten zwei Künstlernamen als derselbe.
const ARTIST_MIN_SIM: f64 = 0.75;
/// Künstler, der nur im Titel eines Videos steht ("Queen - Bohemian Rhapsody").
const ARTIST_IN_TITLE: f64 = 0.85;

// ---------------------------------------------------------------- Normalisieren

/// Buchstaben mit Akzent → Grundbuchstaben (Latein-1 und Latein-erweitert-A).
fn fold(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' | 'ț' => "t",
        'þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        // "Ke$ha"
        '$' => "s",
        _ => return None,
    })
}

/// Kleinbuchstaben, Akzente gefaltet, Apostrophe entfernt ("Don't" → "dont"), "&"/"+" →
/// "and", alle anderen Satzzeichen → Leerzeichen; Wörter durch genau ein Leerzeichen
/// getrennt.
pub(crate) fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        for c in ch.to_lowercase() {
            match c {
                '\'' | '’' | '‘' | '`' | '´' | 'ʼ' => {}
                // kombinierende Akzente (zerlegte Schreibweise)
                '\u{0300}'..='\u{036f}' => {}
                '&' | '+' => out.push_str(" and "),
                _ => {
                    if let Some(f) = fold(c) {
                        out.push_str(f);
                    } else if c.is_alphanumeric() {
                        out.push(c);
                    } else {
                        out.push(' ');
                    }
                }
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `phrase` kommt in `text` als ganze Wörter vor (beide normalisiert).
fn has_phrase(text: &str, phrase: &str) -> bool {
    if phrase.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    text.match_indices(phrase).any(|(i, _)| {
        let end = i + phrase.len();
        (i == 0 || bytes[i - 1] == b' ') && (end == bytes.len() || bytes[end] == b' ')
    })
}

fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(' ').filter(|w| !w.is_empty())
}

// ---------------------------------------------------------------- Ähnlichkeit

fn levenshtein(a: &[char], b: &[char]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// 1 − Editierdistanz / Länge des längeren Texts.
fn lev_sim(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let max = a.len().max(b.len());
    if max == 0 {
        return 1.0;
    }
    1.0 - levenshtein(&a, &b) as f64 / max as f64
}

/// Wort-Überdeckung (Dice-Koeffizient); Wörter ab 4 Zeichen dürfen leicht abweichen
/// ("madchen" ↔ "maedchen").
fn token_dice(a: &str, b: &str) -> f64 {
    let ta: Vec<&str> = words(a).collect();
    let tb: Vec<&str> = words(b).collect();
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let mut used = vec![false; tb.len()];
    let mut sum = 0.0;
    for x in &ta {
        let mut best: Option<(f64, usize)> = None;
        for (j, y) in tb.iter().enumerate() {
            if used[j] {
                continue;
            }
            let s = if x == y {
                1.0
            } else if x.chars().count() >= 4 && y.chars().count() >= 4 {
                let s = lev_sim(x, y);
                if s >= 0.75 {
                    s
                } else {
                    0.0
                }
            } else {
                0.0
            };
            if s > 0.0 && best.is_none_or(|(b, _)| s > b) {
                best = Some((s, j));
                if s >= 1.0 {
                    break;
                }
            }
        }
        if let Some((s, j)) = best {
            used[j] = true;
            sum += s;
        }
    }
    2.0 * sum / (ta.len() + tb.len()) as f64
}

/// Ähnlichkeit zweier normalisierter Texte, 0 – 1.
fn text_sim(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    if a.replace(' ', "") == b.replace(' ', "") {
        return 0.97;
    }
    0.5 * lev_sim(a, b) + 0.5 * token_dice(a, b)
}

// ---------------------------------------------------------------- Versionen

/// Eine Version, die sich vom Original unterscheidet ("Live", "Karaoke", …).
struct TagDef {
    /// Faktor, wenn nur der YouTube-Treffer diese Version ist
    weight: f64,
    /// Wendungen, die überall zählen (auch im Haupttitel ohne Klammer)
    anywhere: &'static [&'static str],
    /// Wendungen, die nur in Klammern, Zusätzen nach " - " und Albumtiteln zählen
    /// ("Live Forever" ist kein Live-Titel, "Song (Live)" schon)
    segment: &'static [&'static str],
    /// Zählt auch im Künstlernamen ("Ameritz Karaoke", "Nightcore Hits")
    in_artist: bool,
}

const TAGS: &[TagDef] = &[
    // live
    TagDef {
        weight: 0.5,
        anywhere: &[
            "live at",
            "live from",
            "en vivo",
            "ao vivo",
            "en directo",
            "mtv unplugged",
        ],
        segment: &[
            "live",
            "unplugged",
            "concert",
            "in concert",
            "session",
            "sessions",
            "recorded at",
            "spotify singles",
            "live version",
            "directo",
        ],
        in_artist: false,
    },
    // remix
    TagDef {
        weight: 0.5,
        anywhere: &[],
        segment: &[
            "remix", "remixes", "remixed", "rmx", "rework", "bootleg", "vip mix", "club mix",
            "dub mix", "flip",
        ],
        in_artist: false,
    },
    // acoustic
    TagDef {
        weight: 0.6,
        anywhere: &[],
        segment: &[
            "acoustic",
            "akustik",
            "akustisch",
            "acustico",
            "acustica",
            "stripped",
        ],
        in_artist: false,
    },
    // karaoke
    TagDef {
        weight: 0.3,
        anywhere: &[
            "karaoke",
            "in the style of",
            "made famous by",
            "originally performed by",
            "backing track",
            "sing along",
            "singalong",
        ],
        segment: &["originally by", "minus one", "playback"],
        in_artist: true,
    },
    // cover
    TagDef {
        weight: 0.45,
        anywhere: &["cover by", "covered by", "tribute to"],
        segment: &["cover", "covers", "tribute"],
        in_artist: true,
    },
    // instrumental
    TagDef {
        weight: 0.45,
        anywhere: &[],
        segment: &["instrumental", "inst", "off vocal", "beat only"],
        in_artist: false,
    },
    // schneller/langsamer/verfremdet
    TagDef {
        weight: 0.35,
        anywhere: &[
            "sped up",
            "speed up",
            "speeded up",
            "nightcore",
            "slowed",
            "slowed down",
            "8d",
            "8d audio",
            "16d",
            "bass boosted",
            "chipmunk",
            "reversed",
            "daycore",
        ],
        segment: &["reverb", "slowed reverb"],
        in_artist: true,
    },
    // lo-fi
    TagDef {
        weight: 0.45,
        anywhere: &["lofi", "lo fi"],
        segment: &[],
        in_artist: true,
    },
    // a cappella
    TagDef {
        weight: 0.45,
        anywhere: &["acapella", "a cappella", "a capella"],
        segment: &["vocals only", "vocal only"],
        in_artist: false,
    },
    // demo
    TagDef {
        weight: 0.65,
        anywhere: &[],
        segment: &["demo", "rehearsal", "outtake", "alternate take"],
        in_artist: false,
    },
    // extended
    TagDef {
        weight: 0.7,
        anywhere: &[],
        segment: &["extended", "12 inch", "12 mix", "club version"],
        in_artist: false,
    },
    // Bearbeitungen (Orchester, Klavier, Spieldose …)
    TagDef {
        weight: 0.5,
        anywhere: &["lullaby", "music box", "piano cover", "string quartet"],
        segment: &[
            "orchestral",
            "symphonic",
            "orchestra",
            "piano version",
            "piano",
        ],
        in_artist: false,
    },
    // Endlosschleifen
    TagDef {
        weight: 0.3,
        anywhere: &["1 hour", "one hour", "10 hours", "hour loop", "hours loop"],
        segment: &["loop"],
        in_artist: false,
    },
    // mashup
    TagDef {
        weight: 0.45,
        anywhere: &["mashup", "mash up"],
        segment: &["medley"],
        in_artist: false,
    },
    // reprise
    TagDef {
        weight: 0.7,
        anywhere: &[],
        segment: &["reprise"],
        in_artist: false,
    },
];

/// Versionen in einem normalisierten Text als Bitmaske (Bit i = `TAGS[i]`).
/// `segment` = auch die nur in Zusätzen gültigen Wendungen prüfen.
fn tag_mask(norm: &str, segment: bool) -> u32 {
    let mut mask = 0;
    for (i, def) in TAGS.iter().enumerate() {
        let hit = def.anywhere.iter().any(|p| has_phrase(norm, p))
            || (segment && def.segment.iter().any(|p| has_phrase(norm, p)));
        if hit {
            mask |= 1 << i;
        }
    }
    mask
}

fn artist_tag_mask(norm: &str) -> u32 {
    let mut mask = 0;
    for (i, def) in TAGS.iter().enumerate() {
        if def.in_artist
            && def
                .anywhere
                .iter()
                .chain(def.segment)
                .any(|p| has_phrase(norm, p))
        {
            mask |= 1 << i;
        }
    }
    mask
}

/// Starker Abzug für Versionen nur beim Treffer, leichter für Versionen nur bei Spotify.
fn tag_factor(sp: u32, yt: u32) -> f64 {
    let mut f = 1.0;
    for (i, def) in TAGS.iter().enumerate() {
        let bit = 1 << i;
        if yt & bit != 0 && sp & bit == 0 {
            f *= def.weight;
        } else if sp & bit != 0 && yt & bit == 0 {
            f *= MISSING_TAG_FACTOR;
        }
    }
    f
}

// ---------------------------------------------------------------- Titel zerlegen

/// Zusätze ohne Bedeutung für den Abgleich.
const NOISE_PHRASES: &[&str] = &[
    "remaster",
    "remastered",
    "remasterizado",
    "remasterizada",
    "remasterise",
    "remasterisee",
    "digitally",
    "radio edit",
    "radio version",
    "radio mix",
    "single version",
    "single edit",
    "album version",
    "original version",
    "original mix",
    "mono",
    "stereo",
    "explicit",
    "clean",
    "censored",
    "uncensored",
    "bonus track",
    "bonus",
    "deluxe",
    "official",
    "video",
    "audio",
    "lyrics",
    "lyric",
    "letra",
    "visualizer",
    "visualiser",
    "hd",
    "hq",
    "4k",
    "prod",
    "from",
    "taken from",
    "soundtrack",
    "ost",
    "motion picture",
    "anniversary",
    "edition",
    "mixed",
    "videoclip",
    "clip officiel",
];

/// Wörter, die allein keinen eigenen Zusatz ergeben ("2011 Version").
const FILLER: &[&str] = &[
    "version",
    "edit",
    "the",
    "a",
    "and",
    "of",
    "by",
    "remaster",
    "remastered",
    "original",
    "new",
    "single",
    "album",
    "mix",
];

const FEAT: &[&str] = &["feat", "ft", "featuring"];

/// Nachgestellte Zusätze ohne Klammer, vor allem bei Videos ("… Official Video").
const TRAILING_NOISE: &[&str] = &[
    "official music video",
    "official video",
    "official audio",
    "official lyric video",
    "lyric video",
    "music video",
    "lyrics",
    "audio",
];

/// Trenner zwischen Titel und Zusatz (Spotify: "Song - Remastered 2011", Videos:
/// "Artist - Song | Official Video").
const DASHES: &[&str] = &[" - ", " – ", " — ", " | ", " // ", " ~ "];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Bedeutungsloser Zusatz (feat., Remaster, Radio Edit, Official Video …)
    Noise,
    /// Andere Version (Live, Remix, Karaoke …)
    Version,
    /// Sonstiger Zusatz ("Part 2", "Taylor's Version")
    Other,
}

fn is_year_or_number(w: &str) -> bool {
    w.chars().all(|c| c.is_ascii_digit())
}

fn classify(norm: &str) -> Kind {
    let mut ws = words(norm).peekable();
    let Some(first) = ws.peek().copied() else {
        return Kind::Noise;
    };
    if FEAT.contains(&first) || first == "with" {
        return Kind::Noise;
    }
    if tag_mask(norm, true) != 0 {
        return Kind::Version;
    }
    if NOISE_PHRASES.iter().any(|p| has_phrase(norm, p)) {
        return Kind::Noise;
    }
    if ws.all(|w| FILLER.contains(&w) || is_year_or_number(w)) {
        return Kind::Noise;
    }
    Kind::Other
}

/// Text ausserhalb der Klammern und die Inhalte der (äussersten) Klammern.
fn split_brackets(raw: &str) -> (String, Vec<String>) {
    let mut outside = String::with_capacity(raw.len());
    let mut groups = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    for c in raw.chars() {
        match c {
            '(' | '[' | '{' | '【' | '「' | '«' => {
                if depth > 0 {
                    cur.push(c);
                } else {
                    outside.push(' ');
                }
                depth += 1;
            }
            ')' | ']' | '}' | '】' | '」' | '»' => {
                if depth == 0 {
                    outside.push(' ');
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    groups.push(std::mem::take(&mut cur));
                    outside.push(' ');
                } else {
                    cur.push(c);
                }
            }
            _ if depth > 0 => cur.push(c),
            _ => outside.push(c),
        }
    }
    if depth > 0 && !cur.trim().is_empty() {
        groups.push(cur);
    }
    (outside, groups)
}

/// An den Trennern " - ", " | " … aufteilen; leere Teile fallen weg.
fn split_dashes(s: &str) -> Vec<String> {
    let mut text = s.to_string();
    for d in DASHES {
        text = text.replace(d, "\u{1f}");
    }
    text.split('\u{1f}')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Wörter ab "feat"/"ft"/"featuring" abschneiden.
fn cut_feat(norm: &str) -> String {
    words(norm)
        .take_while(|w| !FEAT.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_trailing_noise(mut s: String) -> String {
    loop {
        let before = s.len();
        for p in TRAILING_NOISE {
            if s.len() > p.len() && s.ends_with(p) && s.as_bytes()[s.len() - p.len() - 1] == b' ' {
                s.truncate(s.len() - p.len() - 1);
            }
        }
        if s.len() == before {
            return s;
        }
    }
}

/// Ein zerlegter Titel.
#[derive(Debug, Default)]
struct Title {
    /// Kern ohne jeden Zusatz (normalisiert, ohne "feat. …")
    head: String,
    /// Bedeutsame Zusätze (Versionen, "Part 2" …), normalisiert
    extra: Vec<String>,
    /// Versionen als Bitmaske
    tags: u32,
}

impl Title {
    fn full(&self) -> String {
        let mut s = self.head.clone();
        for e in &self.extra {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(e);
        }
        s
    }

    fn numbers(&self) -> Vec<String> {
        let full = self.full();
        let mut n: Vec<String> = words(&full)
            .filter(|w| is_year_or_number(w))
            .map(str::to_string)
            .collect();
        n.sort();
        n.dedup();
        n
    }
}

/// Titel zerlegen. `artists` (normalisiert): Künstlernamen, die bei Videos vor oder hinter
/// dem eigentlichen Titel stehen können ("Queen - Bohemian Rhapsody") und dann wegfallen.
fn parse_title(raw: &str, artists: &[String]) -> Title {
    let (outside, groups) = split_brackets(raw);
    let mut parts = split_dashes(&outside);
    if parts.len() >= 2 && !artists.is_empty() {
        let is_artist = |p: &str| {
            let n = clean_artist(&normalize(p));
            artists.iter().any(|a| artist_pair_sim(&n, a) >= 0.85)
        };
        if is_artist(&parts[0]) {
            parts.remove(0);
        } else if parts.last().is_some_and(|p| is_artist(p)) {
            parts.pop();
        }
    }

    let mut title = Title::default();
    let mut segments: Vec<String> = groups;
    let head_raw = if parts.is_empty() {
        String::new()
    } else {
        let head = parts.remove(0);
        segments.extend(parts);
        head
    };
    let head_full = normalize(&head_raw);
    title.tags |= tag_mask(&head_full, false);
    title.head = strip_trailing_noise(cut_feat(&head_full));

    for seg in &segments {
        let norm = normalize(seg);
        match classify(&norm) {
            Kind::Noise => {}
            kind => {
                if kind == Kind::Version {
                    title.tags |= tag_mask(&norm, true);
                }
                let kept: Vec<&str> = words(&norm)
                    .filter(|w| !FILLER.contains(w) && !(w.len() == 4 && is_year_or_number(w)))
                    .collect();
                if !kept.is_empty() {
                    title.extra.push(kept.join(" "));
                }
            }
        }
    }

    // Nur Klammern ("[Untitled]") → deren Inhalt ist der Titel
    if title.head.is_empty() {
        if let Some(first) = title.extra.first().cloned() {
            title.head = first;
            title.extra.remove(0);
        } else {
            title.head = normalize(raw);
        }
    }
    title
}

/// Ähnlichkeit zweier zerlegter Titel: ganzer Titel oder (etwas weniger) nur der Kern.
fn title_similarity(sp: &Title, yt: &Title) -> f64 {
    let full = text_sim(&sp.full(), &yt.full());
    let bare = BARE_TITLE_FACTOR * text_sim(&sp.head, &yt.head);
    let mut t = full.max(bare);
    // "Symphony No. 5" ↔ "Symphony No. 9", "Part 1" ↔ "Part 2"
    let (a, b) = (sp.numbers(), yt.numbers());
    if !a.is_empty() && !b.is_empty() && !a.iter().any(|n| b.contains(n)) {
        t *= 0.75;
    }
    t
}

/// Suchbegriff aus einem Spotify-Titel: ohne "feat. …", Remaster-, Radio-Edit- und
/// Soundtrack-Zusätze; Versionen ("Live", "Remix") und andere Zusätze bleiben.
fn query_title(raw: &str) -> String {
    // Klammern: bedeutungslose entfernen, die anderen ohne Klammerzeichen behalten
    let mut text = String::with_capacity(raw.len());
    let mut cur = String::new();
    let mut depth = 0usize;
    for c in raw.chars() {
        match c {
            '(' | '[' | '{' => {
                if depth > 0 {
                    cur.push(c);
                }
                depth += 1;
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let group = std::mem::take(&mut cur);
                    text.push(' ');
                    if classify(&normalize(&group)) != Kind::Noise {
                        text.push_str(&group);
                        text.push(' ');
                    }
                } else {
                    cur.push(c);
                }
            }
            _ if depth > 0 => cur.push(c),
            _ => text.push(c),
        }
    }

    let mut parts = split_dashes(&text).into_iter();
    let mut out: Vec<String> = parts.next().into_iter().collect();
    out.extend(parts.filter(|p| classify(&normalize(p)) != Kind::Noise));

    let joined = out.join(" ");
    let kept: Vec<&str> = joined
        .split_whitespace()
        .take_while(|w| {
            let w = w
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            !FEAT.contains(&w.as_str())
        })
        .collect();
    if kept.is_empty() {
        raw.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        kept.join(" ")
    }
}

// ---------------------------------------------------------------- Künstler

/// "the beatles" → "beatles", "queen topic"/"queenvevo"/"queenofficial" → "queen".
fn clean_artist(norm: &str) -> String {
    let mut s = norm.strip_prefix("the ").unwrap_or(norm).to_string();
    for suffix in [" topic", " vevo", " official", "vevo", "official"] {
        if let Some(rest) = s.strip_suffix(suffix) {
            if rest.trim().chars().count() >= 3 {
                s = rest.trim().to_string();
            }
        }
    }
    s
}

fn push_key(keys: &mut Vec<String>, k: String) {
    if !k.is_empty() && !keys.contains(&k) {
        keys.push(k);
    }
}

/// Schlüssel eines Künstlernamens: der ganze Name und seine Teile ("Simon & Garfunkel" →
/// "simon and garfunkel", "simon", "garfunkel").
fn artist_keys(name: &str) -> Vec<String> {
    let mut keys = Vec::new();
    push_key(&mut keys, clean_artist(&normalize(name)));
    for part in name.split([',', ';', '/', '×']) {
        let norm = normalize(part);
        let mut cur: Vec<&str> = Vec::new();
        for w in words(&norm).chain(std::iter::once("and")) {
            if matches!(
                w,
                "and" | "x" | "feat" | "ft" | "featuring" | "with" | "vs" | "und"
            ) {
                let k = clean_artist(&cur.join(" "));
                if k.chars().count() >= 3 {
                    push_key(&mut keys, k);
                }
                cur.clear();
            } else {
                cur.push(w);
            }
        }
    }
    keys
}

/// Ähnlichkeit zweier Künstler-Schlüssel, 0 – 1.
fn artist_pair_sim(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let (ca, cb) = (a.replace(' ', ""), b.replace(' ', ""));
    if ca == cb {
        return 1.0;
    }
    let sim = lev_sim(&ca, &cb);
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let contained = if short.chars().count() >= 4 && has_phrase(long, short) {
        0.85
    } else {
        0.0
    };
    let sim = if sim >= ARTIST_MIN_SIM { sim } else { 0.0 };
    sim.max(contained)
}

fn all_artist_keys<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut keys = Vec::new();
    for n in names {
        for k in artist_keys(n) {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
    }
    keys
}

/// Irgendein Spotify-Künstler ↔ irgendein YouTube-Künstler (oder im Videotitel).
fn artist_similarity(sp: &[String], yt: &[String], yt_title_norm: &str) -> f64 {
    if sp.is_empty() {
        return 0.5;
    }
    let mut best = sp
        .iter()
        .flat_map(|a| yt.iter().map(move |b| artist_pair_sim(a, b)))
        .fold(0.0, f64::max);
    if best < ARTIST_IN_TITLE
        && sp
            .iter()
            .any(|k| k.chars().count() >= 3 && has_phrase(yt_title_norm, k))
    {
        best = ARTIST_IN_TITLE;
    }
    if best == 0.0 && yt.is_empty() {
        // Treffer ohne Künstlerangabe: unbekannt statt falsch
        return 0.4;
    }
    best
}

// ---------------------------------------------------------------- Dauer

/// Bis 3 s ideal, bis 15 s leichter Abzug, darüber stark. Bei Videos (Intro/Outro) zählt
/// die halbe Abweichung. Fehlende Dauer: neutral.
fn duration_factor(sp_ms: u64, yt_ms: Option<u64>, is_video: bool) -> f64 {
    let Some(yt_ms) = yt_ms.filter(|&d| d > 0) else {
        return UNKNOWN_DURATION;
    };
    if sp_ms == 0 {
        return UNKNOWN_DURATION;
    }
    let mut diff = sp_ms.abs_diff(yt_ms) as f64 / 1000.0;
    if is_video {
        diff /= 2.0;
    }
    if diff <= 3.0 {
        1.0
    } else if diff <= 8.0 {
        1.0 - (diff - 3.0) / 5.0 * 0.1
    } else if diff <= 15.0 {
        0.9 - (diff - 8.0) / 7.0 * 0.15
    } else if diff <= 60.0 {
        0.55 - (diff - 15.0) / 45.0 * 0.25
    } else {
        0.25
    }
}

// ---------------------------------------------------------------- Öffentlich

/// Suchanfrage für YouTube Music zu einem Spotify-Titel: erster Künstler + Titel ohne
/// Zusätze wie "- Remastered 2011", "(feat. …)" oder "- Radio Edit". Versionen
/// ("- Live", "(Kygo Remix)") bleiben, damit die passende Fassung gefunden wird.
pub fn match_query(track: &SpTrack) -> String {
    let artist = track
        .artists
        .iter()
        .map(|a| a.trim())
        .find(|a| !a.is_empty())
        .unwrap_or("");
    format!("{artist} {}", query_title(&track.name))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Wie gut ein YouTube-Music-Treffer zum Spotify-Titel passt, 0.0 – 1.0
/// (ab [`MATCH_THRESHOLD`] derselbe Titel).
pub fn match_score(track: &SpTrack, candidate: &YtTrack) -> f64 {
    let sp_title = parse_title(&track.name, &[]);
    if sp_title.head.is_empty() {
        return 0.0;
    }
    let sp_artists = all_artist_keys(track.artists.iter().map(String::as_str));
    let yt_artists = all_artist_keys(candidate.artists.iter().map(|a| a.name.as_str()));
    let mut strip = sp_artists.clone();
    strip.extend(yt_artists.iter().cloned());
    let yt_title = parse_title(&candidate.title, &strip);

    let t = title_similarity(&sp_title, &yt_title);
    let a = artist_similarity(&sp_artists, &yt_artists, &normalize(&candidate.title));
    let d = duration_factor(track.duration_ms, candidate.duration_ms, candidate.is_video);

    let sp_tags = sp_title.tags | tag_mask(&normalize(&track.album), true);
    let mut yt_tags = yt_title.tags;
    if let Some(album) = &candidate.album {
        yt_tags |= tag_mask(&normalize(&album.title), true);
    }
    for artist in &candidate.artists {
        yt_tags |= artist_tag_mask(&normalize(&artist.name));
    }
    let tags = tag_factor(sp_tags, yt_tags);
    let video = if candidate.is_video {
        VIDEO_FACTOR
    } else {
        1.0
    };

    (t.powf(1.5) * (0.45 + 0.55 * a) * d * tags * video).clamp(0.0, 1.0)
}

/// Der beste Treffer ab [`MATCH_THRESHOLD`] mit seinem Wert (bei Gleichstand der frühere,
/// also der von YouTube Music höher eingestufte).
pub fn best_match<'a>(track: &SpTrack, candidates: &'a [YtTrack]) -> Option<(&'a YtTrack, f64)> {
    let mut best: Option<(&YtTrack, f64)> = None;
    for c in candidates {
        let s = match_score(track, c);
        if s >= MATCH_THRESHOLD && best.is_none_or(|(_, b)| s > b) {
            best = Some((c, s));
        }
    }
    best
}

#[cfg(test)]
mod tests;
