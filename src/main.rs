use player_records::PlayerRecords;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;
use steamapi::steam_api_loop;
use steamid_ng::SteamID;
use tokio::sync::mpsc::UnboundedSender;

use clap::{ArgAction, Parser};
use io::{Commands, IOManager};
use launchoptions::LaunchOptions;
use settings::{ConfigFilesError, Settings};
use state::SharedState;
use tappet::SteamAPI;
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

fn main() {
    let _guard = init_tracing();

    // Arg handling
    let args = Args::parse();

    // Load settings
    let settings_path: PathBuf = args
        .config
        .as_ref()
        .map(|i| Ok(i.into()))
        .unwrap_or(Settings::locate_config_file_path()).map_err(|e| {
            tracing::error!("Could not find a suitable location for the configuration: {}\nPlease specify a file path manually with --config", e);
        }).unwrap_or(PathBuf::from("config.yaml"));

    let mut settings = match Settings::load_from(settings_path, &args) {
        Ok(settings) => settings,
        Err(ConfigFilesError::Yaml(path, e)) => {
            tracing::error!("{} could not be loaded: {:?}", path, e);
            tracing::error!(
                "Please resolve any issues or remove the file, otherwise data may be lost."
            );
            panic!("Failed to load configuration")
        }
        Err(ConfigFilesError::IO(path, e)) if e.kind() == ErrorKind::NotFound => {
            tracing::warn!("Could not locate {}, creating new configuration.", &path);
            let mut settings = Settings::default();
            settings.set_config_path(path.into());
            settings.set_overrides(&args);
            settings
        }
        Err(e) => {
            tracing::error!("Could not load configuration: {:?}", e);
            tracing::error!(
                "Please resolve any issues or remove the file, otherwise data may be lost."
            );
            panic!("Failed to load configuration")
        }
    };

    settings.save_ok();

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

    // Playerlist
    let playerlist_path: PathBuf = args
        .playerlist
        .as_ref()
        .map(|i| Ok(i.into()))
        .unwrap_or(PlayerRecords::locate_playerlist_file()).map_err(|e| {
            tracing::error!("Could not find a suitable location for the playerlist: {} \nPlease specify a file path manually with --playerlist otherwise information may not be saved.", e); 
        }).unwrap_or(PathBuf::from("playerlist.json"));

    let playerlist = match PlayerRecords::load_from(playerlist_path) {
        Ok(playerlist) => playerlist,
        Err(ConfigFilesError::Json(path, e)) => {
            tracing::error!("{} could not be loaded: {:?}", path, e);
            tracing::error!(
                "Please resolve any issues or remove the file, otherwise data may be lost."
            );
            panic!("Failed to load playerlist")
        }
        Err(ConfigFilesError::IO(path, e)) if e.kind() == ErrorKind::NotFound => {
            tracing::warn!("Could not locate {}, creating new playerlist.", &path);
            let mut playerlist = PlayerRecords::default();
            playerlist.set_path(path.into());
            playerlist
        }
        Err(e) => {
            tracing::error!("Could not load playerlist: {:?}", e);
            tracing::error!(
                "Please resolve any issues or remove the file, otherwise data may be lost."
            );
            panic!("Failed to load playerlist")
        }
    };

    playerlist.save_ok();

    // Get vars from settings before it is borrowed
    let autolaunch_ui = args.autolaunch_ui || settings.get_autolaunch_ui();
    let steam_api_key = settings.get_steam_api_key();
    let client = SteamAPI::new(steam_api_key);
    let steam_user = settings.get_steam_user();

    // Initialize State
    let io = IOManager::new();
    let state = SharedState::new(settings, playerlist, io.get_command_requester());

    // Start the async part of the program
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            // Friendslist
            {
                let state = state.clone();
                tokio::task::spawn(load_friends_list(state, client, steam_user));
            }

            // Spawn web server
            {
                let state = state.clone();
                tokio::spawn(async move {
                    web::web_main(state, port).await;
                });
            }

            if autolaunch_ui {
                if let Err(e) = open::that(Path::new(&format!("http://localhost:{}", port))) {
                    tracing::error!("Failed to open web browser: {:?}", e);
                }
            }

            // Steam API loop
            let (steam_api_requester, steam_api_receiver) = tokio::sync::mpsc::unbounded_channel();
            {
                let state = state.clone();
                tokio::task::spawn(async move {
                    steam_api_loop(state, steam_api_receiver).await;
                });
            }

            // Main and refresh loop
            {
                let state = state.clone();
                let cmd = io.get_command_requester();
                tokio::task::spawn(async move {
                    refresh_loop(state, cmd).await;
                });
            }

            main_loop(io, state, steam_api_requester).await;
        });
}

async fn main_loop(
    mut io: IOManager,
    state: SharedState,
    steam_api_requester: UnboundedSender<SteamID>,
) {
    let mut new_players = Vec::new();

    loop {
        // Log
        match io.handle_log(&state) {
            Ok(output) => {
                let user = state.settings.read().get_steam_user();
                for new_player in state
                    .server
                    .write()
                    .handle_io_output(output, user)
                    .into_iter()
                {
                    new_players.push(new_player);
                }
            }
            Err(e) => {
                tracing::error!("Failed to get log file contents: {:?}", e);
            }
        }

        // Commands
        match io.handle_waiting_command(&state).await {
            Ok(output) => {
                let user = state.settings.read().get_steam_user();
                for new_player in state
                    .server
                    .write()
                    .handle_io_output(output, user)
                    .into_iter()
                {
                    new_players.push(new_player);
                }
            }
            Err(e) => {
                tracing::error!("Failed to run command: {:?}", e);
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

async fn load_friends_list(
    state: SharedState,
    mut client: SteamAPI,
    steam_user_id: Option<SteamID>,
) {
    let friendslist = match steam_user_id {
        Some(steam_user) => {
            match steamapi::request_account_friends(&mut client, steam_user).await {
                Ok(friendslist) => {
                    tracing::info!("Successfully loaded friendslist.");
                    friendslist
                }
                Err(e) => {
                    tracing::warn!("Failed to load friendslist: {:?}", e);
                    Vec::new()
                }
            }
        }
        None => {
            tracing::warn!("Failed to load friendslist: Steam user not found.");
            Vec::new()
        }
    };

    state.server.write().update_friends_list(friendslist);
}

async fn refresh_loop(state: SharedState, cmd: UnboundedSender<Commands>) {
    tracing::debug!("Entering refresh loop");
    loop {
        state.server.write().refresh();

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
