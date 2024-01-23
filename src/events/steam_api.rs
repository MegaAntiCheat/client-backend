use event_loop::{try_get, Handled, HandlerStruct, Is, StateUpdater};
use steamid_ng::SteamID;
use tappet::SteamAPI;
use thiserror::Error;

use crate::{
    player::{Friend, SteamInfo},
    settings::FriendsAPIUsage,
    state::MACState,
    steamapi,
};

use super::new_players::NewPlayers;

const BATCH_SIZE: usize = 20; // adjust as needed

#[derive(Debug, Error)]
pub enum SteamAPIError {
    #[error("Missing bans for player {0:?}")]
    MissingBans(SteamID),
    #[error("Missing summary for player {0:?}")]
    MissingSummary(SteamID),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    Tappet(#[from] tappet::errors::SteamAPIError),
}

// Messages *************************

pub struct ProfileLookupBatchTick;
impl<S> StateUpdater<S> for ProfileLookupBatchTick {}

pub struct ProfileLookupResult(
    pub Result<Vec<(SteamID, Result<SteamInfo, SteamAPIError>)>, SteamAPIError>,
);
impl StateUpdater<MACState> for ProfileLookupResult {
    fn update_state(self, state: &mut MACState) {
        if let Err(e) = &self.0 {
            tracing::error!("Profile lookup failed: {}", e);
            return;
        }

        for (steamid, result) in self.0.unwrap() {
            match result {
                Ok(steaminfo) => {
                    state
                        .server
                        .players_mut()
                        .steam_info
                        .insert(steamid, steaminfo);
                }
                Err(e) => {
                    tracing::error!("Faield to lookup profile for {}: {}", u64::from(steamid), e);
                }
            }
        }
    }
}

pub struct FriendLookupResult {
    steamid: SteamID,
    result: Result<Vec<Friend>, SteamAPIError>,
}
impl StateUpdater<MACState> for FriendLookupResult {
    fn update_state(self, state: &mut MACState) {
        match self.result {
            Err(_) => {
                state
                    .server
                    .players_mut()
                    .mark_friends_list_private(&self.steamid);
            }
            Ok(friends) => {
                state
                    .server
                    .players_mut()
                    .update_friends_list(self.steamid, friends);
            }
        }
    }
}

// Handlers *************************

pub struct LookupProfiles {
    batch_buffer: Vec<SteamID>,
}

impl LookupProfiles {
    pub fn new() -> LookupProfiles {
        LookupProfiles {
            batch_buffer: Vec::new(),
        }
    }
}

impl<IM, OM> HandlerStruct<MACState, IM, OM> for LookupProfiles
where
    IM: Is<NewPlayers> + Is<ProfileLookupBatchTick>,
    OM: Is<ProfileLookupResult>,
{
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        let NewPlayers(new_players) = try_get::<NewPlayers>(message)?;
        if state.settings.get_steam_api_key().is_empty() {
            return None;
        }

        self.batch_buffer.extend(new_players);

        // TODO - manage batch

        let key = state.settings.get_steam_api_key().clone();
        let batch: Vec<_> = self
            .batch_buffer
            .drain(0..BATCH_SIZE.min(self.batch_buffer.len()))
            .collect();

        Handled::future(async move {
            let mut client = SteamAPI::new(key);
            ProfileLookupResult(steamapi::request_steam_info(&mut client, &batch).await).into()
        })
    }
}

pub struct LookupFriends {
    policy: FriendsAPIUsage,
}

impl LookupFriends {
    pub fn new(policy: FriendsAPIUsage) -> LookupFriends {
        LookupFriends { policy }
    }
}

impl<IM, OM> HandlerStruct<MACState, IM, OM> for LookupFriends
where
    IM: Is<NewPlayers>,
    OM: Is<FriendLookupResult>,
{
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        todo!("Friend lookup policy")
    }
}
