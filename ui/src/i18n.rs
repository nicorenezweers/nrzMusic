//! Übersetzungen. Die deutschen Texte im Quelltext sind zugleich die Schlüssel (wie gettext):
//! `tr("Warteschlange")`, mit Platzhaltern `trf("{0} Titel", &[&n])`. Die Kataloge
//! `locales/<sprache>.json` (deutsch → Sprache) sind beim Übersetzen eingebettet.
//! Fehlt ein Text, erscheint der deutsche. Meldungen des Servers sind ebenfalls deutsch
//! und laufen durch denselben Katalog.

use dioxus::prelude::*;
use std::collections::HashMap;
use std::fmt::Display;

/// Unterstützte Sprachen: Code und Name in der Sprache selbst.
pub const LANGUAGES: &[(&str, &str)] = &[("de", "Deutsch"), ("en", "English")];

const CATALOG_DE: &str = include_str!("../locales/de.json");
const CATALOG_EN: &str = include_str!("../locales/en.json");

/// Aktuelle Sprache (Code aus `LANGUAGES`).
pub static LANG: GlobalSignal<String> = Signal::global(|| "de".to_string());
/// Katalog der aktuellen Sprache (bei Deutsch nur abweichende Einträge).
static CATALOG: GlobalSignal<HashMap<String, String>> = Signal::global(HashMap::new);

const STORAGE_KEY: &str = "nrzmusic-lang";

/// Text in der aktuellen Sprache.
pub fn tr(source: &str) -> String {
    CATALOG
        .read()
        .get(source)
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| source.to_string())
}

/// Text mit Platzhaltern `{0}`, `{1}`, … in der aktuellen Sprache.
pub fn trf(source: &str, args: &[&dyn Display]) -> String {
    fill(&tr(source), args)
}

fn fill(text: &str, args: &[&dyn Display]) -> String {
    let mut out = text.replace("{{", "\u{1}").replace("}}", "\u{2}");
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), &a.to_string());
    }
    out.replace('\u{1}', "{").replace('\u{2}', "}")
}

/// Sprache für Zahlen und Datumsangaben (`Intl`).
pub fn locale() -> &'static str {
    match LANG.read().as_str() {
        "en" => "en-GB",
        _ => "de-CH",
    }
}

/// Gespeicherte oder vom Browser gewünschte Sprache.
fn preferred() -> String {
    if let Some(saved) = crate::util::storage_get(STORAGE_KEY)
        && LANGUAGES.iter().any(|(c, _)| *c == saved) {
            return saved;
        }
    let wanted: Vec<String> = web_sys::window()
        .map(|w| {
            let n = w.navigator();
            let mut v: Vec<String> = n.languages().iter().filter_map(|l| l.as_string()).collect();
            if let Some(l) = n.language() {
                v.push(l);
            }
            v
        })
        .unwrap_or_default();
    for w in wanted {
        let w = w.to_lowercase();
        let base = w.split('-').next().unwrap_or("");
        if let Some((c, _)) = LANGUAGES.iter().find(|(c, _)| *c == base) {
            return c.to_string();
        }
    }
    "de".into()
}

/// Beim Start: Sprache bestimmen.
pub fn init() {
    set_language(&preferred(), false);
}

/// Sprache wechseln (und merken).
pub fn set_language(code: &str, remember: bool) {
    let code = LANGUAGES.iter().find(|(c, _)| *c == code).map(|(c, _)| *c).unwrap_or("de");
    if remember {
        crate::util::storage_set(STORAGE_KEY, code);
    }
    let mut catalog = serde_json::from_str::<HashMap<String, String>>(if code == "en" { CATALOG_EN } else { CATALOG_DE }).unwrap_or_default();
    // Gleiche Einträge (Deutsch → Deutsch) braucht es nicht
    catalog.retain(|k, v| k != v);
    *CATALOG.write() = catalog;
    *LANG.write() = code.to_string();
    if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) {
        let _ = el.set_attribute("lang", code);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fills_placeholders() {
        assert_eq!(super::fill("„{0}“ von {1} – {{x}}", &[&"A", &3]), "„A“ von 3 – {x}");
    }
}
