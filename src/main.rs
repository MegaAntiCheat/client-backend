use crate::player_records::Verdict;
use crate::steamapi::SteamAPIResponse;
use args::Args;
use clap::Parser;
use events::{EventLoop, Handled, HandlerStruct, Is, StateUpdater};
use include_dir::{include_dir, Dir};
use player_records::PlayerRecords;
use server::Server;
use steamapi::SteamAPIManager;
use steamid_ng::SteamID;
use tappet::{Executor, SteamAPI};
use tokio::select;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::task::JoinHandle;
use tracing_subscriber::filter::Directive;

use web::{web_main, SharedState};

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::mpsc::Receiver;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use demo::demo_loop;
use io::{Command, IOManager};
use launchoptions::LaunchOptions;
use settings::Settings;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    fmt::writer::MakeWriterExt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
};

use crate::io::IOManagerMessage;

mod args;
mod demo;
mod events;
mod gamefinder;
mod io;
mod launchoptions;
mod player;
mod player_records;
mod server;
mod settings;
mod steamapi;
mod web;

static UI_DIR: Dir = include_dir!("ui");

define_messages!(Message<State>: Refresh, NewPlayer, ProfileLookup);
define_handlers!(Handler<State, Message>: GetNewPlayers, LookupProfiles);

pub struct State {}

// Handlers ***************

pub struct GetNewPlayers;
impl<M: Is<Refresh> + Is<NewPlayer>> HandlerStruct<State, M> for GetNewPlayers {
    fn handle_message(&mut self, state: &State, message: &M) -> Handled<M> {
        let m: Option<&Refresh> = message.try_get();
        match m {
            Some(_) => Handled::single(NewPlayer {}),
            None => Handled::none(),
        }
    }
}

pub struct LookupProfiles {
    pub api_key: Arc<str>,
}
impl<M: Is<NewPlayer> + Is<ProfileLookup>> HandlerStruct<State, M> for LookupProfiles {
    fn handle_message(&mut self, state: &State, message: &M) -> Handled<M> {
        let m: Option<&NewPlayer> = message.try_get();
        if let Some(_) = m {
            let key = self.api_key.clone();
            Handled::future(async move {
                let _ = SteamAPI::new(key)
                    .get()
                    .ISteamUser()
                    .GetPlayerSummaries(vec![String::from("")])
                    .execute()
                    .await;

                ProfileLookup {}.into()
            })
        } else {
            Handled::none()
        }
    }
}

// Messages **************

pub struct Refresh;
impl StateUpdater<State> for Refresh {
    fn update_state(&self, state: &mut State) {
        println!("Refreshing");
    }
}

pub struct NewPlayer {}
impl StateUpdater<State> for NewPlayer {}

pub struct ProfileLookup {}
impl StateUpdater<State> for ProfileLookup {
    fn update_state(&self, state: &mut State) {
        println!("Got profile result!");
    }
}

// Main **********************

fn main() {
    let (tx, rx) = std::sync::mpsc::channel();

    let mut event_loop: EventLoop<State, Message, Handler> = EventLoop::new()
        .add_handler(GetNewPlayers {})
        .add_handler(LookupProfiles {
            api_key: "boop".into(),
        })
        .add_source(Box::new(rx));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            // Refresh loop
            tokio::task::spawn(async move {
                let mut refresh_interval = tokio::time::interval(Duration::from_secs(3));
                refresh_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    refresh_interval.tick().await;
                    tx.send(Refresh {}).unwrap();
                }
            });

            // Run event loop
            let mut state = State {};

            loop {
                event_loop.execute_cycle(&mut state).await;
            }
        });

    let _guard = init_tracing();

    // Arg handling
    let args = Args::parse();

    // Load settings
    let settings = Settings::load_or_create(&args);
    settings.save_ok();

    // Launch options and overrides
    let launch_opts = match LaunchOptions::new(
        settings
            .get_steam_user()
            .expect("Failed to identify the local steam user (failed to find `loginusers.vdf`)"),
    ) {
        Ok(val) => Some(val),
        Err(why) => {
            tracing::warn!("Couldn't verify app launch options: {:?}", why);
            None
        }
    };

    if let Some(opts) = launch_opts {
        // Warn about missing launch options for TF2
        let missing = opts.check_missing_args();
        match missing {
            Ok(missing_opts) if !missing_opts.is_empty() => {
                tracing::warn!(
                    "Please add the following launch options to your TF2 to allow the MAC client to interface correctly with TF2."
                );
                tracing::warn!("Missing launch options: \"{}\"", missing_opts.join(" "));
            }

            Ok(_) => {
                tracing::info!("All required launch arguments are present!");
            }

            Err(missing_opts_err) => {
                tracing::error!(
                    "Failed to verify app launch options: {:?} (App may continue to function normally)",
                    missing_opts_err
                );
            }
        }
    }

    let webui_port = settings.get_webui_port();
    let rcon_port = settings.get_rcon_port();
    let playerlist = PlayerRecords::load_or_create(&args);
    playerlist.save_ok();

    // Start the async part of the program
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            // Initialize State
            let log_file_path: PathBuf =
                PathBuf::from(settings.get_tf2_directory()).join("tf/console.log");

            // IO Manager
            let (io_send, io_recv) = unbounded_channel();
            let (mut io_recv, mut io_manager) =
                IOManager::new(log_file_path, settings.get_rcon_password(), rcon_port, io_recv);

            tokio::task::spawn(async move {
                io_manager.io_loop().await;
            });

            // Autolaunch UI
            if args.autolaunch_ui || settings.get_autolaunch_ui() {
                if let Err(e) = open::that(Path::new(&format!("http://localhost:{}", webui_port))) {
                    tracing::error!("Failed to open web browser: {:?}", e);
                }
            }

            // Demo manager
            if args.demo_monitoring {
                let demo_path = settings.get_tf2_directory().join("tf");
                tracing::info!("Demo path: {:?}", demo_path);

                std::thread::spawn(move || {
                    if let Err(e) = demo_loop(demo_path) {
                        tracing::error!("Failed to start demo watcher: {:?}", e);
                    }
                });
            }

            // Steam API
            let mut server = Server::new(playerlist);
            server.players_mut().user = settings.get_steam_user();
            let (steam_api_send, steam_api_recv) = unbounded_channel();
            let (mut steam_api_recv, mut steam_api) =
                SteamAPIManager::new(settings.get_steam_api_key(), steam_api_recv);
            tokio::task::spawn(async move {
                steam_api.api_loop().await;
            });


            // Setup web API server
            let settings = Arc::new(RwLock::new(settings));
            let server = Arc::new(RwLock::new(server));

            let shared_state = SharedState {
                ui: Some(&UI_DIR),
                io: io_send.clone(),
                api: steam_api_send.clone(),
                server: server.clone(),
                settings: settings.clone(),
            };
            tokio::task::spawn(async move {
                web_main(shared_state, webui_port).await;
            });

            // Main loop

            let mut refresh_interval = tokio::time::interval(Duration::from_secs(3));
            refresh_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut refresh_iteration: u64 = 0;

            let mut new_players = Vec::new();
            let mut queued_friendlist_req: Vec<SteamID> = Vec::new();
            let mut inprogress_friendlist_req: Vec<SteamID> = Vec::new();
            let mut need_all_friends_lists = false;

            loop {

                select! {
                    // IO output
                    io_output_iter = io_recv.recv() => {
                        for output in io_output_iter.unwrap() {
                            for new_player in server.write().unwrap()
                                .handle_io_output(output)
                                .into_iter()
                            {
                                new_players.push(new_player);
                            }
                        }
                    },

                    // Steam API responses
                    Some(response) = steam_api_recv.recv() => {
                        match response {
                            SteamAPIResponse::SteamInfo((player, Ok(info))) => {
                                server.write().unwrap().players_mut().steam_info.insert(player, info);
                            },
                            SteamAPIResponse::SteamInfo((player, Err(e))) => {
                                tracing::error!("Failed to get steam info for {}: {e}", u64::from(player));
                            },
                            SteamAPIResponse::FriendLists((steamid, result)) => {
                                match result {
                                    // Player has public friend list
                                    Ok(friend_list) => {
                                        server.write().unwrap().players_mut().update_friends_list(steamid, friend_list);
                                    },
                                    // Player has private friend list
                                    Err(_) => {
                                        let mut server = server.write().unwrap();
                                        server.players_mut().mark_friends_list_private(&steamid);
                                        if let Some(record) = server.players_mut().records.get(&steamid) {
                                            if  record.verdict == Verdict::Cheater ||
                                                record.verdict == Verdict::Bot {
                                                need_all_friends_lists = true;
                                            }
                                        }
                                    }
                                };
                                let i = inprogress_friendlist_req.iter().position(|id| *id == steamid);
                                if i.is_some() {
                                    inprogress_friendlist_req.remove(i.unwrap());
                                }
                            }
                        }
                    }

                    // Refresh
                    _ = refresh_interval.tick() => {
                        if refresh_iteration % 2 == 0 {
                            server.write().unwrap().players_mut().refresh();
                            io_send.send(IOManagerMessage::RunCommand(Command::Status)).unwrap();
                        } else {
                            io_send.send(IOManagerMessage::RunCommand(Command::G15)).unwrap();
                        }

                        refresh_iteration += 1;
                    }
                }

                // Request steam API stuff on new players
                for player in &new_players {
                    let verdict = server.read().unwrap()
                        .players()
                        .records.get(player)
                        .map(|r| {
                            r.verdict
                        }).unwrap_or(Verdict::Player);
                    steam_api_send
                        .send(steamapi::SteamAPIMessage::Lookup(*player))
                        .unwrap();
                    let settings_read = settings.read().unwrap();
                    let user = settings_read.get_steam_user();
                    if user.is_some_and(|u| u == *player) {
                        queued_friendlist_req.push(*player);
                        continue;
                    }
                    match settings_read.get_friends_api_usage() {
                        settings::FriendsAPIUsage::All => {
                            queued_friendlist_req.push(*player);
                        },
                        settings::FriendsAPIUsage::CheatersOnly => {
                            if !need_all_friends_lists && (verdict == Verdict::Cheater ||  verdict == Verdict::Bot) {
                                queued_friendlist_req.push(*player);
                            }
                        },
                        settings::FriendsAPIUsage::None => {

                        }
                    }
                }

                // Request friend lists of relevant players (depends on config)
                if need_all_friends_lists || !queued_friendlist_req.is_empty() {
                    // If a cheater's friends list is private, we need everyone's friends list.
                    if need_all_friends_lists {
                        need_all_friends_lists = false;
                        let server_read: std::sync::RwLockReadGuard<'_, Server> = server.read().unwrap();
                        queued_friendlist_req = server_read.players().connected.iter()
                            .filter_map(|steamid| {
                                if inprogress_friendlist_req.contains(steamid) {
                                    return None;
                                }
                                // If friends list visibility is Some, we've looked up that user before.
                                match server_read.players().friend_info.get(steamid).map(|fi| fi.public) {
                                    Some(Some(true)) => {
                                        None
                                    }
                                    Some(Some(false)) => {
                                        let record = server_read.players().records.get(steamid);
                                        if record.is_some_and(|r | {
                                            r.verdict == Verdict::Cheater ||
                                            r.verdict == Verdict::Bot
                                         }) {
                                            need_all_friends_lists = true;
                                        }
                                        None
                                    }
                                    _ => {
                                        Some(*steamid)
                                    }
                                }
                            }).collect();
                    }

                    steam_api_send
                        .send(steamapi::SteamAPIMessage::CheckFriends(queued_friendlist_req.clone()))
                        .unwrap();
                    inprogress_friendlist_req.append(&mut queued_friendlist_req);
                }

                new_players.clear();
                queued_friendlist_req.clear();
            }
        });
}

fn init_tracing() -> Option<WorkerGuard> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    let suppress_hyper = Directive::from_str("hyper=warn").unwrap();
    let suppress_demo_parser = Directive::from_str("tf_demo_parser=warn").unwrap();
    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(
                EnvFilter::from_default_env()
                    .add_directive(suppress_hyper.clone())
                    .add_directive(suppress_demo_parser.clone()),
            ),
    );

    match std::fs::File::create("./macclient.log") {
        Ok(latest_log) => {
            let (file_writer, guard) = tracing_appender::non_blocking(latest_log);
            subscriber
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(file_writer.with_max_level(tracing::Level::TRACE))
                        .with_filter(
                            EnvFilter::builder()
                                .parse("debug")
                                .unwrap()
                                .add_directive(suppress_hyper)
                                .add_directive(suppress_demo_parser),
                        ),
                )
                .init();
            Some(guard)
        }
        Err(e) => {
            subscriber.init();
            tracing::error!(
                "Failed to create log file, continuing without persistent logs: {}",
                e
            );
            None
        }
    }
}
