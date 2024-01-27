use std::{
    collections::HashMap,
    convert::Infallible,
    net::SocketAddr,
    path::Path,
    sync::{
        mpsc::{Receiver, Sender},
        Arc, Mutex,
    },
};

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{sse::Event, IntoResponse, Redirect, Sse},
    routing::{get, post, put},
    Json, Router,
};
use event_loop::{try_get, Handled, HandlerStruct, Is, StateUpdater};
use futures::Stream;
use include_dir::Dir;
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;
use tokio_stream::wrappers::ReceiverStream;

use super::command_manager::Command;
use crate::{
    events::{InternalPreferences, Preferences, UserUpdate, UserUpdates},
    player::{Player, Players},
    server::Gamemode,
    state::MACState,
};

const HEADERS: [(header::HeaderName, &str); 2] = [
    (header::CONTENT_TYPE, "application/json"),
    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
];

#[derive(Debug)]
pub enum WebRequest {
    /// Retrieve info on the active game
    GetGame(Sender<String>),
    /// Retrieve info on specific accounts
    PostUser(UserRequest, Sender<String>),
    /// Set Verdict and customData for specific accounts
    PutUser(HashMap<SteamID, UserUpdate>),
    /// Retrieve client preferences
    GetPrefs(Sender<String>),
    /// Set client preferences
    PutPrefs(Preferences),
    /// Retrieve a range of player history
    GetHistory(Pagination, Sender<String>),
    /// Retrieve the current playerlist
    GetPlayerlist(Sender<String>),
    /// Tell the client to execute console commands
    PostCommand(RequestedCommands),
}
impl StateUpdater<MACState> for WebRequest {}

pub struct WebAPIHandler;
impl<IM, OM> HandlerStruct<MACState, IM, OM> for WebAPIHandler
where
    IM: Is<WebRequest>,
    OM: Is<Command> + Is<Preferences> + Is<UserUpdates>,
{
    fn handle_message(
        &mut self,
        state: &MACState,
        message: &IM,
    ) -> Option<event_loop::Handled<OM>> {
        match try_get::<WebRequest>(message)? {
            WebRequest::GetGame(tx) => {
                tx.send(get_game_response(state)).unwrap();
            }
            WebRequest::PostUser(users, tx) => {
                tx.send(post_user_response(state, users)).unwrap();
            }
            WebRequest::PutUser(users) => {
                return Handled::single(OM::from(UserUpdates(users.clone())));
            }
            WebRequest::GetPrefs(tx) => {
                tx.send(get_prefs_response(state)).unwrap();
            }
            WebRequest::PutPrefs(prefs) => {
                return Handled::single(OM::from(prefs.clone()));
            }
            WebRequest::GetHistory(page, tx) => {
                tx.send(get_history_response(state, page)).unwrap();
            }
            WebRequest::GetPlayerlist(tx) => {
                tx.send(get_playerlist_response(state)).unwrap();
            }
            WebRequest::PostCommand(cmds) => {
                return Handled::multiple(
                    cmds.commands.iter().map(|cmd| Handled::single(cmd.clone())),
                );
            }
        }

        Handled::none()
    }
}

#[derive(Clone)]
pub struct WebState {
    pub request: Sender<WebRequest>,
    pub ui: Option<&'static Dir<'static>>,
}

impl WebState {
    pub fn new(ui: Option<&'static Dir<'static>>) -> (WebState, Receiver<WebRequest>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (WebState { request: tx, ui }, rx)
    }
}

/// Start the web API server
pub async fn web_main(web_state: WebState, port: u16) {
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
        .route("/mac/playerlist/v1", get(get_playerlist))
        .route("/mac/commands/v1", post(post_commands))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(web_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("Starting web interface at http://{addr}");
    axum::Server::bind(&addr)
        .serve(api.into_make_service())
        .await
        .expect("Failed to start web service");
}

async fn ui_redirect() -> impl IntoResponse { Redirect::permanent("/ui/index.html") }

// UI

async fn get_ui(
    State(state): State<WebState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    if let Some(ui) = state.ui {
        match ui.get_file(&path) {
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
    } else {
        (
            StatusCode::NOT_FOUND,
            ([(header::CONTENT_TYPE, "text/html")]),
            "<body><h1>There is no UI bundled with this version of the application.</h1></body>",
        )
            .into_response()
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

async fn get_game(State(state): State<WebState>) -> impl IntoResponse {
    tracing::debug!("API: GET game");
    let (tx, rx) = std::sync::mpsc::channel();
    state.request.send(WebRequest::GetGame(tx)).unwrap();
    match rx.recv() {
        Ok(resp) => (StatusCode::OK, HEADERS, resp),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, HEADERS, format!("{}", e)),
    }
}

fn get_game_response(state: &MACState) -> String {
    #[derive(Serialize)]
    #[allow(non_snake_case)]
    struct Game<'a> {
        map: &'a Option<Arc<str>>,
        ip: &'a Option<Arc<str>>,
        hostname: &'a Option<Arc<str>>,
        maxPlayers: &'a Option<u32>,
        numPlayers: &'a Option<u32>,
        gamemode: Option<&'a Gamemode>,
        players: &'a Players,
    }

    let game = Game {
        map: &state.server.map(),
        ip: &state.server.ip(),
        hostname: &state.server.hostname(),
        maxPlayers: &state.server.max_players(),
        numPlayers: &state.server.num_players(),
        gamemode: state.server.gamemode(),
        players: &state.players,
    };

    serde_json::to_string(&game).unwrap()
}

// User

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct UserRequest {
    users: Vec<u64>,
}

async fn post_user(State(state): State<WebState>, users: Json<UserRequest>) -> impl IntoResponse {
    tracing::debug!("API: POST user");
    let (tx, rx) = std::sync::mpsc::channel();
    state
        .request
        .send(WebRequest::PostUser(users.0, tx))
        .unwrap();
    match rx.recv() {
        Ok(resp) => (StatusCode::OK, HEADERS, resp),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, HEADERS, format!("{}", e)),
    }
}

fn post_user_response(_state: &MACState, _users: &UserRequest) -> String {
    "Not yet implemented".into()
}

async fn put_user(
    State(state): State<WebState>,
    users: Json<HashMap<SteamID, UserUpdate>>,
) -> impl IntoResponse {
    tracing::debug!("API: PUT user");
    state.request.send(WebRequest::PutUser(users.0)).ok();
    (StatusCode::OK, HEADERS)
}

// Preferences

async fn get_prefs(State(state): State<WebState>) -> impl IntoResponse {
    tracing::debug!("API: GET prefs");
    let (tx, rx) = std::sync::mpsc::channel();
    state.request.send(WebRequest::GetPrefs(tx)).unwrap();
    match rx.recv() {
        Ok(resp) => (StatusCode::OK, HEADERS, resp),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, HEADERS, format!("{}", e)),
    }
}

fn get_prefs_response(state: &MACState) -> String {
    let settings = &state.settings;
    let prefs = Preferences {
        internal: Some(InternalPreferences {
            friends_api_usage: Some(settings.get_friends_api_usage()),
            tf2_directory: Some(settings.get_tf2_directory().to_string_lossy().into()),
            rcon_password: Some(settings.get_rcon_password()),
            steam_api_key: Some(settings.get_steam_api_key()),
            rcon_port: Some(settings.get_rcon_port()),
        }),
        external: Some(settings.get_external_preferences().clone()),
    };

    serde_json::to_string(&prefs).unwrap()
}

async fn put_prefs(State(state): State<WebState>, prefs: Json<Preferences>) -> impl IntoResponse {
    tracing::debug!("API: PUT prefs");
    state.request.send(WebRequest::PutPrefs(prefs.0)).ok();
    (StatusCode::OK, HEADERS)
}

// History

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct Pagination {
    pub from: usize,
    pub to: usize,
}

impl Default for Pagination {
    fn default() -> Self { Pagination { from: 0, to: 100 } }
}

async fn get_history(State(state): State<WebState>, page: Query<Pagination>) -> impl IntoResponse {
    tracing::debug!("API: GET history");
    let (tx, rx) = std::sync::mpsc::channel();
    state
        .request
        .send(WebRequest::GetHistory(page.0, tx))
        .unwrap();
    match rx.recv() {
        Ok(resp) => (StatusCode::OK, HEADERS, resp),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, HEADERS, format!("{}", e)),
    }
}

fn get_history_response(state: &MACState, page: &Pagination) -> String {
    // let hVecDeque<SteamID> = &server.players().history;
    let history: Vec<Player> = state
        .players
        .history
        .iter()
        .rev()
        .skip(page.from)
        .take(page.to - page.from)
        .flat_map(|s| state.players.get_serializable_player(s))
        .collect();

    serde_json::to_string(&history).unwrap()
}

// Playerlist

async fn get_playerlist(State(state): State<WebState>) -> impl IntoResponse {
    tracing::debug!("API: GET playerlist");
    let (tx, rx) = std::sync::mpsc::channel();
    state.request.send(WebRequest::GetPlayerlist(tx)).unwrap();
    match rx.recv() {
        Ok(resp) => (StatusCode::OK, HEADERS, resp),
        Err(e) => (StatusCode::SERVICE_UNAVAILABLE, HEADERS, format!("{}", e)),
    }
}

fn get_playerlist_response(_state: &MACState) -> String { "Not yet implemented".into() }

// Commands

#[derive(Deserialize, Debug)]
pub struct RequestedCommands {
    commands: Vec<Command>,
}

async fn post_commands(
    State(state): State<WebState>,
    commands: Json<RequestedCommands>,
) -> impl IntoResponse {
    tracing::debug!("API: POST commands");
    state.request.send(WebRequest::PostCommand(commands.0)).ok();
    (StatusCode::OK, HEADERS)
}

// Events

type Subscriber = tokio::sync::mpsc::Sender<Result<Event, Infallible>>;
static SUBSCRIBERS: Mutex<Option<Vec<Subscriber>>> = Mutex::new(None);

/// Gets a SSE stream to listen for any updates the client can provide.
async fn get_events() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    tracing::debug!("API: Events subcription");

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);

    let mut subscribers = SUBSCRIBERS.lock().unwrap();
    if subscribers.is_none() {
        *subscribers = Some(Vec::new());
    }

    subscribers.as_mut().unwrap().push(tx);

    Sse::new(ReceiverStream::new(rx))
}
