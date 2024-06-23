use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;
use anyhow::Result;


const STEAM_HISTORY_SOURCEBANS: &str = "https://steamhistory.net/api/sourcebans";


/// Documentation for the API is at https://steamhistory.net/api
#[derive(Debug, Serialize, Deserialize)]
struct SteamHistoryRequest {
    /// The SteamHistory API key
    key: String,
    /// The set of SteamID's to look up. Use SteamID64 format, and its a comma separated string
    steamids: String,
    /// 0 | 1 - This sets the api so the responses are keyed with the SteamID, making object arrays for those with multiple sourcebans. Always use 1 for this in batches
    shouldkey: usize,
}


impl SteamHistoryRequest {
    pub fn new_from_steamids(api_key: String, steam_ids: &[SteamID]) -> Self {
        let joined_ids = steam_ids
            .iter()
            .map(|steamid| u64::from(*steamid)
            .to_string())
            .collect::<Vec<String>>()
            .join(",");

        SteamHistoryRequest {
            key: api_key,
            steamids: joined_ids,
            shouldkey: 1 
        }
    }
}

// An example steamhistory.net api response on requesting the status of interceptor, cutie and someone else 
// {
//     "response": {
//         "76561198325285012": [
//             {
//                 "SteamID": "76561198325285012",
//                 "Name": "△",
//                 "CurrentState": "Permanent",
//                 "BanReason": "[Little Anti-Cheat 1.7.1] Angle-Cheats Detected",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1672674840,
//                 "UnbanTimestamp": 0,
//                 "Server": "LBGaming"
//             },
//             {
//                 "SteamID": "76561198325285012",
//                 "Name": "cLIPPY",
//                 "CurrentState": "Permanent",
//                 "BanReason": "[Anti-Cheat] Aimbot Detected (AA) | Weapon: tf_weapon_shotgun_hwg",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1672685640,
//                 "UnbanTimestamp": 0,
//                 "Server": "BlackWonder"
//             },
//             {
//                 "SteamID": "76561198325285012",
//                 "Name": "Martin M",
//                 "CurrentState": "Permanent",
//                 "BanReason": "[Anti-Cheat] Aimbot Detected (TB) | Weapon: tf_weapon_shotgun_soldier",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1673177497,
//                 "UnbanTimestamp": 0,
//                 "Server": "Flux.tf"
//             },
//             {
//                 "SteamID": "76561198325285012",
//                 "Name": "Im a girl",
//                 "CurrentState": "Permanent",
//                 "BanReason": "[StAC] Banned for pSilent after 10 detections",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1673919780,
//                 "UnbanTimestamp": 0,
//                 "Server": "dpg.tf"
//             },
//             {
//                 "SteamID": "76561198325285012",
//                 "Name": "Im a girl",
//                 "CurrentState": "Permanent",
//                 "BanReason": "SMAC NX1",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1674085560,
//                 "UnbanTimestamp": 0,
//                 "Server": "Skial"
//             },
//             {
//                 "SteamID": "76561198325285012",
//                 "Name": "❤❤❤❤❤❤",
//                 "CurrentState": "Expired",
//                 "BanReason": "Toxicity towards others, length due to severity",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1674239191,
//                 "UnbanTimestamp": 1675448791,
//                 "Server": "Scrap.tf"
//             },
//             {
//                 "SteamID": "76561198325285012",
//                 "Name": "✨ im a girl ✨",
//                 "CurrentState": "Permanent",
//                 "BanReason": "[StAC] Banned for 10 fake angle detections",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1674580200,
//                 "UnbanTimestamp": 0,
//                 "Server": "FirePowered Gaming"
//             },
//             {
//                 "SteamID": "76561198325285012",
//                 "Name": "c u t i e ♥",
//                 "CurrentState": "Permanent",
//                 "BanReason": "Aimbot Detected",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1677223740,
//                 "UnbanTimestamp": 0,
//                 "Server": "SG-Gaming.net"
//             },
//             {
//                 "SteamID": "76561198325285012",
//                 "Name": "ATOMIC c u t i e ♥",
//                 "CurrentState": "Permanent",
//                 "BanReason": "Aimbot",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1678725240,
//                 "UnbanTimestamp": 0,
//                 "Server": "Pubs.tf"
//             },
//             {
//                 "SteamID": "76561198325285012",
//                 "Name": "ATOMIC c u t i e ♥",
//                 "CurrentState": "Permanent",
//                 "BanReason": "Aimbot",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1678725274,
//                 "UnbanTimestamp": 0,
//                 "Server": "Pubs.tf"
//             }
//         ],
//         "76561198955714226": [
//             {
//                 "SteamID": "76561198955714226",
//                 "Name": "JFK",
//                 "CurrentState": "Permanent",
//                 "BanReason": "[Anti-Cheat] Aimbot Detected (CMSP) | Weapon: tf_weapon_rocketlauncher",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1661353912,
//                 "UnbanTimestamp": 0,
//                 "Server": "BlackWonder"
//             }
//         ],
//         "76561199387457695": [
//             {
//                 "SteamID": "76561199387457695",
//                 "Name": "\u0026\u0026\u0026I\u0026\u0026\u0026\u0026c\u0026\u0026\u0026\u0026q",
//                 "CurrentState": "Permanent",
//                 "BanReason": "Aimbot Detected",
//                 "UnbanReason": null,
//                 "BanTimestamp": 1700953620,
//                 "UnbanTimestamp": 0,
//                 "Server": "SG-Gaming.net"
//             }
//         ]
//     }
// }

#[derive(Debug, Serialize, Deserialize)]
pub struct SteamHistoryIndividualResponse {
    /// SteamID of the user
    #[serde(rename="SteamID")]
    steamid: String,
    /// Name of the user when banned. NULL when unavailable
    #[serde(rename="Name")]
    name: Option<String>,
    /// Possible states: 'Permanent', 'Temp-Ban', 'Expired', or 'Unbanned'
    #[serde(rename="CurrentState")]
    current_state: String,
    /// Ban reason provided by the server. NULL when unavailable
    #[serde(rename="BanReason")]
    ban_reason: Option<String>,
    /// Unban reason provided by the server. NULL when unavailable
    #[serde(rename="UnbanReason")]
    unban_reason: Option<String>,
    /// When the user was banned
    #[serde(rename="BanTimestamp")]
    ban_timestamp: usize,
    /// When the user was unbanned. 0 when unavailable or permanent
    #[serde(rename="UnbanTimestamp")]
    unban_timestamp: usize,
    /// Name of the server the user received the ban from
    #[serde(rename="Server")]
    server: String
}

#[derive(Debug, Deserialize)]
pub struct SteamHistoryResponse {
    response: HashMap<String, Vec<SteamHistoryIndividualResponse>>
}

pub struct SteamHistoryClient {
    pub client: reqwest::Client,
    pub key: String,
}

pub async fn get_steamhistory_sumamry(client: SteamHistoryClient, steam_ids: &[SteamID]) -> Result<SteamHistoryResponse> {
    let request_data = SteamHistoryRequest::new_from_steamids(client.key, steam_ids);
    let params = serde_url_params::to_string(&request_data).expect("URL Param serialisation failure");
    let response = client.client.get(format!("{}?{}", STEAM_HISTORY_SOURCEBANS, params)).send().await?;
    let response_struct: SteamHistoryResponse = response.json().await?;
    Ok(response_struct)
}