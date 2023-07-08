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
    pub tf2_directory: PathBuf,
    pub rcon_password: Arc<str>,
    pub port: u16,
}

impl Settings {
    /// Attempt to load settings from the user's saved configuration file
    pub fn load() -> Result<Settings, Error> {
        match Self::get_config_file_path() {
            Some(path) => Self::load_from(path),
            None => Err(Error::NoValidPath),
        }
    }

    /// Attempt to load settings from a provided configuration file, or just use default config
    pub fn load_from(path: PathBuf) -> Result<Settings, Error> {
        let contents = std::fs::read_to_string(&path)?;
        let mut settings = serde_yaml::from_str::<Settings>(&contents)?;
        settings.config_path = Some(path);

        Ok(settings)
    }

    /// Attempt to save the settings back to the loaded configuration file
    pub fn save(&self) -> Result<(), Error> {
        let path = Self::get_config_file_path();
        if path.is_none() {
            return Err(Error::NoValidPath);
        }
        let path = path.unwrap();

        let mut open_options = OpenOptions::new();
        let mut file = open_options.create(true).write(true).open(path)?;
        write!(&mut file, "{}", serde_yaml::to_string(self)?)?;

        Ok(())
    }

    fn get_config_file_path() -> Option<PathBuf> {
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
            config_path: Self::get_config_file_path(),
            tf2_directory,
            rcon_password: "mac_rcon".into(),
            port: 3621,
        }
    }
}
