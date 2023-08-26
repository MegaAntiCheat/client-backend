use std::{
    collections::{HashMap, VecDeque},
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use steamid_ng::SteamID;

use crate::{
    player::Player,
    settings::{ConfigFilesError, Settings},
    tfbd::{
        read_tfbd_json, TF2BotDetectorPlayerListSchema, TfbdPlayerAttributes, TfbdPlayerlistEntry,
    },
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

    #[allow(dead_code)]
    pub fn merge(&mut self, other: PlayerRecords) -> Result<(), &'static str> {
        for (other_steam_id, other_record) in other.records {
            // Check if a record with the same SteamID exists in self.records
            if let Some(existing_record) = self.records.get_mut(&other_steam_id) {
                // Perform the merge operation
                let merged_record = existing_record.merge(&other_record)?;
                // Replace the existing record with the merged one
                *existing_record = merged_record;
            } else {
                // If a record with this SteamID doesn't exist, insert it
                self.records.insert(other_steam_id, other_record);
            }
        }

        Ok(())
    }

    pub fn load_from_tfbd(
        tfbd: TF2BotDetectorPlayerListSchema,
    ) -> Result<PlayerRecords, ConfigFilesError> {
        let mut records_map: HashMap<SteamID, PlayerRecord> = HashMap::new();

        if let Some(players) = tfbd.players {
            for player in players {
                // Convert each TfbdPlayerlistEntry into a PlayerRecord
                let record: PlayerRecord = player.into();
                records_map.insert(record.steamid, record);
            }
        }
        tracing::debug!("Loaded {} records from TFBD", records_map.len());

        let path = Self::locate_playerlist_file()?;

        Ok(PlayerRecords {
            path,
            records: records_map,
        })
    }

    pub async fn load_from_tfbd_path(path: PathBuf) -> Result<PlayerRecords, ConfigFilesError> {
        let content = read_tfbd_json(path).await?;
        Self::load_from_tfbd(content)
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

#[allow(dead_code)]
pub fn merge(&self, other: &PlayerRecord) -> Result<PlayerRecord, &'static str> {
    // Make sure both records have the same SteamID (most unnecessary but just in case)
    if self.steamid != other.steamid {
        return Err("SteamIDs must match to merge records");
    }

    let merged_custom_data = merge_json(&self.custom_data, &other.custom_data);

    let merged_verdict = match (self.verdict, other.verdict) {
        (Verdict::Cheater, _) | (_, Verdict::Cheater) => Verdict::Cheater,
        (Verdict::Bot, _) | (_, Verdict::Bot) => Verdict::Bot,
        (Verdict::Suspicious, _) | (_, Verdict::Suspicious) => Verdict::Suspicious,
        _ => Verdict::Player,
    };

    // Merge previous_names avoiding duplicates
    let mut merged_names = self.previous_names.clone();
    for name in &other.previous_names {
        if !merged_names.contains(name) {
            merged_names.push(name.clone());
        }
    }

    Ok(PlayerRecord {
        steamid: self.steamid,
        custom_data: merged_custom_data,
        verdict: merged_verdict,
        previous_names: merged_names,
    })
}


}

#[allow(dead_code)]
fn merge_json(original_json: &Value, priority_json: &Value) -> Value {
    match (original_json, priority_json) {
        (Value::Object(a_map), Value::Object(b_map)) => {
            let mut merged = Map::new();
            for (k, v) in a_map.iter().chain(b_map.iter()) {
                // If the key exists in both a and b, merge the values
                if a_map.contains_key(k) && b_map.contains_key(k) {
                    // Recursive step
                    merged.insert(k.clone(), merge_json(a_map.get(k).unwrap(), b_map.get(k).unwrap()));
                } else if a_map.contains_key(k) {
                    merged.insert(k.clone(), a_map.get(k).unwrap().clone());
                } else {
                    merged.insert(k.clone(), b_map.get(k).unwrap().clone());
                }
            }
            Value::Object(merged)
        },
        (Value::Array(a_vec), Value::Array(b_vec)) => {
            let mut merged = a_vec.clone();
            merged.extend(b_vec.clone());
            Value::Array(merged)
        },
        // Prefer to overwrite with priority_json (new playerlist overwrite value of old playerlist)
        (_, _) => priority_json.clone(),
    }
}


impl From<TfbdPlayerlistEntry> for PlayerRecord {
    fn from(entry: TfbdPlayerlistEntry) -> Self {
        // Extracting steamid
        let steamid =
            SteamID::try_from(entry.steamid).expect("Failed to convert SteamIdFormat to SteamID");

        // Mapping TFBD attributes to Verdict
        let mut verdict = Verdict::Player;
        let mut extra_attributes = Vec::new();

        for attribute in entry.attributes {
            match attribute {
                TfbdPlayerAttributes::Cheater => verdict = Verdict::Cheater,
                TfbdPlayerAttributes::Suspicious => {
                    if verdict != Verdict::Cheater {
                        verdict = Verdict::Suspicious;
                    }
                }
                // Adding extra attributes to a separate vector
                TfbdPlayerAttributes::Exploiter => extra_attributes.push("exploiter".to_string()),
                TfbdPlayerAttributes::Racist => extra_attributes.push("racist".to_string()),
            }
        }

        // Extracting previous names
        let mut previous_names = Vec::new();
        if let Some(player_name) = entry.last_seen.player_name {
            previous_names.push(player_name);
        }

        // Creating custom data
        let custom_data = if !extra_attributes.is_empty() {
            serde_json::json!({
                "attributes": extra_attributes,
            })
        } else {
            serde_json::Value::Null
        };

        PlayerRecord {
            steamid,
            custom_data,
            verdict,
            previous_names,
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
