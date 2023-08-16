use std::{
    collections::{HashMap, VecDeque},
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;

use crate::{
    player::Player,
    settings::{ConfigFilesError, Settings},
};

// PlayerList

#[derive(Serialize, Deserialize)]
pub struct PlayerRecords {
    #[serde(skip)]
    path: PathBuf,
    records: HashMap<SteamID, PlayerRecord>,
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

        // Map all of the steamids to the records. They were not included when
        // serializing/deserializing the records to prevent duplication in the
        // resulting file.
        for (steamid, record) in &mut playerlist.records {
            record.steamid = *steamid;
        }

        Ok(playerlist)
    }

    /// Attempt to save the [Playerlist] to the file it was loaded from
    pub fn save(&self) -> Result<(), ConfigFilesError> {
        let contents = serde_json::to_string(self).context("Failed to serialize playerlist.")?;
        std::fs::write(&self.path, contents)
            .map_err(|e| ConfigFilesError::IO(self.path.to_string_lossy().into(), e))?;
        Ok(())
    }

    pub fn insert_record(&mut self, record: PlayerRecord) {
        self.records.insert(record.steamid, record);
    }

    pub fn get_records(&self) -> &HashMap<SteamID, PlayerRecord> {
        &self.records
    }

    #[allow(dead_code)]
    pub fn get_record(&self, steamid: SteamID) -> Option<&PlayerRecord> {
        self.records.get(&steamid)
    }

    pub fn get_record_mut<'a>(
        &'a mut self,
        steamid: SteamID,
        players: &'a mut HashMap<SteamID, Player>,
        history: &'a mut VecDeque<Player>,
    ) -> Option<PlayerRecordLock> {
        if self.records.contains_key(&steamid) {
            Some(PlayerRecordLock {
                steamid,
                players,
                history,
                playerlist: self,
            })
        } else {
            None
        }
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
    #[serde(skip)]
    pub steamid: SteamID,
    pub custom_data: serde_json::Value,
    pub verdict: Verdict,
    #[serde(default)]
    pub previous_names: Vec<String>,
}

impl PlayerRecord {
    pub fn new(steamid: SteamID) -> PlayerRecord {
        PlayerRecord {
            steamid,
            custom_data: serde_json::Value::default(),
            verdict: Verdict::Player,
            previous_names: Vec::new(),
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

pub struct PlayerRecordLock<'a> {
    playerlist: &'a mut PlayerRecords,
    players: &'a mut HashMap<SteamID, Player>,
    history: &'a mut VecDeque<Player>,
    steamid: SteamID,
}

impl Deref for PlayerRecordLock<'_> {
    type Target = PlayerRecord;

    fn deref(&self) -> &Self::Target {
        self.playerlist
            .records
            .get(&self.steamid)
            .expect("Mutating player record.")
    }
}

impl DerefMut for PlayerRecordLock<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.playerlist
            .records
            .get_mut(&self.steamid)
            .expect("Reading player record.")
    }
}

// Update all players the server has with the updated record
impl Drop for PlayerRecordLock<'_> {
    fn drop(&mut self) {
        let record = self
            .playerlist
            .records
            .get(&self.steamid)
            .expect("Reading player record");

        // Update server players and history
        if let Some(p) = self.players.get_mut(&self.steamid) {
            p.update_from_record(record.clone());
        }

        if let Some(p) = self.history.iter_mut().find(|p| p.steamid == self.steamid) {
            p.update_from_record(record.clone());
        }

        // Update playerlist
        if record.is_empty() {
            self.playerlist.records.remove(&self.steamid);
        }

        if let Err(e) = self.playerlist.save() {
            tracing::error!("Failed to save playerlist: {:?}", e);
        }
    }
}
