//! nrzMusic als native macOS-App – komplett in Rust, im Aussehen wie nrzX: durchsichtiges Fenster
//! mit dem Milchglas des Systems dahinter (`window-vibrancy`), die Oberfläche tönt nur
//! (`html.nrz-glass`, `ui/src/theme.rs`), Titelleiste in die Oberfläche integriert.
//!
//! Zwei Arten zu laufen (gewählt beim ersten Start, wechselbar unter nrzMusic → „Server
//! verbinden …“ oder in den Einstellungen der Oberfläche):
//!
//! * **Mit Server (empfohlen):** Die App zeigt die Oberfläche deiner nrzMusic-Instanz und streamt
//!   von dort – Bibliothek, Downloads, YouTube Music und Spotify laufen auf dem Server.
//! * **Lokal, ohne Server:** Der nrzMusic-Server läuft im selben Prozess auf `127.0.0.1`
//!   (ein Nutzer ohne Anmeldung, Daten in `~/Library/Application Support/nrzMusic`, Musik aus
//!   `~/Music/nrzMusic`). Siehe [`server`].
//!
//! Fenster schliessen lässt die Musik weiterlaufen (Klick aufs Dock-Symbol holt es zurück),
//! ⌘Q beendet. Protokoll: `~/Library/Logs/nrzMusic/nrzmusic.log`.

mod pages;
mod server;
mod settings;

use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use server::LocalServer;
use settings::{Mode, Settings};
use std::sync::Mutex;
use std::time::Duration;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use wry::{BackgroundThrottlingPolicy, NewWindowResponse, WebView, WebViewBuilder};

const ICON_PNG: &[u8] = include_bytes!("../../static/icon.png");

#[derive(Debug)]
enum UserEvent {
    Menu(MenuEvent),
    Ipc(String),
    /// Server-Instanz geprüft: Adresse oder Fehler.
    Checked(Result<String, String>),
    /// Lokaler Server gestartet (oder nicht).
    LocalReady(Result<LocalServer, String>),
    /// Inhaltsprozess von WebKit beendet (Absturz, Speicherdruck) – neu laden.
    Terminated,
}

struct MenuIds {
    connect: MenuItem,
    reload: MenuItem,
    quit: MenuItem,
}

/// Zustand der App im Haupt-Thread.
struct App {
    window: Window,
    webview: WebView,
    proxy: EventLoopProxy<UserEvent>,
    settings: Settings,
    /// Port des lokalen Servers (fest ab dem Start der App).
    local_port: u16,
    server: Option<LocalServer>,
    /// Gerade gezeigte Oberfläche (für „Abbrechen“ auf der Verbinden-Seite).
    current: Option<String>,
    icon: String,
    /// Die Verbinden-Seite ist offen.
    on_connect: bool,
}

impl App {
    fn show_connect(&mut self, error: &str) {
        let music = settings::music_dir().display().to_string().replace(&settings::home().display().to_string(), "~");
        let page = pages::connect_page(&self.icon, &self.settings.server, error, self.current.is_some(), &music);
        let _ = self.webview.load_html(&page);
        self.on_connect = true;
    }

    fn show_loading(&mut self, text: &str) {
        let _ = self.webview.load_html(&pages::loading_page(&self.icon, text));
        self.on_connect = false;
    }

    fn open(&mut self, url: String) {
        let _ = self.webview.load_url(&url);
        self.current = Some(url);
        self.on_connect = false;
        self.window.set_focus();
    }

    /// Server-Instanz im Hintergrund prüfen (`Checked`).
    fn check(&self, input: &str) {
        let candidates = settings::candidates(input);
        let p = self.proxy.clone();
        std::thread::spawn(move || {
            let _ = p.send_event(UserEvent::Checked(server::check_remote(&candidates)));
        });
    }

    /// Lokal: eingebauten Server starten (falls noch nicht) und seine Oberfläche zeigen.
    fn go_local(&mut self) {
        if let Some(url) = self.server.as_ref().map(LocalServer::url) {
            self.open(url);
            return;
        }
        self.show_loading("nrzMusic startet …");
        let p = self.proxy.clone();
        let port = self.local_port;
        std::thread::spawn(move || {
            let _ = p.send_event(UserEvent::LocalReady(server::start(port)));
        });
    }

    fn stop_local(&mut self) {
        if let Some(mut s) = self.server.take() {
            s.stop(Duration::from_secs(4));
        }
    }

    fn on_ipc(&mut self, msg: &str) {
        match msg {
            "window:drag" => {
                let _ = self.window.drag_window();
            }
            "window:zoom" => self.window.set_maximized(!self.window.is_maximized()),
            "connect:open" => self.show_connect(""),
            "connect:cancel" => {
                if let Some(url) = self.current.clone() {
                    self.open(url);
                }
            }
            "connect:local" => {
                self.settings.mode = Mode::Local;
                self.settings.save();
                self.current = None;
                self.go_local();
            }
            _ => {
                if let Some(input) = msg.strip_prefix("connect:server:") {
                    self.check(input);
                } else if let Some(url) = msg.strip_prefix("open:") {
                    open_external(url);
                }
            }
        }
    }

    fn on_checked(&mut self, result: Result<String, String>) {
        match result {
            Ok(url) => {
                tracing::info!("Verbunden mit {url}");
                self.settings.mode = Mode::Server;
                self.settings.server = url.clone();
                self.settings.save();
                // Der lokale Server wird nicht mehr gebraucht
                self.stop_local();
                self.open(url);
            }
            Err(e) => {
                tracing::warn!("Verbinden: {e}");
                if self.on_connect {
                    // Fehler unter dem Feld zeigen, Eingabe bleibt
                    let js = format!("window.__nrzConnectError && window.__nrzConnectError({})", serde_json::Value::from(e.as_str()));
                    let _ = self.webview.evaluate_script(&js);
                } else {
                    // Beim Start: Server nicht erreichbar → Verbinden-Seite mit dem Fehler
                    self.show_connect(&e);
                }
            }
        }
    }
}

fn main() {
    init_logging();

    // Nur eine Instanz: zwei Server auf derselben Datenbank vertragen sich nicht
    let _lock = match instance_lock() {
        Ok(l) => l,
        Err(e) => {
            message(&e);
            return;
        }
    };

    // Vor allen weiteren Threads (PATH für yt-dlp, ffmpeg, Deno)
    server::prepare_env();
    let local_port = server::pick_port();

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    {
        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::Menu(e));
        }));
    }
    let (menu, ids) = build_menu();

    // Titelleiste in die Oberfläche integriert: transparent, Inhalt bis oben, Ampel darüber
    let builder = WindowBuilder::new()
        .with_title("nrzMusic")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 820.0))
        // Schmaler als 768 px zeigt die Oberfläche das Handy-Layout – in der App nie
        .with_min_inner_size(tao::dpi::LogicalSize::new(820.0, 560.0))
        .with_theme(Some(tao::window::Theme::Dark))
        // Durchsichtig: dahinter liegt das Milchglas des Systems (`apply_glass`)
        .with_transparent(true);
    #[cfg(target_os = "macos")]
    let builder = {
        use tao::platform::macos::WindowBuilderExtMacOS;
        builder.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true)
    };
    let window = builder.build(&event_loop).expect("Fenster");

    let icon = format!("data:image/png;base64,{}", base64(ICON_PNG));
    let webview = build_webview(&window, &proxy, &server::local_url(local_port), &icon).expect("WebView");
    // Erst jetzt: wry setzt eine eigene Inhaltsansicht ein, das Milchglas gehört darunter
    if !apply_glass(&window) {
        tracing::warn!("Kein Milchglas – Fenster bleibt dunkel getönt");
    }

    let mut app = App {
        window,
        webview,
        proxy: proxy.clone(),
        settings: Settings::load(),
        local_port,
        server: None,
        current: None,
        icon,
        on_connect: false,
    };
    match app.settings.mode {
        Mode::Unset => app.show_connect(""),
        Mode::Local => app.go_local(),
        Mode::Server => {
            app.show_loading("Verbinde mit dem Server …");
            let server = app.settings.server.clone();
            app.check(&server);
        }
    }

    event_loop.run(move |event, _target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                #[cfg(target_os = "macos")]
                menu.init_for_nsapp();
            }
            Event::Reopen { .. } => {
                app.window.set_visible(true);
                app.window.set_focus();
            }
            // Wie bei Musik-Apps üblich: Fenster schliessen, Musik läuft weiter
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => app.window.set_visible(false),
            Event::UserEvent(ev) => match ev {
                UserEvent::Ipc(msg) => app.on_ipc(&msg),
                UserEvent::Checked(r) => app.on_checked(r),
                UserEvent::LocalReady(Ok(s)) => {
                    let url = s.url();
                    app.server = Some(s);
                    app.open(url);
                }
                UserEvent::LocalReady(Err(e)) => {
                    tracing::error!("Lokaler Server: {e}");
                    app.current = None;
                    app.show_connect(&format!("Lokaler Server startet nicht: {e}"));
                }
                UserEvent::Terminated => {
                    tracing::warn!("Inhaltsprozess beendet – lade neu");
                    match app.current.clone() {
                        Some(url) => app.open(url),
                        None => app.show_connect(""),
                    }
                }
                UserEvent::Menu(e) => {
                    if e.id == ids.connect.id() {
                        app.show_connect("");
                    } else if e.id == ids.reload.id() {
                        if let Some(url) = app.current.clone() {
                            app.open(url);
                        }
                    } else if e.id == ids.quit.id() {
                        app.stop_local();
                        *control_flow = ControlFlow::Exit;
                    }
                }
            },
            Event::LoopDestroyed => app.stop_local(),
            _ => {}
        }
    });
}

fn build_webview(window: &Window, proxy: &EventLoopProxy<UserEvent>, local: &str, icon: &str) -> wry::Result<WebView> {
    let p_ipc = proxy.clone();
    let devtools = cfg!(debug_assertions) || std::env::var("NRZMUSIC_DEVTOOLS").is_ok_and(|v| v == "1");
    let builder = WebViewBuilder::new()
        .with_initialization_script_for_main_only(pages::window_script(local), true)
        .with_transparent(true)
        .with_ipc_handler(move |req| {
            let _ = p_ipc.send_event(UserEvent::Ipc(req.body().clone()));
        })
        // Neue Fenster (Links mit target, window.open) gehen in den Browser des Systems
        .with_new_window_req_handler(|url, _features| {
            if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
                open_external(&url);
            }
            NewWindowResponse::Deny
        })
        // Musik ohne Klick weiterspielen (nächster Titel, Wiederaufnahme nach dem Start)
        .with_autoplay(true)
        .with_clipboard(true)
        .with_user_agent(user_agent())
        .with_devtools(devtools)
        .with_back_forward_navigation_gestures(false)
        // Im Hintergrund (Fenster zu) nicht drosseln: die Wiedergabe soll weiterlaufen
        .with_background_throttling(BackgroundThrottlingPolicy::Disabled)
        .with_html(pages::loading_page(icon, ""));
    #[cfg(target_os = "macos")]
    let builder = {
        use wry::WebViewBuilderExtDarwin;
        let p = proxy.clone();
        builder.with_on_web_content_process_terminate_handler(move || {
            let _ = p.send_event(UserEvent::Terminated);
        })
    };
    builder.build(window)
}

/// Safari-ähnlich (YouTube-Player und Co. erwarten einen bekannten Browser) mit Merkmal der App.
fn user_agent() -> String {
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15 nrzMusicMac/{}",
        env!("CARGO_PKG_VERSION")
    )
}

/// Milchglas hinter der Oberfläche: Vibrancy (`NSVisualEffectView`) unter dem WebView in der
/// Inhaltsansicht, die wry eingesetzt hat. `false`, wenn das nicht geht.
fn apply_glass(window: &Window) -> bool {
    #[cfg(target_os = "macos")]
    {
        let Some(view) = content_view(window) else { return false };
        window_vibrancy::apply_vibrancy(
            view,
            window_vibrancy::NSVisualEffectMaterial::UnderWindowBackground,
            Some(window_vibrancy::NSVisualEffectState::Active),
            None,
        )
        .is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        false
    }
}

/// Aktuelle Inhaltsansicht als Handle für window-vibrancy (wry ersetzt die von tao; `Window`
/// meldet aber weiter die alte, nicht mehr eingehängte).
#[cfg(target_os = "macos")]
struct ContentView(std::ptr::NonNull<std::ffi::c_void>);

#[cfg(target_os = "macos")]
impl tao::rwh_06::HasWindowHandle for ContentView {
    fn window_handle(&self) -> Result<tao::rwh_06::WindowHandle<'_>, tao::rwh_06::HandleError> {
        let raw = tao::rwh_06::RawWindowHandle::AppKit(tao::rwh_06::AppKitWindowHandle::new(self.0));
        // SAFETY: Das Fenster hält seine Inhaltsansicht, solange es offen ist; das Handle wird nur
        // sofort im Haupt-Thread benutzt.
        Ok(unsafe { tao::rwh_06::WindowHandle::borrow_raw(raw) })
    }
}

#[cfg(target_os = "macos")]
fn content_view(window: &Window) -> Option<ContentView> {
    use objc2_app_kit::NSWindow;
    use tao::platform::macos::WindowExtMacOS;
    // SAFETY: NSWindow des lebenden tao-Fensters, Aufruf im Haupt-Thread.
    let ns = unsafe { (window.ns_window() as *const NSWindow).as_ref()? };
    let view = ns.contentView()?;
    std::ptr::NonNull::new(objc2::rc::Retained::as_ptr(&view) as *mut std::ffi::c_void).map(ContentView)
}

fn build_menu() -> (Menu, MenuIds) {
    let ids = MenuIds {
        connect: MenuItem::new("Server verbinden …", true, Some("CmdOrCtrl+Shift+K".parse().unwrap())),
        reload: MenuItem::new("Neu laden", true, Some("CmdOrCtrl+R".parse().unwrap())),
        quit: MenuItem::new("nrzMusic beenden", true, Some("CmdOrCtrl+Q".parse().unwrap())),
    };
    let about = AboutMetadata {
        name: Some("nrzMusic".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some("Selbst gehosteter Musik-Client – komplett in Rust.".into()),
        license: Some("AGPL-3.0-or-later".into()),
        ..Default::default()
    };
    let app = Submenu::with_items(
        "nrzMusic",
        true,
        &[
            &PredefinedMenuItem::about(Some("Über nrzMusic"), Some(about)),
            &PredefinedMenuItem::separator(),
            &ids.connect,
            &ids.reload,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &ids.quit,
        ],
    )
    .unwrap();
    // Ohne diese Einträge gingen ⌘C, ⌘V, ⌘X, ⌘A und ⌘Z im WebView nicht
    let edit = Submenu::with_items(
        "Bearbeiten",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ],
    )
    .unwrap();
    let window = Submenu::with_items(
        "Fenster",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(Some("Fenster schliessen")),
        ],
    )
    .unwrap();
    let menu = Menu::with_items(&[&app, &edit, &window]).unwrap();
    (menu, ids)
}

fn open_external(target: &str) {
    let _ = std::process::Command::new("/usr/bin/open").arg(target).spawn();
}

fn message(text: &str) {
    rfd::MessageDialog::new()
        .set_title("nrzMusic")
        .set_description(text)
        .set_level(rfd::MessageLevel::Info)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

fn instance_lock() -> Result<std::fs::File, String> {
    let dir = settings::data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("app.lock"))
        .map_err(|e| e.to_string())?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(_) => Err("nrzMusic läuft bereits.".into()),
    }
}

/// Protokoll nach `~/Library/Logs/nrzMusic/nrzmusic.log` (ab 10 MB beiseitegelegt).
fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let dir = settings::log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let filter = || {
        EnvFilter::try_from_env("NRZMUSIC_LOG").unwrap_or_else(|_| {
            EnvFilter::new("info,rocket::server=warn,rocket::launch::_=warn,rocket::shield=warn,lofty=error")
        })
    };
    let path = dir.join("nrzmusic.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 10 * 1024 * 1024) {
        let _ = std::fs::rename(&path, dir.join("nrzmusic.old.log"));
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => tracing_subscriber::fmt().with_ansi(false).with_env_filter(filter()).with_writer(Mutex::new(file)).init(),
        Err(_) => tracing_subscriber::fmt().with_env_filter(filter()).init(),
    }
}

/// Base64 (Standard-Alphabet, mit `=`) – nur für das Symbol der App-eigenen Seiten.
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
