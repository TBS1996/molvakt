use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;

use crate::bot::Bot;
use crate::db::Db;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub bot: Bot,
}

pub async fn run() -> anyhow::Result<()> {
    let db = Db::connect().await?;
    db.get_or_create_default_conversation().await?;
    let bot = Bot::new(db.clone()).await?;
    let state = AppState { db, bot };

    let app = Router::new()
        .route("/health", get(health))
        .route("/webhook/whatsapp", get(whatsapp_verify).post(whatsapp_receive))
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("molvakt server listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    match state.db.get_or_create_default_conversation().await {
        Ok(conversation) => (
            StatusCode::OK,
            Json(json!({
                "status": "ok",
                "target_language": conversation.target_language,
                "source_language": conversation.source_language,
            })),
        ),
        Err(error) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "status": "error",
                "message": error.to_string(),
            })),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct WhatsAppVerifyQuery {
    #[serde(rename = "hub.mode")]
    mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    challenge: Option<String>,
}

async fn whatsapp_verify(Query(query): Query<WhatsAppVerifyQuery>) -> impl IntoResponse {
    let expected_token = std::env::var("WHATSAPP_VERIFY_TOKEN").ok();

    let verified = query.mode.as_deref() == Some("subscribe")
        && expected_token
            .zip(query.verify_token)
            .is_some_and(|(expected, actual)| expected == actual);

    if verified {
        if let Some(challenge) = query.challenge {
            return (StatusCode::OK, challenge).into_response();
        }
    }

    StatusCode::FORBIDDEN.into_response()
}

async fn whatsapp_receive(State(state): State<AppState>, body: String) -> impl IntoResponse {
    println!("whatsapp webhook: received {} bytes", body.len());
    let bot = state.bot.clone();
    tokio::spawn(async move {
        if let Err(error) = bot.handle_webhook(&body).await {
            eprintln!("whatsapp webhook error: {error:?}");
        }
    });
    StatusCode::OK
}
