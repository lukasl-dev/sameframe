use std::{
    collections::HashMap,
    fs::{self, DirBuilder, File},
    io::{self, Read, Write},
    net::IpAddr,
    path::Path as FilePath,
    sync::Mutex,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

use http::{HeaderMap, Method, StatusCode};
use rand::TryRng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use topcoat::{
    Result,
    context::{Cx, app_context},
    router::{
        Body, Layer, LayerFuture, Next, Path,
        request::{client_ip, headers, method, uri},
        response::{IntoResponse, Response},
        route, to_bytes,
    },
    view::ViewExt,
};

const COOKIE: &str = "sameframe_access";
const WINDOW: Duration = Duration::from_secs(60);
const MESSAGE: &str = "Enter the site access token to continue.";

pub(crate) struct Access {
    key: Option<Key>,
    attempts: Mutex<Attempts>,
}

struct Key {
    token: String,
    cookie: String,
}

struct Attempts {
    global: AttemptWindow,
    peers: HashMap<Option<IpAddr>, AttemptWindow>,
}

struct AttemptWindow {
    started: Instant,
    count: u16,
}

impl Access {
    pub(crate) fn load(path: &FilePath) -> io::Result<Self> {
        let access = Self::load_file(path)?;
        eprintln!("Site access token file: {}", path.display());
        Ok(access)
    }

    fn load_file(path: &FilePath) -> io::Result<Self> {
        match read_token(path) {
            Ok(token) => return Ok(Self::new(token)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if fs::symlink_metadata(path).is_ok() {
                    return Err(io::Error::other("access token file is unreadable"));
                }
            }
            Err(error) => return Err(error),
        }

        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(FilePath::new("."));
        let mut dirs = DirBuilder::new();
        dirs.recursive(true);
        #[cfg(unix)]
        dirs.mode(0o700);
        dirs.create(parent)?;

        let mut random = [0u8; 32];
        rand::rngs::SysRng
            .try_fill_bytes(&mut random)
            .map_err(|_| io::Error::other("could not generate site access token"))?;
        let token = hex(&random);
        let mut pending = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        pending
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
        writeln!(pending, "{token}")?;
        pending.as_file().sync_all()?;
        match pending.persist_noclobber(path) {
            Ok(_) => {
                File::open(parent)?.sync_all()?;
                Ok(Self::new(token))
            }
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                Ok(Self::new(read_token(path)?))
            }
            Err(error) => Err(error.error),
        }
    }

    fn new(token: String) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"sameframe/site-access-cookie/v1\0");
        digest.update(token.as_bytes());
        Self::with_key(Some(Key {
            token,
            cookie: hex(&digest.finalize()),
        }))
    }

    pub(crate) fn public() -> Self {
        Self::with_key(None)
    }

    fn with_key(key: Option<Key>) -> Self {
        Self {
            key,
            attempts: Mutex::new(Attempts {
                global: AttemptWindow {
                    started: Instant::now(),
                    count: 0,
                },
                peers: HashMap::new(),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(token: String) -> Self {
        assert!(valid_token(&token));
        Self::new(token)
    }

    #[cfg(test)]
    pub(crate) fn test_cookie(&self) -> String {
        format!("{COOKIE}={}", self.key.as_ref().unwrap().cookie)
    }

    pub(crate) fn authenticated(&self, headers: &HeaderMap) -> bool {
        let Some(key) = &self.key else {
            return true;
        };

        let mut found = None;
        for header in headers.get_all("cookie") {
            let Ok(header) = header.to_str() else {
                return false;
            };
            for part in header.split(';') {
                let Some((name, value)) = part.trim().split_once('=') else {
                    continue;
                };
                if name == COOKIE && found.replace(value).is_some() {
                    return false;
                }
            }
        }

        found.is_some_and(|value| fixed_equal(value, &key.cookie))
    }

    fn allow_attempt(&self, peer: Option<IpAddr>) -> bool {
        let now = Instant::now();
        let mut attempts = self.attempts.lock().expect("access limits lock poisoned");
        attempts
            .peers
            .retain(|_, window| now.duration_since(window.started) < WINDOW);
        if now.duration_since(attempts.global.started) >= WINDOW {
            attempts.global = AttemptWindow {
                started: now,
                count: 0,
            };
        }
        if attempts.global.count >= 120
            || (attempts.peers.len() >= 1024 && !attempts.peers.contains_key(&peer))
        {
            return false;
        }

        attempts.global.count += 1;
        let window = attempts.peers.entry(peer).or_insert(AttemptWindow {
            started: now,
            count: 0,
        });
        if window.count >= 12 {
            return false;
        }

        window.count += 1;
        true
    }
}

fn read_token(path: &FilePath) -> io::Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(io::Error::other("access token must be a regular file"));
    }

    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("access token must be a regular file"));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(io::Error::other(
            "access token file permissions must be 0600",
        ));
    }

    let mut bytes = Vec::new();
    file.take(66).read_to_end(&mut bytes)?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    let token =
        String::from_utf8(bytes).map_err(|_| io::Error::other("malformed access token file"))?;
    if !valid_token(&token) {
        return Err(io::Error::other("malformed access token file"));
    }
    Ok(token)
}

fn valid_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn fixed_equal(candidate: &str, expected: &str) -> bool {
    candidate.len() == 64 && bool::from(candidate.as_bytes().ct_eq(expected.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 15) as usize] as char,
            ]
        })
        .collect()
}

pub(crate) fn has_access_query(query: Option<&str>) -> bool {
    query.is_some_and(|query| {
        serde_urlencoded::from_str::<Vec<(String, String)>>(query)
            .is_ok_and(|pairs| pairs.iter().any(|(key, _)| key == "access_token"))
    })
}

pub(crate) fn public_asset(path: &str) -> bool {
    path.strip_prefix("/_topcoat/assets/").is_some_and(|rest| {
        !rest.is_empty()
            && rest.split('/').all(|segment| {
                !segment.is_empty()
                    && segment != "."
                    && segment != ".."
                    && segment
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            })
    })
}

pub(crate) struct AccessGate;

impl Layer for AccessGate {
    fn path(&self) -> Option<&Path> {
        None
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        Box::pin(async move {
            let path = uri(cx).path();
            let safe = matches!(method(cx), &Method::GET | &Method::HEAD);
            let query_token = has_access_query(uri(cx).query());
            let upgrade = headers(cx).contains_key("upgrade");
            let document = safe
                && !upgrade
                && !path.starts_with("/api/")
                && path != "/api"
                && !path.starts_with("/_topcoat/")
                && path != "/_topcoat";
            let public = (method(cx) == Method::POST && path == "/api/access")
                || (safe && !upgrade && public_asset(path));
            let access = app_context::<Access>(cx);
            let authenticated = access.authenticated(headers(cx));

            if !public && (!authenticated || (access.key.is_some() && document && query_token)) {
                if document {
                    if method(cx) == Method::HEAD {
                        return Ok(Response::builder()
                            .status(StatusCode::UNAUTHORIZED)
                            .header("content-type", "text/html; charset=utf-8")
                            .body(Body::empty())?);
                    }
                    let __cx = cx;
                    let gate = topcoat::view::view! { crate::pages::access_gate() };
                    let mut response = gate.single().await?.into_response(cx)?;
                    *response.status_mut() = StatusCode::UNAUTHORIZED;
                    return Ok(response);
                }
                return denied(StatusCode::UNAUTHORIZED);
            }

            let cross_site = headers(cx)
                .get("sec-fetch-site")
                .is_some_and(|site| site != "same-origin" && site != "none");
            if !public
                && (!safe || upgrade)
                && (cross_site || !crate::api::same_origin(headers(cx), upgrade))
            {
                return denied(StatusCode::FORBIDDEN);
            }
            next.run(cx, body).await
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Unlock {
    token: String,
}

#[route(POST "/api/access")]
async fn unlock(cx: &Cx, body: Body) -> Result<Response> {
    let access = app_context::<Access>(cx);
    let Some(key) = &access.key else {
        return denied(StatusCode::NOT_FOUND);
    };
    if !access.allow_attempt(client_ip(cx)) {
        return denied(StatusCode::TOO_MANY_REQUESTS);
    }
    if headers(cx).get_all("origin").iter().count() != 1
        || headers(cx).get_all("host").iter().count() != 1
        || !crate::api::same_origin(headers(cx), true)
    {
        return denied(StatusCode::FORBIDDEN);
    }
    let content_type = headers(cx)
        .get("content-type")
        .and_then(|value| value.to_str().ok());
    if !content_type.is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
    }) {
        return denied(StatusCode::FORBIDDEN);
    }

    let bytes = match to_bytes(body, 1024).await {
        Ok(bytes) => bytes,
        Err(_) => return denied(StatusCode::PAYLOAD_TOO_LARGE),
    };
    let Ok(input) = serde_json::from_slice::<Unlock>(&bytes) else {
        return denied(StatusCode::FORBIDDEN);
    };
    if !fixed_equal(&input.token, &key.token) {
        return denied(StatusCode::FORBIDDEN);
    }

    let secure = if loopback_host(headers(cx)) {
        ""
    } else {
        "; Secure"
    };
    Ok(Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("cache-control", "no-store")
        .header(
            "set-cookie",
            format!(
                "{COOKIE}={}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Strict{secure}",
                key.cookie
            ),
        )
        .body(Body::empty())?)
}

#[route(GET "/api/access")]
async fn verify_cookie() -> Result<Response> {
    Ok(Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header("cache-control", "no-store")
        .body(Body::empty())?)
}

fn loopback_host(headers: &HeaderMap) -> bool {
    headers
        .get("host")
        .and_then(|host| host.to_str().ok())
        .and_then(|host| host.parse::<http::uri::Authority>().ok())
        .is_some_and(|authority| {
            let host = authority.host();
            host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]"
        })
}

fn denied(status: StatusCode) -> Result<Response> {
    Ok(Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("cache-control", "no-store")
        .body(Body::from(
            serde_json::json!({"code":"access_required", "message":MESSAGE}).to_string(),
        ))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrowserPolicy, api::ConnectionLimits, rooms::Rooms};
    use http::Request;
    use topcoat::{
        asset::{AssetConfig, AssetId, AssetOptions},
        router::{BodyLimit, OriginPolicy, RouteFn, Router, RouterBuilderDiscoverExt},
    };

    fn token() -> String {
        "a".repeat(64)
    }

    fn router() -> Router {
        router_with_rooms(Rooms::new())
    }

    fn router_with_rooms(rooms: Rooms) -> Router {
        Router::builder()
            .app_context(Access::for_test(token()))
            .app_context(AssetConfig::hosted_at(
                "/_topcoat/assets",
                topcoat::asset::Manifest {
                    version: 1,
                    assets: [
                        concat!(env!("OUT_DIR"), "/tailwind.css"),
                        "assets/access.js",
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, path)| topcoat::asset::ManifestEntry {
                        id: AssetId::new(
                            env!("CARGO_PKG_NAME"),
                            "src/pages.rs",
                            path,
                            &AssetOptions::NONE,
                        ),
                        file: format!("fixture-{index}.css"),
                        hash: String::new(),
                        content_type: "text/plain".into(),
                    })
                    .collect(),
                },
            ))
            .app_context(rooms)
            .app_context(ConnectionLimits::new())
            .origin_policy(OriginPolicy::dangerous_disable())
            .layer(BodyLimit::max(1024))
            .layer(AccessGate)
            .layer(BrowserPolicy)
            .route(RouteFn::new(
                Method::GET,
                "/_topcoat/assets/gate.css",
                |_, _| Box::pin(async { Ok(Response::new(Body::from("body {}"))) }),
            ))
            .discover()
            .build()
    }

    fn request(method: &str, path: &str, cookie: bool) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "localhost:3000");
        if cookie {
            builder = builder.header("cookie", Access::for_test(token()).test_cookie());
        }
        builder.body(Body::empty()).unwrap()
    }

    async fn text(response: Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    fn unlock_request(body: String, origin: Option<&str>, host: &str) -> Request<Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/api/access")
            .header("host", host)
            .header("content-type", "application/json");
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        builder.body(Body::from(body)).unwrap()
    }

    #[tokio::test]
    async fn every_document_is_locked_before_private_or_unavailable_content() {
        let router = router();

        for path in [
            "/",
            "/room/missing",
            "/not-a-route",
            "/room/missing?access_token=bad",
        ] {
            let response = router.handle(request("GET", path, false)).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()["cache-control"], "no-store");
            assert_eq!(
                response.headers().get_all("cache-control").iter().count(),
                1
            );
            assert_eq!(response.headers()["referrer-policy"], "no-referrer");
            assert_eq!(response.headers()["x-frame-options"], "DENY");
            assert!(
                response
                    .headers()
                    .get_all("vary")
                    .iter()
                    .any(|v| v == "Cookie")
            );
            let body = text(response).await;
            assert!(!body.contains(&token()));
            assert!(!body.contains("room.js"));
            assert!(!body.contains("This room has expired"));
            assert!(body.contains("id=\"access-form\""));
            assert!(!body.contains("missing"));
            assert!(!body.contains("not-a-route"));
        }

        let head = router.handle(request("HEAD", "/unknown", false)).await;
        assert_eq!(head.status(), StatusCode::UNAUTHORIZED);
        assert!(text(head).await.is_empty());

        let authed_unknown = router.handle(request("GET", "/unknown", true)).await;
        assert_eq!(authed_unknown.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn known_room_state_is_not_exposed_by_locked_html() {
        let rooms = Rooms::new();
        let created = rooms.create().unwrap();
        let room = rooms.get(&created.room_id).unwrap();
        let member = room
            .join(Some(created.host_token.clone()), None)
            .await
            .unwrap();
        let snapshot = member.snapshots.borrow().clone();
        let router = router_with_rooms(rooms);

        let response = router
            .handle(request("GET", &format!("/room/{}", created.room_id), false))
            .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = text(response).await;

        for secret in [
            &created.host_token,
            &created.room_id,
            &snapshot.room_name,
            &snapshot.incarnation,
            &token(),
        ] {
            assert!(!body.contains(secret));
        }
        assert!(!body.contains("room.js"));
        assert!(!body.contains("youtube.com"));
    }

    #[tokio::test]
    async fn explicit_query_token_always_gets_gate_even_with_authenticated_cookie() {
        let router = router();

        for path in [
            format!("/?access_token={}", token()),
            "/?%61ccess%5Ftoken=bad&other=1".into(),
            "/unknown?access_token&access_token=bad".into(),
        ] {
            let response = router.handle(request("GET", &path, true)).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()["referrer-policy"], "no-referrer");
            assert!(!text(response).await.contains(&token()));
        }

        let response = router
            .handle(request("GET", "/api/missing?%61ccess_token=bad", true))
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    }

    #[tokio::test]
    async fn locked_api_mutations_upgrades_and_runtime_never_receive_html() {
        let router = router();

        for (method, path) in [
            ("POST", "/api/rooms"),
            ("GET", "/api/access"),
            ("GET", "/api/rooms/known/ws"),
            ("POST", "/_topcoat/procedures/anything"),
            ("GET", "/_topcoat/dev/ws"),
        ] {
            let mut req = request(method, path, false);
            if path.ends_with("ws") {
                req.headers_mut()
                    .insert("upgrade", "websocket".parse().unwrap());
            }
            req.headers_mut()
                .insert("origin", "https://hostile.example".parse().unwrap());

            let response = router.handle(req).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()["content-type"], "application/json");

            let body = text(response).await;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&body).unwrap(),
                serde_json::json!({"code":"access_required", "message":MESSAGE})
            );
            assert!(!body.contains(&token()));
        }

        assert_eq!(
            router
                .handle(request("GET", "/api/access", true))
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn only_safe_canonical_static_asset_paths_are_public() {
        let router = router();
        assert_eq!(
            router
                .handle(request("GET", "/_topcoat/assets/gate.css", false))
                .await
                .status(),
            StatusCode::OK
        );

        let asset = router
            .handle(request(
                "GET",
                "/_topcoat/assets/gate.css?access_token=bad",
                false,
            ))
            .await;
        assert_eq!(asset.status(), StatusCode::OK);
        assert_eq!(asset.headers()["referrer-policy"], "no-referrer");
        assert_eq!(asset.headers()["cache-control"], "no-store");

        assert_ne!(
            router
                .handle(request("HEAD", "/_topcoat/assets/gate.css", false))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );

        for path in [
            "/_topcoat/assets",
            "/_topcoat/assets/",
            "/_topcoat/assets/../procedures",
            "/_topcoat/assets/%2e%2e/dev",
            "/_topcoat/assets//dev",
            "/_topcoat/assetsx/gate.css",
            "/_topcoat/runtime.js",
            "/_topcoat/dev",
            "/_topcoat/assets/gate.css/../../dev",
        ] {
            assert_eq!(
                router.handle(request("GET", path, false)).await.status(),
                StatusCode::UNAUTHORIZED,
                "{path}"
            );
        }

        assert_eq!(
            router
                .handle(request("POST", "/_topcoat/assets/gate.css", false))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn unlock_is_strict_bounded_same_origin_and_sets_private_persistent_cookie() {
        let router = router();
        let body = serde_json::json!({"token":token()}).to_string();

        for origin in [
            None,
            Some("null"),
            Some("https://evil.test"),
            Some("http://localhost:3000.evil.test"),
        ] {
            let response = router
                .handle(unlock_request(body.clone(), origin, "localhost:3000"))
                .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(!text(response).await.contains(&token()));
        }

        for malformed in [
            "{}".into(),
            "not json".into(),
            format!("{{\"token\":\"{}\",\"extra\":1}}", token()),
            format!("{{\"token\":\"{}\",\"token\":\"{}\"}}", token(), token()),
            serde_json::json!({"token":"b".repeat(64)}).to_string(),
        ] {
            let response = router
                .handle(unlock_request(
                    malformed,
                    Some("http://localhost:3000"),
                    "localhost:3000",
                ))
                .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(!text(response).await.contains(&token()));
        }

        let oversized = router
            .handle(unlock_request(
                "x".repeat(1025),
                Some("http://localhost:3000"),
                "localhost:3000",
            ))
            .await;
        assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let response = router
            .handle(unlock_request(
                body,
                Some("http://localhost:3000"),
                "localhost:3000",
            ))
            .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let cookie = response.headers()["set-cookie"].to_str().unwrap();
        assert_eq!(
            cookie,
            format!(
                "{}; Path=/; Max-Age=31536000; HttpOnly; SameSite=Strict",
                Access::for_test(token()).test_cookie()
            )
        );
        assert!(!cookie.contains(&token()));
        assert_eq!(response.headers()["cache-control"], "no-store");

        assert!(text(response).await.is_empty());
    }

    #[tokio::test]
    async fn unlock_rejects_missing_content_type_and_duplicate_origin() {
        let router = router();
        let body = serde_json::json!({"token":token()}).to_string();

        let mut missing_type = unlock_request(
            body.clone(),
            Some("http://localhost:3000"),
            "localhost:3000",
        );
        missing_type.headers_mut().remove("content-type");
        assert_eq!(
            router.handle(missing_type).await.status(),
            StatusCode::FORBIDDEN
        );

        let mut duplicate_origin =
            unlock_request(body, Some("http://localhost:3000"), "localhost:3000");
        duplicate_origin
            .headers_mut()
            .append("origin", "http://localhost:3000".parse().unwrap());
        assert_eq!(
            router.handle(duplicate_origin).await.status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn cookie_security_uses_public_host_not_forwarded_headers_and_duplicates_fail_closed() {
        for (host, origin, secure) in [
            ("localhost:3000", "http://localhost:3000", false),
            ("127.0.0.1:3000", "http://127.0.0.1:3000", false),
            ("[::1]:3000", "http://[::1]:3000", false),
            ("watch.example", "https://watch.example", true),
            ("localhost.evil.test", "https://localhost.evil.test", true),
        ] {
            let router = router();
            let mut req = unlock_request(
                serde_json::json!({"token":token()}).to_string(),
                Some(origin),
                host,
            );
            req.headers_mut()
                .insert("x-forwarded-host", "localhost".parse().unwrap());
            req.headers_mut()
                .insert("x-forwarded-proto", "http".parse().unwrap());
            let response = router.handle(req).await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert_eq!(
                response.headers()["set-cookie"]
                    .to_str()
                    .unwrap()
                    .contains("; Secure"),
                secure
            );
        }

        let router = router();
        let cookie = Access::for_test(token()).test_cookie();
        for separate_headers in [false, true] {
            let mut req = request("GET", "/api/access", true);
            if separate_headers {
                req.headers_mut().append("cookie", cookie.parse().unwrap());
            } else {
                req.headers_mut()
                    .insert("cookie", format!("{cookie}; {cookie}").parse().unwrap());
            }
            assert_eq!(router.handle(req).await.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn rate_limits_are_bounded_reusable_and_do_not_limit_authenticated_probes() {
        let router = router();

        for _ in 0..12 {
            assert_eq!(
                router
                    .handle(unlock_request(
                        "{}".into(),
                        Some("http://localhost:3000"),
                        "localhost:3000"
                    ))
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            router
                .handle(unlock_request(
                    "{}".into(),
                    Some("http://localhost:3000"),
                    "localhost:3000"
                ))
                .await
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            router
                .handle(request("GET", "/api/access", true))
                .await
                .status(),
            StatusCode::NO_CONTENT
        );

        let access = Access::for_test(token());
        for index in 0..120 {
            assert!(access.allow_attempt(Some(IpAddr::from([10, 0, 0, index]))));
        }
        assert!(!access.allow_attempt(Some(IpAddr::from([10, 0, 0, 121]))));

        {
            let mut attempts = access.attempts.lock().unwrap();
            attempts.global.started -= WINDOW;
            for window in attempts.peers.values_mut() {
                window.started -= WINDOW;
            }
        }

        assert!(access.allow_attempt(None));
        assert_eq!(access.attempts.lock().unwrap().peers.len(), 1);
    }

    #[tokio::test]
    async fn authenticated_runtime_mutations_still_require_origin_defences() {
        let router = router();
        let mut req = request("POST", "/_topcoat/procedures/unknown", true);
        req.headers_mut()
            .insert("origin", "https://evil.test".parse().unwrap());
        assert_eq!(router.handle(req).await.status(), StatusCode::FORBIDDEN);

        let mut req = request("POST", "/_topcoat/procedures/unknown", true);
        req.headers_mut()
            .insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert_eq!(router.handle(req).await.status(), StatusCode::FORBIDDEN);

        assert_eq!(
            router
                .handle(request("POST", "/_topcoat/procedures/unknown", true))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn startup_is_persistent_atomic_and_rotation_invalidates_cookies() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private/nested/access-token");

        let first = Access::load_file(&path).unwrap();
        let first_token = &first.key.as_ref().unwrap().token;
        assert!(valid_token(first_token));
        assert_ne!(first_token, &token());
        assert_eq!(read_token(&path).unwrap(), *first_token);

        let second = Access::load_file(&path).unwrap();
        assert_eq!(first_token, &second.key.as_ref().unwrap().token);
        assert_eq!(first.test_cookie(), second.test_cookie());

        #[cfg(unix)]
        {
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap().parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }

        fs::write(&path, token()).unwrap();
        let rotated = Access::load_file(&path).unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("cookie", first.test_cookie().parse().unwrap());
        assert!(second.authenticated(&headers));
        assert!(!rotated.authenticated(&headers));

        headers.insert("cookie", rotated.test_cookie().parse().unwrap());
        assert!(rotated.authenticated(&headers));
    }

    #[test]
    fn concurrent_starts_publish_one_complete_token() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("new/access-token");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    Access::load_file(&path).unwrap().key.unwrap().token
                })
            })
            .collect();

        let tokens: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert!(tokens.iter().all(|token| token == &tokens[0]));
        assert_eq!(read_token(&path).unwrap(), tokens[0]);
    }

    #[test]
    fn malformed_unreadable_or_insecure_existing_files_are_never_rotated() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("token");

        for contents in [
            "bad".to_string(),
            "A".repeat(64),
            format!("{}\nextra", token()),
            format!(" {}", token()),
        ] {
            fs::write(&path, &contents).unwrap();
            #[cfg(unix)]
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            assert!(Access::load_file(&path).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }

        #[cfg(unix)]
        {
            fs::write(&path, token()).unwrap();
            for mode in [0o644, 0o000] {
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
                assert!(Access::load_file(&path).is_err());
                assert_eq!(
                    fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    mode
                );
            }

            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(temp.path().join("missing"), &path).unwrap();
            assert!(Access::load_file(&path).is_err());
            assert!(
                fs::symlink_metadata(&path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            fs::remove_file(&path).unwrap();
        }

        fs::create_dir(&path).unwrap();
        assert!(Access::load_file(&path).is_err());
        assert!(path.is_dir());
    }
}
