mod access;
mod api;
mod cli;
mod components;
mod pages;
mod rooms;

#[cfg(test)]
mod socket_tests;

use clap::Parser;
use http::header::{HeaderName, HeaderValue};
use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    context::Cx,
    router::{
        Body, BodyLimit, Layer, LayerFuture, Next, OriginPolicy, Path, Router,
        RouterBuilderDiscoverExt,
        request::{headers, uri},
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
        // AccessGate owns origin enforcement so locked upgrades always return 401.
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
        let referrer = if access::has_access_query(uri(cx).query())
            || !topcoat::context::app_context::<access::Access>(cx).authenticated(headers(cx))
        {
            "no-referrer"
        } else {
            "strict-origin-when-cross-origin"
        };
        let headers = response_headers(cx);

        for (name, value) in [
            ("x-content-type-options", "nosniff"),
            ("x-frame-options", "DENY"),
            // YouTube needs the embedding origin, but not the private room path.
            ("referrer-policy", referrer),
        ] {
            headers.append(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }

        let private = access::has_access_query(uri(cx).query())
            || !access::public_asset(uri(cx).path())
            || topcoat::router::request::headers(cx).contains_key("upgrade")
            || !matches!(
                topcoat::router::request::method(cx),
                &http::Method::GET | &http::Method::HEAD
            );
        if private {
            headers.append(
                HeaderName::from_static("vary"),
                HeaderValue::from_static("Cookie"),
            );
        }

        Box::pin(async move {
            let result = next.run(cx, body).await;
            if private {
                match result {
                    Ok(mut response) => {
                        response
                            .headers_mut()
                            .insert("cache-control", HeaderValue::from_static("no-store"));
                        return Ok(response);
                    }
                    Err(error) => {
                        headers.append(
                            HeaderName::from_static("cache-control"),
                            HeaderValue::from_static("no-store"),
                        );
                        return Err(error);
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
    use topcoat::router::{Body, Router, RouterBuilderDiscoverExt, to_bytes};

    use super::*;

    fn api_router() -> Router {
        router_with_access(access::Access::for_test("a".repeat(64)))
    }

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
            // Keep these admission/API tests independent of an asset build.
            .route(topcoat::router::RouteFn::new(
                http::Method::GET,
                "/test-admission",
                |_, _| {
                    Box::pin(async {
                        Ok(topcoat::router::response::Response::new(Body::from("home")))
                    })
                },
            ))
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
            assert!(
                !String::from_utf8(body.to_vec())
                    .unwrap()
                    .contains("access-form")
            );
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
        let router = api_router();
        let request = Request::builder()
            .method("POST")
            .uri("/api/rooms")
            .header(
                "cookie",
                access::Access::for_test("a".repeat(64)).test_cookie(),
            )
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
        let router = api_router();
        let cross_origin = Request::builder()
            .method("POST")
            .uri("/api/rooms")
            .header(
                "cookie",
                access::Access::for_test("a".repeat(64)).test_cookie(),
            )
            .header("host", "localhost:3000")
            .header("origin", "https://other.example")
            .body(Body::empty())
            .unwrap();

        assert_eq!(
            router.handle(cross_origin).await.status(),
            StatusCode::FORBIDDEN
        );

        let large = Request::builder()
            .method("POST")
            .uri("/api/rooms")
            .header(
                "cookie",
                access::Access::for_test("a".repeat(64)).test_cookie(),
            )
            .body(Body::from("x".repeat(2048)))
            .unwrap();

        assert_eq!(
            router.handle(large).await.status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }
}
