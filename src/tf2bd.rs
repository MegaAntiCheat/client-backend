use std::{
    fs::File,
    io::Read,
    path::PathBuf,
};
use reqwest;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};


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
pub struct TF2BDDefinitions {
    pub tfbd_player_attributes: TfbdPlayerAttributes,
    pub tfbd_player_attributes_array: TfbdPlayerAttributesArray,
    pub color: Color,
    pub steamid: SteamID,
    pub file_info: FileInfo,
    pub tfbd_text_match: TfbdTextMatch,
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
    #[serde(rename = "type")]
    pub type_: String,
    pub unique_items: bool,
    pub items: Vec<TfbdPlayerlistEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Color {
    #[serde(rename = "type")]
    pub type_: String,
    pub min_items: u8,
    pub max_items: u8,
    pub items: ChannelIntensity,
    #[serde(default)]
    pub default: Vec<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelIntensity {
    #[serde(rename = "type")]
    pub type_: String,
    pub description: String,
    pub minimum: f64,
    pub maximum: f64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SteamID {
    StringPattern { type_: String, pattern: String },
    Integer { type_: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfo {
    #[serde(rename = "type")]
    pub type_: String,
    pub description: String,
    pub additional_properties: bool,
    pub properties: FileInfoProperties,
    pub required: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileInfoProperties {
    pub authors: Vec<String>,
    pub title: String,
    pub description: String,
    pub update_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TfbdTextMatch {
    #[serde(rename = "type")]
    pub type_: String,
    pub additional_properties: bool,
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
        if let Some(url) = &data.file_info.properties.update_url {
            if let Ok(updated_data) = fetch_data_from_url(url).await {
                data = updated_data;
            } else{
                return Err(anyhow!("Unable to fetch data from URL"));
            }
        }
    }

    Ok(data)
}