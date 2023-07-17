use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use directories_next::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::gamefinder;

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(skip)]
    config_path: Option<PathBuf>,
    tf2_directory: PathBuf,
    rcon_password: Arc<str>,
    steam_api_key: Arc<str>,
    port: u16,
}

#[allow(dead_code)]
impl Settings {
    /// Attempt to load settings from the user's saved configuration file
    pub fn load() -> Result<Settings> {
        match Self::locate_config_file_path() {
            Some(path) => Self::load_from(path),
            None => Err(anyhow!("No valid home directory could be found.")),
        }
    }

    /// Attempt to load settings from a provided configuration file, or just use default config
    pub fn load_from(path: PathBuf) -> Result<Settings> {
        let contents =
            std::fs::read_to_string(&path).context("Failed to load existing config file.")?;
        let mut settings =
            serde_yaml::from_str::<Settings>(&contents).context("Failed to parse settings.")?;
        settings.config_path = Some(path);

        tracing::debug!("Successfully loaded settings.");
        Ok(settings)
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
        tracing::debug!("Settings saved to {:?}", self.config_path);
    }

    // Setters & Getters

    pub fn get_config_path(&self) -> Option<&PathBuf> {
        self.config_path.as_ref()
    }
    pub fn get_tf2_directory(&self) -> &Path {
        &self.tf2_directory
    }
    pub fn get_rcon_password(&self) -> Arc<str> {
        self.rcon_password.clone()
    }
    pub fn get_port(&self) -> u16 {
        self.port
    }
    pub fn get_steam_api_key(&self) -> Arc<str> {
        self.steam_api_key.clone()
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
        let dirs = ProjectDirs::from(
            "com.megascatterbomb",
            "Mega Anti Cheat",
            "MegaAntiCheat Client",
        )?;
        let dir = dirs.config_dir();
        std::fs::create_dir_all(dir).ok();
        Some(PathBuf::from(dir).join("config.yaml"))
    }
}

impl Default for Settings {
    fn default() -> Self {
        let tf2_directory = gamefinder::locate_tf2_folder().unwrap_or(PathBuf::new());

        Settings {
            config_path: Self::locate_config_file_path(),
            tf2_directory,
            rcon_password: "mac_rcon".into(),
            steam_api_key: "".into(),
            port: 3621,
        }
    }
}
