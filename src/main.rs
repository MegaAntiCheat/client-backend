use io::IOManager;
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

#[tokio::main]
async fn main() {
    if let Err(e) = init_log() {
        eprintln!(
            "Failed to initialise logger, continuing without logs: {:?}",
            e
        );
    }

    // Load settings
    let settings = Settings::default();

    // Spawn web server
    tokio::spawn(async move {
        web::web_main().await;
    });

    // Spawn IO thread
    let io = IOManager::start(&settings);

    // Main loop
    main_loop(io).await;
}

async fn main_loop(mut io: IOManager) {
    log::debug!("Entering main loop");
    loop {
        let response = io.recv();
        println!("Got response: {:?}", &response);
        let mut state = STATE.write().unwrap();
        state.handle_io_response(response);
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
