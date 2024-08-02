use std::{
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use args::Args;
use clap::Parser;
use event_loop::{define_events, EventLoop};
use events::emit_on_timer;
use launchoptions::LaunchOptions;
use player::Players;
use player_records::PlayerRecords;
use reqwest::StatusCode;
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
mod masterbase;
mod new_players;
mod parties;
mod player;
mod player_records;
mod server;
mod settings;
mod sse_events;
mod state;
mod steam_api;
mod web;

use command_manager::{Command, CommandManager, DumbAutoKick};
use console::{ConsoleLog, ConsoleOutput, ConsoleParser, RawConsoleOutput};
use demo::{DemoBytes, DemoManager, DemoMessage, DemoWatcher, PrintVotes};
use events::{Preferences, Refresh, UserUpdates};
use new_players::{ExtractNewPlayers, NewPlayers};
use sse_events::SseEventBroadcaster;
use steam_api::{
    FriendLookupResult, LookupFriends, LookupProfiles, ProfileLookupBatchTick,
    ProfileLookupRequest, ProfileLookupResult,
};
use web::{WebAPIHandler, WebRequest};

define_events!(
    MACState,
    Message {
        Refresh,

        Command,

        RawConsoleOutput,
        ConsoleOutput,

        NewPlayers,

        ProfileLookupBatchTick,
        ProfileLookupResult,
        FriendLookupResult,
        ProfileLookupRequest,

        Preferences,
        UserUpdates,

        WebRequest,

        DemoBytes,
        DemoMessage,
    },
    Handler {
        CommandManager,
        ConsoleParser,
        ExtractNewPlayers,

        LookupProfiles,
        LookupFriends,

        WebAPIHandler,
        SseEventBroadcaster,

        DemoManager,
        PrintVotes,
        DumbAutoKick,
    },
);

#[cfg(target_os = "windows")]
fn enable_ansi() {
    let ansi_res = ansi_term::enable_ansi_support();
    if let Err(code) = ansi_res {
        tracing::warn!("Failed to enable ANSI support. Error code: {:?}", code);
    }
}

#[allow(clippy::too_many_lines)]
fn main() {
    let _guard = init_tracing();

    #[cfg(target_os = "windows")]
    enable_ansi();

    let args = Args::parse();

    let settings = Settings::load_or_create(&args);
    settings.save_ok();

    let mut playerlist = PlayerRecords::load_or_create(&args);
    playerlist.save_ok();

    let players = Players::new(playerlist, settings.steam_user());

    let mut state = MACState {
        server: Server::new(),
        settings,
        players,
    };

    // Steam user overrides usually imply the TF2 dir cannot be found
    // so don't check launch options.
    if !state.settings.is_steam_user_overridden() {
        check_launch_options(&state.settings);
    }

    let web_port = state.settings.webui_port();

    // The juicy part of the program
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build async runtime")
        .block_on(async {
            if state.settings.masterbase_key().is_empty() {
                tracing::warn!("No masterbase key is set. If you would like to enable demo uploads, please provision a key at https://megaanticheat.com/provision");
            }

            // Close any previous masterbase sessions that might not have finished up
            // properly.
            if state.settings.upload_demos() {
                const TIMEOUT: u64 = 4;
                match tokio::time::timeout(Duration::from_secs(TIMEOUT), async { masterbase::force_close_session(
                    state.settings.masterbase_host(),
                    state.settings.masterbase_key(),
                    state.settings.use_masterbase_http(),
                ).await})
                .await
                {
                    // Successfully closed existing session
                    Ok(Ok(r)) if r.status().is_success() => tracing::warn!(
                        "User was previously in a Masterbase session that has now been closed."
                    ),
                    // Server error
                    Ok(Ok(r)) if r.status().is_server_error() => tracing::error!(
                        "Server error when trying to close previous Masterbase sessions: Status code {}",
                        r.status()
                    ),
                    // Not authorized, invalid key
                    Ok(Ok(r)) if r.status() == StatusCode::UNAUTHORIZED => {
                        tracing::warn!("Your Masterbase key is not valid, demo uploads will be disabled. Please provision a new one at https://megaanticheat.com/provision");
                    }
                    // Forbidden, no session was open
                    Ok(Ok(r)) if r.status() == StatusCode::FORBIDDEN => {
                        tracing::info!("Successfully authenticated with the Masterbase.");
                    }
                    // Remaining responses will be client failures
                    Ok(Ok(r)) => tracing::info!("Client error when trying to contact masterbase: Status code {}", r.status()),
                    Ok(Err(e)) => tracing::error!("Couldn't reach Masterbase: {e}"),
                    Err(_) => {
                        tracing::error!("Connection to masterbase timed out after {TIMEOUT} seconds");
                    }
                }
            }

            // Info about demo uploads
            if !state.settings.masterbase_key().is_empty() {
                if state.settings.upload_demos() {
                    tracing::info!("Demo uploading is enabled.");
                } else {
                    tracing::warn!("Demo uploading is not enabled. If you would like to use all the Masterbase features, you will need to agree to the TOS.");
                }
            }

            // Exit handler
            let running = Arc::new(AtomicBool::new(true));
            let r = running.clone();
            tokio::task::spawn(async move {
                if let Err(e) = tokio::signal::ctrl_c().await {
                    tracing::error!("Error with Ctrl+C handler: {e}");
                }
                r.store(false, Ordering::SeqCst);
            });

            // Autolaunch UI
            if args.autolaunch_ui || state.settings.autolaunch_ui() {
                if let Err(e) = open::that(Path::new(&format!("http://localhost:{web_port}"))) {
                    tracing::error!("Failed to open web browser: {:?}", e);
                }
            }

            // Demo watcher and manager
            let demo_path = state.settings.tf2_directory().join("tf");
            let demo_watcher = DemoWatcher::new(&demo_path)
                .map_err(|e| {
                    tracing::error!("Could not initialise demo watcher: {e}");
                })
                .ok();

            // Web API
            let (web_state, web_requests) = WebState::new(state.settings.web_ui_source());
            tokio::task::spawn(async move {
                web_main(web_state, web_port).await;
            });

            // Watch console log
            let log_file_path: PathBuf =
                PathBuf::from(state.settings.tf2_directory()).join("tf/console.log");
            let console_log = Box::new(ConsoleLog::new(log_file_path).await);

            let mut event_loop: EventLoop<MACState, Message, Handler> = EventLoop::new()
                .add_source(console_log)
                .add_source(emit_on_timer(Duration::from_secs(3), || Refresh).await)
                .add_source(emit_on_timer(Duration::from_millis(500), || ProfileLookupBatchTick).await)
                .add_source(Box::new(web_requests))
                .add_handler(DemoManager::new())
                .add_handler(CommandManager::new())
                .add_handler(ConsoleParser::default())
                .add_handler(ExtractNewPlayers)
                .add_handler(LookupProfiles::new())
                .add_handler(LookupFriends::new())
                .add_handler(DumbAutoKick)
                .add_handler(WebAPIHandler::new())
                .add_handler(SseEventBroadcaster::new());

            if args.print_votes {
                event_loop = event_loop.add_handler(PrintVotes::new());
            }

            if let Some(dw) = demo_watcher {
                event_loop = event_loop.add_source(Box::new(dw));
            }

            loop {
                if !running.load(Ordering::SeqCst) {
                    tracing::info!("Saving and exiting.");
                    state.players.records.save_ok();
                    state.settings.save_ok();
                    state.players.save_steam_info_ok();
                    std::process::exit(0);
                }

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
            .steam_user()
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
