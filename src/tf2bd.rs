use std::{
    fs::File,
    io::Read,
    path::PathBuf,
};
use reqwest;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;

#[derive(Debug, Serialize, Deserialize)]
pub struct TF2BotDetectorPlayerListSchema {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub file_info: FileInfo,  // This FileInfo struct is from the previous translation
    pub players: Vec<TfbdPlayerlistEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TfbdPlayerlistEntry {
    pub steamid: SteamID,  // This SteamID type is from the previous translation
    pub attributes: TfbdPlayerAttributesArray,  // This struct is from the previous translation
    pub proof: Option<Vec<String>>,  // Assuming the array holds strings, adjust if necessary
    pub last_seen: LastSeen,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LastSeen {
    pub player_name: Option<String>,
    pub time: i64,  // Using i64 for the time, assuming it's a UNIX timestamp
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TfbdPlayerAttributes {
    Cheater,
    Suspicious,
    Exploiter,
    Racist,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TfbdPlayerAttributesArray {
    pub items: Vec<TfbdPlayerlistEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Color {
    #[serde(rename = "type")]
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
    #[serde(default)]
    pub default: Vec<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfo {
    pub authors: Vec<String>,
    pub title: String,
    pub description: String,
    pub update_url: Option<String>,
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

#[allow(dead_code)]
async fn fetch_data_from_url(url: &str) -> anyhow::Result<TF2BotDetectorPlayerListSchema> {
    let response = reqwest::blocking::get(url)?;
    let data: TF2BotDetectorPlayerListSchema = response.json()?;
    Ok(data)
}

#[allow(dead_code)]
pub async fn read_tfbd_json(path: PathBuf) -> Result<TF2BotDetectorPlayerListSchema, anyhow::Error> {
    let mut file = File::open(path).expect("Unable to open the file");
    let mut contents = String::new();
    file.read_to_string(&mut contents).expect("Unable to read the file");
    
    let mut data: TF2BotDetectorPlayerListSchema = serde_json::from_str(&contents)?;

    // If players list is missing and update_url exists, fetch data from that URL
    if data.players.is_empty() {
        if let Some(url) = &data.file_info.update_url {
            if let Ok(updated_data) = fetch_data_from_url(url).await {
                data = updated_data;
            } else{
                return Err(anyhow!("Unable to fetch data from URL"));
            }
        }
    }

    Ok(data)
}