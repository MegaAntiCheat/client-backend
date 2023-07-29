use std::{collections::HashMap, path::PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;

use crate::settings::{ConfigFilesError, Settings};

// PlayerList

#[derive(Serialize, Deserialize)]
pub struct PlayerRecords {
    #[serde(skip)]
    path: PathBuf,
    pub records: HashMap<SteamID, PlayerRecord>,
}

impl PlayerRecords {
    /// Attempt to load the [Playerlist] from the default file
    pub fn load() -> Result<PlayerRecords, ConfigFilesError> {
        Self::load_from(Self::locate_playerlist_file()?)
    }

    /// Attempt to load the [Playerlist] from the provided file
    pub fn load_from(path: PathBuf) -> Result<PlayerRecords, ConfigFilesError> {
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| ConfigFilesError::IO(path.to_string_lossy().into(), e))?;
        let mut playerlist: PlayerRecords = serde_json::from_str(&contents)
            .map_err(|e| ConfigFilesError::Json(path.to_string_lossy().into(), e))?;
        playerlist.path = path;
        Ok(playerlist)
    }

    /// Attempt to save the [Playerlist] to the file it was loaded from
    pub fn save(&self) -> Result<(), ConfigFilesError> {
        let contents = serde_json::to_string(self).context("Failed to serialize playerlist.")?;
        std::fs::write(&self.path, contents)
            .map_err(|e| ConfigFilesError::IO(self.path.to_string_lossy().into(), e))?;
        Ok(())
    }

    fn locate_playerlist_file() -> Result<PathBuf, ConfigFilesError> {
        Settings::locate_config_directory().map(|dir| dir.join("playerlist.json"))
    }
}

impl Default for PlayerRecords {
    fn default() -> Self {
        let path = Self::locate_playerlist_file()
            .map_err(|e| tracing::warn!("Failed to create config directory: {:?}", e))
            .unwrap_or("playerlist.json".into());

        PlayerRecords {
            path,
            records: HashMap::new(),
        }
    }
}

// PlayerRecord

/// A Record of a player stored in the persistent personal playerlist
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlayerRecord {
    pub steamid: SteamID,
    pub custom_data: serde_json::Value,
    pub verdict: Verdict,
}

impl PlayerRecord {
    pub fn new(steamid: SteamID) -> PlayerRecord {
        PlayerRecord {
            steamid,
            custom_data: serde_json::Value::default(),
            verdict: Verdict::Player,
        }
    }

    /// Returns true if the record does not hold any meaningful information
    pub fn is_empty(&self) -> bool {
        self.verdict == Verdict::Player && {
            self.custom_data.is_null()
                || self
                    .custom_data
                    .as_object()
                    .map(|o| o.is_empty())
                    .unwrap_or(false)
                || self
                    .custom_data
                    .as_array()
                    .map(|a| a.is_empty())
                    .unwrap_or(false)
                || self
                    .custom_data
                    .as_str()
                    .map(|s| s.is_empty())
                    .unwrap_or(false)
        }
    }
}

/// What a player is marked as in the personal playerlist
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Player,
    Bot,
    Suspicious,
    Cheater,
    Trusted,
}
