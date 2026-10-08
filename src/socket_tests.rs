//! Exercise real upgrades and multiple clients, not just the room state machine.

use std::{net::SocketAddr, time::Duration};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Error, Message, client::IntoClientRequest},
};
use topcoat::router::{BodyLimit, OriginPolicy, Router, RouterBuilderDiscoverExt};

use crate::{
    BrowserPolicy,
    access::{Access, AccessGate},
    api::ConnectionLimits,
    rooms::Rooms,
};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct TestServer {
    address: SocketAddr,
    rooms: Rooms,
    cookie: Option<String>,
    task: JoinHandle<()>,
    _shutdown: oneshot::Sender<()>,
}

impl TestServer {
    async fn start() -> Self {
        Self::start_with_access(Access::for_test("a".repeat(64)), false).await
    }

    async fn start_with_access(access: Access, public: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let rooms = Rooms::new();
        let cookie = if public {
            None
        } else {
            Some(access.test_cookie())
        };
        let router = Router::builder()
            .app_context(access)
            .origin_policy(OriginPolicy::dangerous_disable())
            .layer(BodyLimit::max(1024))
            .layer(AccessGate)
            .layer(BrowserPolicy)
            .app_context(rooms.clone())
            .app_context(ConnectionLimits::new())
            .discover()
            .build();
        let (shutdown, signal) = oneshot::channel();
        let task = tokio::spawn(async move {
            topcoat::serve_until(listener, router, async {
                let _ = signal.await;
            })
            .await
            .unwrap();
        });

        Self {
            address,
            rooms,
            cookie,
            task,
            _shutdown: shutdown,
        }
    }

    async fn connect(&self, room_id: &str) -> Socket {
        let mut request = format!("ws://{}/api/rooms/{room_id}/ws", self.address)
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "origin",
            format!("http://{}", self.address).parse().unwrap(),
        );
        if let Some(cookie) = &self.cookie {
            request
                .headers_mut()
                .insert("cookie", cookie.parse().unwrap());
        }
        let (socket, _) = connect_async(request).await.unwrap();
        socket
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(Message::text(message.to_string()))
        .await
        .unwrap();
}

async fn receive(socket: &mut Socket, predicate: impl Fn(&Value) -> bool) -> Value {
    timeout(Duration::from_secs(5), async {
        loop {
            let message = socket
                .next()
                .await
                .expect("socket ended")
                .expect("socket error");
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if predicate(&value) {
                    return value;
                }
            }
        }
    })
    .await
    .expect("expected room message within five seconds")
}

async fn join(socket: &mut Socket, token: Option<&str>) -> Value {
    send(socket, json!({ "type": "join", "host_token": token })).await;
    receive(socket, |message| message["type"] == "welcome").await
}

async fn command(socket: &mut Socket, id: &str, revision: u64, action: Value) {
    send(
        socket,
        json!({ "type": "command", "id": id, "revision": revision, "action": action }),
    )
    .await;
}

async fn ack(socket: &mut Socket, id: &str) -> Value {
    receive(socket, |message| {
        message["type"] == "ack" && message["id"] == id
    })
    .await
}

#[tokio::test]
async fn public_websockets_need_no_site_key_but_guests_still_cannot_control_playback() {
    let server = TestServer::start_with_access(Access::public(), true).await;
    let created = server.rooms.create().unwrap();
    let mut guest = server.connect(&created.room_id).await;

    let welcome = join(&mut guest, None).await;
    assert_eq!(welcome["role"], "guest");

    command(
        &mut guest,
        "forbidden",
        0,
        json!({"type":"set_video", "video_id":"dQw4w9WgXcQ"}),
    )
    .await;
    let error = receive(&mut guest, |message| {
        message["type"] == "error" && message["id"] == "forbidden"
    })
    .await;
    assert_eq!(error["code"], "forbidden");
}

#[tokio::test]
async fn host_guest_and_reconnect_converge_over_real_websockets() {
    let server = TestServer::start().await;
    let created = server.rooms.create().unwrap();
    let mut host = server.connect(&created.room_id).await;
    let welcome = join(&mut host, Some(&created.host_token)).await;

    assert_eq!(welcome["role"], "host");
    assert_eq!(welcome["snapshot"]["members"], 1);
    assert_eq!(welcome["snapshot"]["media_revision"], 0);

    let mut guest = server.connect(&created.room_id).await;
    let welcome = join(&mut guest, None).await;
    assert_eq!(welcome["role"], "guest");
    assert_eq!(welcome["snapshot"]["members"], 2);
    assert!(!welcome.to_string().contains(&created.host_token));

    command(
        &mut guest,
        "forbidden",
        0,
        json!({ "type": "set_video", "video_id": "dQw4w9WgXcQ" }),
    )
    .await;
    let error = receive(&mut guest, |message| {
        message["type"] == "error" && message["id"] == "forbidden"
    })
    .await;
    assert_eq!(error["code"], "forbidden");

    command(
        &mut host,
        "load",
        0,
        json!({ "type": "set_video", "video_id": "dQw4w9WgXcQ" }),
    )
    .await;
    assert_eq!(ack(&mut host, "load").await["revision"], 1);
    let snapshot = receive(&mut guest, |message| {
        message["type"] == "snapshot" && message["snapshot"]["revision"] == 1
    })
    .await;
    assert_eq!(snapshot["snapshot"]["video_id"], "dQw4w9WgXcQ");
    assert_eq!(snapshot["snapshot"]["media_revision"], 1);

    command(
        &mut host,
        "load",
        0,
        json!({ "type": "set_video", "video_id": "dQw4w9WgXcQ" }),
    )
    .await;
    assert_eq!(ack(&mut host, "load").await["revision"], 1);

    command(
        &mut host,
        "play",
        1,
        json!({ "type": "play", "position_secs": 12.0 }),
    )
    .await;
    assert_eq!(ack(&mut host, "play").await["revision"], 2);
    let playing = receive(&mut guest, |message| {
        message["type"] == "snapshot" && message["snapshot"]["revision"] == 2
    })
    .await;
    assert_eq!(playing["snapshot"]["playing"], true);

    guest.close(None).await.unwrap();
    let mut returning = server.connect(&created.room_id).await;
    let latest = join(&mut returning, None).await;

    assert_eq!(latest["snapshot"]["revision"], 2);
    assert_eq!(latest["snapshot"]["media_revision"], 1);
    assert_eq!(
        latest["snapshot"]["incarnation"],
        playing["snapshot"]["incarnation"]
    );
    assert_eq!(
        latest["snapshot"]["anchor_ms"],
        playing["snapshot"]["anchor_ms"]
    );
    assert_eq!(latest["snapshot"]["position_secs"], 12.0);
    assert_eq!(latest["snapshot"]["playing"], true);
    assert!(
        latest["server_ms"].as_u64().unwrap() >= latest["snapshot"]["anchor_ms"].as_u64().unwrap()
    );

    command(
        &mut host,
        "stale",
        1,
        json!({ "type": "seek", "position_secs": 400.0 }),
    )
    .await;
    let error = receive(&mut host, |message| {
        message["type"] == "error" && message["id"] == "stale"
    })
    .await;
    assert_eq!(error["code"], "stale_revision");
    let fresh = receive(&mut host, |message| {
        message["type"] == "snapshot" && message["snapshot"]["revision"] == 2
    })
    .await;
    assert_eq!(fresh["snapshot"]["position_secs"], 12.0);

    command(
        &mut host,
        "pause",
        2,
        json!({ "type": "pause", "position_secs": 13.0 }),
    )
    .await;
    assert_eq!(ack(&mut host, "pause").await["revision"], 3);
    let paused = receive(&mut returning, |message| {
        message["type"] == "snapshot" && message["snapshot"]["revision"] == 3
    })
    .await;
    assert_eq!(paused["snapshot"]["playing"], false);

    command(
        &mut host,
        "rate",
        3,
        json!({ "type": "set_rate", "playback_rate": 1.5 }),
    )
    .await;
    assert_eq!(ack(&mut host, "rate").await["revision"], 4);
    let rate = receive(&mut returning, |message| {
        message["type"] == "snapshot" && message["snapshot"]["revision"] == 4
    })
    .await;
    assert_eq!(rate["snapshot"]["playback_rate"], 1.5);
    assert_eq!(rate["snapshot"]["playing"], false);
    assert_eq!(rate["snapshot"]["position_secs"], 13.0);
    assert_eq!(rate["snapshot"]["media_revision"], 1);

    host.close(None).await.unwrap();
    returning.close(None).await.unwrap();
}

async fn social(socket: &mut Socket, id: &str, action: Value) {
    send(
        socket,
        json!({ "type": "social", "id": id, "action": action }),
    )
    .await;
}

async fn social_result(socket: &mut Socket, id: &str) -> Value {
    receive(socket, |message| {
        message["id"] == id
            && (message["type"] == "social_ack"
                || message["type"] == "error"
                || message["type"] == "ack")
    })
    .await
}

async fn sync_snapshot(socket: &mut Socket) -> Value {
    // A pong orders earlier responses before the subsequent fresh sync.
    send(socket, json!({ "type": "ping", "client_ms": 123.0 })).await;
    receive(socket, |message| message["type"] == "pong").await;
    send(socket, json!({ "type": "sync" })).await;
    receive(socket, |message| message["type"] == "snapshot").await["snapshot"].clone()
}

#[tokio::test]
async fn social_ack_identity_and_moderation_are_independent_of_playback_over_sockets() {
    let server = TestServer::start().await;
    let created = server.rooms.create().unwrap();
    let mut host = server.connect(&created.room_id).await;
    let host_welcome = join(&mut host, Some(&created.host_token)).await;
    let mut guest = server.connect(&created.room_id).await;
    let guest_welcome = join(&mut guest, None).await;
    let member_id = guest_welcome["member_id"].as_u64().unwrap();
    let author = guest_welcome["snapshot"]["participants"][1].clone();
    let room_name = guest_welcome["snapshot"]["room_name"].clone();

    assert!(room_name.as_str().unwrap().starts_with("The "));
    assert_eq!(room_name, host_welcome["snapshot"]["room_name"]);
    assert_ne!(
        author["name"],
        host_welcome["snapshot"]["participants"][0]["name"]
    );
    let initial = sync_snapshot(&mut host).await;
    assert_eq!(
        initial["participants"],
        guest_welcome["snapshot"]["participants"]
    );
    assert_eq!(initial["events"].as_array().unwrap().len(), 2);

    social(
        &mut guest,
        "chat",
        json!({ "type": "message", "text": "  hello <b>room</b>  " }),
    )
    .await;
    let chat_ack = social_result(&mut guest, "chat").await;
    assert_eq!(chat_ack["type"], "social_ack");
    assert_eq!(chat_ack.as_object().unwrap().len(), 3);
    let event_id = chat_ack["event_id"].as_u64().unwrap();
    let chat = sync_snapshot(&mut host).await;
    let entry = chat["events"].as_array().unwrap().last().unwrap();
    assert_eq!(entry.as_object().unwrap().len(), 9);
    assert_eq!(entry["id"], event_id);
    assert_eq!(entry["member_id"], member_id);
    assert_eq!(entry["name"], author["name"]);
    assert_eq!(entry["avatar"], author["avatar"]);
    assert_eq!(entry["kind"], "message");
    assert_eq!(entry["text"], "hello <b>room</b>");
    assert_eq!(entry["video_id"], Value::Null);
    assert_eq!(entry["position_secs"], Value::Null);
    assert_eq!(chat["revision"], initial["revision"]);
    assert_eq!(chat["anchor_ms"], initial["anchor_ms"]);

    social(
        &mut guest,
        "chat",
        json!({ "type": "message", "text": "hello <b>room</b>" }),
    )
    .await;
    assert_eq!(social_result(&mut guest, "chat").await, chat_ack);
    assert_eq!(sync_snapshot(&mut host).await, chat);
    social(
        &mut guest,
        "chat",
        json!({ "type": "message", "text": "changed" }),
    )
    .await;
    let conflict = social_result(&mut guest, "chat").await;
    assert_eq!(conflict["type"], "error");
    assert_eq!(conflict["code"], "invalid_command");

    social(
        &mut guest,
        "suggest",
        json!({ "type": "propose_video", "video_id": "dQw4w9WgXcQ" }),
    )
    .await;
    assert_eq!(
        social_result(&mut guest, "suggest").await["type"],
        "social_ack"
    );
    let proposed = sync_snapshot(&mut host).await;
    let proposal = proposed["events"].as_array().unwrap().last().unwrap();
    assert_eq!(proposal["kind"], "proposal");
    assert_eq!(proposal["member_id"], member_id);
    assert_eq!(proposal["name"], author["name"]);
    assert_eq!(proposal["avatar"], author["avatar"]);
    assert_eq!(proposal["video_id"], "dQw4w9WgXcQ");
    assert_eq!(proposal["text"], Value::Null);
    assert_eq!(proposed["video_id"], Value::Null);
    assert_eq!(proposed["revision"], 0);
    assert_eq!(proposed["media_revision"], 0);
    assert_eq!(proposed["anchor_ms"], initial["anchor_ms"]);

    let grant = json!({ "type": "set_moderator", "member_id": member_id, "enabled": true });
    social(&mut guest, "grant", grant.clone()).await;
    let denied = social_result(&mut guest, "grant").await;
    assert_eq!(denied["type"], "error");
    assert_eq!(denied["code"], "forbidden");
    social(&mut host, "grant", grant.clone()).await;
    assert_eq!(
        social_result(&mut host, "grant").await["type"],
        "social_ack"
    );
    let granted = sync_snapshot(&mut guest).await;
    assert_eq!(granted["participants"][1]["role"], "moderator");
    assert_eq!(granted["revision"], 0);
    assert_eq!(granted["anchor_ms"], initial["anchor_ms"]);
    assert_eq!(granted["room_name"], room_name);
    social(&mut guest, "grant", grant).await;
    assert_eq!(
        social_result(&mut guest, "grant").await["code"],
        "forbidden"
    );

    command(
        &mut guest,
        "load",
        0,
        json!({ "type": "set_video", "video_id": "dQw4w9WgXcQ" }),
    )
    .await;
    assert_eq!(ack(&mut guest, "load").await["revision"], 1);
    let loaded = sync_snapshot(&mut host).await;
    assert_eq!(
        loaded["events"].as_array().unwrap().last().unwrap()["member_id"],
        member_id
    );
    assert_eq!(
        loaded["events"].as_array().unwrap().last().unwrap()["kind"],
        "set_video"
    );

    social(
        &mut host,
        "revoke",
        json!({ "type": "set_moderator", "member_id": member_id, "enabled": false }),
    )
    .await;
    assert_eq!(
        social_result(&mut host, "revoke").await["type"],
        "social_ack"
    );
    let revoked = sync_snapshot(&mut guest).await;
    assert_eq!(revoked["participants"][1]["role"], "guest");
    assert_eq!(revoked["revision"], loaded["revision"]);
    assert_eq!(revoked["media_revision"], loaded["media_revision"]);
    assert_eq!(revoked["anchor_ms"], loaded["anchor_ms"]);
    command(
        &mut guest,
        "load",
        0,
        json!({ "type": "set_video", "video_id": "dQw4w9WgXcQ" }),
    )
    .await;
    let error = receive(&mut guest, |message| {
        message["type"] == "error" && message["id"] == "load"
    })
    .await;
    assert_eq!(error["code"], "forbidden");

    guest.close(None).await.unwrap();
    let departed = receive(&mut host, |message| {
        message["type"] == "snapshot" && message["snapshot"]["members"] == 1
    })
    .await;
    let left = departed["snapshot"]["events"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(left["kind"], "left");
    assert_eq!(left["name"], author["name"]);
    assert_eq!(left["avatar"], author["avatar"]);
    assert!(
        departed["snapshot"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["id"] == event_id)
    );
    assert_eq!(departed["snapshot"]["room_name"], room_name);
    host.close(None).await.unwrap();
}

#[tokio::test]
async fn escaped_chat_fits_the_bounded_transport_and_social_errors_never_ack_playback() {
    let server = TestServer::start().await;
    let created = server.rooms.create().unwrap();
    let mut guest = server.connect(&created.room_id).await;
    join(&mut guest, None).await;
    let before = sync_snapshot(&mut guest).await;

    // Five hundred escaped control characters need more than the old 2048-byte cap.
    let text = "\u{0001}".repeat(500);
    let wire =
        json!({ "type": "social", "id": "escaped", "action": { "type": "message", "text": text } });
    assert!(wire.to_string().len() > 2048);
    assert!(wire.to_string().len() < 4096);
    send(&mut guest, wire).await;
    assert_eq!(
        social_result(&mut guest, "escaped").await["type"],
        "social_ack"
    );
    let accepted = sync_snapshot(&mut guest).await;
    assert_eq!(
        accepted["events"].as_array().unwrap().last().unwrap()["text"],
        text
    );
    assert_eq!(accepted["revision"], before["revision"]);
    assert_eq!(accepted["anchor_ms"], before["anchor_ms"]);

    for (id, action) in [
        ("empty", json!({ "type": "message", "text": " \n " })),
        (
            "long",
            json!({ "type": "message", "text": "x".repeat(501) }),
        ),
        (
            "bad-video",
            json!({ "type": "propose_video", "video_id": "invalid" }),
        ),
    ] {
        social(&mut guest, id, action).await;
        let error = social_result(&mut guest, id).await;
        assert_eq!(error["type"], "error");
        assert_eq!(error["code"], "invalid_command");
        assert!(error.get("revision").is_none());
        assert!(error.get("event_id").is_none());
    }
    assert_eq!(sync_snapshot(&mut guest).await, accepted);

    social(
        &mut guest,
        "unknown-field",
        json!({ "type": "message", "text": "hello", "extra": true }),
    )
    .await;
    let malformed = receive(&mut guest, |message| message["type"] == "error").await;
    assert_eq!(malformed["code"], "invalid_command");
    assert_eq!(malformed["id"], Value::Null);
    assert_eq!(sync_snapshot(&mut guest).await, accepted);
    guest.close(None).await.unwrap();
}

#[tokio::test]
async fn full_escaped_and_nonascii_history_reaches_welcome_live_updates_and_sync() {
    let server = TestServer::start().await;
    let created = server.rooms.create().unwrap();
    let room = server.rooms.get(&created.room_id).unwrap();
    let mut authors = Vec::new();
    let text = format!("{}{}", "\u{0001}".repeat(400), "😀".repeat(100));
    assert_eq!(text.chars().count(), 500);

    // Separate memberships stay within the existing 20-command burst without
    // sleeping or loosening production rate limits to fill the retained tail.
    for author_index in 0..5 {
        let author = room.join(None).await.unwrap();
        for message_index in 0..20 {
            room.social(
                author.id,
                format!("{author_index}-{message_index}"),
                crate::rooms::SocialAction::Message { text: text.clone() },
            )
            .await
            .unwrap();
        }
        authors.push(author);
    }
    let expected_before_join = authors[0].snapshots.borrow().clone();
    assert_eq!(expected_before_join.events.len(), 100);
    let encoded_len = serde_json::to_string(&expected_before_join).unwrap().len();
    assert!(encoded_len > 250_000);
    assert!(encoded_len < 1024 * 1024);

    let mut guest = server.connect(&created.room_id).await;
    let welcome = join(&mut guest, None).await;
    let expected_welcome = authors[0].snapshots.borrow().clone();
    assert_eq!(
        welcome["snapshot"],
        serde_json::to_value(&expected_welcome).unwrap()
    );
    assert_eq!(welcome["snapshot"]["events"].as_array().unwrap().len(), 100);
    assert_eq!(welcome["snapshot"]["revision"], 0);
    assert_eq!(
        welcome["snapshot"]["anchor_ms"],
        expected_before_join.anchor_ms
    );

    social(
        &mut guest,
        "full-tail",
        json!({ "type": "message", "text": text }),
    )
    .await;
    let latest = receive(&mut guest, |message| {
        message["type"] == "snapshot"
            && message["snapshot"]["events"]
                .as_array()
                .is_some_and(|events| {
                    events.last().is_some_and(|event| {
                        event["kind"] == "message" && event["member_id"] == welcome["member_id"]
                    })
                })
    })
    .await["snapshot"]
        .clone();
    let expected_latest = authors[0].snapshots.borrow().clone();
    assert_eq!(latest, serde_json::to_value(&expected_latest).unwrap());
    assert_eq!(latest["events"].as_array().unwrap().len(), 100);
    assert_eq!(
        latest["events"].as_array().unwrap().last().unwrap()["text"],
        text
    );
    assert_eq!(latest["anchor_ms"], expected_before_join.anchor_ms);
    assert_eq!(latest["revision"], 0);
    assert_eq!(latest["media_revision"], 0);

    assert_eq!(sync_snapshot(&mut guest).await, latest);
    guest.close(None).await.unwrap();
}

#[tokio::test]
async fn invalid_tokens_and_unknown_rooms_are_explicit() {
    let server = TestServer::start().await;
    let created = server.rooms.create().unwrap();
    let mut invalid_host = server.connect(&created.room_id).await;
    send(
        &mut invalid_host,
        json!({ "type": "join", "host_token": "wrong" }),
    )
    .await;
    let error = receive(&mut invalid_host, |message| message["type"] == "error").await;
    assert_eq!(error["code"], "invalid_token");

    let mut missing = server.connect("missing").await;
    let error = receive(&mut missing, |message| message["type"] == "error").await;
    assert_eq!(error["code"], "unavailable");
    let close = timeout(Duration::from_secs(5), missing.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(close, Message::Close(Some(frame)) if u16::from(frame.code) == 4004));
}

#[tokio::test]
async fn cross_origin_socket_upgrade_is_rejected() {
    let server = TestServer::start().await;
    let created = server.rooms.create().unwrap();
    let mut request = format!("ws://{}/api/rooms/{}/ws", server.address, created.room_id)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", "https://hostile.example".parse().unwrap());
    request
        .headers_mut()
        .insert("cookie", server.cookie.as_ref().unwrap().parse().unwrap());

    assert!(
        matches!(connect_async(request).await, Err(Error::Http(response)) if response.status() == 403)
    );
}

#[tokio::test]
async fn raw_upgrades_without_cookie_are_rejected_before_room_lookup_even_with_url_token() {
    let server = TestServer::start().await;
    let created = server.rooms.create().unwrap();
    for room in [created.room_id.as_str(), "missing"] {
        for query in [String::new(), format!("?access_token={}", "a".repeat(64))] {
            let mut request = format!("ws://{}/api/rooms/{room}/ws{query}", server.address)
                .into_client_request()
                .unwrap();
            request.headers_mut().insert(
                "origin",
                format!("http://{}", server.address).parse().unwrap(),
            );
            let Err(Error::Http(response)) = connect_async(request).await else {
                panic!("locked websocket must not upgrade");
            };
            assert_eq!(response.status(), 401);
            assert_eq!(response.headers()["content-type"], "application/json");
            let body = response.body().as_ref().unwrap();
            let failure: Value = serde_json::from_slice(body).unwrap();
            assert_eq!(failure["code"], "access_required");
            assert!(!String::from_utf8_lossy(body).contains(&"a".repeat(64)));
        }
    }
    // Site admission does not grant any room or host authority.
    let mut guest = server.connect(&created.room_id).await;
    assert_eq!(join(&mut guest, None).await["role"], "guest");
    guest.close(None).await.unwrap();
}

#[tokio::test]
async fn malformed_and_oversized_messages_cannot_mutate_a_room() {
    let server = TestServer::start().await;
    let created = server.rooms.create().unwrap();
    let mut socket = server.connect(&created.room_id).await;
    join(&mut socket, None).await;
    send(&mut socket, json!({ "type": "command", "id": "malformed" })).await;
    let error = receive(&mut socket, |message| message["type"] == "error").await;
    assert_eq!(error["code"], "invalid_command");

    socket.send(Message::text("x".repeat(4097))).await.unwrap();
    let _ = timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap();
    drop(socket);

    let mut guest = server.connect(&created.room_id).await;
    let welcome = join(&mut guest, None).await;
    assert_eq!(welcome["snapshot"]["revision"], 0);
    assert_eq!(welcome["snapshot"]["video_id"], Value::Null);
    guest.close(None).await.unwrap();
}
