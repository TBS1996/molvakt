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
use crate::reminders;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub bot: Bot,
}

pub async fn run() -> anyhow::Result<()> {
    let db = Db::connect().await?;
    let bot = Bot::new(db.clone()).await?;
    let whatsapp = bot.whatsapp().clone();
    let reminder_db = db.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = reminders::run_tick(&reminder_db, &whatsapp).await {
                eprintln!("morning reminder tick: {error:?}");
            }
        }
    });

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

async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
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
