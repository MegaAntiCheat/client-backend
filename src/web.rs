use std::net::SocketAddr;

use axum::{
    routing::{get, post},
    Json, Router,
};

/// Start the web API server
pub async fn web_main() {
    let api = Router::new()
        .route("/", get(root))
        .route("/mac/state", get(state))
        .route("/mac/mark", post(mark));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    log::info!("Starting web server at {addr}");
    axum::Server::bind(&addr)
        .serve(api.into_make_service())
        .await
        .expect("Failed to start web service");
}

async fn root() -> &'static str {
    "Web app is not hosted here yet."
}

/// API endpoint to retrieve the current server state
async fn state() -> Json<()> {
    log::debug!("State requested");
    Json::from(())
}

/// API endpoint to mark a player
async fn mark(Json(mark): Json<()>) {
    log::debug!("Mark player requested");
}
