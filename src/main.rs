#![feature(hash_extract_if)]

use std::sync::Arc;
use std::time::Duration;
use steamapi::steam_api_loop;
use steamid_ng::SteamID;
use tokio::sync::mpsc::Sender;

use clap::Parser;
use io::IOManager;
use log::{LevelFilter, SetLoggerError};
use log4rs::append::console::{ConsoleAppender, Target};
use log4rs::append::file::FileAppender;
use log4rs::config::{Appender, Config, Root};
use log4rs::encode::pattern::PatternEncoder;
use log4rs::filter::threshold::ThresholdFilter;

use settings::Settings;
use state::State;

use crate::io::command_manager::{CMD_STATUS, CMD_TF_LOBBY_DEBUG};

mod gamefinder;
mod io;
mod player;
mod server;
mod settings;
mod state;
mod steamapi;
mod web;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Override the port to host the web-ui and API on
    #[arg(short, long)]
    pub port: Option<u16>,
    /// Override the config file to use
    #[arg(short, long)]
    pub config: Option<String>,
}

#[tokio::main]
async fn main() {
    if let Err(e) = init_log() {
        eprintln!(
            "Failed to initialise logger, continuing without logs: {:?}",
            e
        );
    }

    // Arg handling
    let args = Args::parse();

    // Load settings
    let settings = if let Some(config_path) = &args.config {
        Settings::load_from(config_path.into())
    } else {
        Settings::load()
    };

    let mut settings = match settings {
        Ok(settings) => settings,
        Err(e) => {
            log::error!("Failed to load settings, continuing with defaults: {:?}", e);
            Settings::default()
        }
    };

    if let Some(port) = args.port {
        settings.set_port(port);
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
        match io.handle_log() {
            Ok(Some(output)) => {
                let mut state = State::write_state();
                state.log_file_state = Ok(());
                if let Some(new_player) = state.server.handle_io_response(output) {
                    new_players.push(new_player);
                }
            }
            Ok(None) => {}
            Err(e) => {
                State::write_state().log_file_state = Err(e);
            }
        }

        match io.handle_waiting_command().await {
            Ok(output) => {
                let mut state = State::write_state();
                state.rcon_state = Ok(());
                for output in output {
                    if let Some(new_player) = state.server.handle_io_response(output) {
                        new_players.push(new_player);
                    }
                }
            }
            Err(e) => {
                State::write_state().rcon_state = Err(e);
            }
        }

        // Request steam API stuff on new players and clear
        for player in &new_players {
            steam_api_requester.send(*player).await.unwrap();
        }

        new_players.clear();
    }
}

async fn refresh_loop(cmd: Sender<Arc<str>>) {
    log::debug!("Entering refresh loop");
    loop {
        State::write_state().server.refresh();

        cmd.send(CMD_STATUS.into()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        cmd.send(CMD_TF_LOBBY_DEBUG.into()).await.unwrap();
        tokio::time::sleep(Duration::from_secs(3)).await;
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn init_log() -> Result<(), SetLoggerError> {
    let level = log::LevelFilter::Debug;
    let file_path = "./log/client_backend.log";

    // Build a stderr logger.
    let stderr = ConsoleAppender::builder().target(Target::Stderr).build();

    // Logging to log file.
    let logfile = FileAppender::builder()
        // Pattern: https://docs.rs/log4rs/*/log4rs/encode/pattern/index.html
        .encoder(Box::new(PatternEncoder::new("{l} - {m}\n")))
        .build(file_path)
        .unwrap();

    // Log Trace level output to file where trace is the default level
    // and the programmatically specified level to stderr.
    let config = Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .appender(
            Appender::builder()
                .filter(Box::new(ThresholdFilter::new(level)))
                .build("stderr", Box::new(stderr)),
        )
        .build(
            Root::builder()
                .appender("logfile")
                .appender("stderr")
                .build(LevelFilter::Trace),
        )
        .unwrap();

    let _handle = log4rs::init_config(config)?;

    Ok(())
}
