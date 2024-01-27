use std::{
    collections::HashMap,
    convert::Infallible,
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
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
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio_stream::wrappers::ReceiverStream;

use crate::{player_records::Verdict, settings::FriendsAPIUsage, state::MACState};

use super::command_manager::Command;

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
impl StateUpdater<MACState> for WebRequest {
    fn update_state(self, state: &mut MACState) {
        todo!()
    }
}

pub struct WebAPIHandler;
impl<IM, OM> HandlerStruct<MACState, IM, OM> for WebAPIHandler
where
    IM: Is<WebRequest>,
{
    fn handle_message(
        &mut self,
        state: &MACState,
        message: &IM,
    ) -> Option<event_loop::Handled<OM>> {
        match try_get::<WebRequest>(message)? {
            WebRequest::GetGame(tx) => {
                tx.blocking_send(serde_json::to_string(&state.server).unwrap())
                    .unwrap();
                Handled::none()
            }
            WebRequest::PostUser(users, tx) => todo!(),
            WebRequest::PutUser(users) => todo!(),
            WebRequest::GetPrefs(tx) => todo!(),
            WebRequest::PutPrefs(prefs) => todo!(),
            WebRequest::GetHistory(page, tx) => todo!(),
            WebRequest::GetPlayerlist(tx) => todo!(),
            WebRequest::PostCommand(cmds) => todo!(),
        }
    }
}

#[derive(Clone)]
pub struct WebState {
    pub request: Sender<WebRequest>,
    pub ui: Option<&'static Dir<'static>>,
}

impl WebState {
    pub fn new(ui: Option<&'static Dir<'static>>) -> (WebState, Receiver<WebRequest>) {
        let (tx, rx) = channel(24);
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

async fn ui_redirect() -> impl IntoResponse {
    Redirect::permanent("/ui/index.html")
}

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
    let (tx, mut rx) = channel(1);
    state.request.send(WebRequest::GetGame(tx)).await.unwrap();
    (StatusCode::OK, HEADERS, rx.recv().await.unwrap())
}

// User

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct UserRequest {
    users: Vec<u64>,
}

async fn post_user(State(state): State<WebState>, users: Json<UserRequest>) -> impl IntoResponse {
    tracing::debug!("API: POST user");
    let resp: String = todo!();
    (StatusCode::OK, HEADERS, resp)
}

#[derive(Debug, Deserialize)]
struct UserUpdate {
    #[serde(rename = "localVerdict")]
    local_verdict: Option<Verdict>,
    #[serde(rename = "customData")]
    custom_data: Option<serde_json::Value>,
}

async fn put_user(
    State(state): State<WebState>,
    users: Json<HashMap<SteamID, UserUpdate>>,
) -> impl IntoResponse {
    tracing::debug!("API: PUT user");
    let resp: String = todo!();
    (StatusCode::OK, HEADERS, resp)
}

// Preferences

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct InternalPreferences {
    pub friends_api_usage: Option<FriendsAPIUsage>,
    pub tf2_directory: Option<Arc<str>>,
    pub rcon_password: Option<Arc<str>>,
    pub steam_api_key: Option<Arc<str>>,
    pub rcon_port: Option<u16>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Preferences {
    pub internal: Option<InternalPreferences>,
    pub external: Option<serde_json::Value>,
}

async fn get_prefs(State(state): State<WebState>) -> impl IntoResponse {
    tracing::debug!("API: GET prefs");
    let resp: String = todo!();
    (StatusCode::OK, HEADERS, resp)
}

async fn put_prefs(State(state): State<WebState>, prefs: Json<Preferences>) -> impl IntoResponse {
    tracing::debug!("API: PUT prefs");
    let resp: String = todo!();
    (StatusCode::OK, HEADERS, resp)
}

// History

#[derive(Deserialize, Debug)]
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

async fn get_history(State(state): State<WebState>, page: Query<Pagination>) -> impl IntoResponse {
    tracing::debug!("API: GET history");
    let resp: String = todo!();
    (StatusCode::OK, HEADERS, resp)
}

// Playerlist

async fn get_playerlist(State(state): State<WebState>) -> impl IntoResponse {
    tracing::debug!("API: GET playerlist");
    let resp: String = todo!();
    (StatusCode::OK, HEADERS, resp)
}

// Commands

#[derive(Deserialize, Debug)]
struct RequestedCommands {
    commands: Vec<Command>,
}

async fn post_commands(
    State(state): State<WebState>,
    commands: Json<RequestedCommands>,
) -> impl IntoResponse {
    tracing::debug!("API: POST commands");
    (StatusCode::OK, HEADERS)
}

// Events

type Subscriber = Sender<Result<Event, Infallible>>;
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
