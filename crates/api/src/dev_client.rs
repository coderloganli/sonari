//! The browser test client (ADR-0018).
//!
//! A page for trying a call by hand. It is served by the binary itself, so it
//! is same-origin with the API and needs no CORS layer, and its assets are
//! compiled in, so there is no directory to mount and nothing to be missing at
//! runtime.
//!
//! Two routes, both literal. Nothing else under `/dev` exists, so there is no
//! path to walk.

use axum::{
    Router,
    http::header::{CONTENT_TYPE, HeaderValue},
    response::IntoResponse,
    routing::get,
};

const INDEX_HTML: &str = include_str!("../assets/dev-client/index.html");
const LIVEKIT_SDK: &str = include_str!("../assets/dev-client/livekit-client.umd.min.js");

pub fn build_dev_client_router() -> Router {
    Router::new()
        .route("/dev", get(index))
        .route("/dev/livekit-client.umd.min.js", get(livekit_sdk))
}

async fn index() -> impl IntoResponse {
    served_as(INDEX_HTML, "text/html; charset=utf-8")
}

async fn livekit_sdk() -> impl IntoResponse {
    served_as(LIVEKIT_SDK, "text/javascript; charset=utf-8")
}

fn served_as(body: &'static str, content_type: &'static str) -> impl IntoResponse {
    (
        [(CONTENT_TYPE, HeaderValue::from_static(content_type))],
        body,
    )
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::build_dev_client_router;

    /// Requests a path from the dev-client router and returns status,
    /// content-type and body.
    async fn get(path: &str) -> (StatusCode, String, String) {
        let router = build_dev_client_router();
        let request = Request::builder()
            .uri(path)
            .method("GET")
            .body(Body::empty())
            .expect("build request");
        let response = router.oneshot(request).await.expect("route the request");
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read the body");
        (
            status,
            content_type,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    /// Test case 1 — the page is served.
    #[tokio::test]
    async fn the_page_is_served_as_html() {
        let (status, content_type, body) = get("/dev").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            content_type.starts_with("text/html"),
            "content-type was {content_type}"
        );
        assert!(!body.is_empty());
    }

    /// Test case 2 — the page and the route serving its SDK cannot drift apart.
    #[tokio::test]
    async fn the_page_loads_the_sdk_from_the_route_that_serves_it() {
        let (_, _, body) = get("/dev").await;
        assert!(
            body.contains("/dev/livekit-client.umd.min.js"),
            "the page must load the SDK from the path the router serves"
        );
    }

    /// Test case 3 — the vendored SDK is present and exposes the global the
    /// page calls.
    #[tokio::test]
    async fn the_vendored_sdk_is_served_and_exposes_its_global() {
        let (status, content_type, body) = get("/dev/livekit-client.umd.min.js").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            content_type.contains("javascript"),
            "content-type was {content_type}"
        );
        assert!(
            body.contains("LivekitClient"),
            "the UMD build must expose the LivekitClient global the page uses"
        );
    }

    /// Test case 4 — every request the page makes is same-origin.
    ///
    /// Same-origin is the reason no CORS layer exists anywhere in this API. A
    /// weaker check for the absence of `http://` would pass an `https://`, a
    /// protocol-relative `//host/...`, or a host held in a variable.
    #[tokio::test]
    async fn every_request_the_page_makes_is_a_relative_path() {
        let (_, _, body) = get("/dev").await;
        let targets = fetch_targets(&body);
        assert!(
            !targets.is_empty(),
            "the page must call the API; no fetch target was found"
        );
        for target in &targets {
            assert!(
                target.starts_with("/api/") || target.starts_with("/dev/"),
                "fetch target {target:?} is not a same-origin path"
            );
        }
    }

    /// Test case 5 — only the two declared files exist under `/dev`.
    #[tokio::test]
    async fn nothing_else_under_dev_is_served() {
        let (status, _, _) = get("/dev/anything-else.js").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// Test case 6 — the page names itself a test client (ADR-0018).
    #[tokio::test]
    async fn the_page_title_names_it_a_test_client() {
        let (_, _, body) = get("/dev").await;
        let title = between(&body, "<title>", "</title>")
            .expect("the page must have a title")
            .to_lowercase();
        assert!(
            title.contains("test client"),
            "the title was {title:?}; it must not read as a product surface"
        );
    }

    /// Test case 7 — the room token is the one nested under `realtime`.
    ///
    /// Two different tokens in this API are called `access_token`. Using the
    /// session token here produces a page that authenticates correctly and then
    /// cannot join a room. Asserting the absence of `sessionToken` is not
    /// enough: `null`, or any third variable, would pass that. So this follows
    /// the value — the name bound to `realtime.access_token` must be the name
    /// `connect` is handed.
    #[tokio::test]
    async fn the_room_is_joined_with_the_realtime_token() {
        let (_, _, body) = get("/dev").await;
        let binding = body
            .lines()
            .find(|line| line.contains("realtime.access_token"))
            .expect("the page must read the LiveKit token from realtime.access_token");
        let token_variable = binding
            .split('=')
            .next()
            .and_then(|left| left.split_whitespace().last())
            .expect("the token must be bound to a name");

        let arguments = between(&body, ".connect(", ")").expect("the page must connect to a room");
        let second = arguments
            .split(',')
            .nth(1)
            .map(str::trim)
            .expect("connect takes a url and a token");
        assert_eq!(
            second, token_variable,
            "connect was handed {second:?}, not the realtime token bound as {token_variable:?}"
        );
    }

    /// The argument of every `fetch(` call in the page, unquoted, with any
    /// `${...}` interpolation left as written.
    fn fetch_targets(page: &str) -> Vec<String> {
        let mut targets = Vec::new();
        for fragment in page.split("fetch(").skip(1) {
            let mut characters = fragment.chars();
            let Some(quote) = characters.next() else {
                continue;
            };
            if quote != '"' && quote != '\'' && quote != '`' {
                // A target held in a variable is not a literal path, and this
                // page is not allowed one.
                targets.push(fragment.chars().take(40).collect());
                continue;
            }
            targets.push(
                characters
                    .take_while(|character| *character != quote)
                    .collect(),
            );
        }
        targets
    }

    /// The text between the first `open` and the next `close` after it.
    fn between<'a>(haystack: &'a str, open: &str, close: &str) -> Option<&'a str> {
        let start = haystack.find(open)? + open.len();
        let rest = &haystack[start..];
        let end = rest.find(close)?;
        Some(&rest[..end])
    }
}
