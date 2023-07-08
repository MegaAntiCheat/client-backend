use std::time::Duration;

use clap::Parser;
use io::{IOCommander, IOManager};
use log::{LevelFilter, SetLoggerError};
use log4rs::append::console::{ConsoleAppender, Target};
use log4rs::append::file::FileAppender;
use log4rs::config::{Appender, Config, Root};
use log4rs::encode::pattern::PatternEncoder;
use log4rs::filter::threshold::ThresholdFilter;

use settings::Settings;
use state::STATE;

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
        settings.port = port;
    }

    // Spawn web server
    tokio::spawn(async move {
        web::web_main(settings.port).await;
    });

    // Spawn IO thread
    let io = IOManager::start(&settings);

    // Spawn refresh task
    let commander = io.get_commander();
    tokio::spawn(async move {
        refresh_loop(commander).await;
    });

    // Main loop
    main_loop(io).await;
}

async fn main_loop(mut io: IOManager) {
    log::debug!("Entering main loop");
    loop {
        let response = io.recv();
        let mut state = STATE.write().unwrap();
        state.handle_io_response(response);
    }
}

async fn refresh_loop(mut io: IOCommander) {
    log::debug!("Entering refresh loop");
    loop {
        io.send(io::IORequest::RunCommand("status".to_string()));
        std::thread::sleep(Duration::from_secs(3));
        io.send(io::IORequest::RunCommand("tf_lobby_debug".to_string()));
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
