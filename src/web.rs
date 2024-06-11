use std::{
    collections::HashMap,
    convert::Infallible,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{sse::Event, IntoResponse, Redirect, Sse},
    routing::{get, post, put},
    Json, Router,
};
use chrono::{DateTime, Utc};
use event_loop::{try_get, Handled, Is, MessageHandler};
use futures::Stream;
use include_dir::Dir;
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;
use tappet::SteamAPI;
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    Mutex,
};
use tokio_stream::wrappers::ReceiverStream;

use super::command_manager::Command;
use crate::{
    events::{InternalPreferences, Preferences, UserUpdate, UserUpdates},
    player::{serialize_steamid_as_string, Friend, FriendInfo, Player, Players, SteamInfo},
    server::Gamemode,
    state::MACState,
    steam_api::{request_steam_info, ProfileLookupResult},
};
const HEADERS: [(header::HeaderName, &str); 2] = [
    (header::CONTENT_TYPE, "application/json"),
    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
];

#[derive(Debug)]
#[allow(clippy::module_name_repetitions)]
pub enum WebRequest {
    /// Retrieve info on the active game
    GetGame(UnboundedSender<String>),
    /// Retrieve info on specific accounts
    PostUser(UserPostRequest, UnboundedSender<String>),
    /// Set Verdict and customData for specific accounts
    PutUser(HashMap<SteamID, UserUpdate>),
    /// Retrieve client preferences
    GetPrefs(UnboundedSender<String>),
    /// Set client preferences
    PutPrefs(Preferences),
    /// Retrieve a range of player history
    GetHistory(Pagination, UnboundedSender<String>),
    /// Retrieve the current playerlist
    GetPlayerlist(UnboundedSender<String>),
    /// Tell the client to execute console commands
    PostCommand(RequestedCommands),
}
impl<S> event_loop::Message<S> for WebRequest {}

struct PostUserRequest {
    send: UnboundedSender<String>,
    users: Vec<(SteamID, Option<SteamInfo>)>,
    waiting_users: Vec<SteamID>,
}

#[allow(clippy::module_name_repetitions)]
pub struct WebAPIHandler {
    profile_requests_in_progress: Vec<SteamID>,
    post_user_queue: Vec<PostUserRequest>,
}

impl<IM, OM> MessageHandler<MACState, IM, OM> for WebAPIHandler
where
    IM: Is<WebRequest> + Is<ProfileLookupResult>,
    OM: Is<Command> + Is<Preferences> + Is<UserUpdates> + Is<ProfileLookupResult>,
{
    #[allow(clippy::cognitive_complexity)]
    fn handle_message(
        &mut self,
        state: &MACState,
        message: &IM,
    ) -> Option<event_loop::Handled<OM>> {
        if let Some(lookup_result) = try_get::<ProfileLookupResult>(message) {
            self.handle_profile_lookup(state, lookup_result);
        }

        match try_get::<WebRequest>(message)? {
            WebRequest::GetGame(tx) => {
                if tx.send(get_game_response(state)).is_err() {
                    tracing::error!("Failed to send response to API task.");
                }
            }
            WebRequest::PostUser(users, tx) => {
                return self.handle_post_user_request(state, users, tx.clone());
            }
            WebRequest::PutUser(users) => {
                return Handled::single(OM::from(UserUpdates(users.clone())));
            }
            WebRequest::GetPrefs(tx) => {
                if tx.send(get_prefs_response(state)).is_err() {
                    tracing::error!("Failed to send response to API task.");
                }
            }
            WebRequest::PutPrefs(prefs) => {
                return Handled::single(OM::from(prefs.clone()));
            }
            WebRequest::GetHistory(page, tx) => {
                if tx.send(get_history_response(state, page)).is_err() {
                    tracing::error!("Failed to send response to API task.");
                }
            }
            WebRequest::GetPlayerlist(tx) => {
                if tx.send(get_playerlist_response(state)).is_err() {
                    tracing::error!("Failed to send response to API task.");
                }
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

impl WebAPIHandler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            profile_requests_in_progress: Vec::new(),
            post_user_queue: Vec::new(),
        }
    }

    fn handle_post_user_request<OM: Is<ProfileLookupResult>>(
        &mut self,
        state: &MACState,
        users: &UserPostRequest,
        send: UnboundedSender<String>,
    ) -> Option<Handled<OM>> {
        if state.settings.steam_api_key().is_empty() {
            return None;
        }

        let mut request = PostUserRequest {
            send,
            users: users
                .users
                .iter()
                .map(|s| (*s, state.players.steam_info.get(s).cloned()))
                .collect(),
            waiting_users: Vec::new(),
        };

        // Find just the users we don't have steam info on yet
        request.waiting_users = users
            .users
            .iter()
            .filter(|&s| {
                !self.profile_requests_in_progress.contains(s)
                    || !state.players.steam_info.contains_key(s)
            })
            .copied()
            .collect();

        self.profile_requests_in_progress
            .extend_from_slice(&request.waiting_users);

        // Make steam api requests
        let client = Arc::new(SteamAPI::new(state.settings.steam_api_key()));
        let out = Handled::multiple(request.waiting_users.chunks(100).map(|accounts| {
            let accounts = accounts.to_vec();
            let client = client.clone();
            Handled::future(async move {
                Some(ProfileLookupResult(request_steam_info(&client, &accounts).await).into())
            })
        }));

        let ready = request.waiting_users.is_empty();
        self.post_user_queue.push(request);

        if ready {
            self.send_waiting_post_user_responses(state);
        }

        out
    }

    fn handle_profile_lookup(&mut self, state: &MACState, result: &ProfileLookupResult) {
        let accounts = match &result.0 {
            Err(e) => {
                tracing::error!("Failed to lookup steam profiles: {e:?}");

                // Clear out waiting users and just send what we got,
                // because something has gone wrong.
                self.post_user_queue
                    .iter_mut()
                    .for_each(|req| req.waiting_users.clear());
                self.send_waiting_post_user_responses(state);

                return;
            }
            Ok(a) => a,
        };

        // Add any results that have been fetched to the appropriate requests
        for (id, lookup_result) in accounts {
            self.profile_requests_in_progress.retain(|s| s != id);

            let steam_info = match lookup_result {
                Err(e) => {
                    tracing::error!("Couldn't lookup profile {}: {e:?}", u64::from(*id));
                    continue;
                }
                Ok(a) => a,
            };

            self.post_user_queue.iter_mut().for_each(|req| {
                if let Some((id, info)) = req.users.iter_mut().find(|(s, _)| s == id) {
                    *info = Some(steam_info.clone());
                    req.waiting_users.retain(|s| s != id);
                }
            });
        }

        self.send_waiting_post_user_responses(state);
    }

    fn send_waiting_post_user_responses(&mut self, state: &MACState) {
        self.post_user_queue
            .iter()
            .filter(|req| req.waiting_users.is_empty())
            .for_each(|req| {
                let users: Vec<Player> = req
                    .users
                    .iter()
                    .map(|(id, si)| {
                        let mut player = state.players.get_serializable_player(*id);
                        player.steamInfo = si.as_ref();

                        if let Some(si) = si {
                            player.name = &si.account_name;
                        }

                        player
                    })
                    .collect();

                req.send
                    .send(serde_json::to_string(&users).expect("Epic serialization fail"))
                    .ok();
            });

        self.post_user_queue
            .retain(|req| !req.waiting_users.is_empty());
    }
}

impl Default for WebAPIHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct WebState {
    pub request: UnboundedSender<WebRequest>,
    pub ui: UISource,
}

impl WebState {
    #[must_use]
    pub fn new(ui: &UISource) -> (Self, UnboundedReceiver<WebRequest>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Self {
                request: tx,
                ui: ui.clone(),
            },
            rx,
        )
    }
}

/// Start the web API server
///
/// # Panics
/// If the web server could not be started
#[allow(clippy::module_name_repetitions)]
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
    match axum::Server::try_bind(&addr) {
        Ok(builder) => {
            builder
                .serve(api.into_make_service())
                .await
                .expect("Server wasn't meant to exit.");
        }
        Err(e) => {
            tracing::error!("Failed to start web server: {e}");
            tracing::error!("This may occur if the app is already running.");
            std::process::exit(1);
        }
    }
}

async fn ui_redirect() -> impl IntoResponse {
    Redirect::permanent("/ui/index.html")
}

// UI

async fn get_ui(
    State(state): State<WebState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl IntoResponse {
    state.ui.get_ui(&path).await
}

#[derive(Clone, Debug, Default)]
/// Where to get the web UI files to serve
pub enum UISource {
    Bundled(&'static Dir<'static>),
    /// Load from disk
    Dynamic(PathBuf),
    #[default]
    None,
}

impl UISource {
    pub async fn get_ui(&self, path: &str) -> impl IntoResponse {
        match self {
            Self::Bundled(dir) => Self::get_bundled_ui(dir, path).into_response(),
            Self::Dynamic(dir) => Self::get_dynamic_ui(dir, path).await.into_response(),
            Self::None => {
                (
                    StatusCode::NOT_FOUND,
                    ([(header::CONTENT_TYPE, "text/html")]),
                    "<body><h1>There is no UI bundled with this version of the application.</h1></body>",
                ).into_response()
            }
        }
    }

    fn get_bundled_ui(dir: &'static Dir<'static>, path: &str) -> impl IntoResponse {
        dir.get_file(path).map_or_else(
            || {
                (
                    StatusCode::NOT_FOUND,
                    ([(header::CONTENT_TYPE, "text/html")]),
                    "<body><h1>404 Not Found</h1></body>",
                )
                    .into_response()
            },
            |file| {
                // Serve included file
                let content_type = guess_content_type(file.path());
                let headers = [
                    (header::CONTENT_TYPE, content_type),
                    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                ];
                (StatusCode::OK, headers, file.contents()).into_response()
            },
        )
    }

    async fn get_dynamic_ui(dir: &Path, path: &str) -> impl IntoResponse {
        let file_path = dir.join(path);

        if file_path.is_file() {
            let contents = tokio::fs::read(&file_path).await;

            match contents {
                Ok(contents) => {
                    let content_type = guess_content_type(&file_path);
                    let headers = [
                        (header::CONTENT_TYPE, content_type),
                        (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                    ];
                    (StatusCode::OK, headers, contents).into_response()
                }
                Err(e) => {
                    tracing::error!("Failed to read file {:?}: {e}", &file_path);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        ([(header::CONTENT_TYPE, "text/html")]),
                        "<body><h1>500 Internal Server Error</h1><p>Failed to read file in override web-ui directory</p></body>",
                    )
                        .into_response()
                }
            }
        } else {
            (
                StatusCode::NOT_FOUND,
                ([(header::CONTENT_TYPE, "text/html")]),
                "<body><h1>404 Not Found</h1></body>",
            )
                .into_response()
        }
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
        "js" => "text/javascript",
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
        "css" => "text/css",
        _ => "application/octet-stream",
    }
}

// Game

async fn get_game(State(state): State<WebState>) -> impl IntoResponse {
    tracing::debug!("API: GET game");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    if state.request.send(WebRequest::GetGame(tx)).is_err() {
        tracing::error!("Couldn't send API request to main thread.");
    }
    (rx.recv().await).map_or_else(
        || (StatusCode::SERVICE_UNAVAILABLE, HEADERS, String::new()),
        |resp| (StatusCode::OK, HEADERS, resp),
    )
}

fn get_game_response(state: &MACState) -> String {
    #[derive(Serialize)]
    #[allow(non_snake_case)]
    struct Game<'a> {
        map: Option<&'a str>,
        ip: Option<&'a str>,
        hostname: Option<&'a str>,
        maxPlayers: Option<u32>,
        numPlayers: Option<u32>,
        gamemode: Option<&'a Gamemode>,
        players: &'a Players,
    }

    let game = Game {
        map: state.server.map(),
        ip: state.server.ip(),
        hostname: state.server.hostname(),
        maxPlayers: state.server.max_players(),
        numPlayers: state.server.num_players(),
        gamemode: state.server.gamemode(),
        players: &state.players,
    };

    serde_json::to_string(&game).expect("Epic serialization fail")
}

// User

#[derive(Debug, Clone, Deserialize)]
pub struct UserPostRequest {
    pub users: Vec<SteamID>,
}

async fn post_user(
    State(state): State<WebState>,
    users: Json<UserPostRequest>,
) -> impl IntoResponse {
    tracing::debug!("API: POST user");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    if state
        .request
        .send(WebRequest::PostUser(users.0, tx))
        .is_err()
    {
        tracing::error!("Couldn't send API request to main thread.");
    }
    (rx.recv().await).map_or_else(
        || (StatusCode::SERVICE_UNAVAILABLE, HEADERS, String::new()),
        |resp| (StatusCode::OK, HEADERS, resp),
    )
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
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    if state.request.send(WebRequest::GetPrefs(tx)).is_err() {
        tracing::error!("Couldn't send API request to main thread.");
    }
    (rx.recv().await).map_or_else(
        || (StatusCode::SERVICE_UNAVAILABLE, HEADERS, String::new()),
        |resp| (StatusCode::OK, HEADERS, resp),
    )
}

fn get_prefs_response(state: &MACState) -> String {
    let settings = &state.settings;
    let prefs = Preferences {
        internal: Some(InternalPreferences {
            friends_api_usage: Some(settings.friends_api_usage()),
            tf2_directory: Some(settings.tf2_directory().to_string_lossy().into()),
            rcon_password: Some(settings.rcon_password().to_owned()),
            steam_api_key: Some(settings.steam_api_key().to_owned()),
            masterbase_key: Some(settings.masterbase_key().to_owned()),
            masterbase_host: Some(settings.masterbase_host().to_owned()),
            rcon_port: Some(settings.rcon_port()),
            dumb_autokick: Some(settings.autokick_bots()),
        }),
        external: Some(settings.external_preferences().clone()),
    };

    serde_json::to_string(&prefs).expect("Epic serialization fail")
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
    fn default() -> Self {
        Self { from: 0, to: 100 }
    }
}

async fn get_history(State(state): State<WebState>, page: Query<Pagination>) -> impl IntoResponse {
    tracing::debug!("API: GET history");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    if state
        .request
        .send(WebRequest::GetHistory(page.0, tx))
        .is_err()
    {
        tracing::error!("Couldn't send API request to main thread.");
    }
    (rx.recv().await).map_or_else(
        || (StatusCode::SERVICE_UNAVAILABLE, HEADERS, String::new()),
        |resp| (StatusCode::OK, HEADERS, resp),
    )
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
        .map(|&s| state.players.get_serializable_player(s))
        .collect();

    serde_json::to_string(&history).expect("Epic serialization fail")
}

// Playerlist

async fn get_playerlist(State(state): State<WebState>) -> impl IntoResponse {
    tracing::debug!("API: GET playerlist");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    if state.request.send(WebRequest::GetPlayerlist(tx)).is_err() {
        tracing::error!("Couldn't send API request to main thread.");
    }
    (rx.recv().await).map_or_else(
        || (StatusCode::SERVICE_UNAVAILABLE, HEADERS, String::new()),
        |resp| (StatusCode::OK, HEADERS, resp),
    )
}

// Allowing non-snake-case here because this is the format the UI expects.
#[allow(non_snake_case)]
#[derive(Debug, Serialize)]
struct PlayerRecordResponse<'a> {
    name: &'a str,
    isSelf: bool,
    #[serde(serialize_with = "serialize_steamid_as_string")]
    steamID64: SteamID,
    convicted: Option<bool>,
    localVerdict: Option<String>,
    steamInfo: Option<&'a SteamInfo>,
    customData: &'a serde_json::Value,
    previousNames: Option<&'a [String]>,
    friends: Option<&'a [Friend]>,
    friendsIsPublic: Option<bool>,
    modified: DateTime<Utc>,
    created: DateTime<Utc>,
}

fn get_playerlist_response(state: &MACState) -> String {
    let records = &state.players.records.records;

    let records_mapped: Vec<PlayerRecordResponse> = records
        .iter()
        .map(|(id, record)| {
            let friends = state.players.friend_info.get(id);

            PlayerRecordResponse {
                name: state.players.get_name(*id).unwrap_or(""),
                isSelf: state.settings.steam_user().is_some_and(|user| user == *id),
                steamID64: *id,
                convicted: Some(false),
                localVerdict: Some(record.verdict().to_string()),
                steamInfo: state.players.steam_info.get(id),
                customData: record.custom_data(),
                previousNames: Some(record.previous_names()),
                friends: friends.map(FriendInfo::friends),
                friendsIsPublic: friends.and_then(|f| f.public),
                modified: record.modified(),
                created: record.created(),
            }
        })
        .collect();

    serde_json::to_string(&records_mapped).expect("Epic serialization fail")
}

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
static SUBSCRIBERS: Mutex<Option<Vec<Subscriber>>> = Mutex::const_new(None);

/// Gets a SSE stream to listen for any updates the client can provide.
/// This returns the `rx` channel to the client that hit this endpoint. The corresponding `tx` channel is stored in the SUBSCRIBERS
/// Mutex lock. You may send events to these subscribed clients by calling 'Send' on the `tx` channel.
async fn get_events() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    tracing::debug!("API: Events subcription");

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(16);

    {
        let mut subscribers = SUBSCRIBERS.lock().await;
        if subscribers.is_none() {
            *subscribers = Some(Vec::new());
        }
        // subscribers will never be None here, so calling expect will never panic.
        subscribers.as_mut().expect("Just set it to Some").push(tx);
    }

    Sse::new(ReceiverStream::new(rx))
}

/// Given a serialised JSON string (we do not actually verify the string is json, but it is expected), broadcast to all subscribers.
/// Iterates all `tx` channels in the SUBSCRIBERS Mutex. calling 'send' on each of them an `Axum::response::sse::Event` containing
/// the input `event_json` as the Event data. This is fire and forget, as in, we do not care if the message fails to send for whatever
/// reason. We just attempt best efforts to shove a message down the channel, and ignore any failures. We also prune any closed
/// `tx` channels out of the SUBSCRIBERS Mutex.
///
/// # Panics
/// Will panic if the subscribers Mutex does not actually contain a mutable vector we can broadcast into.
pub async fn broadcast_event(event_json: String) {
    let mut subscribers = SUBSCRIBERS.lock().await;
    if subscribers.is_some() {
        let subs = subscribers.as_mut().expect("Vector to publish to");
        // prune closed tx/rx pairs out of the subscribers list
        subs.retain(|sender| !sender.is_closed());
        // futs stands for Futures, not... hentai women
        let futs = subs
            .iter()
            .map(|sender| sender.send(Ok(Event::default().data(&event_json))));

        // We have created an iterator of Futures that promise to send the message down the channel
        // So we await them all by calling join_all, which does this, but without promising true concurrency.
        futures::future::join_all(futs).await;
        drop(subscribers);
    }
}
