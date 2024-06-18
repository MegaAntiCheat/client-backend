use std::{collections::HashMap, path::PathBuf, time::Duration};

use event_loop::StateUpdater;
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;
use tokio::sync::mpsc::Receiver;

use crate::{player_records::Verdict, settings::FriendsAPIUsage, state::MACState};

#[derive(Debug, Clone, Copy)]
pub struct Refresh;
impl StateUpdater<MACState> for Refresh {
    fn update_state(self, state: &mut MACState) {
        state.players.refresh();
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct UserUpdate {
    #[serde(rename = "localVerdict")]
    pub local_verdict: Option<Verdict>,
    #[serde(rename = "customData")]
    pub custom_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct UserUpdates(pub HashMap<SteamID, UserUpdate>);
impl StateUpdater<MACState> for UserUpdates {
    fn update_state(self, state: &mut MACState) {
        for (k, v) in self.0 {
            let name = state.players.get_name(k).map(ToOwned::to_owned);

            // Insert record if it didn't exist
            let record = state.players.records.entry(k).or_default();

            if let Some(custom_data) = v.custom_data {
                record.set_custom_data(custom_data);
            }

            if let Some(verdict) = v.local_verdict {
                record.set_verdict(verdict);
                if let Some(name) = name {
                    record.add_previous_name(&name);
                }
            }

            if record.is_empty() {
                state.players.records.remove(&k);
            }
        }

        state.players.records.save_ok();
    }
}

#[allow(clippy::unused_async)]
pub async fn emit_on_timer<M: 'static + Send>(
    interval: Duration,
    emit: fn() -> M,
) -> Box<Receiver<M>> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);

    let mut interval = tokio::time::interval(interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    tokio::task::spawn(async move {
        loop {
            interval.tick().await;
            if matches!(tx.send(emit()).await, Ok(())) {
                continue;
            }

            tracing::error!("Couldn't send refresh message. Exiting refresh loop.");
        }
    });

    Box::new(rx)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InternalPreferences {
    pub friends_api_usage: Option<FriendsAPIUsage>,
    pub tf2_directory: Option<String>,
    pub rcon_password: Option<String>,
    pub steam_api_key: Option<String>,
    pub masterbase_key: Option<String>,
    pub masterbase_host: Option<String>,
    pub rcon_port: Option<u16>,
    pub query_sourcebans: Option<bool>,
    pub dumb_autokick: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Preferences {
    pub internal: Option<InternalPreferences>,
    pub external: Option<serde_json::Value>,
}

impl StateUpdater<MACState> for Preferences {
    fn update_state(self, state: &mut MACState) {
        if let Some(internal) = self.internal {
            if let Some(tf2_dir) = internal.tf2_directory {
                let path: PathBuf = tf2_dir.into();
                state.settings.set_tf2_directory(path);
            }
            if let Some(rcon_pwd) = internal.rcon_password {
                state.settings.set_rcon_password(rcon_pwd);
            }
            if let Some(rcon_port) = internal.rcon_port {
                state.settings.set_rcon_port(rcon_port);
            }
            if let Some(steam_api_key) = internal.steam_api_key {
                state.settings.set_steam_api_key(steam_api_key);
            }
            if let Some(friends_api_usage) = internal.friends_api_usage {
                state.settings.set_friends_api_usage(friends_api_usage);
            }
            if let Some(masterbase_key) = internal.masterbase_key {
                state.settings.set_masterbase_key(masterbase_key);
            }
            if let Some(masterbase_host) = internal.masterbase_host {
                state.settings.set_masterbase_host(masterbase_host);
            }
            if let Some(autokick) = internal.dumb_autokick {
                state.settings.set_autokick_bots(autokick);
            }
            if let Some(query_sb) = internal.query_sourcebans {
                state.settings.set_sourcebans_querying(query_sb)
            }
        }

        if let Some(external) = self.external {
            state.settings.update_external_preferences(external);
        }

        state.settings.save_ok();
    }
}
