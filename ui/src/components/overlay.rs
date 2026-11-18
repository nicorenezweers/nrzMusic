//! Dialog, Bestätigung, Menü und Meldungen – Aussehen von rust-ui (Dialog, Alert Dialog,
//! Dropdown Menu, Toast), der Zustand liegt hier in Dioxus-Signalen. Ersetzt überall die
//! Browser-Popups (`confirm`, `alert`, `prompt`).

use crate::i18n::tr;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use futures_channel::oneshot;
use std::cell::RefCell;
use std::rc::Rc;
use tw_merge::tw_merge;
use wasm_bindgen::JsCast;

// ------------------------------------------------------------------ Dialog

/// Erstes Eingabefeld im obersten Dialog fokussieren, sonst den Dialog selbst (für Escape).
fn focus_dialog() {
    let Some(doc) = crate::util::document() else { return };
    let Ok(list) = doc.query_selector_all("[role=dialog]") else { return };
    let Some(d) = list.item(list.length().saturating_sub(1)).and_then(|n| n.dyn_into::<web_sys::Element>().ok()) else { return };
    let field = d
        .query_selector("[autofocus], input:not([type=hidden]):not([readonly]):not([disabled]), textarea:not([readonly]), select:not([disabled])")
        .ok()
        .flatten()
        .or_else(|| d.parent_element());
    if let Some(el) = field.and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()) {
        let _ = el.focus();
    }
}

/// Modales Fenster. Schliesst mit Esc, Klick daneben (abschaltbar) oder dem X.
#[component]
pub fn Modal(
    open: Signal<bool>,
    #[props(into)] title: String,
    #[props(into, optional)] description: Option<String>,
    #[props(into, optional)] class: Option<String>,
    #[props(default = true)] dismissible: bool,
    children: Element,
) -> Element {
    if !open() {
        return rsx! {};
    }
    let content = tw_merge!(
        "relative bg-background border rounded-2xl shadow-lg p-6 w-full max-w-lg max-h-[85vh] overflow-y-auto animate-in",
        class.as_deref().unwrap_or("")
    );
    rsx! {
        div {
            class: "fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-4 backdrop-blur-[2px]",
            tabindex: "-1",
            onmounted: move |_| {
                spawn(async move {
                    gloo_timers::future::TimeoutFuture::new(0).await;
                    focus_dialog();
                });
            },
            onkeydown: move |e| if e.key() == Key::Escape && dismissible { open.set(false) },
            onclick: move |_| if dismissible { open.set(false) },
            div {
                class: "{content}",
                role: "dialog",
                "aria-modal": "true",
                onclick: move |e| e.stop_propagation(),
                button {
                    class: "absolute top-4 right-4 rounded-sm p-1 text-muted-foreground hover:text-foreground focus:outline-none focus:ring-2 focus:ring-ring",
                    title: tr("Schliessen"),
                    onclick: move |_| open.set(false),
                    icons::X { class: "size-4 pointer-events-none" }
                }
                div { class: "flex flex-col gap-2 pr-6",
                    h3 { class: "text-lg leading-none font-semibold", "{title}" }
                    if let Some(d) = description {
                        p { class: "text-muted-foreground text-sm", "{d}" }
                    }
                }
                div { class: "mt-5", {children} }
            }
        }
    }
}

// ------------------------------------------------------------------ Bestätigung

#[derive(Clone)]
pub struct Ask {
    pub title: String,
    pub description: String,
    pub confirm: String,
    pub destructive: bool,
}

impl Ask {
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Ask { title: title.into(), description: description.into(), confirm: tr("Bestätigen"), destructive: false }
    }
    pub fn confirm(mut self, label: impl Into<String>) -> Self {
        self.confirm = label.into();
        self
    }
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}

type Pending = Rc<RefCell<Option<oneshot::Sender<bool>>>>;

static ASK: GlobalSignal<Option<(Ask, Pending)>> = Signal::global(|| None);

/// Eigene Rückfrage statt `window.confirm` – `true`, wenn bestätigt.
pub async fn confirm(ask: Ask) -> bool {
    let (tx, rx) = oneshot::channel();
    if let Some((_, old)) = ASK.write().replace((ask, Rc::new(RefCell::new(Some(tx)))))
        && let Some(s) = old.borrow_mut().take() {
            let _ = s.send(false);
        }
    rx.await.unwrap_or(false)
}

fn answer(yes: bool) {
    if let Some((_, pending)) = ASK.write().take()
        && let Some(s) = pending.borrow_mut().take() {
            let _ = s.send(yes);
        }
}

/// Einmal in der App einbinden.
#[component]
pub fn ConfirmHost() -> Element {
    let Some((ask, _)) = ASK.read().clone() else { return rsx! {} };
    let action = if ask.destructive {
        "bg-destructive text-white hover:bg-destructive/90"
    } else {
        "bg-primary text-primary-foreground hover:bg-primary/90"
    };
    rsx! {
        div {
            class: "fixed inset-0 z-[70] flex items-center justify-center bg-black/60 p-4 backdrop-blur-[2px]",
            tabindex: "-1",
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            onkeydown: move |e| match e.key() {
                Key::Escape => answer(false),
                Key::Enter => answer(true),
                _ => {}
            },
            div { class: "w-full max-w-md rounded-2xl border bg-background p-6 shadow-lg animate-in", role: "alertdialog",
                div { class: "flex gap-4",
                    if ask.destructive {
                        div { class: "grid size-10 shrink-0 place-items-center rounded-full bg-destructive/15 text-destructive",
                            icons::TriangleAlert { class: "size-5" }
                        }
                    }
                    div { class: "flex flex-col gap-2",
                        h3 { class: "text-lg leading-none font-semibold", "{ask.title}" }
                        p { class: "text-muted-foreground text-sm", "{ask.description}" }
                    }
                }
                footer { class: "mt-6 flex flex-col-reverse gap-2 sm:flex-row sm:justify-end",
                    button {
                        class: "inline-flex h-9 items-center justify-center rounded-md border bg-background px-4 text-sm font-medium shadow-xs hover:bg-accent dark:bg-input/30 dark:border-input",
                        onclick: move |_| answer(false),
                        {tr("Abbrechen")}
                    }
                    button {
                        class: "inline-flex h-9 items-center justify-center rounded-md px-4 text-sm font-medium shadow-xs {action}",
                        onclick: move |_| answer(true),
                        "{ask.confirm}"
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Meldungen

#[derive(Clone, PartialEq)]
struct Toast {
    id: u64,
    text: String,
    error: bool,
    /// Verweis nach aussen: (Beschriftung, Adresse)
    link: Option<(String, String)>,
    /// Wird gerade ausgeblendet (Animation läuft, danach entfernt)
    leaving: bool,
}

static TOASTS: GlobalSignal<Vec<Toast>> = Signal::global(Vec::new);
static NEXT: GlobalSignal<u64> = Signal::global(|| 0);

/// Dauer der Ausblende-Animation (`animate-toast-out`, siehe theme.rs).
const LEAVE_MS: u32 = crate::theme::TOAST_OUT_MS;

fn push(text: String, error: bool, link: Option<(String, String)>) {
    let id = {
        let mut n = NEXT.write();
        *n += 1;
        *n
    };
    let long = error || link.is_some();
    TOASTS.write().push(Toast { id, text, error, link, leaving: false });
    // Höchstens drei gleichzeitig
    let excess: Vec<u64> = {
        let t = TOASTS.peek();
        t.iter().filter(|t| !t.leaving).map(|t| t.id).collect::<Vec<_>>().into_iter().rev().skip(3).collect()
    };
    for old in excess {
        dismiss(old);
    }
    spawn_forever(async move {
        gloo_timers::future::TimeoutFuture::new(if long { 8000 } else { 3500 }).await;
        dismiss(id);
    });
}

/// Meldung sanft ausblenden und danach entfernen.
fn dismiss(id: u64) {
    let mut found = false;
    for t in TOASTS.write().iter_mut().filter(|t| t.id == id && !t.leaving) {
        t.leaving = true;
        found = true;
    }
    if found {
        spawn_forever(async move {
            gloo_timers::future::TimeoutFuture::new(LEAVE_MS).await;
            TOASTS.write().retain(|t| t.id != id);
        });
    }
}

/// Kurze Meldung.
pub fn toast(text: impl Into<String>) {
    push(text.into(), false, None);
}

/// Fehlermeldung (bleibt länger stehen).
pub fn toast_error(text: impl std::fmt::Display) {
    push(text.to_string(), true, None);
}

/// Meldung mit einem Verweis nach aussen (öffnet in neuem Tab).
pub fn toast_link(text: impl Into<String>, label: impl Into<String>, url: impl Into<String>, error: bool) {
    push(text.into(), error, Some((label.into(), url.into())));
}

#[component]
pub fn Toaster() -> Element {
    let pos = "toaster-pos bottom-24 right-4 max-md:bottom-[8.25rem]";
    rsx! {
        div { class: "pointer-events-none fixed z-[80] flex w-[calc(100%-2rem)] max-w-sm flex-col gap-2 {pos}",
            crate::upload::UploadToast {}
            for t in TOASTS.read().iter().cloned() {
                div {
                    key: "{t.id}",
                    role: if t.error { "alert" } else { "status" },
                    class: format!(
                        "pointer-events-auto flex items-start gap-3 rounded-xl border bg-popover px-4 py-3 text-sm text-popover-foreground shadow-lg {} {}",
                        if t.error { "border-destructive/40" } else { "" },
                        if t.leaving { "animate-toast-out" } else { "animate-toast-in" },
                    ),
                    if t.error {
                        icons::CircleAlert { class: "mt-0.5 size-4 shrink-0 text-destructive" }
                    } else {
                        icons::Check { class: "mt-0.5 size-4 shrink-0 text-muted-foreground" }
                    }
                    div { class: "flex min-w-0 flex-1 flex-col gap-1.5",
                        span { "{t.text}" }
                        if let Some((label, url)) = t.link.clone() {
                            a { class: "inline-flex w-fit items-center gap-1.5 font-medium underline-offset-4 hover:underline",
                                href: "{url}", target: "_blank", rel: "noopener noreferrer",
                                icons::ExternalLink { class: "size-3.5" }
                                "{label}"
                            }
                        }
                    }
                    button {
                        class: "text-muted-foreground hover:text-foreground",
                        title: tr("Schliessen"),
                        onclick: move |_| dismiss(t.id),
                        icons::X { class: "size-3.5 pointer-events-none" }
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Menü

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Align {
    #[default]
    Start,
    End,
}

/// Aufklappmenü (rust-ui Dropdown Menu). `trigger` ist der sichtbare Knopf.
#[component]
pub fn Dropdown(
    trigger: Element,
    #[props(default)] align: Align,
    #[props(into, optional)] class: Option<String>,
    #[props(into, optional)] trigger_class: Option<String>,
    #[props(into, optional)] title: Option<String>,
    children: Element,
) -> Element {
    let mut open = use_signal(|| false);
    provide_context(MenuOpen(open));
    let pos = match align {
        Align::Start => "left-0 top-full mt-1.5",
        Align::End => "right-0 top-full mt-1.5",
    };
    let menu = tw_merge!(
        "absolute z-50 min-w-[12rem] overflow-hidden rounded-lg border bg-popover p-1 text-popover-foreground shadow-lg animate-in",
        pos,
        class.as_deref().unwrap_or("")
    );
    rsx! {
        div { class: "relative",
            button {
                class: trigger_class.unwrap_or_default(),
                title: title.unwrap_or_default(),
                "aria-expanded": "{open}",
                onclick: move |e| {
                    e.stop_propagation();
                    open.toggle();
                },
                {trigger}
            }
            if open() {
                div { class: "fixed inset-0 z-40", onclick: move |e| { e.stop_propagation(); open.set(false); } }
                div { class: "{menu}", role: "menu", onclick: move |e| e.stop_propagation(), {children} }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct MenuOpen(Signal<bool>);

/// Eintrag im Menü; schliesst es nach dem Klick.
#[component]
pub fn MenuItem(
    onclick: EventHandler<MouseEvent>,
    #[props(default)] destructive: bool,
    #[props(default)] active: bool,
    #[props(default)] disabled: bool,
    children: Element,
) -> Element {
    let ctx = try_consume_context::<MenuOpen>();
    let color = if destructive { "text-destructive hover:bg-destructive/10" } else { "hover:bg-accent hover:text-accent-foreground" };
    let act = if active { "bg-accent/60" } else { "" };
    rsx! {
        button {
            class: "relative flex w-full cursor-default select-none items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none disabled:opacity-50 [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:pointer-events-none {color} {act}",
            role: "menuitem",
            disabled,
            onclick: move |e| {
                if let Some(MenuOpen(mut o)) = ctx {
                    o.set(false);
                }
                onclick.call(e);
            },
            {children}
        }
    }
}

#[component]
pub fn MenuLabel(children: Element) -> Element {
    rsx! { div { class: "px-2 py-1.5 text-xs font-medium text-muted-foreground", {children} } }
}

#[component]
pub fn MenuSeparator() -> Element {
    rsx! { div { class: "-mx-1 my-1 h-px bg-border" } }
}
