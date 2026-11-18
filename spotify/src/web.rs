//! Anmeldung wie der Spotify-Webplayer: mit dem Cookie `sp_dc` von open.spotify.com holt
//! sich der Server dieselben kurzlebigen Tokens, die auch der Webplayer benutzt. Damit
//! braucht es weder eine eigene Spotify-App noch Premium beim App-Besitzer.
//!
//! Der Token-Endpunkt (`GET open.spotify.com/api/token`) verlangt seit 2025 einen TOTP-Code.
//! Das Geheimnis dafür steckt im Webplayer und wechselt immer wieder; nrzMusic liest die
//! aktuellen Geheimnisse aus einer Liste, die die Community stündlich aus dem Webplayer
//! zieht ([`DEFAULT_SECRETS_URL`], änderbar mit `NRZMUSIC_SPOTIFY_SECRETS_URL`).
//!
//! Nicht offiziell: Spotify kann das jederzeit ändern, und die Nutzungsbedingungen erlauben
//! es nicht.

use std::time::{Duration, Instant};

use hmac::{Hmac, KeyInit, Mac};
use serde_json::Value;
use sha1::Sha1;

use crate::{Error, Result, Tokens};

/// `Tokens::scope` einer Anmeldung über das Cookie (`refresh_token` ist dann `sp_dc`).
pub const WEB_PLAYER_SCOPE: &str = "web-player";

/// Liste der TOTP-Geheimnisse (`{ "61": [44, 55, …], … }`).
pub const DEFAULT_SECRETS_URL: &str =
    "https://code.thetadev.de/ThetaDev/spotify-secrets/raw/branch/main/secrets/secretDict.json";

/// So lange gilt die geladene Liste, bevor sie neu geholt wird.
pub(crate) const SECRETS_TTL: Duration = Duration::from_secs(60 * 60);

/// Webplayer-Tokens gelten eine Stunde; fehlt die Angabe, so lange.
const DEFAULT_LIFETIME_SECS: i64 = 3600;

/// Wie ein gewöhnlicher Browser – der Endpunkt lehnt Fremde gern ab.
pub(crate) const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/146.0 Safari/537.36";

/// Ein TOTP-Geheimnis mit seiner Version (`totpVer`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Secret {
    pub version: u32,
    /// Schlüssel für HMAC-SHA1 (schon umgewandelt)
    pub key: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct SecretCache {
    pub loaded: Option<(Instant, Secret)>,
}

impl SecretCache {
    pub fn fresh(&self) -> Option<Secret> {
        self.loaded
            .as_ref()
            .filter(|(at, _)| at.elapsed() < SECRETS_TTL)
            .map(|(_, s)| s.clone())
    }
}

/// Den Wert von `sp_dc` aus der Eingabe holen: der Wert allein, `sp_dc=…` oder eine ganze
/// Cookie-Zeile aus den Entwicklertools.
pub fn parse_cookie(input: &str) -> Option<String> {
    let s = input.trim().trim_matches('"').trim();
    let value = if s.contains("sp_dc=") {
        s.split(';')
            .filter_map(|part| part.trim().strip_prefix("sp_dc="))
            .next()?
    } else {
        s
    };
    let value = value.trim().trim_matches('"');
    let ok = value.len() >= 20
        && value.len() <= 1024
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then(|| value.to_string())
}

/// Die neueste Version aus der Liste, ihr Schlüssel so umgewandelt wie im Webplayer: jedes
/// Byte mit `(i % 33) + 9` verknüpft (XOR), die Zahlen dezimal aneinandergehängt.
pub(crate) fn newest_secret(list: &Value) -> Option<Secret> {
    list.as_object()?
        .iter()
        .filter_map(|(version, bytes)| {
            let version: u32 = version.trim().parse().ok()?;
            let bytes: Vec<u8> = bytes
                .as_array()?
                .iter()
                .map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()))
                .collect::<Option<_>>()?;
            (!bytes.is_empty()).then_some((version, bytes))
        })
        .max_by_key(|(v, _)| *v)
        .map(|(version, bytes)| Secret {
            version,
            key: transform(&bytes),
        })
}

fn transform(cipher: &[u8]) -> Vec<u8> {
    cipher
        .iter()
        .enumerate()
        .map(|(i, &b)| (b ^ ((i % 33) as u8 + 9)).to_string())
        .collect::<String>()
        .into_bytes()
}

/// TOTP nach RFC 6238 (HMAC-SHA1, 30 Sekunden, 6 Stellen).
pub(crate) fn totp(key: &[u8], unix_secs: i64) -> String {
    let counter = (unix_secs.max(0) as u64) / 30;
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(key).expect("HMAC nimmt jeden Schlüssel");
    mac.update(&counter.to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = (hash[hash.len() - 1] & 0x0f) as usize;
    let code = u32::from_be_bytes([
        hash[offset] & 0x7f,
        hash[offset + 1],
        hash[offset + 2],
        hash[offset + 3],
    ]) % 1_000_000;
    format!("{code:06}")
}

/// Antwort von `/api/token` → [`Tokens`]. Anonym heisst: Cookie ungültig oder abgelaufen.
pub(crate) fn tokens_from(v: &Value, sp_dc: &str, now: i64) -> Result<Tokens> {
    if v.get("isAnonymous").and_then(Value::as_bool) == Some(true) {
        return Err(Error::Unauthorized);
    }
    let access_token = v
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| Error::Parse("Webplayer-Token fehlt".into()))?;
    let expires_at = v
        .get("accessTokenExpirationTimestampMs")
        .and_then(Value::as_i64)
        .map(|ms| ms / 1000)
        .filter(|&s| s > now)
        .unwrap_or(now + DEFAULT_LIFETIME_SECS);
    Ok(Tokens {
        access_token: access_token.to_string(),
        refresh_token: sp_dc.to_string(),
        expires_at,
        scope: WEB_PLAYER_SCOPE.to_string(),
    })
}

/// Spotify meldet ein veraltetes TOTP-Geheimnis (dann die Liste neu laden).
pub(crate) fn totp_rejected(status: u16, body: &str) -> bool {
    status == 400 || body.contains("totpVerExpired") || body.contains("totp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totp_rfc6238() {
        // RFC 6238, Anhang B (SHA1): 94287082 bei T=59, 07081804 bei T=1111111109
        let key = b"12345678901234567890";
        assert_eq!(totp(key, 59), "287082");
        assert_eq!(totp(key, 1_111_111_109), "081804");
        assert_eq!(totp(key, 1_234_567_890), "005924");
    }

    #[test]
    fn picks_newest_secret() {
        let list: Value =
            serde_json::from_str(r#"{"9":[1,2],"61":[44,55,47],"10":[3],"x":[1],"62":[]}"#)
                .unwrap();
        let s = newest_secret(&list).unwrap();
        assert_eq!(s.version, 61);
        // 44^9=37, 55^10=61, 47^11=36
        assert_eq!(s.key, b"376136".to_vec());
        assert_eq!(newest_secret(&serde_json::json!({})), None);
    }

    #[test]
    fn cookie_input() {
        let v = "AQB1234567890abcdefXYZ_-1234";
        assert_eq!(parse_cookie(v).as_deref(), Some(v));
        assert_eq!(parse_cookie(&format!(" sp_dc={v} ")).as_deref(), Some(v));
        assert_eq!(
            parse_cookie(&format!("sp_t=abc; sp_dc={v}; sp_key=x")).as_deref(),
            Some(v)
        );
        assert_eq!(parse_cookie("kurz"), None);
        assert_eq!(parse_cookie("sp_t=abc; foo=bar"), None);
        assert_eq!(parse_cookie("mit leerzeichen und so weiter"), None);
    }

    #[test]
    fn token_response() {
        let v = serde_json::json!({
            "accessToken": "BQAtoken",
            "accessTokenExpirationTimestampMs": 1_800_003_600_000i64,
            "isAnonymous": false,
        });
        let t = tokens_from(&v, "cookie", 1_800_000_000).unwrap();
        assert_eq!(t.access_token, "BQAtoken");
        assert_eq!(t.refresh_token, "cookie");
        assert_eq!(t.expires_at, 1_800_003_600);
        assert_eq!(t.scope, WEB_PLAYER_SCOPE);
        let anon = serde_json::json!({ "accessToken": "x", "isAnonymous": true });
        assert!(matches!(tokens_from(&anon, "c", 0), Err(Error::Unauthorized)));
    }
}
