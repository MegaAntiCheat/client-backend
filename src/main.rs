use player_records::PlayerRecords;
use server::Server;
use steamapi::SteamAPIManager;
use tokio::select;

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{ArgAction, Parser};
use demo::demo_loop;
use io::{Command, IOManager};
use launchoptions::LaunchOptions;
use settings::Settings;
use tappet::SteamAPI;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    fmt::writer::MakeWriterExt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer,
};

mod demo;
mod gamefinder;
mod io;
mod launchoptions;
mod player;
mod player_records;
mod server;
mod settings;
mod state;
mod steamapi;
// mod web;

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
    /// Enable monitoring of demo files
    #[arg(long = "demo_monitoring", action=ArgAction::SetTrue, default_value_t=false)]
    pub demo_monitoring: bool,
}

fn main() {
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
                            "Missing required launch options in TF2 for MAC to function. Aborting...\n
                            (Add the command-line argument '--ignore_launch_opts' to ignore this)."
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

            let mut io =
                IOManager::new(log_file_path, settings.get_rcon_password().to_string()).await;
            let cmd = io.get_command_requester();

            // Spawn web server
            /*
            {
                let state = state.clone();
                tokio::spawn(async move {
                    web::web_main(state, port).await;
                });
            }
            */

            if args.autolaunch_ui || settings.get_autolaunch_ui() {
                if let Err(e) = open::that(Path::new(&format!("http://localhost:{}", port))) {
                    tracing::error!("Failed to open web browser: {:?}", e);
                }
            }

            // Demo manager
            if args.demo_monitoring {
                let demo_path = settings.get_tf2_directory().join("tf/demos");
                tracing::info!("Demo path: {:?}", demo_path);

                tokio::task::spawn(async move {
                    let _ = demo_loop(demo_path).await;
                });
            }

            let mut server = Server::new(playerlist);
            let mut steam_api =
                SteamAPIManager::new(settings.get_steam_api_key().to_string()).await;

            // Request friends
            if let Some(user) = settings.get_steam_user() {
                let steam_api_key = settings.get_steam_api_key().to_string();
                let mut client = SteamAPI::new(steam_api_key);
                match steamapi::request_account_friends(&mut client, user).await {
                    Ok(friends) => {
                        server.update_friends_list(friends);
                    }
                    Err(e) => {
                        tracing::error!("Failed to retrieve friends: {:?}", e);
                    }
                }
            }

            // Main loop

            let mut refresh_interval = tokio::time::interval(Duration::from_secs(3));
            refresh_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut refresh_iteration: u64 = 0;
            let mut new_players = Vec::new();

            loop {
                select! {
                    // IO output
                    io_output_iter = io.next_io_output() => {
                        for output in io_output_iter {
                            let user = settings.get_steam_user();
                            for new_player in server
                                .handle_io_output(output, user)
                                .into_iter()
                            {
                                new_players.push(new_player);
                            }
                        }
                    },

                    // Steam API responses
                    (steamid, steaminfo) = steam_api.next_response() => {
                        server.insert_steam_info(steamid, steaminfo);
                    }

                    // Refresh
                    _ = refresh_interval.tick() => {
                        if refresh_iteration % 2 == 0 {
                            server.refresh();
                            cmd.run_command(Command::Status);
                        } else {
                            cmd.run_command(Command::G15);
                        }

                        refresh_iteration += 1;
                    }
                }

                // Request steam API stuff on new players and clear
                for player in &new_players {
                    steam_api.request_lookup(*player);
                }

                new_players.clear();
            }
        });
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
