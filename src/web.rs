use std::{
    collections::HashMap,
    convert::Infallible,
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};

use axum::{
    extract::Query,
    http::{header, StatusCode},
    response::{sse::Event, IntoResponse, Redirect, Sse},
    routing::{get, post, put},
    Json, Router,
};
use include_dir::{include_dir, Dir};
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;
use tokio::sync::mpsc::{Sender, UnboundedSender};
use tokio_stream::{wrappers::ReceiverStream, Stream};

use crate::{
    io::Commands,
    player_records::{PlayerRecord, Verdict},
    state::State,
};

const HEADERS: [(header::HeaderName, &str); 2] = [
    (header::CONTENT_TYPE, "application/json"),
    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
];

static UI_DIR: Dir = include_dir!("ui");

/// Start the web API server
pub async fn web_main(port: u16) {
    // Check everything is initialized
    if !State::is_initialized() {
        panic!("State is not initialized.");
    }

    if COMMAND_ISSUER
        .lock()
        .expect("Command issuer poisoned.")
        .is_none()
    {
        panic!("Command issuer channel not initialized.");
    }

    let api = Router::new()
        .route("/", get(ui_redirect))
        .route("/ui", get(ui_redirect))
        .route("/ui/*ui", get(get_ui))
        .route("/mac/game/v1", get(get_game))
        .route("/mac/user/v1", post(post_user))
        .route("/mac/user/v1", put(put_user))
        .route("/mac/pref/v1", get(get_prefs))
        .route("/mac/pref/v1", put(put_prefs))
        .route("/mac/game/events/v1", get(get_events))
        .route("/mac/history/v1", get(get_history))
        .route("/mac/commands/v1", post(post_commands))
        .layer(tower_http::cors::CorsLayer::permissive());

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("Starting web interface at http://{addr}");
    axum::Server::bind(&addr)
        .serve(api.into_make_service())
        .await
        .expect("Failed to start web service");
}

async fn ui_redirect() -> impl IntoResponse {
    Redirect::permanent("/ui/index.html")
}

// UI

async fn get_ui(axum::extract::Path(path): axum::extract::Path<String>) -> impl IntoResponse {
    match UI_DIR.get_file(&path) {
        Some(file) => {
            // Serve included file
            let content_type = guess_content_type(file.path());
            let headers = [
                (header::CONTENT_TYPE, content_type),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
            ];
            (StatusCode::OK, headers, file.contents()).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            ([(header::CONTENT_TYPE, "text/html")]),
            "<body><h1>404 Not Found</h1></body>",
        )
            .into_response(),
    }
}

/// Attempts to guess the http MIME type of a given file extension.
/// Defaults to "application/octet-stream" if it is not recognised.
fn guess_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|osstr| osstr.to_str())
        .unwrap_or("bin")
    {
        "htm" | "html" => "text/html",
        "jpg" | "jpeg" => "image/jpeg",
        "js" => "test/javascript",
        "json" => "application/json",
        "png" => "image/png",
        "weba" => "audio/weba",
        "webm" => "video/webm",
        "webp" => "image/webp",
        "txt" => "text/plain",
        "mp3" => "audio/mp3",
        "mp4" => "video/mp4",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
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
        if !state.server.has_player_record(k) {
            state.server.insert_player_record(PlayerRecord::new(k));
        }

        // Update record
        let mut record = state
            .server
            .get_player_record_mut(k)
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

// Commands

static COMMAND_ISSUER: Mutex<Option<UnboundedSender<Commands>>> = Mutex::new(None);

pub async fn init_command_issuer(cmd_issuer: UnboundedSender<Commands>) {
    *COMMAND_ISSUER.lock().expect("Command issuer mutex") = Some(cmd_issuer);
}

#[derive(Deserialize, Debug)]
struct RequestedCommands {
    commands: Vec<Commands>,
}

async fn post_commands(commands: Json<RequestedCommands>) -> impl IntoResponse {
    tracing::debug!("Commands sent: {:?}", commands);

    let command_issuer = COMMAND_ISSUER.lock().expect("Command issuer mutex");
    let cmd = command_issuer
        .as_ref()
        .expect("Command issuer should be initialised");

    for command in commands.0.commands {
        cmd.send(command).expect("Sending command from web API.");
    }

    (StatusCode::OK, HEADERS)
}
