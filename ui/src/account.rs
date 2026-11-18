//! Konten: Stand der Anmeldung, Anmelde-/Registrierseite, Kontoseite (Name, Passwort,
//! Konto löschen) und für Admins die Verwaltung aller Konten.
//!
//! Nach dem Anmelden, Registrieren und Abmelden lädt die Seite neu – so beginnen Player,
//! Live-Ereignisse und alle Listen sauber mit dem neuen Konto.

use crate::api;
use crate::components::common::*;
use crate::components::overlay::{Ask, confirm, toast, toast_error};
use crate::i18n::{tr, trf};
use crate::{ICON, util};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use nrzmusic_types::{Account, AccountUpdate, AuthStatus, LoginRequest, PasswordChange, RegisterRequest, UserUpdate};
use serde_json::{Value, json};

/// Stand der Anmeldung (`None` = noch nicht geladen).
pub static AUTH: GlobalSignal<Option<AuthStatus>> = Signal::global(|| None);

pub async fn load() {
    match api::get::<AuthStatus>("/auth/status").await {
        Ok(s) => *AUTH.write() = Some(s),
        // Server nicht erreichbar: wie ohne Konten weiter, die Seiten zeigen den Fehler
        Err(_) => {
            *AUTH.write() = Some(AuthStatus { enabled: false, ..Default::default() });
        }
    }
}

/// Angemeldetes Konto.
pub fn me() -> Option<Account> {
    AUTH.read().as_ref().and_then(|a| a.user.clone())
}

/// Darf Einstellungen ändern, hochladen, neu einlesen.
pub fn is_admin() -> bool {
    me().is_some_and(|a| a.is_admin)
}

/// Konten eingeschaltet?
pub fn enabled() -> bool {
    AUTH.read().as_ref().is_some_and(|a| a.enabled)
}

/// Muss sich jemand anmelden, bevor die App etwas zeigt?
pub fn needs_login() -> bool {
    AUTH.read().as_ref().is_some_and(|a| a.enabled && a.user.is_none())
}

/// Die API meldet „Bitte anmelden.“ (Sitzung abgelaufen oder gelöscht).
pub fn session_lost() {
    if let Some(a) = AUTH.write().as_mut()
        && a.enabled
    {
        a.user = None;
    }
}

fn reload() {
    if let Some(w) = web_sys::window() {
        let _ = w.location().reload();
    }
}

pub fn logout() {
    spawn_forever(async {
        let _ = api::action("/auth/logout").await;
        crate::player::persist_clear();
        reload();
    });
}

// ------------------------------------------------------------------ Anmeldeseite

#[component]
pub fn LoginScreen() -> Element {
    let status = AUTH.read().clone().unwrap_or_default();
    let mut register = use_signal(|| status.setup);
    let mut username = use_signal(String::new);
    let mut display_name = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut password2 = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let can_register = status.registration_open;

    let submit = move |e: FormEvent| {
        e.prevent_default();
        if busy() {
            return;
        }
        let reg = register();
        if reg && password() != password2() {
            error.set(Some(tr("Die Passwörter stimmen nicht überein.")));
            return;
        }
        spawn(async move {
            busy.set(true);
            error.set(None);
            let res = if reg {
                api::post::<Value, _>(
                    "/auth/register",
                    &RegisterRequest { username: username().trim().to_string(), password: password(), display_name: display_name().trim().to_string() },
                )
                .await
            } else {
                api::post::<Value, _>("/auth/login", &LoginRequest { username: username().trim().to_string(), password: password() }).await
            };
            match res {
                Ok(_) => reload(),
                Err(e) => {
                    error.set(Some(e.message));
                    busy.set(false);
                }
            }
        });
    };

    rsx! {
        div { class: "grid min-h-dvh place-items-center bg-background p-4",
            div { class: "w-full max-w-sm",
                div { class: "mb-6 flex flex-col items-center gap-3 text-center",
                    img { src: ICON, alt: "", class: "size-14 rounded-2xl shadow-lg" }
                    h1 { class: "text-2xl font-semibold tracking-tight", "nrzMusic" }
                    p { class: "text-sm text-muted-foreground",
                        if status.setup {
                            {tr("Willkommen! Lege das erste Konto an – es wird Admin.")}
                        } else if register() {
                            {tr("Neues Konto anlegen")}
                        } else {
                            {tr("Melde dich an, um deine Musik zu hören.")}
                        }
                    }
                }
                form { class: "grid gap-4 rounded-xl border bg-card p-5 shadow-sm", onsubmit: submit,
                    Field { label: tr("Benutzername"),
                        input { class: INPUT, r#type: "text", autocomplete: "username", required: true, autofocus: true,
                            autocapitalize: "none", spellcheck: "false",
                            value: "{username}", oninput: move |e| username.set(e.value()),
                        }
                    }
                    if register() {
                        Field { label: tr("Angezeigter Name"), hint: tr("Optional"),
                            input { class: INPUT, r#type: "text", autocomplete: "nickname",
                                value: "{display_name}", oninput: move |e| display_name.set(e.value()),
                            }
                        }
                    }
                    Field { label: tr("Passwort"), hint: if register() { Some(tr("Mindestens 8 Zeichen")) } else { None },
                        input { class: INPUT, r#type: "password", required: true,
                            autocomplete: if register() { "new-password" } else { "current-password" },
                            value: "{password}", oninput: move |e| password.set(e.value()),
                        }
                    }
                    if register() {
                        Field { label: tr("Passwort wiederholen"),
                            input { class: INPUT, r#type: "password", required: true, autocomplete: "new-password",
                                value: "{password2}", oninput: move |e| password2.set(e.value()),
                            }
                        }
                    }
                    if let Some(err) = error() {
                        Callout { text: err, error: true }
                    }
                    button { class: "{btn(BTN_PRIMARY)} w-full", r#type: "submit", disabled: busy(),
                        if busy() { icons::LoaderCircle { class: "size-4 animate-spin" } }
                        if register() { {tr("Konto anlegen")} } else { {tr("Anmelden")} }
                    }
                }
                if !status.setup {
                    p { class: "mt-4 text-center text-sm text-muted-foreground",
                        if register() {
                            {tr("Schon ein Konto?")}
                            " "
                            button { class: "font-medium text-foreground underline-offset-4 hover:underline", onclick: move |_| { register.set(false); error.set(None); }, {tr("Anmelden")} }
                        } else if can_register {
                            {tr("Noch kein Konto?")}
                            " "
                            button { class: "font-medium text-foreground underline-offset-4 hover:underline", onclick: move |_| { register.set(true); error.set(None); }, {tr("Registrieren")} }
                        } else {
                            {tr("Neue Konten vergibt der Admin.")}
                        }
                    }
                }
                if !crate::android::available() {
                    p { class: "mt-6 text-center",
                        a { class: "inline-flex items-center gap-1.5 text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline", href: "/static/nrzMusic.apk", download: "nrzMusic.apk",
                            icons::Smartphone { class: "size-3.5" }
                            {tr("Android-App herunterladen")}
                        }
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Kontoseite

#[component]
pub fn AccountPage() -> Element {
    let Some(me) = me() else {
        return rsx! {
            div { class: "mx-auto flex max-w-3xl flex-col gap-6",
                PageHeader { title: tr("Konto") }
                Callout { text: tr("Konten sind auf diesem Server ausgeschaltet – alle teilen sich eine Bibliothek ohne Anmeldung.") }
            }
        };
    };
    let accounts = enabled();
    rsx! {
        div { class: "mx-auto flex max-w-3xl flex-col gap-6",
            PageHeader { title: tr("Konto"), description: trf("Angemeldet als {0}", &[&me.username]) }
            if accounts {
                ProfileSection { me: me.clone() }
                PasswordSection {}
            }
            if me.is_admin && accounts {
                UsersSection { me_id: me.id }
            }
            AndroidSection {}
            if accounts {
                Section { title: tr("Abmelden"), description: tr("Auf diesem Gerät abmelden. Deine Playlists und Favoriten bleiben im Konto."),
                    button { class: btn(BTN_OUTLINE), onclick: move |_| logout(), icons::LogOut { class: "size-4" } {tr("Abmelden")} }
                }
                DeleteSection {}
            }
        }
    }
}

/// Die App fürs Handy (bzw. in der App: die Offline-Bibliothek).
#[component]
fn AndroidSection() -> Element {
    if crate::android::available() {
        let n = crate::android::OFFLINE.read().len();
        return rsx! {
            Section { title: tr("Offline auf dem Handy"), description: trf("{0} Titel auf diesem Handy. Sie spielen auch ohne Verbindung.", &[&n]),
                button { class: btn(BTN_OUTLINE), onclick: move |_| crate::android::open_offline(), icons::Smartphone { class: "size-4" } {tr("Offline-Bibliothek öffnen")} }
            }
        };
    }
    rsx! {
        Section { title: tr("Android-App"), description: tr("nrzMusic fürs Handy: verbindet sich mit diesem Server, spielt im Hintergrund weiter und lädt Titel, Alben und Playlists aufs Handy – zum Hören ohne Verbindung."),
            a { class: btn(BTN_OUTLINE), href: "/static/nrzMusic.apk", download: "nrzMusic.apk", icons::Download { class: "size-4" } {tr("APK herunterladen")} }
            p { class: "mt-3 text-xs text-muted-foreground", {tr("Auf dem Handy öffnen und die Installation aus dieser Quelle erlauben. Die App verbindet sich standardmässig mit music.zweers.ch – in der App unter „Server ändern“ lässt sich ein anderer eintragen.")} }
        }
    }
}

#[component]
fn ProfileSection(me: Account) -> Element {
    let mut name = use_signal(|| me.display_name.clone());
    let mut busy = use_signal(|| false);
    rsx! {
        Section { title: tr("Profil"),
            form { class: "grid gap-4 sm:grid-cols-[1fr_auto] sm:items-end",
                onsubmit: move |e| {
                    e.prevent_default();
                    spawn(async move {
                        busy.set(true);
                        match api::patch::<Account, _>("/auth/account", &AccountUpdate { display_name: Some(name().trim().to_string()) }).await {
                            Ok(a) => {
                                if let Some(s) = AUTH.write().as_mut() {
                                    s.user = Some(a);
                                }
                                toast(tr("Gespeichert."));
                            }
                            Err(e) => toast_error(e),
                        }
                        busy.set(false);
                    });
                },
                Field { label: tr("Angezeigter Name"), hint: trf("Benutzername: {0}", &[&me.username]),
                    input { class: INPUT, value: "{name}", maxlength: "64", oninput: move |e| name.set(e.value()) }
                }
                button { class: btn(BTN_OUTLINE), r#type: "submit", disabled: busy(), {tr("Speichern")} }
            }
        }
    }
}

#[component]
fn PasswordSection() -> Element {
    let mut current = use_signal(String::new);
    let mut new = use_signal(String::new);
    let mut new2 = use_signal(String::new);
    let mut busy = use_signal(|| false);
    rsx! {
        Section { title: tr("Passwort ändern"), description: tr("Andere Geräte werden danach abgemeldet."),
            form { class: "grid gap-4",
                onsubmit: move |e| {
                    e.prevent_default();
                    if new() != new2() {
                        toast_error(tr("Die Passwörter stimmen nicht überein."));
                        return;
                    }
                    spawn(async move {
                        busy.set(true);
                        match api::put::<Value, _>("/auth/password", &PasswordChange { current: current(), new: new() }).await {
                            Ok(_) => {
                                toast(tr("Passwort geändert."));
                                current.set(String::new());
                                new.set(String::new());
                                new2.set(String::new());
                            }
                            Err(e) => toast_error(e),
                        }
                        busy.set(false);
                    });
                },
                Field { label: tr("Aktuelles Passwort"),
                    input { class: INPUT, r#type: "password", autocomplete: "current-password", required: true, value: "{current}", oninput: move |e| current.set(e.value()) }
                }
                div { class: "grid gap-4 sm:grid-cols-2",
                    Field { label: tr("Neues Passwort"), hint: tr("Mindestens 8 Zeichen"),
                        input { class: INPUT, r#type: "password", autocomplete: "new-password", required: true, value: "{new}", oninput: move |e| new.set(e.value()) }
                    }
                    Field { label: tr("Neues Passwort wiederholen"),
                        input { class: INPUT, r#type: "password", autocomplete: "new-password", required: true, value: "{new2}", oninput: move |e| new2.set(e.value()) }
                    }
                }
                div { button { class: btn(BTN_OUTLINE), r#type: "submit", disabled: busy(), icons::KeyRound { class: "size-4" } {tr("Passwort ändern")} } }
            }
        }
    }
}

#[component]
fn DeleteSection() -> Element {
    let mut password = use_signal(String::new);
    rsx! {
        Section { title: tr("Konto löschen"), description: tr("Löscht deine Playlists, Favoriten, den Verlauf und die Spotify-Verbindung. Die Musik im gemeinsamen Ordner bleibt."),
            form { class: "grid gap-4 sm:grid-cols-[1fr_auto] sm:items-end",
                onsubmit: move |e| {
                    e.prevent_default();
                    spawn(async move {
                        let ok = confirm(Ask::new(tr("Konto wirklich löschen?"), tr("Das lässt sich nicht rückgängig machen.")).confirm(tr("Konto löschen")).destructive()).await;
                        if !ok {
                            return;
                        }
                        match api::send::<Value, _>("DELETE", "/auth/account", &json!({ "password": password() })).await {
                            Ok(_) => {
                                crate::player::persist_clear();
                                reload();
                            }
                            Err(e) => toast_error(e),
                        }
                    });
                },
                Field { label: tr("Passwort zur Bestätigung"),
                    input { class: INPUT, r#type: "password", autocomplete: "current-password", required: true, value: "{password}", oninput: move |e| password.set(e.value()) }
                }
                button { class: "{BTN} h-9 px-4 bg-destructive text-white hover:bg-destructive/90", r#type: "submit", icons::Trash2 { class: "size-4" } {tr("Konto löschen")} }
            }
        }
    }
}

/// Admin: alle Konten – Admin-Rechte, neues Passwort, löschen.
#[component]
fn UsersSection(me_id: i64) -> Element {
    let mut version = use_signal(|| 0u32);
    let users = use_resource(move || async move {
        let _ = version();
        api::get::<Vec<Account>>("/users").await
    });
    let mut reset_for = use_signal(|| None::<Account>);
    let mut new_pw = use_signal(String::new);
    rsx! {
        Section { title: tr("Konten"), description: tr("Alle Konten auf diesem Server. Die Registrierung öffnest oder schliesst du unter Einstellungen."),
            match &*users.read() {
                None => rsx! { Loading {} },
                Some(Err(e)) => rsx! { Callout { text: e.to_string(), error: true } },
                Some(Ok(list)) => rsx! {
                    div { class: "divide-y rounded-lg border",
                        for u in list.iter().cloned() {
                            div { key: "{u.id}", class: "flex flex-wrap items-center gap-3 px-3 py-2.5",
                                div { class: "grid size-9 shrink-0 place-items-center rounded-full bg-muted text-sm font-semibold uppercase",
                                    {u.label().chars().next().map(|c| c.to_string()).unwrap_or_default()}
                                }
                                div { class: "min-w-0 flex-1",
                                    p { class: "truncate text-sm font-medium",
                                        "{u.label()}"
                                        if u.is_admin { span { class: "ml-2 rounded-full bg-primary/15 px-2 py-0.5 text-[11px] font-medium text-primary", "Admin" } }
                                        if u.id == me_id { span { class: "ml-2 text-xs text-muted-foreground", {tr("(du)")} } }
                                    }
                                    p { class: "truncate text-xs text-muted-foreground",
                                        "@{u.username} · "
                                        {match u.last_login {
                                            Some(t) => trf("zuletzt angemeldet {0}", &[&util::date(t)]),
                                            None => tr("noch nie angemeldet"),
                                        }}
                                    }
                                }
                                if u.id != me_id {
                                    div { class: "flex items-center gap-1",
                                        Toggle { checked: u.is_admin, label: "Admin",
                                            onchange: {
                                                let id = u.id;
                                                move |on: bool| {
                                                    spawn(async move {
                                                        match api::patch::<Account, _>(&format!("/users/{id}"), &UserUpdate { is_admin: Some(on), password: None }).await {
                                                            Ok(_) => version += 1,
                                                            Err(e) => toast_error(e),
                                                        }
                                                    });
                                                }
                                            },
                                        }
                                        button { class: "{btn(BTN_GHOST)} {BTN_SM}", title: tr("Neues Passwort setzen"),
                                            onclick: { let u = u.clone(); move |_| { new_pw.set(String::new()); reset_for.set(Some(u.clone())); } },
                                            icons::KeyRound { class: "size-4" }
                                        }
                                        button { class: "{btn(BTN_GHOST)} {BTN_SM} text-destructive", title: tr("Konto löschen"),
                                            onclick: {
                                                let u = u.clone();
                                                move |_| {
                                                    let u = u.clone();
                                                    spawn(async move {
                                                        let ok = confirm(Ask::new(trf("Konto „{0}“ löschen?", &[&u.label()]), tr("Playlists, Favoriten und Verlauf dieses Kontos werden gelöscht.")).confirm(tr("Löschen")).destructive()).await;
                                                        if ok {
                                                            match api::delete(&format!("/users/{}", u.id)).await {
                                                                Ok(()) => { toast(tr("Konto gelöscht.")); version += 1; }
                                                                Err(e) => toast_error(e),
                                                            }
                                                        }
                                                    });
                                                }
                                            },
                                            icons::Trash2 { class: "size-4" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
            if let Some(u) = reset_for() {
                form { class: "mt-4 grid gap-3 rounded-lg border bg-muted/30 p-3 sm:grid-cols-[1fr_auto_auto] sm:items-end",
                    onsubmit: move |e| {
                        e.prevent_default();
                        let id = u.id;
                        spawn(async move {
                            match api::patch::<Account, _>(&format!("/users/{id}"), &UserUpdate { is_admin: None, password: Some(new_pw()) }).await {
                                Ok(_) => { toast(tr("Passwort gesetzt – das Konto wurde überall abgemeldet.")); reset_for.set(None); }
                                Err(e) => toast_error(e),
                            }
                        });
                    },
                    Field { label: trf("Neues Passwort für {0}", &[&u.username]),
                        input { class: INPUT, r#type: "password", autocomplete: "new-password", required: true, value: "{new_pw}", oninput: move |e| new_pw.set(e.value()) }
                    }
                    button { class: btn(BTN_PRIMARY), r#type: "submit", {tr("Setzen")} }
                    button { class: btn(BTN_GHOST), r#type: "button", onclick: move |_| reset_for.set(None), {tr("Abbrechen")} }
                }
            }
        }
    }
}
