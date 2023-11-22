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
