use player_records::PlayerRecords;
use std::path::Path;
use std::time::Duration;
use steamapi::steam_api_loop;
use steamid_ng::SteamID;
use tokio::sync::mpsc::UnboundedSender;

use clap::{ArgAction, Parser};
use io::{Commands, IOManager};
use launchoptions::LaunchOptions;
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
mod player_records;
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
    /// Override the playerlist to use
    #[arg(long)]
    pub playerlist: Option<String>,
    /// Override the default tf2 directory
    #[arg(short = 'd', long)]
    pub tf2_dir: Option<String>,
    /// Override the configured/default rcon password
    #[arg(short, long)]
    pub rcon_pword: Option<String>,
    /// Override the configured Steam API key,
    #[arg(short, long)]
    pub api_key: Option<String>,
    /// Rewrite the user localconfig.vdf to append the corrected set of launch options if necessary (only works when steam is not running).
    #[arg(long = "rewrite_launch_opts", action=ArgAction::SetTrue, default_value_t=false)]
    pub rewrite_launch_options: bool,
    /// Do not panic on detecting missing launch options or failure to read/parse the localconfig.vdf file.
    #[arg(short, long = "ignore_launch_opts", action=ArgAction::SetTrue, default_value_t=false)]
    pub ignore_launch_options: bool,
    /// Launch the web-ui in the default browser on startup
    #[arg(long = "autolaunch_ui", action=ArgAction::SetTrue, default_value_t=false)]
    pub autolaunch_ui: bool,
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

    let mut settings = match settings {
        Ok(settings) => settings,
        Err(e) => {
            tracing::warn!("Failed to load settings, continuing with defaults: {:?}", e);
            Settings::default()
        }
    };

    // Locate TF2 directory
    match gamefinder::locate_tf2_folder() {
        Ok(tf2_directory) => {
            settings.set_tf2_directory(tf2_directory);
        }
        Err(e) => {
            if args.tf2_dir.is_none() {
                tracing::error!("Could not locate TF2 directory: {:?}", e);
                tracing::error!("If you have a valid TF2 installation you can specify it manually by appending ' --tf2_dir \"Path to Team Fortress 2 folder\"' when running the program.");
            }
        }
    }

    // Launch options and overrides
    let launch_opts = match LaunchOptions::new(
        settings
            .get_steam_user()
            .expect("Failed to identify the local steam user (failed to find `loginusers.vdf`)"),
        gamefinder::TF2_GAME_ID.to_string(),
    ) {
        Ok(val) => Some(val),
        Err(why) => {
            // Error only if "no_panic_on_missing_launch_options" is not true.
            if !(args.ignore_launch_options) {
                panic!("Failed to get information on the current TF2 launch options from the local steam library: {}", why);
            } else {
                tracing::warn!("Couldn't verify app launch options, ignoring...");
                None
            }
        }
    };

    if let Some(mut opts) = launch_opts {
        // Warn about missing launch options for TF2
        let missing = opts.check_missing_args();
        if args.rewrite_launch_options {
            // Add missing launch options to the localconfig.vdf for the current user.
            // This only sticks if steam is closed when the write occurs.
            let _ = opts.write_corrected_args_to_file();
        } else {
            match missing {
                Ok(missing_opts) if !missing_opts.is_empty() => {
                    tracing::warn!(
                        "Please add the following launch options to your TF2 to allow the MAC client to interface correctly with TF2."
                    );
                    tracing::warn!("Missing launch options: {:?}", missing_opts);
                    if !(args.ignore_launch_options) {
                        panic!(
                            "Missing required launch options in TF2 for MAC to function. Aborting..."
                        );
                    }
                }

                Ok(_) => {
                    tracing::info!("All required launch arguments are present!");
                }

                Err(missing_opts_err) => {
                    if !(args.ignore_launch_options) {
                        panic!("Failed to verify app launch options: {}", missing_opts_err);
                    } else {
                        tracing::error!(
                            "Failed to verify app launch options: {:?}",
                            missing_opts_err
                        );
                    }
                }
            }
        }
    }

    // Just some settings we'll need later
    let port = settings.get_port();
    let steam_api_key = settings.get_steam_api_key();

    // Playerlist
    let playerlist = if let Some(playerlist_path) = &args.playerlist {
        tracing::info!(
            "Overrode default playerlist path with provided '{}'",
            playerlist_path
        );
        PlayerRecords::load_from(playerlist_path.into())
    } else {
        PlayerRecords::load()
    };

    let playerlist = match playerlist {
        Ok(playerlist) => {
            tracing::info!("Successfully loaded playerlist.");
            playerlist
        }
        Err(e) => {
            tracing::warn!(
                "Failed to load playerlist, creating new playerlist: {:?}",
                e
            );
            PlayerRecords::default()
        }
    };

    if let Err(e) = playerlist.save() {
        tracing::error!("Failed to save playerlist: {:?}", e);
    }

    // Check autolaunch ui setting before settings is borrowed
    let autolaunch_ui = args.autolaunch_ui || settings.get_autolaunch_ui();

    // Initialize State
    State::initialize_state(State::new(settings, playerlist));
    let io = IOManager::new();

    web::init_command_issuer(io.get_command_requester()).await;

    // Spawn web server
    tokio::spawn(async move {
        web::web_main(port).await;
    });

    if autolaunch_ui {
        if let Err(e) = open::that(Path::new(&format!("http://localhost:{}", port))) {
            tracing::error!("Failed to open web browser: {:?}", e);
        }
    }

    // Steam API loop
    let (steam_api_requester, steam_api_receiver) = tokio::sync::mpsc::unbounded_channel();
    tokio::task::spawn(async move {
        steam_api_loop(steam_api_receiver, steam_api_key).await;
    });

    // Main and refresh loop
    let cmd = io.get_command_requester();
    tokio::task::spawn(async move {
        refresh_loop(cmd).await;
    });

    main_loop(io, steam_api_requester).await;
}

async fn main_loop(mut io: IOManager, steam_api_requester: UnboundedSender<SteamID>) {
    let mut new_players = Vec::new();

    loop {
        // Log
        match io.handle_log() {
            Ok(output) => {
                let mut state = State::write_state();
                let user = state.settings.get_steam_user();
                state.log_file_state = Ok(());
                for new_player in state.server.handle_io_output(output, user).into_iter() {
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
                let user = state.settings.get_steam_user();
                state.rcon_state = Ok(());
                for new_player in state.server.handle_io_output(output, user).into_iter() {
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
                .expect("Steam API task ded");
        }

        new_players.clear();
    }
}

async fn refresh_loop(cmd: UnboundedSender<Commands>) {
    tracing::debug!("Entering refresh loop");
    loop {
        {
            State::write_state().server.refresh();
        }

        cmd.send(Commands::Status)
            .expect("communication with main loop from refresh loop");
        tokio::time::sleep(Duration::from_secs(3)).await;
        cmd.send(Commands::G15)
            .expect("communication with main loop from refresh loop");
        tokio::time::sleep(Duration::from_secs(3)).await;
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
