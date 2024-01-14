use clap::{ArgAction, Parser};

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
    /// Override the RCON port for connecting to the game
    #[arg(long)]
    pub rcon_port: Option<u16>,
    /// Override the configured Steam API key,
    #[arg(short, long)]
    pub api_key: Option<String>,
    /// Launch the web-ui in the default browser on startup
    #[arg(long = "autolaunch_ui", action=ArgAction::SetTrue, default_value_t=false)]
    pub autolaunch_ui: bool,
    /// Enable monitoring of demo files
    #[arg(long = "demo_monitoring", action=ArgAction::SetTrue, default_value_t=false)]
    pub demo_monitoring: bool,
}
