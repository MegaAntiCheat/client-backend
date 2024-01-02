use std::{
    fs::OpenOptions,
    io::{self, ErrorKind, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use directories_next::ProjectDirs;
use keyvalues_parser::Vdf;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use steamid_ng::SteamID;
use thiserror::Error;

use crate::args::Args;
use crate::gamefinder;

#[derive(Debug, Error)]
pub enum ConfigFilesError {
    #[error("No valid home directory found")]
    NoValidHome,
    #[error("IO error on {0}, {1:?}")]
    IO(String, io::Error),
    #[error("Failed to parse yaml file {0}, {1:?}")]
    Yaml(String, serde_yaml::Error),
    #[error("Failed to parse json file {0}, {1:?}")]
    Json(String, serde_json::Error),
    #[error("{0:?}")]
    Other(#[from] anyhow::Error),
}
#[derive(Debug, Serialize, Deserialize, Copy, Clone)]
pub enum FriendsAPIUsage {
    None,
    CheatersOnly,
    All
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(skip)]
    config_path: Option<PathBuf>,
    #[serde(skip)]
    steam_user: Option<SteamID>,
    #[serde(skip)]
    tf2_directory: PathBuf,
    friends_api_usage: FriendsAPIUsage,
    rcon_password: Arc<str>,
    steam_api_key: Arc<str>,
    port: u16,
    autolaunch_ui: bool,
    external: serde_json::Value,
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
    /// Attempts to load the [Settings] at the specified override location, or the default location.
    /// If it cannot be found, new [Settings] will be created at that location.
    ///
    /// **Panic**:
    /// This function will panic if the settings were found but could not be loaded or
    /// some other unexpected error occurs to prevent data loss.
    pub fn load_or_create(args: &Args) -> Settings {
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

        settings
    }

    /// Attempt to load settings from the user's saved configuration file
    pub fn load(args: &Args) -> Result<Settings, ConfigFilesError> {
        Self::load_from(Self::locate_config_file_path()?, args)
    }

    /// Attempt to load settings from a provided configuration file, or just use default config
    pub fn load_from(path: PathBuf, args: &Args) -> Result<Settings, ConfigFilesError> {
        // Read config.yaml file if it exists, otherwise try to create a default file.
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| ConfigFilesError::IO(path.to_string_lossy().into(), e))?;
        let mut settings = serde_yaml::from_str::<Settings>(&contents)
            .map_err(|e| ConfigFilesError::Yaml(path.to_string_lossy().into(), e))?;

        settings.config_path = Some(path);

        tracing::debug!("Successfully loaded settings.");
        settings.set_overrides(args);
        Ok(settings)
    }

    /// Reads the Steam/config/loginusers.vdf file to find the currently logged in
    /// steam ID.
    fn load_current_steam_user() -> Result<SteamID, anyhow::Error> {
        tracing::debug!("Loading steam user login data from Steam directory");
        let user_conf_path = gamefinder::locate_steam_logged_in_users()
            .context("Could not locate logged in steam user.")?;
        let user_conf_contents = std::fs::read(user_conf_path)
            .context("Failed to read logged in user configuration.")?;

        match Vdf::parse(&String::from_utf8_lossy(&user_conf_contents)) {
            Ok(login_vdf) => {
                let users_obj = login_vdf
                    .value
                    .get_obj()
                    .ok_or(anyhow!("Failed to parse loginusers.vdf"))?;
                let mut latest_timestamp = 0;
                let mut latest_user_sid64: Option<SteamID> = None;

                for (user_sid64, user_data_values) in users_obj.iter() {
                    user_data_values
                        .iter()
                        .filter_map(|value| value.get_obj())
                        .for_each(|user_data_obj| {
                            if let Some(timestamp) = user_data_obj
                                .get("Timestamp")
                                .and_then(|timestamp_values| timestamp_values.get(0))
                                .and_then(|timestamp_vdf| timestamp_vdf.get_str())
                                .and_then(|timestamp_str| timestamp_str.parse::<i64>().ok())
                            {
                                if timestamp > latest_timestamp {
                                    if let Ok(user_steamid) =
                                        user_sid64.parse::<u64>().map(SteamID::from)
                                    {
                                        latest_timestamp = timestamp;
                                        latest_user_sid64 = Some(user_steamid);
                                    }
                                }
                            }
                        });
                }

                latest_user_sid64.ok_or(anyhow!("No user with a valid timestamp found."))
            }
            Err(parse_err) => Err(anyhow!(
                "Failed to parse loginusers VDF data: {}.",
                parse_err
            )),
        }
    }

    /// Pull all values from the args struct and set to our override values,
    /// make sure to add tracing for any values overridden!
    pub fn set_overrides(&mut self, args: &Args) {
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
            .truncate(true)
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
        tracing::debug!("Settings saved to {:?}", self.config_path.clone().unwrap());
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
    pub fn get_external_preferences(&self) -> &serde_json::Value {
        &self.external
    }
    pub fn set_tf2_directory(&mut self, dir: PathBuf) {
        self.tf2_directory = dir;
    }
    pub fn set_rcon_password(&mut self, pwd: Arc<str>) {
        self.rcon_password = pwd;
    }
    pub fn set_port(&mut self, port: u16) {
        self.port = port;
    }

    pub fn get_autolaunch_ui(&self) -> bool {
        self.autolaunch_ui
    }

    pub fn set_steam_api_key(&mut self, key: Arc<str>) {
        self.steam_api_key = key;
    }

    pub fn update_external_preferences(&mut self, prefs: serde_json::Value) {
        merge_json_objects(&mut self.external, prefs);
    }

    pub fn set_config_path(&mut self, config: PathBuf) {
        self.config_path = Some(config);
    }

    pub fn set_friends_api_usage(&mut self, friends_api_usage: FriendsAPIUsage) {
        self.friends_api_usage = friends_api_usage;
    }

    pub fn get_friends_api_usage(&self) -> &FriendsAPIUsage {
        &self.friends_api_usage
    }

    /// Attempts to find (and create) a directory to be used for configuration files
    pub fn locate_config_directory() -> Result<PathBuf, ConfigFilesError> {
        let dirs = ProjectDirs::from("com.megascatterbomb", "MAC", "MACClient")
            .ok_or(ConfigFilesError::NoValidHome)?;
        let dir = dirs.config_dir();
        std::fs::create_dir_all(dir)
            .map_err(|e| ConfigFilesError::IO(dir.to_string_lossy().into(), e))?;
        Ok(PathBuf::from(dir))
    }

    pub fn locate_config_file_path() -> Result<PathBuf, ConfigFilesError> {
        Self::locate_config_directory().map(|dir| dir.join("config.yaml"))
    }
}

impl Default for Settings {
    fn default() -> Self {
        let config_path = Self::locate_config_file_path()
            .map_err(|e| tracing::error!("Failed to create config directory: {:?}", e))
            .ok();
        let steam_user = Self::load_current_steam_user()
            .map_err(|e| tracing::error!("Failed to load steam user: {:?}", e))
            .ok();
        if let Some(steam_user) = &steam_user {
            tracing::info!(
                "Identified current steam user as {}",
                u64::from(*steam_user)
            );
        }

        Settings {
            steam_user,
            config_path,
            tf2_directory: PathBuf::default(),
            rcon_password: "mac_rcon".into(),
            steam_api_key: "YOUR_API_KEY_HERE".into(),
            friends_api_usage: FriendsAPIUsage::CheatersOnly,
            port: 3621,
            autolaunch_ui: false,
            override_tf2_dir: None,
            override_rcon_password: None,
            override_steam_api_key: None,
            override_port: None,
            override_steam_user: None,
            external: serde_json::Value::Object(Map::new()),
        }
    }
}

// Useful

/// Combines the second provided Json Object into the first. If the given [Value]s are not [Value::Object]s, this will do nothing.
fn merge_json_objects(a: &mut Value, b: Value) {
    if let Value::Object(a) = a {
        if let Value::Object(b) = b {
            for (k, v) in b {
                if v.is_null() {
                    a.remove(&k);
                } else {
                    merge_json_objects(a.entry(k).or_insert(Value::Null), v);
                }
            }

            return;
        }
    }

    *a = b;
}
