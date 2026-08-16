//! Listing the configured personas (ADR-0020).
//!
//! A call is started by id, and the id is derived from the persona's name, so
//! without this a client would have to recompute a server rule to call
//! anything. Unauthenticated, beside `POST /api/session`: the token is free to
//! mint and identifies nobody, so requiring one here would be ceremony.

use std::sync::Arc;

use axum::{Router, extract::State, routing::get};
use character_context::{CharacterCatalogReadPort, CharacterSummary};
use serde::Serialize;

use crate::{error::ApiError, response::ok};

/// A persona on the wire. The prompts and the synthesis voice are the
/// operator's material and stay off it.
#[derive(Debug, Serialize)]
struct PersonaView {
    character_id: i64,
    name: String,
    scene_name: Option<String>,
}

impl From<CharacterSummary> for PersonaView {
    fn from(summary: CharacterSummary) -> Self {
        Self {
            character_id: summary.character_id,
            name: summary.name,
            scene_name: summary.scene_name,
        }
    }
}

pub fn build_personas_router(catalog: Arc<dyn CharacterCatalogReadPort>) -> Router {
    Router::new()
        .route("/api/personas", get(list_personas))
        .with_state(catalog)
}

async fn list_personas(
    State(catalog): State<Arc<dyn CharacterCatalogReadPort>>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    let personas: Vec<PersonaView> = catalog
        .list_characters()
        .await?
        .into_iter()
        .map(PersonaView::from)
        .collect();
    Ok(ok(personas))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use character_context::{CharacterCatalogReadPort, CharacterSummary};
    use serde_json::Value;
    use shared_kernel::{AppError, AppResult};
    use tower::ServiceExt;

    use super::build_personas_router;

    /// A catalog that answers with whatever it was built with.
    struct StubCatalog(AppResult<Vec<CharacterSummary>>);

    #[async_trait]
    impl CharacterCatalogReadPort for StubCatalog {
        async fn list_characters(&self) -> AppResult<Vec<CharacterSummary>> {
            match &self.0 {
                Ok(personas) => Ok(personas.clone()),
                Err(error) => Err(error.clone()),
            }
        }
    }

    fn two_personas() -> Vec<CharacterSummary> {
        vec![
            CharacterSummary {
                character_id: 11,
                name: "companion".to_owned(),
                scene_name: Some("evening-call".to_owned()),
            },
            CharacterSummary {
                character_id: 22,
                name: "another".to_owned(),
                scene_name: None,
            },
        ]
    }

    /// Requests the persona list from a router built over `catalog`.
    async fn list(catalog: StubCatalog, authorization: Option<&str>) -> (StatusCode, Value) {
        let router = build_personas_router(Arc::new(catalog));
        let mut builder = Request::builder().uri("/api/personas").method("GET");
        if let Some(header) = authorization {
            builder = builder.header("authorization", header);
        }
        let request = builder.body(Body::empty()).expect("build request");
        let response = router.oneshot(request).await.expect("route the request");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read the body");
        let payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, payload)
    }

    /// Test case 8 — the configured personas are returned.
    #[tokio::test]
    async fn the_configured_personas_are_listed() {
        let (status, payload) = list(StubCatalog(Ok(two_personas())), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["code"], 0);
        assert_eq!(payload["message"], "ok");
        let personas = payload["data"].as_array().expect("data is a list");
        assert_eq!(personas.len(), 2);
        assert_eq!(personas[0]["character_id"], 11);
        assert_eq!(personas[0]["name"], "companion");
        assert_eq!(personas[0]["scene_name"], "evening-call");
        assert_eq!(personas[1]["character_id"], 22);
        assert_eq!(personas[1]["name"], "another");
        assert_eq!(personas[1]["scene_name"], Value::Null);
    }

    /// Test case 9 — nothing but those three fields reaches a client.
    ///
    /// The prompts and the synthesis voice are the operator's material
    /// (ADR-0020). Asserting only that the expected fields are present would
    /// not notice the rest of the persona being added beside them.
    #[tokio::test]
    async fn a_persona_carries_exactly_three_fields() {
        let (_, payload) = list(StubCatalog(Ok(two_personas())), None).await;
        for persona in payload["data"].as_array().expect("data is a list") {
            let object = persona.as_object().expect("a persona is an object");
            let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert_eq!(keys, ["character_id", "name", "scene_name"]);
        }
    }

    /// Test case 10 — listing is unauthenticated by decision, not by accident.
    #[tokio::test]
    async fn listing_needs_no_token() {
        let (status, _) = list(StubCatalog(Ok(two_personas())), None).await;
        assert_eq!(status, StatusCode::OK);
    }

    /// Test case 11 — no persona configured is a state, not an error.
    #[tokio::test]
    async fn an_empty_catalog_is_an_empty_list() {
        let (status, payload) = list(StubCatalog(Ok(Vec::new())), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload["data"].as_array().expect("data is a list").len(), 0);
    }

    /// Test case 12 — a failure is reported, never returned as an empty list.
    #[tokio::test]
    async fn a_failing_catalog_is_reported_as_a_failure() {
        let (status, payload) = list(
            StubCatalog(Err(AppError::internal("the settings file is unreadable"))),
            None,
        )
        .await;
        assert!(
            !status.is_success(),
            "a failure returned {status}, which reads as no personas configured"
        );
        // And it says what went wrong: a status alone leaves the page with
        // nothing to show but a number.
        assert_eq!(payload["message"], "the settings file is unreadable");
    }
}
