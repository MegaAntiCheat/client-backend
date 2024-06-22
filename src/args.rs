use std::path::PathBuf;

use clap::{ArgAction, Parser};

#[allow(clippy::struct_excessive_bools)]
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
    /// Override the Steam User
    #[arg(long)]
    pub steam_user: Option<String>,
    /// Override the configured/default rcon password
    #[arg(short, long)]
    pub rcon_pword: Option<String>,
    /// Override the RCON port for connecting to the game
    #[arg(long)]
    pub rcon_port: Option<u16>,
    /// Override the configured Steam API key,
    #[arg(short, long)]
    pub api_key: Option<String>,
    /// Override the configured masterbase key
    #[arg(long)]
    pub mb_key: Option<String>,
    /// Override the default masterbase endpoint
    #[arg(long)]
    pub mb_host: Option<String>,
    /// Launch the web-ui in the default browser on startup
    #[arg(long, action=ArgAction::SetTrue, default_value_t=false)]
    pub autolaunch_ui: bool,

    /// Only parse the bare minimum to allow demo uploads (may improve
    /// performance)
    #[arg(long, action=ArgAction::SetTrue, default_value_t=false)]
    pub minimal_demo_parsing: bool,
    /// Don't monitor or parse demos (may improve performance, but also prevents
    /// demo uploads)
    #[arg(long, action=ArgAction::SetTrue, default_value_t=false)]
    pub dont_parse_demos: bool,
    /// Don't upload demos to the masterbase
    #[arg(long, action = ArgAction::SetTrue, default_value_t=false)]
    pub dont_upload_demos: bool,
    /// Use http (inscure) connections to the masterbase
    #[arg(long, action=ArgAction::SetTrue, default_value_t=false)]
    pub masterbase_http: bool,

    /// Print player votes parsed from demos (requires demo parsing to be enabled)
    #[arg(long, action=ArgAction::SetTrue, default_value_t=false)]
    pub print_votes: bool,

    /// Serve web-ui files from this directory
    #[arg(short, long)]
    pub web_dir: Option<PathBuf>,
}
