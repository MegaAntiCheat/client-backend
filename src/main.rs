use anyhow::Context;
use std::time::Duration;
use steamapi::steam_api_loop;
use steamid_ng::SteamID;
use tokio::sync::mpsc::Sender;

use clap::Parser;
use io::{Commands, IOManager};
use launchoptions::LaunchOptionsV2;
use settings::Settings;
use state::State;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    fmt::writer::MakeWriterExt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
};

mod gamefinder;
mod io;
mod launchoptions;
mod player;
mod server;
mod settings;
mod state;
mod steamapi;
mod web;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Override the port to host the web-ui and API on
    #[arg(short, long)]
    pub port: Option<u16>,
    /// Override the config file to use
    #[arg(short, long)]
    pub config: Option<String>,
    /// Override the default tf2 directory
    #[arg(short, long)]
    pub tf2_dir: Option<String>,
    /// Override the configured/default rcon password
    #[arg(short, long)]
    pub rcon_pword: Option<String>,
    /// Override the configured Steam API key,
    #[arg(short, long)]
    pub api_key: Option<String>,
    /// Whether to rewrite the user localconfig.vdf to append the new launch options,
    #[arg(short='l')]
    pub rewrite_launch_options: Option<bool>,
    /// Whether to panic on detecting missing launch options
    #[arg(short)]
    pub no_panic_on_missing_launch_options: Option<bool>,
}

#[tokio::main]
async fn main() {
    let _guard = init_tracing();

    // Arg handling
    let args = Args::parse();

    // Load settings
    let settings = if let Some(config_path) = &args.config {
        tracing::info!(
            "Overrode default config path with provided '{}'",
            config_path
        );
        Settings::load_from(config_path.into(), &args)
    } else {
        Settings::load(&args)
    };

    let settings = match settings {
        Ok(settings) => settings,
        Err(e) => {
            tracing::warn!("Failed to load settings, continuing with defaults: {:?}", e);
            Settings::default()
        }
    };

    let mut launch_opts = LaunchOptionsV2::new(
        settings.get_steam_user().unwrap(),
        gamefinder::TF2_GAME_ID.to_string(),
    )
    .context("Failed to extract launch options for main.")
    .unwrap();
    // Warn about missing launch options for TF2
    let missing = launch_opts.check_missing_args();
    if args.rewrite_launch_options.unwrap_or(false) {
        // Add missing launch options to the localconfig.vdf for the current user.
        // This only sticks if steam is closed when the write occurs.
        // TODO: Make this work always (i dont think this is possible but oh well)
        launch_opts.add_opts_if_missing();
        let _ = launch_opts.write_changes_to_file();
    } else if let Ok(missing_opts) = missing {
        if !missing_opts.is_empty() {
            tracing::error!(
                "Please add the following launch options to your 
            TF2 to allow the MAC client to interface correctly with TF2."
            );
            tracing::error!("Missing launch options: {:?}", missing_opts);
            if !(args.no_panic_on_missing_launch_options.unwrap_or(false)) {
                panic!("Missing required launch options in TF2 for MAC to function. Aborting...");
            }
        }
    }

    // Just some settings we'll need later
    let port = settings.get_port();
    let steam_api_key = settings.get_steam_api_key();

    // Initialize State
    State::initialize_state(State::new(settings));

    // Spawn web server
    tokio::spawn(async move {
        web::web_main(port).await;
    });

    // Steam API loop
    let (steam_api_requester, steam_api_receiver) = tokio::sync::mpsc::channel(32);
    tokio::task::spawn(async move {
        steam_api_loop(steam_api_receiver, steam_api_key).await;
    });

    // Main and refresh loop
    let io = IOManager::new();

    let cmd = io.get_command_requester();
    tokio::task::spawn(async move {
        refresh_loop(cmd).await;
    });

    main_loop(io, steam_api_requester).await;
}

async fn main_loop(mut io: IOManager, steam_api_requester: Sender<SteamID>) {
    let mut new_players = Vec::new();

    loop {
        // Log
        match io.handle_log() {
            Ok(output) => {
                let mut state = State::write_state();
                state.log_file_state = Ok(());
                if let Some(new_player) = state.server.handle_io_response(output) {
                    new_players.push(new_player);
                }
            }
            Err(e) => {
                let mut state = State::write_state();
                // This one runs very frequently so we'll only print diagnostics
                // once when it first fails.
                if state.log_file_state.is_ok() {
                    tracing::error!("Failed to get log file contents: {:?}", e);
                }
                state.log_file_state = Err(e);
            }
        }

        // Commands
        match io.handle_waiting_command().await {
            Ok(output) => {
                let mut state = State::write_state();
                state.rcon_state = Ok(());
                if let Some(new_player) = state.server.handle_io_response(output) {
                    new_players.push(new_player);
                }
            }
            Err(e) => {
                tracing::error!("Failed to run command: {:?}", e);
                State::write_state().rcon_state = Err(e);
            }
        }

        // Request steam API stuff on new players and clear
        for player in &new_players {
            steam_api_requester
                .send(*player)
                .await
                .expect("Steam API task ded");
        }

        new_players.clear();
    }
}

async fn refresh_loop(cmd: Sender<Commands>) {
    tracing::debug!("Entering refresh loop");
    loop {
        State::write_state().server.refresh();

        cmd.send(Commands::Status)
            .await
            .expect("communication with main loop from refresh loop");
        tokio::time::sleep(Duration::from_secs(3)).await;
        cmd.send(Commands::G15)
            .await
            .expect("communication with main loop from refresh loop");
        tokio::time::sleep(Duration::from_secs(3)).await;
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn init_tracing() -> Option<WorkerGuard> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info,hyper::proto=warn");
    }

    let subscriber = tracing_subscriber::registry().with(
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(EnvFilter::from_default_env()),
    );

    match std::fs::File::create("./macclient.log") {
        Ok(latest_log) => {
            let (file_writer, guard) = tracing_appender::non_blocking(latest_log);
            subscriber
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(file_writer.with_max_level(tracing::Level::TRACE)),
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
