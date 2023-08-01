use std::{
    collections::HashMap,
    convert::Infallible,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{
    extract::Query,
    http::{header, StatusCode},
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;
use tokio::sync::mpsc::Sender;
use tokio_stream::{wrappers::ReceiverStream, Stream};

use crate::{
    player_records::{PlayerRecord, Verdict},
    state::State,
};

const HEADERS: [(header::HeaderName, &str); 2] = [
    (header::CONTENT_TYPE, "application/json"),
    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
];

/// Start the web API server
pub async fn web_main(port: u16) {
    let api = Router::new()
        .route("/mac/game/v1", get(get_game))
        .route("/mac/user/v1", post(post_user))
        .route("/mac/user/v1", put(put_user))
        .route("/mac/pref/v1", get(get_prefs))
        .route("/mac/pref/v1", put(put_prefs))
        .route("/mac/game/events/v1", get(get_events))
        .route("/mac/history/v1", get(get_history))
        .layer(tower_http::cors::CorsLayer::permissive());

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("Starting web server at {addr}");
    axum::Server::bind(&addr)
        .serve(api.into_make_service())
        .await
        .expect("Failed to start web service");
}

// Game

/// API endpoint to retrieve the current server state
async fn get_game() -> impl IntoResponse {
    tracing::debug!("State requested");
    (
        StatusCode::OK,
        HEADERS,
        serde_json::to_string(&State::read_state().server).expect("Serialize game state"),
    )
}

// User

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct UserRequest {
    users: Vec<u64>,
}

/// Posts a list of SteamIDs to lookup, returns the players.
async fn post_user(users: Json<UserRequest>) -> impl IntoResponse {
    tracing::debug!("Players requested: {:?}", users);
    // TODO
    (StatusCode::OK, HEADERS, "Not implemented".to_string())
}

#[derive(Debug, Deserialize)]
struct UserUpdate {
    #[serde(rename = "localVerdict")]
    local_verdict: Option<Verdict>,
    #[serde(rename = "customData")]
    custom_data: Option<serde_json::Value>,
}

/// Puts a user's details to insert them into the persistent storage for that user.
async fn put_user(users: Json<HashMap<SteamID, UserUpdate>>) -> impl IntoResponse {
    tracing::debug!("Player updates sent: {:?}", &users);

    let mut state = State::write_state();
    for (k, v) in users.0 {
        // Insert record if it didn't exist
        if !state.server.has_player_record(&k) {
            state.server.insert_player_record(PlayerRecord::new(k));
        }

        // Update record
        let mut record = state
            .server
            .get_player_record_mut(&k)
            .expect("Mutating player record that was just inserted.");

        if let Some(custom_data) = v.custom_data {
            record.custom_data = custom_data;
        }

        if let Some(verdict) = v.local_verdict {
            record.verdict = verdict;
        }
    }

    (StatusCode::OK, HEADERS)
}

// Preferences

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InternalPreferences {
    pub tf2_directory: Option<Arc<str>>,
    pub rcon_password: Option<Arc<str>>,
    pub steam_api_key: Option<Arc<str>>,
}

#[derive(Serialize, Deserialize)]
struct Preferences {
    pub internal: Option<InternalPreferences>,
    pub external: Option<serde_json::Value>,
}

/// Get the current preferences
async fn get_prefs() -> impl IntoResponse {
    tracing::debug!("Preferences requested.");

    let state = State::read_state();
    let prefs = Preferences {
        internal: Some(InternalPreferences {
            tf2_directory: Some(state.settings.get_tf2_directory().to_string_lossy().into()),
            rcon_password: Some(state.settings.get_rcon_password()),
            steam_api_key: Some(state.settings.get_steam_api_key()),
        }),
        external: Some(state.settings.get_external_preferences().clone()),
    };

    (
        StatusCode::OK,
        HEADERS,
        serde_json::to_string(&prefs).expect("Serialize preferences"),
    )
}

/// Puts any preferences to be updated
async fn put_prefs(prefs: Json<Preferences>) -> impl IntoResponse {
    tracing::debug!("Preferences updates sent.");

    let mut state = State::write_state();
    let settings = &mut state.settings;

    if let Some(internal) = prefs.0.internal {
        if let Some(tf2_dir) = internal.tf2_directory {
            settings.set_tf2_directory(tf2_dir.to_string().into());
        }
        if let Some(rcon_pwd) = internal.rcon_password {
            settings.set_rcon_password(rcon_pwd);
        }
        if let Some(steam_api_key) = internal.steam_api_key {
            settings.set_steam_api_key(steam_api_key);
        }
    }

    if let Some(external) = prefs.0.external {
        settings.update_external_preferences(external);
    }

    (StatusCode::OK, HEADERS)
}

// Events

pub type Subscriber = Sender<Result<Event, Infallible>>;
pub static SUBSCRIBERS: Mutex<Option<Vec<Subscriber>>> = Mutex::new(None);

/// Gets a SSE stream to listen for any updates the client can provide.
async fn get_events() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    tracing::debug!("Events subcription sent.");

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);

    let mut subscribers = SUBSCRIBERS.lock().unwrap();
    if subscribers.is_none() {
        *subscribers = Some(Vec::new());
    }

    subscribers.as_mut().unwrap().push(tx);

    Sse::new(ReceiverStream::new(rx))
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

/// Gets a historical record of the last (up to) 100 players that the user has
/// been on servers with.
async fn get_history(page: Query<Pagination>) -> impl IntoResponse {
    tracing::debug!("History requested");
    (
        StatusCode::OK,
        HEADERS,
        serde_json::to_string(
            &State::read_state()
                .server
                .get_history(page.0.from..page.0.to),
        )
        .expect("Serialize player history"),
    )
}
