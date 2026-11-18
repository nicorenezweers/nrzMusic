//! Sicherheits-Header für jede Antwort. Die Content-Security-Policy erlaubt nur eigene
//! Inhalte (Oberfläche, CSS, Schrift sind eingebettet, siehe routes::spa und routes::assets;
//! Audio – auch YouTube – kommt über den Server) sowie Vorschaubilder von YouTubes und
//! Spotifys Bild-Servern. WebAssembly braucht 'wasm-unsafe-eval', Dioxus' `eval`
//! 'unsafe-eval'. Inline-Stile bleiben erlaubt (Dioxus setzt Stile direkt am Element).

use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Header;
use rocket::{Request, Response};

pub struct SecurityHeaders;

pub const CSP: &str = "default-src 'self'; \
    script-src 'self' 'wasm-unsafe-eval' 'unsafe-eval'; \
    style-src 'self' 'unsafe-inline'; \
    font-src 'self'; \
    img-src 'self' data: blob: https://*.ytimg.com https://*.googleusercontent.com https://*.ggpht.com https://*.scdn.co https://*.spotifycdn.com; \
    media-src 'self' blob:; \
    frame-src 'none'; \
    worker-src 'self' blob:; \
    connect-src 'self'; \
    object-src 'none'; \
    frame-ancestors 'self'; \
    base-uri 'self'; \
    form-action 'self'";

#[rocket::async_trait]
impl Fairing for SecurityHeaders {
    fn info(&self) -> Info {
        Info {
            name: "Sicherheits-Header",
            kind: Kind::Response,
        }
    }

    async fn on_response<'r>(&self, _req: &'r Request<'_>, res: &mut Response<'r>) {
        for (name, value) in [
            ("Content-Security-Policy", CSP),
            ("X-Content-Type-Options", "nosniff"),
            ("X-Frame-Options", "SAMEORIGIN"),
            ("Referrer-Policy", "same-origin"),
            (
                "Permissions-Policy",
                "camera=(), microphone=(), geolocation=()",
            ),
            // Links "Auf YouTube Music öffnen" öffnen ein Fenster
            ("Cross-Origin-Opener-Policy", "same-origin-allow-popups"),
        ] {
            if !res.headers().contains(name) {
                res.set_header(Header::new(name, value));
            }
        }
    }
}
