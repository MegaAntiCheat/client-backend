use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use event_loop::{try_get, Handled, HandlerStruct, Is, StateUpdater};
use steamid_ng::SteamID;
use tappet::{
    response_types::{
        GetFriendListResponseBase, GetPlayerBansResponseBase, GetPlayerSummariesResponseBase,
        PlayerBans, PlayerSummary,
    },
    Executor, SteamAPI,
};
use thiserror::Error;

use super::new_players::NewPlayers;
use crate::{
    player::{Friend, SteamInfo},
    player_records::Verdict,
    settings::FriendsAPIUsage,
    state::MACState,
};

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

#[derive(Debug, Clone, Copy)]
pub struct ProfileLookupBatchTick;
impl<S> StateUpdater<S> for ProfileLookupBatchTick {}

type ProfileResult = Result<Vec<(SteamID, Result<SteamInfo, SteamAPIError>)>, SteamAPIError>;

#[derive(Debug)]
pub struct ProfileLookupResult(pub ProfileResult);
impl StateUpdater<MACState> for ProfileLookupResult {
    fn update_state(self, state: &mut MACState) {
        if let Err(e) = &self.0 {
            tracing::error!("Profile lookup failed: {}", e);
            return;
        }

        for (steamid, result) in self.0.expect("Just checked it was some") {
            match result {
                Ok(steaminfo) => {
                    state.players.steam_info.insert(steamid, steaminfo);
                }
                Err(e) => {
                    tracing::error!("Faield to lookup profile for {}: {}", u64::from(steamid), e);
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct FriendLookupResult {
    steamid: SteamID,
    result: Result<Vec<Friend>, SteamAPIError>,
}
impl StateUpdater<MACState> for FriendLookupResult {
    fn update_state(self, state: &mut MACState) {
        match self.result {
            Err(_) => {
                state.players.mark_friends_list_private(self.steamid);
            }
            Ok(friends) => {
                state.players.update_friends_list(self.steamid, friends);
            }
        }
    }
}

// Handlers *************************

pub struct LookupProfiles {
    batch_buffer: VecDeque<SteamID>,
}

impl LookupProfiles {
    pub const fn new() -> Self {
        Self {
            batch_buffer: VecDeque::new(),
        }
    }
}

impl<IM, OM> HandlerStruct<MACState, IM, OM> for LookupProfiles
where
    IM: Is<NewPlayers> + Is<ProfileLookupBatchTick>,
    OM: Is<ProfileLookupResult>,
{
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        if state.settings.get_steam_api_key().is_empty() {
            return None;
        }

        if let Some(NewPlayers(new_players)) = try_get::<NewPlayers>(message) {
            self.batch_buffer.extend(new_players);
        }

        if try_get::<ProfileLookupBatchTick>(message).is_some() {
            if self.batch_buffer.is_empty() {
                return Handled::none();
            }

            let key = state.settings.get_steam_api_key();
            let batch: Vec<_> = self
                .batch_buffer
                .drain(0..BATCH_SIZE.min(self.batch_buffer.len()))
                .collect();

            return Handled::future(async move {
                let client = SteamAPI::new(key);
                Some(ProfileLookupResult(request_steam_info(&client, &batch).await).into())
            });
        }

        None
    }
}

pub struct LookupFriends;

fn lookup_players<M: Is<FriendLookupResult>>(
    api_key: &Arc<str>,
    players: &[SteamID],
) -> Option<Handled<M>> {
    let out = Handled::multiple(players.iter().map(|&p| {
        let key = api_key.clone();
        Handled::future(async move {
            let client = SteamAPI::new(key);
            Some(
                FriendLookupResult {
                    steamid: p,
                    result: request_account_friends(&client, p).await,
                }
                .into(),
            )
        })
    }));
    out
}

impl<IM, OM> HandlerStruct<MACState, IM, OM> for LookupFriends
where
    IM: Is<NewPlayers>,
    OM: Is<FriendLookupResult>,
{
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        if let Some(NewPlayers(new_players)) = try_get(message) {
            // Need all friends if there's a cheater/bot on the server with a private
            // friends list
            let need_all_friends = state.players.connected.iter().any(|p| {
                state
                    .players
                    .records
                    .get(p)
                    .is_some_and(|r| r.verdict == Verdict::Cheater || r.verdict == Verdict::Bot)
                    && state
                        .players
                        .friend_info
                        .get(p)
                        .is_some_and(|f| f.public == Some(false))
            });

            let mut queued_friendlist_req: Vec<SteamID> = Vec::new();

            for &p in new_players {
                if state.settings.get_steam_user().is_some_and(|s| p == s) {
                    queued_friendlist_req.push(p);
                    continue;
                }

                match state.settings.get_friends_api_usage() {
                    FriendsAPIUsage::CheatersOnly => {
                        let verdict = state
                            .players
                            .records
                            .get(&p)
                            .map(|r| r.verdict)
                            .unwrap_or_default();

                        if !need_all_friends
                            && (verdict == Verdict::Cheater || verdict == Verdict::Bot)
                        {
                            queued_friendlist_req.push(p);
                        }
                    }
                    FriendsAPIUsage::All => queued_friendlist_req.push(p),
                    FriendsAPIUsage::None => {}
                }
            }

            if !queued_friendlist_req.is_empty() {
                queued_friendlist_req.retain(|s| state.players.friend_info.get(s).is_some());

                return lookup_players(&state.settings.get_steam_api_key(), &queued_friendlist_req);
            }
        }

        Handled::none()
    }
}

// Utility ***************************************

/// Make a request to the Steam web API for the chosen player and return the
/// important steam info.
pub async fn request_steam_info(
    client: &SteamAPI,
    playerids: &[SteamID],
) -> Result<Vec<(SteamID, Result<SteamInfo, SteamAPIError>)>, SteamAPIError> {
    tracing::debug!("Requesting steam accounts: {:?}", playerids);

    let summaries = request_player_summary(client, playerids).await?;
    let bans = request_account_bans(client, playerids).await?;

    let id_to_summary: HashMap<_, _> = summaries
        .into_iter()
        .map(|summary| (summary.steamid.clone(), summary))
        .collect();
    let id_to_ban: HashMap<_, _> = bans
        .into_iter()
        .map(|ban| (ban.steam_id.clone(), ban))
        .collect();

    Ok(playerids
        .iter()
        .map(|&player| {
            let id = format!("{}", u64::from(player));

            let build_steam_info = || {
                let summary = id_to_summary
                    .get(&id)
                    .ok_or(SteamAPIError::MissingSummary(player))?;
                let ban = id_to_ban
                    .get(&id)
                    .ok_or(SteamAPIError::MissingBans(player))?;
                let steam_info = SteamInfo {
                    account_name: summary.personaname.clone().into(),
                    pfp_url: summary.avatarfull.clone().into(),
                    profile_url: summary.profileurl.clone().into(),
                    pfp_hash: summary.avatarhash.clone().into(),
                    profile_visibility: summary.communityvisibilitystate.into(),
                    time_created: summary.timecreated,
                    country_code: summary.loccountrycode.clone().map(Into::into),
                    vac_bans: ban.number_of_vac_bans,
                    game_bans: ban.number_of_game_bans,
                    days_since_last_ban: if ban.number_of_vac_bans > 0
                        || ban.number_of_game_bans > 0
                    {
                        Some(ban.days_since_last_ban)
                    } else {
                        None
                    },
                };
                Ok(steam_info)
            };

            (player, build_steam_info())
        })
        .collect())
}

async fn request_player_summary(
    client: &SteamAPI,
    players: &[SteamID],
) -> Result<Vec<PlayerSummary>, SteamAPIError> {
    let summaries = client
        .get()
        .ISteamUser()
        .GetPlayerSummaries(
            players
                .iter()
                .map(|player| format!("{}", u64::from(*player)))
                .collect(),
        )
        .execute()
        .await?;
    let summaries = serde_json::from_str::<GetPlayerSummariesResponseBase>(&summaries)?;
    Ok(summaries.response.players)
}

pub async fn request_account_friends(
    client: &SteamAPI,
    player: SteamID,
) -> Result<Vec<Friend>, SteamAPIError> {
    tracing::debug!(
        "Requesting friends list from Steam API for {}",
        u64::from(player)
    );
    let friends = client
        .get()
        .ISteamUser()
        .GetFriendList(player.into(), "all".to_string())
        .execute()
        .await?;
    let friends = serde_json::from_str::<GetFriendListResponseBase>(&friends)?;
    Ok(friends
        .friendslist
        .map_or(Vec::new(), |fl| fl.friends)
        .iter()
        .filter_map(|f| {
            f.steamid.parse::<u64>().map_or(None, |id| {
                Some(Friend {
                    steamid: SteamID::from(id),
                    friend_since: f.friend_since,
                })
            })
        })
        .collect())
}

async fn request_account_bans(
    client: &SteamAPI,
    players: &[SteamID],
) -> Result<Vec<PlayerBans>, SteamAPIError> {
    let bans = client
        .get()
        .ISteamUser()
        .GetPlayerBans(
            players
                .iter()
                .map(|player| format!("{}", u64::from(*player)))
                .collect(),
        )
        .execute()
        .await?;
    let bans = serde_json::from_str::<GetPlayerBansResponseBase>(&bans)?;
    Ok(bans.players)
}
