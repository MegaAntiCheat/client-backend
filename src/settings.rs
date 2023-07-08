use std::{
    fmt::Display,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use directories_next::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::gamefinder;

#[derive(Debug)]
pub enum Error {
    IO(std::io::Error),
    Serde(serde_yaml::Error),
    NoValidPath,
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::IO(value)
    }
}
impl From<serde_yaml::Error> for Error {
    fn from(value: serde_yaml::Error) -> Self {
        Self::Serde(value)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IO(io) => {
                write!(f, "IO: {}", io)
            }
            Self::Serde(serde) => {
                write!(f, "Serialize/Deserialize: {}", serde)
            }
            Self::NoValidPath => {
                write!(f, "No valid home directory was found.")
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(skip)]
    config_path: Option<PathBuf>,
    tf2_directory: PathBuf,
    rcon_password: Arc<str>,
    port: u16,
}

impl Settings {
    /// Attempt to load settings from the user's saved configuration file
    pub fn load() -> Result<Settings, Error> {
        match Self::locate_config_file_path() {
            Some(path) => Self::load_from(path),
            None => Err(Error::NoValidPath),
        }
    }

    /// Attempt to load settings from a provided configuration file, or just use default config
    pub fn load_from(path: PathBuf) -> Result<Settings, Error> {
        let contents = std::fs::read_to_string(&path)?;
        let mut settings = serde_yaml::from_str::<Settings>(&contents)?;
        settings.config_path = Some(path);

        log::debug!("Successfully loaded settings.");
        Ok(settings)
    }

    /// Attempt to save the settings back to the loaded configuration file
    pub fn save(&self) -> Result<(), Error> {
        if self.config_path.is_none() {
            return Err(Error::NoValidPath);
        }

        let mut open_options = OpenOptions::new();
        let mut file = open_options
            .create(true)
            .write(true)
            .open(self.config_path.as_ref().unwrap())?;
        write!(&mut file, "{}", serde_yaml::to_string(self)?)?;

        Ok(())
    }

    /// Attempt to save the settings, log errors and ignore result
    pub fn save_ok(&self) {
        if let Err(e) = self.save() {
            log::error!("Failed to save settings: {}", e);
            return;
        }
        log::debug!("Settings saved to {:?}", self.config_path);
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
            port: 3621,
        }
    }
}
