use std::{
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use args::Args;
use clap::Parser;
use demo::demo_loop;
use event_loop::{
    define_handlers, define_messages, EventLoop, Handled, HandlerStruct, StateUpdater,
};
use events::emit_on_timer;
use include_dir::{include_dir, Dir};
use launchoptions::LaunchOptions;
use player::Players;
use player_records::PlayerRecords;
use server::Server;
use settings::Settings;
use state::MACState;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    filter::Directive, fmt::writer::MakeWriterExt, layer::SubscriberExt, util::SubscriberInitExt,
    EnvFilter, Layer,
};
use web::{web_main, WebState};

mod args;
mod command_manager;
mod console;
mod demo;
mod events;
mod gamefinder;
mod io;
mod launchoptions;
mod new_players;
mod player;
mod player_records;
mod server;
mod settings;
mod state;
mod steam_api;
mod web;

use command_manager::{Command, CommandManager};
use console::{ConsoleLog, ConsoleOutput, ConsoleParser, RawConsoleOutput};
use events::{Preferences, Refresh, UserUpdates};
use new_players::{ExtractNewPlayers, NewPlayers};
use steam_api::{
    FriendLookupResult, LookupFriends, LookupProfiles, ProfileLookupBatchTick, ProfileLookupResult,
};
use web::{WebAPIHandler, WebRequest};

define_messages!(Message<MACState>:
    Refresh,

    Command,

    RawConsoleOutput,
    ConsoleOutput,

    NewPlayers,

    ProfileLookupBatchTick,
    ProfileLookupResult,
    FriendLookupResult,

    Preferences,
    UserUpdates,

    WebRequest
);

define_handlers!(Handler<MACState, Message>:
    CommandManager,

    ConsoleParser,

    ExtractNewPlayers,

    LookupProfiles,
    LookupFriends,

    WebAPIHandler
);

static UI_DIR: Dir = include_dir!("ui");

fn main() {
    let _guard = init_tracing();

    let args = Args::parse();

    let settings = Settings::load_or_create(&args);
    settings.save_ok();

    let playerlist = PlayerRecords::load_or_create(&args);
    playerlist.save_ok();

    let mut state = MACState {
        server: Server::new(),
        settings,
        players: Players::new(playerlist),
    };

    check_launch_options(&state.settings);

    let web_port = state.settings.get_webui_port();

    // The juicy part of the program
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build async runtime")
        .block_on(async {
            // Autolaunch UI
            if args.autolaunch_ui || state.settings.get_autolaunch_ui() {
                if let Err(e) = open::that(Path::new(&format!("http://localhost:{web_port}"))) {
                    tracing::error!("Failed to open web browser: {:?}", e);
                }
            }

            // Demo manager
            if args.demo_monitoring {
                let demo_path = state.settings.get_tf2_directory().join("tf");
                tracing::info!("Demo path: {:?}", demo_path);

                std::thread::spawn(move || {
                    if let Err(e) = demo_loop(&demo_path) {
                        tracing::error!("Failed to start demo watcher: {:?}", e);
                    }
                });
            }

            // Web API
            let (web_state, web_requests) = WebState::new(Some(&UI_DIR));
            tokio::task::spawn(async move {
                web_main(web_state, web_port).await;
            });

            // Watch console log
            let log_file_path: PathBuf =
                PathBuf::from(state.settings.get_tf2_directory()).join("tf/console.log");
            let console_log = Box::new(ConsoleLog::new(log_file_path).await);

            let lookup_batch_timer =
                emit_on_timer(Duration::from_millis(500), || ProfileLookupBatchTick).await;
            let refresh_timer = emit_on_timer(Duration::from_secs(3), || Refresh).await;

            let mut event_loop: EventLoop<MACState, Message, Handler> = EventLoop::new()
                .add_source(console_log)
                .add_source(refresh_timer)
                .add_source(lookup_batch_timer)
                .add_source(Box::new(web_requests))
                .add_handler(CommandManager::new())
                .add_handler(ConsoleParser::default())
                .add_handler(ExtractNewPlayers)
                .add_handler(LookupProfiles::new())
                .add_handler(LookupFriends)
                .add_handler(WebAPIHandler);

            loop {
                if event_loop.execute_cycle(&mut state).await.is_none() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        });
}

#[allow(clippy::cognitive_complexity)]
fn check_launch_options(settings: &Settings) {
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
        match opts.check_missing_args() {
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
}

fn init_tracing() -> Option<WorkerGuard> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    let suppress_hyper = Directive::from_str("hyper=warn").expect("Bad directive");
    let suppress_demo_parser = Directive::from_str("tf_demo_parser=warn").expect("Bad directive");
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
                                .expect("Bad env")
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
