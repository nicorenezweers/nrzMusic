//! Anmeldung: PKCE-Werte (Verifier, Challenge, `state`), die Rückruf-Adresse und die
//! Antworten des Token-Endpunkts (`/api/token`).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::Rng;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{CallbackParams, Error, Result, Tokens};

/// Länge des PKCE-Verifiers (Spotify erlaubt 43 – 128 Zeichen).
pub(crate) const VERIFIER_LEN: usize = 64;
/// Länge des `state`-Werts gegen CSRF.
const STATE_LEN: usize = 32;
/// Zeichen für Verifier und `state` (Teilmenge der "unreserved characters" aus RFC 7636).
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
/// Lebensdauer eines Access-Tokens, falls Spotify keine nennt (Spotify: eine Stunde).
const DEFAULT_EXPIRES_IN: i64 = 3600;

/// Zufällige Zeichenkette aus [`ALPHABET`] (kryptografisch sicherer Zufall von `rand`).
fn random_string(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

pub(crate) fn new_verifier() -> String {
    random_string(VERIFIER_LEN)
}

pub(crate) fn new_state() -> String {
    random_string(STATE_LEN)
}

/// `code_challenge` zur Methode `S256`: SHA-256 des Verifiers, base64url ohne `=`.
pub(crate) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Jetzt in Unix-Sekunden.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Antwort von `/api/token` → [`Tokens`]. `old_refresh` ersetzt ein fehlendes Refresh-Token
/// (Spotify schickt bei der Erneuerung nicht immer ein neues); ohne beides gibt es
/// [`Error::Parse`].
pub(crate) fn tokens_from(v: &Value, old_refresh: Option<&str>, now: i64) -> Result<Tokens> {
    let access_token = v
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Parse("Token-Antwort ohne access_token".into()))?;
    let refresh_token = v
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or(old_refresh.filter(|s| !s.is_empty()))
        .ok_or_else(|| Error::Parse("Token-Antwort ohne refresh_token".into()))?;
    let expires_in = v
        .get("expires_in")
        .and_then(|e| {
            e.as_i64()
                .or_else(|| e.as_str().and_then(|s| s.parse().ok()))
        })
        .filter(|&e| e > 0)
        .unwrap_or(DEFAULT_EXPIRES_IN);
    let scope = v
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(Tokens {
        access_token: access_token.to_string(),
        refresh_token: refresh_token.to_string(),
        expires_at: now + expires_in,
        scope,
    })
}

/// `code`/`state` oder `error` aus der Adresse lesen, auf die Spotify weitergeleitet hat.
///
/// Angenommen werden die ganze Adresse (`http://127.0.0.1:8350/…/callback?code=…&state=…`),
/// eine Adresse mit den Werten im Fragment (`…#code=…`) und der blosse Query-Teil
/// (`code=…&state=…`, auch mit führendem `?`). `None`, wenn weder ein Fehler noch Code
/// *und* `state` darin stehen.
pub fn parse_callback(url: &str) -> Option<CallbackParams> {
    let input = url.trim();
    if input.is_empty() {
        return None;
    }
    // Query und Fragment getrennt herausschneiden; beide werden gelesen (Query zuerst).
    let (before_fragment, fragment) = match input.split_once('#') {
        Some((a, b)) => (a, b),
        None => (input, ""),
    };
    let query = match before_fragment.split_once('?') {
        Some((_, q)) => q,
        // Ohne "?" ist es entweder der blosse Query-Teil oder eine Adresse ohne Werte
        None if before_fragment.contains("://") => "",
        None => before_fragment,
    };

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for part in [query, fragment] {
        for (k, v) in url::form_urlencoded::parse(part.as_bytes()) {
            let v = v.trim().to_string();
            if v.is_empty() {
                continue;
            }
            match k.as_ref() {
                "code" if code.is_none() => code = Some(v),
                "state" if state.is_none() => state = Some(v),
                "error" if error.is_none() => error = Some(v),
                _ => {}
            }
        }
    }

    if let Some(error) = error {
        return Some(CallbackParams::Error { error, state });
    }
    match (code, state) {
        (Some(code), Some(state)) => Some(CallbackParams::Code { code, state }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn verifier_and_state_are_random_and_valid() {
        let a = new_verifier();
        let b = new_verifier();
        assert_eq!(a.len(), VERIFIER_LEN);
        assert_ne!(a, b);
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(new_state().len(), STATE_LEN);
        assert_ne!(new_state(), new_state());
    }

    #[test]
    fn challenge_matches_rfc7636_example() {
        // RFC 7636, Anhang B
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let c = challenge(&new_verifier());
        assert_eq!(c.len(), 43);
        assert!(!c.contains('=') && !c.contains('+') && !c.contains('/'));
    }

    #[test]
    fn tokens_from_exchange_and_refresh() {
        let v: Value =
            serde_json::from_str(include_str!("../tests/fixtures/token_exchange.json")).unwrap();
        let t = tokens_from(&v, None, 1_000).unwrap();
        assert_eq!(t.access_token, "BQD-access-token-1");
        assert_eq!(t.refresh_token, "AQC-refresh-token-1");
        assert_eq!(t.expires_at, 1_000 + 3600);
        assert_eq!(
            t.scope,
            "playlist-read-private playlist-read-collaborative user-library-read user-read-private"
        );

        // Erneuerung ohne neues Refresh-Token: das alte bleibt
        let v: Value =
            serde_json::from_str(include_str!("../tests/fixtures/token_refresh.json")).unwrap();
        let t = tokens_from(&v, Some("old-refresh"), 50).unwrap();
        assert_eq!(t.access_token, "BQD-access-token-2");
        assert_eq!(t.refresh_token, "old-refresh");
        assert_eq!(t.expires_at, 50 + 3600);

        // Neues Refresh-Token gewinnt
        let t = tokens_from(
            &json!({"access_token": "a", "refresh_token": "new", "expires_in": 10}),
            Some("old"),
            0,
        )
        .unwrap();
        assert_eq!((t.refresh_token.as_str(), t.expires_at), ("new", 10));
    }

    #[test]
    fn tokens_from_broken_answers() {
        assert!(matches!(
            tokens_from(&json!({"token_type": "Bearer"}), Some("r"), 0),
            Err(Error::Parse(_))
        ));
        assert!(matches!(
            tokens_from(&json!({"access_token": "a"}), None, 0),
            Err(Error::Parse(_))
        ));
        // Fehlende/kaputte Lebensdauer → eine Stunde
        let t = tokens_from(
            &json!({"access_token": "a", "expires_in": "x"}),
            Some("r"),
            0,
        )
        .unwrap();
        assert_eq!(t.expires_at, 3600);
        assert_eq!(t.scope, "");
    }

    #[test]
    fn callback_full_url() {
        assert_eq!(
            parse_callback(
                "http://127.0.0.1:8350/api/v1/spotify/callback?code=AQB%2Dx_y&state=abc123"
            ),
            Some(CallbackParams::Code {
                code: "AQB-x_y".into(),
                state: "abc123".into()
            })
        );
        assert_eq!(
            parse_callback(
                "  https://music.example.com/api/v1/spotify/callback?state=s&code=c#_=_ "
            ),
            Some(CallbackParams::Code {
                code: "c".into(),
                state: "s".into()
            })
        );
    }

    #[test]
    fn callback_fragment_and_bare_query() {
        assert_eq!(
            parse_callback("http://127.0.0.1:8350/api/v1/spotify/callback#code=c1&state=s1"),
            Some(CallbackParams::Code {
                code: "c1".into(),
                state: "s1".into()
            })
        );
        assert_eq!(
            parse_callback("code=c2&state=s2"),
            Some(CallbackParams::Code {
                code: "c2".into(),
                state: "s2".into()
            })
        );
        assert_eq!(
            parse_callback("?code=c3&state=s3"),
            Some(CallbackParams::Code {
                code: "c3".into(),
                state: "s3".into()
            })
        );
        // Teilweise in Query und Fragment
        assert_eq!(
            parse_callback("http://127.0.0.1/cb?code=c4#state=s4"),
            Some(CallbackParams::Code {
                code: "c4".into(),
                state: "s4".into()
            })
        );
    }

    #[test]
    fn callback_errors() {
        assert_eq!(
            parse_callback(
                "http://127.0.0.1:8350/api/v1/spotify/callback?error=access_denied&state=xyz"
            ),
            Some(CallbackParams::Error {
                error: "access_denied".into(),
                state: Some("xyz".into())
            })
        );
        assert_eq!(
            parse_callback("error=invalid_scope"),
            Some(CallbackParams::Error {
                error: "invalid_scope".into(),
                state: None
            })
        );
        // Fehler hat Vorrang vor einem Code
        assert!(matches!(
            parse_callback("code=c&state=s&error=server_error"),
            Some(CallbackParams::Error { .. })
        ));
    }

    #[test]
    fn callback_without_values() {
        assert_eq!(parse_callback(""), None);
        assert_eq!(parse_callback("   "), None);
        assert_eq!(
            parse_callback("http://127.0.0.1:8350/api/v1/spotify/callback"),
            None
        );
        assert_eq!(parse_callback("https://example.com/callback?foo=bar"), None);
        // Code ohne state wird nicht angenommen (state ist Pflicht gegen CSRF)
        assert_eq!(parse_callback("http://127.0.0.1/cb?code=abc"), None);
        assert_eq!(parse_callback("http://127.0.0.1/cb?code=&state=s"), None);
        assert_eq!(parse_callback("irgendein text"), None);
    }
}
