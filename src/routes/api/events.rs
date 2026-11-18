//! Server-Sent Events (`GET /api/v1/events`): jedes `data:` ist ein `ServerEvent` als JSON.
//! Ein Herzschlag alle 15 Sekunden hält Verbindungen durch Proxys offen.

use crate::library::Events;
use rocket::futures::Stream;
use rocket::response::stream::{Event, EventStream};
use rocket::response::{self, Responder};
use rocket::tokio::sync::broadcast::error::RecvError;
use rocket::{get, routes, Request, Route, Shutdown, State};
use std::time::Duration;

pub fn routes() -> Vec<Route> {
    routes![events]
}

/// Ereignisstrom, den auch nginx & Co. nicht puffern.
pub struct Sse<S>(EventStream<S>);

impl<'r, S: Stream<Item = Event> + Send + 'r> Responder<'r, 'r> for Sse<S> {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'r> {
        let mut res = self.0.respond_to(req)?;
        res.set_raw_header("X-Accel-Buffering", "no");
        Ok(res)
    }
}

#[get("/events")]
fn events(
    me: crate::auth::CurrentUser,
    events: &State<Events>,
    mut shutdown: Shutdown,
) -> Sse<impl Stream<Item = Event>> {
    let mut rx = events.subscribe();
    let uid = me.id();
    let stream = EventStream! {
        loop {
            let event = rocket::tokio::select! {
                ev = rx.recv() => match ev {
                    Ok((Some(to), _)) if to != uid => continue,
                    Ok((_, ev)) => ev,
                    Err(RecvError::Closed) => break,
                    // Zu langsam gelesen – verpasste Ereignisse sind weg, weiter mit den neuen
                    Err(RecvError::Lagged(_)) => continue,
                },
                _ = &mut shutdown => break,
            };
            yield Event::json(&event);
        }
    };
    Sse(stream.heartbeat(Duration::from_secs(15)))
}
