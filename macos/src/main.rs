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

#[cfg(target_os = "macos")]
mod lights;
mod pages;
mod server;
mod settings;
mod updater;

use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use server::LocalServer;
use settings::{Mode, Settings};
use std::sync::Mutex;
use std::time::Duration;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use wry::{BackgroundThrottlingPolicy, NewWindowResponse, PageLoadEvent, WebView, WebViewBuilder};

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
    /// Eine Seite ist geladen (bzw. beginnt zu laden).
    PageLoad { finished: bool },
    /// Titel der Seite („Song – nrzMusic“) → Fenstertitel (Fenster-Menü, Mission Control).
    Title(String),
    /// Ergebnis einer Update-Prüfung; `manual` = über das Menü (Ergebnis als Dialog).
    UpdateChecked { result: Result<Option<updater::Update>, String>, manual: bool },
    /// Fortschritt des Update-Downloads (0…1, −1 unbekannt).
    UpdateProgress(f64),
    /// Update geladen (Pfad der DMG) oder Fehler.
    UpdateDownloaded(Result<std::path::PathBuf, String>),
}

/// Stand der Updates.
#[derive(Default)]
struct Updates {
    /// Gefundene neuere Version.
    found: Option<updater::Update>,
    /// Läuft gerade ein Download?
    downloading: bool,
    progress: f64,
    /// Fertig geladene DMG (zu `found`).
    ready: Option<std::path::PathBuf>,
}

struct MenuIds {
    settings: MenuItem,
    connect: MenuItem,
    reload: MenuItem,
    search: MenuItem,
    toggle: MenuItem,
    next: MenuItem,
    prev: MenuItem,
    forward: MenuItem,
    backward: MenuItem,
    show: MenuItem,
    check: MenuItem,
    install: MenuItem,
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
    /// Das Fenster erscheint erst mit der ersten geladenen Seite (kein leeres Aufblitzen).
    shown: bool,
    updates: Updates,
    /// Laufzeit für Update-Prüfung und Download.
    rt: tokio::runtime::Runtime,
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

    /// Fenster zeigen (erste Seite geladen, Dock-Symbol, Menü).
    fn show(&mut self) {
        self.shown = true;
        self.window.set_visible(true);
        self.window.set_focus();
    }

    /// Lage der Ampel an die Seite (oberste Reihe richtet sich danach).
    fn send_lights(&self) {
        #[cfg(target_os = "macos")]
        {
            let js = match lights::spot(&self.window) {
                Some((y, r)) => format!("window.__nrzLights && window.__nrzLights({y:.1}, {r:.1})"),
                None => "window.__nrzLights && window.__nrzLights(26, 0)".to_string(),
            };
            let _ = self.webview.evaluate_script(&js);
        }
    }

    /// Lage und Grösse merken (beim Schliessen und Beenden).
    fn save_frame(&mut self) {
        if self.window.is_maximized() || self.window.fullscreen().is_some() || !self.window.is_visible() {
            return;
        }
        let scale = self.window.scale_factor();
        let (Ok(pos), size) = (self.window.outer_position(), self.window.inner_size()) else { return };
        let pos = pos.to_logical::<f64>(scale);
        let size = size.to_logical::<f64>(scale);
        self.settings.window = Some(settings::Frame { x: pos.x, y: pos.y, w: size.width, h: size.height });
        self.settings.save();
    }

    /// Stand der Updates an die Seite (Knopf unten in der Seitenleiste).
    fn send_update(&self) {
        let u = &self.updates;
        let js = match (&u.found, &u.ready, u.downloading) {
            (Some(f), Some(_), _) => format!("window.__nrzUpdate && window.__nrzUpdate('ready', '{}')", f.version),
            (Some(f), None, true) => format!("window.__nrzUpdate && window.__nrzUpdate('progress', '{}', {:.3})", f.version, u.progress),
            _ => "window.__nrzUpdate && window.__nrzUpdate('none')".to_string(),
        };
        self.js(&js);
    }

    /// Nach Updates fragen (im Hintergrund).
    fn check_updates(&self, manual: bool) {
        let p = self.proxy.clone();
        self.rt.spawn(async move {
            let result = updater::check(&updater::current_version()).await;
            let _ = p.send_event(UserEvent::UpdateChecked { result, manual });
        });
    }

    fn on_update_checked(&mut self, result: Result<Option<updater::Update>, String>, manual: bool, ids: &MenuIds) {
        match result {
            Ok(Some(update)) => {
                let same = self.updates.found.as_ref().is_some_and(|f| f.version == update.version);
                if same && (self.updates.downloading || self.updates.ready.is_some()) {
                    if manual {
                        let what = if self.updates.ready.is_some() { "ist bereit – unten in der Seitenleiste oder im Menü „Update installieren“ klicken" } else { "wird gerade geladen" };
                        message(&format!("nrzMusic {} {what}.", update.version));
                    }
                    return;
                }
                tracing::info!("Update gefunden: {}", update.version);
                self.updates = Updates { found: Some(update.clone()), downloading: true, progress: 0.0, ready: None };
                ids.install.set_enabled(false);
                ids.install.set_text(format!("Update {} wird geladen …", update.version));
                self.send_update();
                // Automatisch im Hintergrund laden
                let p = self.proxy.clone();
                self.rt.spawn(async move {
                    let last = std::sync::Mutex::new(std::time::Instant::now() - Duration::from_secs(1));
                    let r = updater::download(&update, |f| {
                        let mut l = last.lock().unwrap();
                        if l.elapsed() > Duration::from_millis(150) {
                            *l = std::time::Instant::now();
                            let _ = p.send_event(UserEvent::UpdateProgress(f));
                        }
                    })
                    .await;
                    let _ = p.send_event(UserEvent::UpdateDownloaded(r));
                });
            }
            Ok(None) => {
                if manual {
                    message(&format!("nrzMusic ist aktuell (Version {}).", updater::current_version()));
                }
            }
            Err(e) => {
                tracing::warn!("Update-Prüfung: {e}");
                if manual {
                    message(&format!("Nach Updates suchen ging nicht:\n{e}"));
                }
            }
        }
    }

    fn on_update_downloaded(&mut self, result: Result<std::path::PathBuf, String>, ids: &MenuIds) {
        self.updates.downloading = false;
        match result {
            Ok(path) => {
                let v = self.updates.found.as_ref().map(|f| f.version.to_string()).unwrap_or_default();
                tracing::info!("Update {v} geladen: {}", path.display());
                self.updates.ready = Some(path);
                ids.install.set_text(format!("Update auf {v} installieren und neu starten"));
                ids.install.set_enabled(true);
            }
            Err(e) => {
                tracing::warn!("Update-Download: {e}");
                // Beim nächsten Prüfen neu versuchen
                self.updates = Updates::default();
                ids.install.set_text("Update installieren …");
                ids.install.set_enabled(false);
            }
        }
        self.send_update();
    }

    /// Geladenes Update einsetzen und neu starten. `true`, wenn die App jetzt beenden soll.
    fn install_update(&mut self, ids: &MenuIds) -> bool {
        let (Some(found), Some(dmg)) = (self.updates.found.clone(), self.updates.ready.clone()) else { return false };
        self.js(&format!("window.__nrzUpdate && window.__nrzUpdate('installing', '{}')", found.version));
        ids.install.set_enabled(false);
        self.save_frame();
        match updater::install(&dmg) {
            Ok(updater::Installed::Replaced(bundle)) => {
                tracing::info!("Update {} installiert – Neustart", found.version);
                self.stop_local();
                updater::relaunch_after_exit(&bundle);
                true
            }
            Ok(updater::Installed::OpenedDmg(msg)) => {
                message(&msg);
                false
            }
            Err(e) => {
                tracing::error!("Update installieren: {e}");
                message(&format!("Update lässt sich nicht installieren:\n{e}"));
                ids.install.set_enabled(true);
                self.send_update();
                false
            }
        }
    }

    fn js(&self, js: &str) {
        let _ = self.webview.evaluate_script(js);
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

    let settings = Settings::load();
    let frame = settings.window.filter(|f| f.w >= 820.0 && f.h >= 560.0 && on_screen(&event_loop, f));

    // Titelleiste in die Oberfläche integriert: transparent, Inhalt bis oben, Ampel darüber
    let builder = WindowBuilder::new()
        .with_title("nrzMusic")
        // Erst sichtbar, wenn die erste Seite steht
        .with_visible(false)
        .with_inner_size(frame.map_or(tao::dpi::LogicalSize::new(1280.0, 820.0), |f| tao::dpi::LogicalSize::new(f.w, f.h)))
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
    let builder = match frame {
        Some(f) => builder.with_position(tao::dpi::LogicalPosition::new(f.x, f.y)),
        None => builder,
    };
    let window = builder.build(&event_loop).expect("Fenster");
    #[cfg(target_os = "macos")]
    lights::install(&window);
    if frame.is_none() {
        center(&window);
    }

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
        settings,
        local_port,
        server: None,
        current: None,
        icon,
        on_connect: false,
        shown: false,
        updates: Updates::default(),
        rt: tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("Tokio"),
    };
    // Updates: kurz nach dem Start, dann alle sechs Stunden
    {
        let p = proxy.clone();
        app.rt.spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            loop {
                let result = updater::check(&updater::current_version()).await;
                let _ = p.send_event(UserEvent::UpdateChecked { result, manual: false });
                tokio::time::sleep(updater::CHECK_INTERVAL).await;
            }
        });
    }
    // Spätestens nach 2 s zeigen, auch wenn die Seite noch lädt
    {
        let p = proxy.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            let _ = p.send_event(UserEvent::PageLoad { finished: true });
        });
    }
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
            Event::Reopen { .. } => app.show(),
            // Wie bei Musik-Apps üblich: Fenster schliessen, Musik läuft weiter
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                app.save_frame();
                app.window.set_visible(false);
            }
            Event::WindowEvent { event: WindowEvent::Resized(_), .. } => app.send_lights(),
            Event::UserEvent(ev) => match ev {
                UserEvent::Ipc(msg) if msg == "update:install" => {
                    if app.install_update(&ids) {
                        *control_flow = ControlFlow::Exit;
                    }
                }
                UserEvent::Ipc(msg) => app.on_ipc(&msg),
                UserEvent::UpdateChecked { result, manual } => app.on_update_checked(result, manual, &ids),
                UserEvent::UpdateProgress(f) => {
                    app.updates.progress = f;
                    app.send_update();
                }
                UserEvent::UpdateDownloaded(r) => app.on_update_downloaded(r, &ids),
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
                UserEvent::PageLoad { finished } => {
                    // Durchsichtig bleiben, auch wenn WebKit für eine andere Adresse einen neuen
                    // Inhaltsprozess startet
                    let _ = app.webview.set_background_color((0, 0, 0, 0));
                    if finished {
                        app.send_lights();
                        app.send_update();
                        if !app.shown {
                            app.show();
                        }
                    }
                }
                UserEvent::Title(t) => {
                    let t = t.trim();
                    app.window.set_title(if t.is_empty() { "nrzMusic" } else { t });
                }
                UserEvent::Menu(e) => {
                    let id = &e.id;
                    if id == ids.connect.id() {
                        app.show();
                        app.show_connect("");
                    } else if id == ids.settings.id() {
                        app.show();
                        app.js("window.__nrzOpenSettings && window.__nrzOpenSettings()");
                    } else if id == ids.search.id() {
                        app.show();
                        app.js("window.__nrzFocusSearch && window.__nrzFocusSearch()");
                    } else if id == ids.reload.id() {
                        if let Some(url) = app.current.clone() {
                            app.open(url);
                        }
                    } else if id == ids.show.id() {
                        app.show();
                    } else if id == ids.toggle.id() {
                        app.js("window.__nrzMedia && window.__nrzMedia('toggle')");
                    } else if id == ids.next.id() {
                        app.js("window.__nrzMedia && window.__nrzMedia('next')");
                    } else if id == ids.prev.id() {
                        app.js("window.__nrzMedia && window.__nrzMedia('prev')");
                    } else if id == ids.forward.id() {
                        app.js("window.__nrzMedia && window.__nrzMedia('forward')");
                    } else if id == ids.backward.id() {
                        app.js("window.__nrzMedia && window.__nrzMedia('backward')");
                    } else if id == ids.check.id() {
                        app.check_updates(true);
                    } else if id == ids.install.id() {
                        if app.install_update(&ids) {
                            *control_flow = ControlFlow::Exit;
                        }
                    } else if id == ids.quit.id() {
                        app.save_frame();
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
    let p_load = proxy.clone();
    let p_title = proxy.clone();
    let devtools = cfg!(debug_assertions) || std::env::var("NRZMUSIC_DEVTOOLS").is_ok_and(|v| v == "1");
    let builder = WebViewBuilder::new()
        .with_initialization_script_for_main_only(pages::window_script(local), true)
        .with_transparent(true)
        .with_ipc_handler(move |req| {
            let _ = p_ipc.send_event(UserEvent::Ipc(req.body().clone()));
        })
        .with_on_page_load_handler(move |ev, _url| {
            let _ = p_load.send_event(UserEvent::PageLoad { finished: matches!(ev, PageLoadEvent::Finished) });
        })
        .with_document_title_changed_handler(move |title| {
            let _ = p_title.send_event(UserEvent::Title(title));
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
    let acc = |s: &str| Some(s.parse().unwrap());
    let ids = MenuIds {
        settings: MenuItem::new("Einstellungen …", true, acc("CmdOrCtrl+Comma")),
        connect: MenuItem::new("Server verbinden …", true, acc("CmdOrCtrl+Shift+K")),
        reload: MenuItem::new("Neu laden", true, acc("CmdOrCtrl+R")),
        search: MenuItem::new("Suchen", true, acc("CmdOrCtrl+F")),
        toggle: MenuItem::new("Abspielen/Pause", true, None),
        next: MenuItem::new("Nächster Titel", true, acc("CmdOrCtrl+ArrowRight")),
        prev: MenuItem::new("Vorheriger Titel", true, acc("CmdOrCtrl+ArrowLeft")),
        forward: MenuItem::new("10 Sekunden vor", true, acc("CmdOrCtrl+Alt+ArrowRight")),
        backward: MenuItem::new("10 Sekunden zurück", true, acc("CmdOrCtrl+Alt+ArrowLeft")),
        show: MenuItem::new("nrzMusic", true, acc("CmdOrCtrl+Digit0")),
        check: MenuItem::new("Nach Updates suchen …", true, None),
        install: MenuItem::new("Update installieren …", false, None),
        quit: MenuItem::new("nrzMusic beenden", true, acc("CmdOrCtrl+Q")),
    };
    let about = AboutMetadata {
        name: Some("nrzMusic".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some("Selbst gehosteter Musik-Client – komplett in Rust.".into()),
        license: Some("AGPL-3.0-or-later".into()),
        ..Default::default()
    };
    let sep = PredefinedMenuItem::separator;
    let app = Submenu::with_items(
        "nrzMusic",
        true,
        &[
            &PredefinedMenuItem::about(Some("Über nrzMusic"), Some(about)),
            &ids.check,
            &ids.install,
            &sep(),
            &ids.settings,
            &ids.connect,
            &sep(),
            &PredefinedMenuItem::services(None),
            &sep(),
            &PredefinedMenuItem::hide(Some("nrzMusic ausblenden")),
            &PredefinedMenuItem::hide_others(Some("Andere ausblenden")),
            &PredefinedMenuItem::show_all(Some("Alle einblenden")),
            &sep(),
            &ids.quit,
        ],
    )
    .unwrap();
    // Ohne diese Einträge gingen ⌘C, ⌘V, ⌘X, ⌘A und ⌘Z im WebView nicht
    let edit = Submenu::with_items(
        "Bearbeiten",
        true,
        &[
            &PredefinedMenuItem::undo(Some("Widerrufen")),
            &PredefinedMenuItem::redo(Some("Wiederholen")),
            &sep(),
            &PredefinedMenuItem::cut(Some("Ausschneiden")),
            &PredefinedMenuItem::copy(Some("Kopieren")),
            &PredefinedMenuItem::paste(Some("Einsetzen")),
            &PredefinedMenuItem::select_all(Some("Alles auswählen")),
            &sep(),
            &ids.search,
        ],
    )
    .unwrap();
    let view = Submenu::with_items(
        "Darstellung",
        true,
        &[&ids.reload, &sep(), &PredefinedMenuItem::fullscreen(Some("Vollbild"))],
    )
    .unwrap();
    let controls = Submenu::with_items(
        "Steuerung",
        true,
        &[&ids.toggle, &ids.next, &ids.prev, &sep(), &ids.forward, &ids.backward],
    )
    .unwrap();
    let window = Submenu::with_items(
        "Fenster",
        true,
        &[
            &PredefinedMenuItem::minimize(Some("Im Dock ablegen")),
            &PredefinedMenuItem::maximize(Some("Zoomen")),
            &sep(),
            &ids.show,
            &sep(),
            &PredefinedMenuItem::close_window(Some("Fenster schliessen")),
        ],
    )
    .unwrap();
    let menu = Menu::with_items(&[&app, &edit, &view, &controls, &window]).unwrap();
    (menu, ids)
}

/// Liegt der gemerkte Rahmen (zumindest mit der oberen linken Ecke) auf einem Bildschirm?
fn on_screen(event_loop: &tao::event_loop::EventLoop<UserEvent>, f: &settings::Frame) -> bool {
    event_loop.available_monitors().any(|m| {
        let s = m.scale_factor();
        let p = m.position().to_logical::<f64>(s);
        let z = m.size().to_logical::<f64>(s);
        f.x + 80.0 >= p.x && f.x + 80.0 <= p.x + z.width && f.y >= p.y - 40.0 && f.y + 40.0 <= p.y + z.height
    })
}

/// Erstes Öffnen: mittig auf dem Hauptbildschirm (etwas über der Mitte, wie AppKit).
fn center(window: &Window) {
    let Some(m) = window.current_monitor() else { return };
    let s = m.scale_factor();
    let p = m.position().to_logical::<f64>(s);
    let z = m.size().to_logical::<f64>(s);
    let w = window.outer_size().to_logical::<f64>(s);
    let x = p.x + (z.width - w.width) / 2.0;
    let y = p.y + ((z.height - w.height) / 2.0 * 0.8).max(24.0);
    window.set_outer_position(tao::dpi::LogicalPosition::new(x, y));
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
