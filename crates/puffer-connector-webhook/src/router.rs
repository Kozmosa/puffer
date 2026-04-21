use crate::handler::{handle_command, CommandOutcome};
use crate::WebhookConfig;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use puffer_connector_core::ConnectorRuntime;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Shared axum state: the runtime plus a snapshot of the config.
#[derive(Clone)]
pub struct AppState {
    pub runtime: Arc<ConnectorRuntime>,
    pub config: Arc<WebhookConfig>,
}

/// JSON payload accepted by the webhook endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookRequest {
    pub conversation_id: String,
    pub message: String,
}

/// JSON payload returned on a successful dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookResponse {
    /// The bound session id when the agent was consulted. `None` for
    /// local replies like `/help`.
    pub session_id: Option<String>,
    /// The text to display to the user.
    pub reply: String,
}

/// Builds the axum router used by both the live connector and the
/// integration tests.
pub fn build_router(state: AppState) -> Router {
    let config_path = state.config.resolved_path();
    Router::new()
        .route("/health", get(health))
        .route(&config_path, post(webhook))
        .with_state(state)
}

async fn health() -> Response {
    (StatusCode::OK, "puffer").into_response()
}

async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<WebhookRequest>,
) -> Response {
    let origin = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok());
    if !state.config.is_origin_allowed(origin) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }

    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let runtime = state.runtime.clone();
    let config = state.config.clone();
    let payload_cid = payload.conversation_id.clone();
    let payload_msg = payload.message.clone();

    // Dispatch is blocking (it acquires the runtime mutex and may run a
    // whole agent turn); run it on a blocking worker so the reactor
    // stays responsive.
    let result = tokio::task::spawn_blocking(move || {
        handle_command(
            &runtime,
            &payload_cid,
            auth_header.as_deref(),
            &payload_msg,
            config.as_ref(),
        )
    })
    .await;

    let outcome = match result {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Puffer error: {error}"),
            )
                .into_response();
        }
        Err(join_error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Puffer worker panicked: {join_error}"),
            )
                .into_response();
        }
    };

    match outcome {
        CommandOutcome::Ignored => {
            // Either the auth check failed or the body was empty. Both
            // are client errors from the webhook's perspective.
            let had_auth = state.config.auth_token.is_some();
            let status = if had_auth {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, "request rejected").into_response()
        }
        CommandOutcome::Reply(text) => (
            StatusCode::OK,
            Json(WebhookResponse {
                session_id: None,
                reply: text,
            }),
        )
            .into_response(),
        CommandOutcome::AgentReply {
            session_id, text, ..
        } => (
            StatusCode::OK,
            Json(WebhookResponse {
                session_id: Some(session_id.to_string()),
                reply: text,
            }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt as _;
    use puffer_config::{ConfigPaths, PufferConfig};
    use puffer_connector_core::{
        ConnectorRuntime, ConnectorRuntimeConfig, ConversationSessionMap,
    };
    use puffer_provider_registry::{AuthStore, ProviderRegistry};
    use puffer_resources::LoadedResources;
    use puffer_session_store::SessionStore;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tower::ServiceExt; // for `oneshot`

    fn test_runtime(root: std::path::PathBuf) -> Arc<ConnectorRuntime> {
        let paths = ConfigPaths {
            workspace_root: root.clone(),
            workspace_config_dir: root.join(".puffer"),
            user_config_dir: root.join(".puffer-user"),
            builtin_resources_dir: root.join("resources"),
        };
        std::fs::create_dir_all(&paths.user_config_dir).unwrap();
        Arc::new(ConnectorRuntime::new(ConnectorRuntimeConfig {
            config: PufferConfig::default(),
            resources: LoadedResources::default(),
            providers: ProviderRegistry::default(),
            auth_store: AuthStore::default(),
            auth_path: root.join("auth.json"),
            session_store: SessionStore::from_paths(&paths).unwrap(),
            session_map: ConversationSessionMap::in_memory(),
            default_cwd: root,
        }))
    }

    fn base_config() -> WebhookConfig {
        WebhookConfig {
            bind_address: "127.0.0.1:0".to_string(),
            path: None,
            auth_token: None,
            allowed_origins: Vec::new(),
            welcome_message: None,
        }
    }

    #[tokio::test]
    async fn health_endpoint_returns_200_ok() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let router = build_router(AppState {
            runtime,
            config: Arc::new(base_config()),
        });
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"puffer");
    }

    #[tokio::test]
    async fn webhook_help_returns_local_reply_over_http() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let router = build_router(AppState {
            runtime,
            config: Arc::new(base_config()),
        });
        let body = serde_json::to_vec(&serde_json::json!({
            "conversation_id": "conv-xyz",
            "message": "/help"
        }))
        .unwrap();
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/puffer")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        let parsed: WebhookResponse = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.reply.contains("/help"));
        assert!(parsed.session_id.is_none());
    }

    #[tokio::test]
    async fn webhook_rejects_missing_auth_when_token_set() {
        let runtime = test_runtime(tempdir().unwrap().path().to_path_buf());
        let mut config = base_config();
        config.auth_token = Some("sekret".to_string());
        let router = build_router(AppState {
            runtime,
            config: Arc::new(config),
        });
        let body = serde_json::to_vec(&serde_json::json!({
            "conversation_id": "conv-xyz",
            "message": "/help"
        }))
        .unwrap();
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/puffer")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
