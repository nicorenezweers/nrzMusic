//! Kleine Helfer: Zeiten, Grössen, Browser-Speicher, Zufall.

use crate::i18n::{tr, trf};
use wasm_bindgen::JsCast;

/// Sekunden → "3:07" bzw. "1:02:03".
pub fn clock(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "0:00".into();
    }
    let s = secs as u64;
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Millisekunden → "3:07".
pub fn clock_ms(ms: u64) -> String {
    clock(ms as f64 / 1000.0)
}

/// Gesamtdauer: "42 Min." bzw. "2 Std. 5 Min."
pub fn long_duration(ms: u64) -> String {
    let mins = (ms + 30_000) / 60_000;
    if mins >= 60 {
        trf("{0} Std. {1} Min.", &[&(mins / 60), &(mins % 60)])
    } else {
        trf("{0} Min.", &[&mins])
    }
}

/// "1 Titel" / "12 Titel"
pub fn tracks_count(n: u64) -> String {
    if n == 1 { tr("1 Titel") } else { trf("{0} Titel", &[&n]) }
}

/// Unix-Sekunden → "24.10.2026".
pub fn date(ts: i64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts as f64 * 1000.0));
    let opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"medium".into());
    d.to_locale_date_string(crate::i18n::locale(), &opts).into()
}

/// Unix-Sekunden → "24.10.2026, 14:05".
pub fn datetime(ts: i64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ts as f64 * 1000.0));
    let opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"medium".into());
    let _ = js_sys::Reflect::set(&opts, &"timeStyle".into(), &"short".into());
    d.to_locale_string(crate::i18n::locale(), &opts).into()
}

/// Zahl mit Tausendertrennzeichen der Sprache.
pub fn number(n: u64) -> String {
    js_sys::Intl::NumberFormat::new(&js_sys::Array::of1(&crate::i18n::locale().into()), &js_sys::Object::new())
        .format()
        .call1(&wasm_bindgen::JsValue::NULL, &(n as f64).into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| n.to_string())
}

/// 812 B, 14,2 MB, 3,1 GB
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    let num = |x: f64| {
        let opts = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&opts, &"maximumFractionDigits".into(), &1.into());
        js_sys::Intl::NumberFormat::new(&js_sys::Array::of1(&crate::i18n::locale().into()), &opts)
            .format()
            .call1(&wasm_bindgen::JsValue::NULL, &x.into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| format!("{x:.1}"))
    };
    const K: f64 = 1024.0;
    if b >= K * K * K {
        format!("{} GB", num(b / (K * K * K)))
    } else if b >= K * K {
        format!("{} MB", num(b / (K * K)))
    } else if b >= K {
        format!("{} KB", num(b / K))
    } else {
        format!("{bytes} B")
    }
}

/// Begrüssung nach Tageszeit.
pub fn greeting() -> String {
    let h = js_sys::Date::new_0().get_hours();
    match h {
        5..=10 => tr("Guten Morgen"),
        11..=13 => tr("Guten Tag"),
        14..=17 => tr("Guten Nachmittag"),
        18..=22 => tr("Guten Abend"),
        _ => tr("Gute Nacht"),
    }
}

pub fn storage_get(key: &str) -> Option<String> {
    web_sys::window()?.local_storage().ok()??.get_item(key).ok()?
}

pub fn storage_set(key: &str, value: &str) {
    if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = s.set_item(key, value);
    }
}

/// Zufallszahl in `0..n`.
pub fn random_index(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    ((js_sys::Math::random() * n as f64) as usize).min(n - 1)
}

/// Fisher-Yates mit `Math.random`.
pub fn shuffle<T>(v: &mut [T]) {
    for i in (1..v.len()).rev() {
        let j = random_index(i + 1);
        v.swap(i, j);
    }
}

/// Schmaler Bildschirm (Handy-Ansicht, wie Tailwinds `md`-Grenze).
pub fn is_phone() -> bool {
    web_sys::window()
        .and_then(|w| w.inner_width().ok())
        .and_then(|v| v.as_f64())
        .is_some_and(|w| w < 768.0)
}

pub fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

/// Element über seine ID fokussieren (z. B. das Suchfeld).
pub fn focus(id: &str) {
    if let Some(el) = document().and_then(|d| d.get_element_by_id(id)).and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()) {
        let _ = el.focus();
    }
}

/// Dateien aus einem <input type=file> (onchange) samt relativem Pfad bei Ordnern; leert das Feld danach.
pub fn input_files(e: &dioxus::prelude::FormEvent) -> Vec<(web_sys::File, String)> {
    let input = e
        .data()
        .downcast::<web_sys::Event>()
        .and_then(|ev| ev.target())
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
    let Some(input) = input else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(list) = input.files() {
        for i in 0..list.length() {
            if let Some(f) = list.get(i) {
                // Bei "Ordner wählen" bringt jede Datei ihren Pfad im Ordner mit
                let rel = js_sys::Reflect::get(&f, &"webkitRelativePath".into()).ok().and_then(|v| v.as_string()).unwrap_or_default();
                let name = if rel.is_empty() { f.name() } else { rel };
                out.push((f, name));
            }
        }
    }
    input.set_value("");
    out
}

/// Absolute Adresse eines Pfads (z. B. für Links nach aussen).
pub fn open_external(url: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.open_with_url_and_target(url, "_blank");
    }
}

/// YouTube-Music-Adresse eines Videos.
pub fn yt_music_url(video_id: &str) -> String {
    format!("https://music.youtube.com/watch?v={video_id}")
}

/// Datum aus ISO 8601 (z. B. Spotify `added_at`) → "24.10.2026".
pub fn date_iso(iso: &str) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(iso));
    if d.get_time().is_nan() {
        return String::new();
    }
    let opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"medium".into());
    d.to_locale_date_string(crate::i18n::locale(), &opts).into()
}

/// Text in die Zwischenablage. Ohne sicheren Kontext (http im Heimnetz) über ein
/// unsichtbares Textfeld und `execCommand("copy")`.
pub fn copy_text(text: &str) -> bool {
    use js_sys::{Function, Reflect};
    use wasm_bindgen::JsValue;
    let Some(w) = web_sys::window() else { return false };
    let secure = Reflect::get(&w, &"isSecureContext".into()).ok().and_then(|v| v.as_bool()).unwrap_or(false);
    if secure {
        let clip = Reflect::get(&w.navigator(), &"clipboard".into()).ok().filter(|c| !c.is_undefined() && !c.is_null());
        if let Some(clip) = clip
            && let Some(f) = Reflect::get(&clip, &"writeText".into()).ok().and_then(|f| f.dyn_into::<Function>().ok())
            && f.call1(&clip, &JsValue::from_str(text)).is_ok()
        {
            return true;
        }
    }
    let Some(doc) = w.document() else { return false };
    let Some(body) = doc.body() else { return false };
    let Ok(ta) = doc.create_element("textarea") else { return false };
    ta.set_text_content(Some(text));
    let _ = ta.set_attribute("style", "position:fixed;top:0;left:0;opacity:0");
    if body.append_child(&ta).is_err() {
        return false;
    }
    if let Some(f) = Reflect::get(&ta, &"select".into()).ok().and_then(|f| f.dyn_into::<Function>().ok()) {
        let _ = f.call0(&ta);
    }
    let ok = Reflect::get(&doc, &"execCommand".into())
        .ok()
        .and_then(|f| f.dyn_into::<Function>().ok())
        .and_then(|f| f.call1(&doc, &JsValue::from_str("copy")).ok())
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let _ = body.remove_child(&ta);
    ok
}
