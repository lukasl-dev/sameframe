mod access;
mod api;
mod cli;
mod components;
mod pages;
mod rooms;

#[cfg(test)]
mod socket_tests;

use clap::Parser;
use http::{
    Method,
    header::{HeaderName, HeaderValue},
};
use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    context::{Cx, app_context},
    router::{
        Body, BodyLimit, Layer, LayerFuture, Next, OriginPolicy, Path, Router,
        RouterBuilderDiscoverExt,
        request::{headers, method, uri},
        response::response_headers,
    },
};

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::Cli::parse();
    let access = if args.public {
        access::Access::public()
    } else {
        access::Access::load(
            args.access_token_file
                .as_deref()
                .unwrap_or(std::path::Path::new("access-token")),
        )?
    };
    let router = Router::builder()
        .app_context(access)
        .origin_policy(OriginPolicy::dangerous_disable())
        .app_context(rooms::Rooms::new())
        .app_context(api::ConnectionLimits::new())
        .layer(BodyLimit::max(1024))
        .layer(access::AccessGate)
        .layer(BrowserPolicy)
        .assets(AssetBundle::load()?)
        .discover()
        .build();

    topcoat::start(router).await?;
    Ok(())
}

struct BrowserPolicy;

impl Layer for BrowserPolicy {
    fn path(&self) -> Option<&Path> {
        None
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        let query_token = access::has_access_query(uri(cx).query());
        let referrer =
            if query_token || !app_context::<access::Access>(cx).authenticated(headers(cx)) {
                "no-referrer"
            } else {
                "strict-origin-when-cross-origin"
            };
        let policy_headers = response_headers(cx);

        for (name, value) in [
            ("x-content-type-options", "nosniff"),
            ("x-frame-options", "DENY"),
            ("referrer-policy", referrer),
        ] {
            policy_headers.append(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }

        let private = query_token
            || !access::public_asset(uri(cx).path())
            || headers(cx).contains_key("upgrade")
            || !matches!(method(cx), &Method::GET | &Method::HEAD);
        if private {
            policy_headers.append(
                HeaderName::from_static("vary"),
                HeaderValue::from_static("Cookie"),
            );
        }

        Box::pin(async move {
            let mut result = next.run(cx, body).await;
            if private {
                match &mut result {
                    Ok(response) => {
                        response
                            .headers_mut()
                            .insert("cache-control", HeaderValue::from_static("no-store"));
                    }
                    Err(_) => {
                        policy_headers.append(
                            HeaderName::from_static("cache-control"),
                            HeaderValue::from_static("no-store"),
                        );
                    }
                }
            }

            result
        })
    }
}

#[cfg(test)]
mod tests {
    use http::{Request, StatusCode};
    use topcoat::router::{RouteFn, response::Response, to_bytes};

    use super::*;

    fn router_with_access(access: access::Access) -> Router {
        Router::builder()
            .app_context(access)
            .origin_policy(OriginPolicy::dangerous_disable())
            .app_context(rooms::Rooms::new())
            .app_context(api::ConnectionLimits::new())
            .layer(BodyLimit::max(1024))
            .layer(access::AccessGate)
            .layer(BrowserPolicy)
            .discover()
            .route(RouteFn::new(Method::GET, "/test-admission", |_, _| {
                Box::pin(async { Ok(Response::new(Body::from("home"))) })
            }))
            .build()
    }

    #[tokio::test]
    async fn public_mode_opens_pages_and_apis_but_preserves_origin_checks() {
        let router = router_with_access(access::Access::public());

        for path in ["/test-admission", "/test-admission?access_token=unused"] {
            let response = router
                .handle(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            assert_eq!(String::from_utf8(body.to_vec()).unwrap(), "home");
        }

        let response = router
            .handle(
                Request::builder()
                    .method("POST")
                    .uri("/api/rooms")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(!response.headers().contains_key("set-cookie"));

        let cross_origin = Request::builder()
            .method("POST")
            .uri("/api/rooms")
            .header("host", "localhost:3000")
            .header("origin", "https://other.example")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router.handle(cross_origin).await.status(),
            StatusCode::FORBIDDEN
        );

        let unlock = Request::builder()
            .method("POST")
            .uri("/api/access")
            .body(Body::from(r#"{"token":"unused"}"#))
            .unwrap();
        let response = router.handle(unlock).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(!response.headers().contains_key("set-cookie"));
    }

    #[tokio::test]
    async fn create_room_returns_private_credentials_without_caching() {
        let access = access::Access::for_test("a".repeat(64));
        let cookie = access.test_cookie();
        let router = router_with_access(access);

        let request = Request::builder()
            .method("POST")
            .uri("/api/rooms")
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        let response = router.handle(request).await;

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");

        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let created: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert!(created["room_id"].as_str().unwrap().len() >= 16);
        assert!(created["host_token"].as_str().unwrap().len() >= 32);
        assert_ne!(created["room_id"], created["host_token"]);
    }

    #[tokio::test]
    async fn cross_origin_creation_and_large_bodies_are_rejected() {
        let access = access::Access::for_test("a".repeat(64));
        let cookie = access.test_cookie();
        let router = router_with_access(access);

        let cross_origin = Request::builder()
            .method("POST")
            .uri("/api/rooms")
            .header("cookie", &cookie)
            .header("host", "localhost:3000")
            .header("origin", "https://other.example")
            .body(Body::empty())
            .unwrap();

        let response = router.handle(cross_origin).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(
            response
                .headers()
                .get_all("vary")
                .iter()
                .any(|value| value == "Cookie")
        );

        let large = Request::builder()
            .method("POST")
            .uri("/api/rooms")
            .header("cookie", cookie)
            .body(Body::from("x".repeat(2048)))
            .unwrap();

        let response = router.handle(large).await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(
            response
                .headers()
                .get_all("vary")
                .iter()
                .any(|value| value == "Cookie")
        );
    }
}
