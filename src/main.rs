mod api;
mod components;
mod pages;
mod rooms;

#[cfg(test)]
mod socket_tests;

use http::header::{HeaderName, HeaderValue};
use topcoat::{
    Result,
    asset::{AssetBundle, RouterBuilderAssetExt},
    context::Cx,
    router::{
        Body, BodyLimit, Layer, LayerFuture, Next, Path, Router, RouterBuilderDiscoverExt,
        request::uri, response::response_headers,
    },
};

#[tokio::main]
async fn main() -> Result<()> {
    let router = Router::builder()
        .app_context(rooms::Rooms::new())
        .app_context(api::ConnectionLimits::new())
        .layer(BodyLimit::max(1024))
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
        let headers = response_headers(cx);

        for (name, value) in [
            ("x-content-type-options", "nosniff"),
            ("x-frame-options", "DENY"),
            // YouTube needs the embedding origin, but not the private room path.
            ("referrer-policy", "strict-origin-when-cross-origin"),
        ] {
            headers.append(
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            );
        }

        if !uri(cx).path().starts_with("/_topcoat/") {
            headers.append(
                HeaderName::from_static("cache-control"),
                HeaderValue::from_static("no-store"),
            );
        }

        next.run(cx, body)
    }
}

#[cfg(test)]
mod tests {
    use http::{Request, StatusCode};
    use topcoat::router::{Body, Router, RouterBuilderDiscoverExt, to_bytes};

    use super::*;

    fn api_router() -> Router {
        Router::builder()
            .app_context(rooms::Rooms::new())
            .app_context(api::ConnectionLimits::new())
            .layer(BodyLimit::max(1024))
            .layer(BrowserPolicy)
            .discover()
            .build()
    }

    #[tokio::test]
    async fn create_room_returns_private_credentials_without_caching() {
        let router = api_router();
        let request = Request::builder()
            .method("POST")
            .uri("/api/rooms")
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
            .body(Body::from("x".repeat(2048)))
            .unwrap();

        assert_eq!(
            router.handle(large).await.status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }
}
