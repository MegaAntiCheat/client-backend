use std::sync::Arc;
use std::time::Duration;
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
    let port = settings.get_port();

    // Initialize State
    *State::write_state() = Some(State::new(settings));

    // Spawn web server
    tokio::spawn(async move {
        web::web_main(port).await;
    });

    // Main and refresh loop
    let io = IOManager::new();

    let cmd = io.get_command_requester();
    tokio::task::spawn(async move {
        refresh_loop(cmd).await;
    });

    main_loop(io).await;
}

async fn main_loop(mut io: IOManager) {
    loop {
        match io.handle_log() {
            Ok(Some(output)) => {
                let mut state_lock = State::write_state();
                let state = state_lock.as_mut().unwrap();
                state.log_file_state = Ok(());
                state.server.handle_io_response(output);
            }
            Ok(None) => {}
            Err(e) => {
                let mut state_lock = State::write_state();
                state_lock.as_mut().unwrap().log_file_state = Err(e);
            }
        }

        match io.handle_waiting_command().await {
            Ok(Some(output)) => {
                let mut state_lock = State::write_state();
                let state = state_lock.as_mut().unwrap();
                state.rcon_state = Ok(());
                state.server.handle_io_response(output);
            }
            Ok(None) => {}
            Err(e) => {
                let mut state_lock = State::write_state();
                state_lock.as_mut().unwrap().rcon_state = Err(e);
            }
        }
    }
}

async fn refresh_loop(cmd: Sender<Arc<str>>) {
    log::debug!("Entering refresh loop");
    loop {
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
