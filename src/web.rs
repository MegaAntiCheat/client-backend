use std::net::SocketAddr;

use axum::{
    extract::Query,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;

use crate::state::State;

/// Start the web API server
pub async fn web_main(port: u16) {
    let api = Router::new()
        .route("/", get(root))
        .route("/mac/game/v1", get(game))
        .route("/mac/mark/v1", post(mark))
        .route("/mac/history/v1", get(get_history));

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    log::info!("Starting web server at {addr}");
    axum::Server::bind(&addr)
        .serve(api.into_make_service())
        .await
        .expect("Failed to start web service");
}

async fn root() -> &'static str {
    "Web app is not hosted here yet."
}

// Game

/// API endpoint to retrieve the current server state
async fn game() -> impl IntoResponse {
    log::debug!("State requested");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        serde_json::to_string(&State::read_state().server).unwrap(),
    )
}

// History

#[derive(Deserialize)]
#[serde(default)]
struct Pagination {
    pub from: usize,
    pub to: usize,
}

impl Default for Pagination {
    fn default() -> Self {
        Pagination { from: 0, to: 100 }
    }
}

async fn get_history(page: Query<Pagination>) -> impl IntoResponse {
    log::debug!("History requested");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
        serde_json::to_string(
            &State::read_state()
                .server
                .get_history(page.0.from..page.0.to),
        )
        .unwrap(),
    )
}

// Mark

/// API endpoint to mark a player
async fn mark(Json(mark): Json<()>) {
    log::debug!("Mark player requested");
}
