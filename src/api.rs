use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use futures_util::{SinkExt, StreamExt, stream::SplitSink};
use http::{HeaderMap, StatusCode, Uri};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{Semaphore, mpsc, watch},
    time::timeout,
};
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body,
        content::websocket::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
        error::{bad_request, forbidden, service_unavailable, too_many_requests},
        path_param,
        request::{Bytes, client_ip, headers},
        response::Response,
        route,
    },
};

use crate::rooms::{Action, Role, RoomError, RoomHandle, Rooms, Snapshot, SocialAction};

const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(45);
const JOIN_TIMEOUT: Duration = Duration::from_secs(10);

path_param!(room_id);

#[derive(Clone)]
pub(crate) struct ConnectionLimits {
    sockets: Arc<Semaphore>,
    creations: Arc<Mutex<HashMap<Option<IpAddr>, CreationWindow>>>,
}

struct CreationWindow {
    started: Instant,
    count: u8,
}

impl ConnectionLimits {
    pub(crate) fn new() -> Self {
        Self {
            sockets: Arc::new(Semaphore::new(512)),
            creations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn allow_creation(&self, ip: Option<IpAddr>) -> bool {
        let now = Instant::now();
        let mut windows = self
            .creations
            .lock()
            .expect("creation limits lock poisoned");
        windows.retain(|_, window| now.duration_since(window.started) < Duration::from_secs(60));

        if windows.len() >= 1024 && !windows.contains_key(&ip) {
            return false;
        }

        let window = windows.entry(ip).or_insert(CreationWindow {
            started: now,
            count: 0,
        });

        if window.count >= 8 {
            return false;
        }

        window.count += 1;
        true
    }
}

#[route(POST "/api/rooms")]
async fn create_room(cx: &Cx, body: Bytes) -> Result<Response> {
    if !body.is_empty() {
        return Err(bad_request("Room creation does not accept a request body.").into());
    }

    if !same_origin(headers(cx), false) {
        return Err(forbidden().into());
    }

    if !app_context::<ConnectionLimits>(cx).allow_creation(client_ip(cx)) {
        return Err(too_many_requests(60).into());
    }

    match app_context::<Rooms>(cx).create() {
        Ok(created) => json_response(StatusCode::CREATED, &created),
        Err(error) => json_response(StatusCode::SERVICE_UNAVAILABLE, &failure(error, None)),
    }
}

#[route(GET "/api/rooms/{room_id}/ws")]
async fn room_socket(cx: &Cx, upgrade: WebSocketUpgrade) -> Result<Response> {
    if !same_origin(headers(cx), true) {
        return Err(forbidden().into());
    }

    let limits = app_context::<ConnectionLimits>(cx);
    let permit = limits
        .sockets
        .clone()
        .try_acquire_owned()
        .map_err(|_| service_unavailable(5))?;

    let rooms = app_context::<Rooms>(cx).clone();
    let room = rooms.get(path_param::<RoomId>(cx));

    upgrade
        .read_buffer_size(4096)
        .write_buffer_size(0)
        .max_write_buffer_size(1024 * 1024)
        .max_message_size(4096)
        .max_frame_size(4096)
        .on_upgrade(move |mut socket| async move {
            let _permit = permit;

            let Some(room) = room else {
                let _ = send_socket(
                    &mut socket,
                    &ServerMessage::Error {
                        code: "unavailable",
                        message: "This room has expired or does not exist.",
                        id: None,
                    },
                )
                .await;
                close_socket(&mut socket, 4004, "Room unavailable").await;
                return;
            };

            serve_member(socket, room, rooms).await;
        })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ClientMessage {
    Join {
        host_token: Option<String>,
    },
    Ping {
        client_ms: f64,
    },
    Sync {},
    Command {
        id: String,
        revision: u64,
        action: Action,
    },
    Social {
        id: String,
        action: SocialAction,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Welcome {
        member_id: u64,
        role: Role,
        snapshot: Snapshot,
        server_ms: u64,
    },
    Snapshot {
        snapshot: Snapshot,
        server_ms: u64,
    },
    Pong {
        client_ms: f64,
        server_ms: u64,
    },
    Ack {
        id: String,
        revision: u64,
    },
    SocialAck {
        id: String,
        event_id: u64,
    },
    Error {
        code: &'static str,
        message: &'static str,
        id: Option<String>,
    },
}

async fn serve_member(mut socket: WebSocket, room: RoomHandle, rooms: Rooms) {
    let Ok(Some(Ok(Message::Text(text)))) = timeout(JOIN_TIMEOUT, socket.recv()).await else {
        return;
    };
    let Ok(ClientMessage::Join { host_token }) = serde_json::from_str(&text) else {
        close_socket(&mut socket, 1008, "First message must join the room").await;
        return;
    };

    let member = match room.join(host_token).await {
        Ok(member) => member,
        Err(error) => {
            let _ = send_socket(&mut socket, &failure(error, None)).await;
            close_socket(&mut socket, 1008, "Could not join room").await;
            return;
        }
    };

    let mut snapshots = member.snapshots;
    let welcome = ServerMessage::Welcome {
        member_id: member.id,
        role: member.role,
        snapshot: snapshots.borrow_and_update().clone(),
        server_ms: rooms.now_ms(),
    };

    if send_socket(&mut socket, &welcome).await.is_err() {
        room.leave(member.id);
        return;
    }

    let (sink, mut stream) = socket.split();
    let (responses, response_rx) = mpsc::channel(8);
    let mut writer = tokio::spawn(write_updates(
        sink,
        snapshots.clone(),
        response_rx,
        rooms.clone(),
    ));
    let mut rate = MessageRate::new();

    loop {
        let incoming = tokio::select! {
            _ = &mut writer => {
                room.leave(member.id);
                return;
            },
            incoming = timeout(READ_TIMEOUT, stream.next()) => incoming,
        };

        let Ok(Some(Ok(message))) = incoming else {
            break;
        };

        if !rate.allow() {
            break;
        }

        let text = match message {
            Message::Text(text) => text,
            Message::Close(_) | Message::Binary(_) => break,
            _ => continue,
        };

        let response = match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Ping { client_ms }) if client_ms.is_finite() && client_ms >= 0.0 => {
                ServerMessage::Pong {
                    client_ms,
                    server_ms: rooms.now_ms(),
                }
            }
            Ok(ClientMessage::Sync {}) => ServerMessage::Snapshot {
                snapshot: snapshots.borrow_and_update().clone(),
                server_ms: rooms.now_ms(),
            },
            Ok(ClientMessage::Command {
                id,
                revision,
                action,
            }) => match room.command(member.id, id.clone(), revision, action).await {
                Ok(revision) => ServerMessage::Ack { id, revision },
                Err(error) => {
                    if responses.try_send(failure(error, Some(id))).is_err() {
                        break;
                    }

                    ServerMessage::Snapshot {
                        snapshot: snapshots.borrow_and_update().clone(),
                        server_ms: rooms.now_ms(),
                    }
                }
            },
            Ok(ClientMessage::Social { id, action }) => {
                match room.social(member.id, id.clone(), action).await {
                    Ok(event_id) => ServerMessage::SocialAck { id, event_id },
                    Err(error) => failure(error, Some(id)),
                }
            }
            _ => ServerMessage::Error {
                code: "invalid_command",
                message: "Invalid room message.",
                id: None,
            },
        };

        if responses.try_send(response).is_err() {
            break;
        }
    }

    room.leave(member.id);
    writer.abort();
    let _ = writer.await;
}

async fn write_updates(
    mut sink: SplitSink<WebSocket, Message>,
    mut snapshots: watch::Receiver<Snapshot>,
    mut responses: mpsc::Receiver<ServerMessage>,
    rooms: Rooms,
) {
    loop {
        let response = tokio::select! {
            response = responses.recv() => match response {
                Some(response) => response,
                None => break,
            },
            changed = snapshots.changed() => {
                if changed.is_err() {
                    break;
                }

                ServerMessage::Snapshot {
                    snapshot: snapshots.borrow_and_update().clone(),
                    server_ms: rooms.now_ms(),
                }
            },
        };

        let response = match response {
            ServerMessage::Snapshot { .. } => ServerMessage::Snapshot {
                snapshot: snapshots.borrow_and_update().clone(),
                server_ms: rooms.now_ms(),
            },
            response => response,
        };

        let Ok(json) = serde_json::to_string(&response) else {
            break;
        };

        if !matches!(
            timeout(WRITE_TIMEOUT, sink.send(Message::text(json))).await,
            Ok(Ok(()))
        ) {
            break;
        }
    }
}

async fn send_socket(socket: &mut WebSocket, message: &ServerMessage) -> Result<()> {
    let json = serde_json::to_string(message)?;
    timeout(WRITE_TIMEOUT, socket.send(Message::text(json)))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "socket write timed out"))?
}

async fn close_socket(socket: &mut WebSocket, code: u16, reason: &'static str) {
    let _ = timeout(
        WRITE_TIMEOUT,
        socket.send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        }))),
    )
    .await;
}

fn failure(error: RoomError, id: Option<String>) -> ServerMessage {
    ServerMessage::Error {
        code: error.code,
        message: error.message,
        id,
    }
}

fn json_response(status: StatusCode, value: &impl Serialize) -> Result<Response> {
    Ok(Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("cache-control", "no-store")
        .body(Body::from(serde_json::to_string(value)?))?)
}

pub(crate) fn same_origin(headers: &HeaderMap, required: bool) -> bool {
    let Some(origin) = headers.get("origin") else {
        return !required;
    };
    let Some(host) = headers.get("host").and_then(|host| host.to_str().ok()) else {
        return false;
    };
    let Some(origin) = origin
        .to_str()
        .ok()
        .and_then(|origin| origin.parse::<Uri>().ok())
    else {
        return false;
    };

    matches!(origin.scheme_str(), Some("http" | "https"))
        && origin
            .authority()
            .is_some_and(|authority| authority.as_str().eq_ignore_ascii_case(host))
        && origin.path() == "/"
        && origin.query().is_none()
}

struct MessageRate {
    tokens: f64,
    updated: Instant,
}

impl MessageRate {
    fn new() -> Self {
        Self {
            tokens: 40.0,
            updated: Instant::now(),
        }
    }

    fn allow(&mut self) -> bool {
        let now = Instant::now();
        self.tokens =
            (self.tokens + now.duration_since(self.updated).as_secs_f64() * 10.0).min(40.0);
        self.updated = now;

        if self.tokens < 1.0 {
            return false;
        }

        self.tokens -= 1.0;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin_headers(origin: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("host", "localhost:3000".parse().unwrap());
        headers.insert("origin", origin.parse().unwrap());
        headers
    }

    #[test]
    fn origin_checks_authority_not_host_suffix() {
        assert!(same_origin(&origin_headers("http://localhost:3000"), true));

        assert!(!same_origin(
            &origin_headers("http://localhost:3000.evil.test"),
            true
        ));
        assert!(!same_origin(&origin_headers("http://evil.test"), true));
        assert!(!same_origin(&origin_headers("null"), true));
        assert!(!same_origin(
            &origin_headers("http://localhost:3000/path"),
            true
        ));

        assert!(!same_origin(&HeaderMap::new(), true));
        assert!(same_origin(&HeaderMap::new(), false));
    }

    #[test]
    fn creation_limits_bound_each_peer() {
        let limits = ConnectionLimits::new();
        let ip = Some("127.0.0.1".parse().unwrap());

        for _ in 0..8 {
            assert!(limits.allow_creation(ip));
        }

        assert!(!limits.allow_creation(ip));
        assert!(limits.allow_creation(Some("127.0.0.2".parse().unwrap())));
    }

    #[test]
    fn incoming_messages_are_strict_and_small() {
        assert!(serde_json::from_str::<ClientMessage>(r#"{"type":"sync"}"#).is_ok());

        assert!(
            serde_json::from_str::<ClientMessage>(r#"{"type":"sync","unexpected":1}"#).is_err()
        );
        assert!(serde_json::from_str::<ClientMessage>(r#"{"type":"command","id":"a"}"#).is_err());
    }
}
