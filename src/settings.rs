use std::{
    fs::File,
    fs::OpenOptions,
    io::{self, Write},
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use directories_next::ProjectDirs;
use keyvalues_parser::Vdf;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use steamid_ng::SteamID;
use thiserror::Error;

use crate::gamefinder;
use crate::Args;

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
    /// Attempt to load settings from the user's saved configuration file
    pub fn load(args: &Args) -> Result<Settings, ConfigFilesError> {
        Self::load_from(Self::locate_config_file_path()?, args)
    }

    /// Attempt to load settings from a provided configuration file, or just use default config
    pub fn load_from(path: PathBuf, args: &Args) -> Result<Settings, ConfigFilesError> {
        // Read config.yaml file if it exists, otherwise try to create a default file.
        let contents = match std::fs::read_to_string(&path) {
            Ok(content) => Ok(content),
            Err(why) => match why.kind() {
                ErrorKind::NotFound => {
                    tracing::warn!(
                        "No config file found in config directory. Creating default file..."
                    );
                    let def_settings = Settings::default();
                    def_settings.save()?;
                    tracing::info!("Saved default config file to {:?}...", path);
                    return Ok(def_settings); // Short circuit due to fresh default settings
                }
                _ => Err(why),
            },
        }
        .map_err(|e| ConfigFilesError::IO(path.to_string_lossy().into(), e))?;

        let mut settings = serde_yaml::from_str::<Settings>(&contents)
            .map_err(|e| ConfigFilesError::Yaml(path.to_string_lossy().into(), e))?;

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
                let users_obj = if let Some(obj) = login_vdf.value.get_obj() {
                    obj
                } else {
                    tracing::error!("Failed to get user data from VDF.");
                    return None;
                };
                let mut latest_timestamp = 0;
                let mut latest_user_sid64: Option<&str> = None;

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
                                    latest_timestamp = timestamp;
                                    latest_user_sid64 = Some(user_sid64);
                                }
                            }
                        });
                }

                let user_sid64 = if let Some(sid64) = latest_user_sid64 {
                    sid64
                } else {
                    tracing::error!("No user with a valid timestamp found.");
                    return None;
                };

                user_sid64.parse::<u64>().map_or_else(
                    |why| {
                        tracing::error!("Invalid SID64 found in user data: {}.", why);
                        None
                    },
                    |user_int64| {
                        tracing::info!("Parsed most recent steam user <{}>", user_int64);
                        Some(SteamID::from(user_int64))
                    },
                )
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
    pub fn get_external_preferences(&self) -> &serde_json::Value {
        &self.external
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

    pub fn get_autolaunch_ui(&self) -> bool {
        self.autolaunch_ui
    }

    pub fn set_steam_api_key(&mut self, key: Arc<str>) {
        self.steam_api_key = key;
        self.save_ok();
    }
    pub fn update_external_preferences(&mut self, prefs: serde_json::Value) {
        merge_json_objects(&mut self.external, prefs);
        self.save_ok();
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

    fn locate_config_file_path() -> Result<PathBuf, ConfigFilesError> {
        Self::locate_config_directory().map(|dir| dir.join("config.yaml"))
    }
}

impl Default for Settings {
    fn default() -> Self {
        let tf2_directory = gamefinder::locate_tf2_folder().unwrap_or(PathBuf::new());
        let config_path = Self::locate_config_file_path()
            .map_err(|e| tracing::error!("Failed to create config directory: {:?}", e))
            .ok();

        Settings {
            steam_user: Self::load_current_steam_user(),
            config_path,
            tf2_directory,
            rcon_password: "mac_rcon".into(),
            steam_api_key: "YOUR_API_KEY_HERE".into(),
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
