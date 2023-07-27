use std::{
    fs::{File, OpenOptions},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use directories_next::ProjectDirs;
use keyvalues_parser::Vdf;
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;

use crate::gamefinder;
use crate::Args;

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(skip)]
    config_path: Option<PathBuf>,
    #[serde(skip)]
    steam_user: Option<SteamID>,
    tf2_directory: PathBuf,
    rcon_password: Arc<str>,
    steam_api_key: Arc<str>,
    port: u16,
    #[serde(skip)]
    override_tf2_dir: Option<PathBuf>,
    #[serde(skip)]
    override_rcon_password: Option<Arc<str>>,
    #[serde(skip)]
    override_steam_api_key: Option<Arc<str>>,
    #[serde(skip)]
    override_port: Option<u16>,
    #[serde(skip)]
    override_steam_user: Option<SteamID>,
}

#[allow(dead_code)]
impl Settings {
    /// Attempt to load settings from the user's saved configuration file
    pub fn load(args: &Args) -> Result<Settings> {
        match Self::locate_config_file_path() {
            Some(path) => Self::load_from(path, args),
            None => Err(anyhow!("No valid home directory could be found.")),
        }
    }

    /// Attempt to load settings from a provided configuration file, or just use default config
    pub fn load_from(path: PathBuf, args: &Args) -> Result<Settings> {
        // Read config.yaml file if it exists, otherwise try to create a default file.
        let contents = match std::fs::read_to_string(&path) {
            Ok(content) => Ok(content),
            Err(why) => match why.kind() {
                ErrorKind::NotFound => {
                    tracing::warn!(
                        "No config file found in config directory. Creating default file..."
                    );
                    let def_settings = Settings::default();
                    def_settings.save_ok();
                    tracing::info!("Saved default config file to {:?}...", path);
                    return Ok(def_settings); // Short circuit due to fresh default settings
                }
                _ => Err(why).context("Encountered unexpected error when reading config file"),
            },
        }?;

        let mut settings =
            serde_yaml::from_str::<Settings>(&contents).context("Failed to parse settings.")?;
        settings.config_path = Some(path);

        // settings.steam_user = Settings::load_current_steam_user();

        tracing::debug!("Successfully loaded settings.");
        settings.set_overrides(args);
        Ok(settings)
    }

    /// Reads the Steam/config/loginusers.vdf file to find the currently logged in
    /// steam ID.
    fn load_current_steam_user() -> Option<SteamID> {
        tracing::debug!("Loading steam user login data from Steam directory");
        let steam_user_conf = gamefinder::locate_steam_logged_in_users().unwrap_or(PathBuf::new());
        let mut steam_use_conf_str: Vec<u8> = Vec::new();
        if let Ok(mut file) = File::open(steam_user_conf.as_path()) {
            let _ = file
                .read_to_end(&mut steam_use_conf_str)
                .context("Failed reading loginusers.vdf.");
            tracing::info!("Loaded steam user login data.");
        } else {
            tracing::error!("Could not open loginusers.vdf from Steam dir.");
        }

        match Vdf::parse(&String::from_utf8_lossy(&steam_use_conf_str)) {
            Ok(login_vdf) => {
                let user_obj = login_vdf.value.unwrap_obj();
                let user_sid64 = user_obj.keys().next().unwrap();
                match user_sid64.parse::<u64>() {
                    Ok(user_int64) => {
                        tracing::info!("Parsed current logged in steam user <{}>", user_int64);
                        Some(SteamID::from(user_int64))
                    }
                    Err(why) => {
                        tracing::error!("Invalid SID64 found in user data: {}.", why);
                        None
                    }
                }
            }
            Err(parse_err) => {
                tracing::error!("Failed to parse loginusers VDF data: {}.", parse_err);
                None
            }
        }
    }

    /// Pull all values from the args struct and set to our override values,
    /// make sure to add tracing for any values overridden!
    fn set_overrides(&mut self, args: &Args) {
        // Override (and log if) the Port used to host the middleware API (default 3621)
        self.override_port = args.port.map(|val| {
            tracing::info!("Overrode configured port value {:?}->{:?}", self.port, val);
            val
        });
        // Override (and log if) the RCON password. (default mac_rcon)
        self.override_rcon_password = args.rcon_pword.as_ref().map(|val| {
            tracing::info!(
                "Overrode configured rcon_password {:?}->{:?}",
                self.rcon_password,
                val
            );
            Arc::from(val.clone())
        });
        // Override (and log if) the Steam API key. (No default value, but can be configured from config.yaml)
        self.override_steam_api_key = args.api_key.as_ref().map(|val| {
            tracing::info!(
                "Overrode configured Steam API key {:?}->{:?}",
                self.steam_api_key,
                val
            );
            Arc::from(val.clone())
        });
        // Override (and log if) the TF2 game directory. (Can be configured, but by default we search via steam library for it)
        self.override_steam_api_key = args.tf2_dir.as_ref().map(|val| {
            tracing::info!(
                "Overrode configured TF2 directory {:?}->{:?}",
                self.tf2_directory,
                val
            );
            Arc::from(val.clone())
        });
    }

    /// Attempt to save the settings back to the loaded configuration file
    pub fn save(&self) -> Result<()> {
        let config_path = self.config_path.as_ref().context("No config file set.")?;

        let mut open_options = OpenOptions::new();
        let mut file = open_options
            .create(true)
            .write(true)
            .open(config_path)
            .context("Failed to create or open config file.")?;
        write!(
            &mut file,
            "{}",
            serde_yaml::to_string(self).context("Failed to serialize configuration.")?
        )
        .context("Failed to write to config file.")?;

        Ok(())
    }

    /// Attempt to save the settings, log errors and ignore result
    pub fn save_ok(&self) {
        if let Err(e) = self.save() {
            tracing::error!("Failed to save settings: {:?}", e);
            return;
        }
        // this will never fail to unwrap because the above error would have occured first and broken control flow.
        tracing::info!("Settings saved to {:?}", self.config_path.clone().unwrap());
    }

    // Setters & Getters
    pub fn get_steam_user(&self) -> Option<SteamID> {
        self.steam_user
    }

    pub fn get_config_path(&self) -> Option<&PathBuf> {
        self.config_path.as_ref()
    }
    pub fn get_tf2_directory(&self) -> &Path {
        self.override_tf2_dir
            .as_ref()
            .unwrap_or(&self.tf2_directory)
    }
    pub fn get_rcon_password(&self) -> Arc<str> {
        self.override_rcon_password
            .as_ref()
            .unwrap_or(&self.rcon_password)
            .clone()
    }
    pub fn get_port(&self) -> u16 {
        self.override_port.unwrap_or(self.port)
    }
    pub fn get_steam_api_key(&self) -> Arc<str> {
        self.override_steam_api_key
            .as_ref()
            .unwrap_or(&self.steam_api_key)
            .clone()
    }
    pub fn set_tf2_directory(&mut self, dir: PathBuf) {
        self.tf2_directory = dir;
        self.save_ok();
    }
    pub fn set_rcon_password(&mut self, pwd: Arc<str>) {
        self.rcon_password = pwd;
        self.save_ok();
    }
    pub fn set_port(&mut self, port: u16) {
        self.port = port;
        self.save_ok();
    }
    pub fn set_steam_api_key(&mut self, key: Arc<str>) {
        self.steam_api_key = key;
        self.save_ok();
    }

    fn locate_config_file_path() -> Option<PathBuf> {
        let dirs = ProjectDirs::from("com.megascatterbomb", "MAC", "Client")?;
        let dir = dirs.config_dir();
        std::fs::create_dir_all(dir).ok();
        Some(PathBuf::from(dir).join("config.yaml"))
    }
}

impl Default for Settings {
    fn default() -> Self {
        let tf2_directory = gamefinder::locate_tf2_folder().unwrap_or(PathBuf::new());

        Settings {
            steam_user: Self::load_current_steam_user(),
            config_path: Self::locate_config_file_path(),
            tf2_directory,
            rcon_password: "mac_rcon".into(),
            steam_api_key: "YOUR_API_KEY_HERE".into(),
            port: 3621,
            override_tf2_dir: None,
            override_rcon_password: None,
            override_steam_api_key: None,
            override_port: None,
            override_steam_user: None,
        }
    }
}
