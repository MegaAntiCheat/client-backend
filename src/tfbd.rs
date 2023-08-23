use std::{
    convert::TryFrom,
    path::PathBuf,
};
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;

#[derive(Debug, Serialize, Deserialize)]
pub struct TF2BotDetectorPlayerListSchema {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub file_info: Option<FileInfo>, 
    pub players: Option<Vec<TfbdPlayerlistEntry>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfo {
    pub authors: Vec<String>,
    pub title: String,
    pub description: String,
    pub update_url: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct TfbdPlayerlistEntry {
    pub steamid: SteamIdFormat,  
    pub attributes: Vec<TfbdPlayerAttributes>,  
    pub proof: Option<Vec<String>>, 
    pub last_seen: LastSeen,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SteamIdFormat {
    SteamIdString(String),
    SteamIdInteger(i64),
}

impl TryFrom<SteamIdFormat> for SteamID {
    type Error = &'static str;

    fn try_from(value: SteamIdFormat) -> Result<Self, Self::Error> {
        match value {
            SteamIdFormat::SteamIdString(s) => {
                // First try to convert using Steam3 format
                SteamID::from_steam3(&s)
                    .or_else(|_| {
                        // If the above fails, try to convert using Steam2 format
                        SteamID::from_steam2(&s)
                    })
                    .map_err(|_| "Failed to convert from both steam3 and steam2 formats")
            },
            SteamIdFormat::SteamIdInteger(i) => {
                // Convert the i64 to u64
                Ok(SteamID::from(i as u64))
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LastSeen {
    pub player_name: Option<String>,
    pub time: i64, 
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TfbdPlayerAttributes {
    Cheater,
    Suspicious,
    Exploiter,
    Racist,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
    #[serde(default)]
    pub default: Vec<f64>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct TfbdTextMatch {
    pub description: String,
    pub properties: TextMatchProperties,
    pub required: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TextMatchProperties {
    pub mode: TextMatchMode,
    pub patterns: Vec<String>,
    pub case_sensitive: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TextMatchMode {
    Equal,
    Contains,
    StartsWith,
    EndsWith,
    Regex,
    Word,
}

async fn fetch_data_from_url(url: &str) -> anyhow::Result<TF2BotDetectorPlayerListSchema> {
    let response = reqwest::get(url).await?;
    let data: TF2BotDetectorPlayerListSchema = response.json().await?;
    Ok(data)
}

pub async fn read_tfbd_json(path: PathBuf) -> Result<TF2BotDetectorPlayerListSchema, anyhow::Error> {
    let mut file = File::open(path).await.expect("Unable to open the file");
    let mut contents = String::new();
    file.read_to_string(&mut contents).await.expect("Unable to read the file");
    
    let mut data: TF2BotDetectorPlayerListSchema = serde_json::from_str(&contents)?;

    // If players list is missing or empty and update_url exists, fetch data from that URL
    if let Some(file_info) = &data.file_info {
        if let Some(url) = &file_info.update_url {
            if data.players.as_ref().map_or(true, Vec::is_empty) {
                if let Ok(updated_data) = fetch_data_from_url(url).await {
                    data = updated_data;
                } else {
                    return Err(anyhow!("Unable to fetch data from URL"));
                }
            }
        }
    }

    Ok(data)
}
