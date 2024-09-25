use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use chrono::Utc;
use event_loop::{try_get, Handled, Is, Message, MessageHandler};
use steam_rs::{
    steam_user::{get_friend_list, get_player_bans, get_player_summaries},
    Steam,
};
use steamid_ng::SteamID;
use thiserror::Error;
use tokio::task::JoinSet;

use super::new_players::NewPlayers;
use crate::{
    events::{InternalPreferences, Preferences, UserUpdates},
    gamefinder::TF2_GAME_ID,
    player::{Friend, SteamInfo},
    player_records::{PlayerRecord, Verdict},
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
    SteamAPIPlayerService(#[from] steam_rs::errors::PlayerServiceError),
    #[error(transparent)]
    SteamAPIUser(#[from] steam_rs::errors::SteamUserError),
    #[error("Player does not own TF2")]
    GameNotOwned,
}

// Messages *************************

#[derive(Debug, Clone, Copy)]
pub struct ProfileLookupBatchTick;
impl<S> event_loop::Message<S> for ProfileLookupBatchTick {}

type ProfileResult = Result<Vec<(SteamID, Result<SteamInfo, SteamAPIError>)>, SteamAPIError>;

#[derive(Debug)]
pub struct ProfileLookupResult(pub ProfileResult);
impl Message<MACState> for ProfileLookupResult {
    fn update_state(self, state: &mut MACState) {
        let results = match &self.0 {
            Err(e) => {
                tracing::error!("Profile lookup failed: {e}");
                return;
            }
            Ok(results) => results,
        };

        for (steamid, result) in results {
            match result {
                Ok(steaminfo) => {
                    if let Some(r) = state.players.records.get_mut(steamid) {
                        r.add_previous_name(&steaminfo.account_name);
                    }
                    state.players.steam_info.insert(*steamid, steaminfo.clone());
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to lookup profile for {}: {}",
                        u64::from(*steamid),
                        e
                    );
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
impl Message<MACState> for FriendLookupResult {
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

#[derive(Debug)]
pub enum ProfileLookupRequest {
    Single(SteamID),
    Multiple(Vec<SteamID>),
}

impl<S> Message<S> for ProfileLookupRequest {}

// Handlers *************************

pub struct LookupProfiles {
    batch_buffer: VecDeque<SteamID>,
    in_progress: Vec<SteamID>,
}

impl LookupProfiles {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            batch_buffer: VecDeque::new(),
            in_progress: Vec::new(),
        }
    }
}

impl Default for LookupProfiles {
    fn default() -> Self {
        Self::new()
    }
}

impl<IM, OM> MessageHandler<MACState, IM, OM> for LookupProfiles
where
    IM: Is<NewPlayers> + Is<ProfileLookupBatchTick> + Is<Preferences> + Is<ProfileLookupRequest>,
    OM: Is<ProfileLookupResult>,
{
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        // Re-request connected players if the API key has changed
        if let Some(Preferences {
            internal:
                Some(InternalPreferences {
                    steam_api_key: Some(new_key),
                    ..
                }),
            external: _,
        }) = try_get(message)
        {
            if new_key.is_empty() {
                self.batch_buffer.clear();
                return Handled::none();
            }

            self.batch_buffer.extend(&state.players.connected);
        }

        // Don't request anything if there's no API key
        if state.settings.steam_api_key().is_empty() {
            return None;
        }

        // Request new players
        if let Some(NewPlayers(new_players)) = try_get::<NewPlayers>(message) {
            self.batch_buffer.extend(new_players);
        }

        // Request specifically-requested accounts
        if let Some(lookup) = try_get::<ProfileLookupRequest>(message) {
            match lookup {
                ProfileLookupRequest::Single(p) => self.batch_buffer.push_back(*p),
                ProfileLookupRequest::Multiple(ps) => self.batch_buffer.extend(ps),
            }
        }

        // Send of lookup batch
        if try_get::<ProfileLookupBatchTick>(message).is_some() {
            self.batch_buffer.retain(|s| {
                // Already retrieving
                if self.in_progress.contains(s) {
                    return false;
                }

                // Already present and reasonably recent
                !state
                    .players
                    .steam_info
                    .get(s)
                    .is_some_and(|si| !si.expired())
            });
            if self.batch_buffer.is_empty() {
                return Handled::none();
            }

            let batch: Vec<_> = self
                .batch_buffer
                .drain(0..BATCH_SIZE.min(self.batch_buffer.len()))
                .collect();

            self.in_progress.extend_from_slice(&batch);

            let client = Arc::new(Steam::new(state.settings.steam_api_key()));
            let request_playtime = state.settings.request_playtime();
            return Handled::future(async move {
                Some(
                    ProfileLookupResult(request_steam_info(client, &batch, request_playtime).await)
                        .into(),
                )
            });
        }

        None
    }
}

pub struct LookupFriends {
    in_progess: Vec<SteamID>,
}

impl LookupFriends {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            in_progess: Vec::new(),
        }
    }

    fn lookup_players<'a, M: Is<FriendLookupResult>>(
        &mut self,
        key: &str,
        players: impl IntoIterator<Item = &'a SteamID>,
    ) -> Option<Handled<M>> {
        Handled::multiple(players.into_iter().map(|&p| {
            self.in_progess.push(p);
            let client = Steam::new(key);
            Handled::future(async move {
                Some(
                    FriendLookupResult {
                        steamid: p,
                        result: request_account_friends(&client, p).await,
                    }
                    .into(),
                )
            })
        }))
    }

    /// Takes a list of steamids and does friend lookups on the ones which fit
    /// the current friend lookup policy and the circumstances deem it
    /// necessary.
    ///
    /// Argument `force`: Force lookup of all players, e.g. if a cheater's
    /// friend lookup failed but that has not yet been propagated to the
    /// state
    fn handle_players<'a, M: Is<FriendLookupResult>>(
        &mut self,
        state: &MACState,
        players: impl IntoIterator<Item = &'a SteamID>,
        policy: FriendsAPIUsage,
        key: &str,
        force: bool,
    ) -> Option<Handled<M>> {
        // Need all friends if there's a cheater/bot on the server with a private
        // friends list
        let need_all_friends = force
            || state.players.connected.iter().any(|p| {
                let verdict = state.players.verdict(*p);
                (verdict == Verdict::Cheater || verdict == Verdict::Bot)
                    && state
                        .players
                        .friend_info
                        .get(p)
                        .is_some_and(|f| f.public == Some(false))
            });

        let mut queued_friendlist_req: Vec<SteamID> = Vec::new();

        for &p in players {
            // Lookup user regardless of policy
            if state.settings.steam_user().is_some_and(|s| p == s) {
                queued_friendlist_req.push(p);
                continue;
            }

            match policy {
                FriendsAPIUsage::CheatersOnly => {
                    let verdict = state
                        .players
                        .records
                        .get(&p)
                        .map(PlayerRecord::verdict)
                        .unwrap_or_default();

                    if need_all_friends || verdict == Verdict::Cheater || verdict == Verdict::Bot {
                        queued_friendlist_req.push(p);
                    }
                }
                FriendsAPIUsage::All => queued_friendlist_req.push(p),
                FriendsAPIUsage::None => {}
            }
        }

        queued_friendlist_req.retain(|s| {
            !state
                .players
                .friend_info
                .get(s)
                .is_some_and(|f| f.public.is_some())
                && !self.in_progess.contains(s)
        });

        if queued_friendlist_req.is_empty() {
            return Handled::none();
        }

        self.lookup_players(key, &queued_friendlist_req)
    }
}

impl Default for LookupFriends {
    fn default() -> Self {
        Self::new()
    }
}

impl<IM, OM> MessageHandler<MACState, IM, OM> for LookupFriends
where
    IM: Is<NewPlayers> + Is<FriendLookupResult> + Is<UserUpdates> + Is<Preferences>,
    OM: Is<FriendLookupResult>,
{
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        if state.settings.steam_api_key().is_empty() {
            return Handled::none();
        }

        if let Some(NewPlayers(new_players)) = try_get(message) {
            return self.handle_players(
                state,
                new_players,
                state.settings.friends_api_usage(),
                state.settings.steam_api_key(),
                false,
            );
        }

        // Lookup all players if it failed to get the friends list of a cheater and
        // we're using CheatersOnly policy
        if let Some(FriendLookupResult { steamid, result }) = try_get(message) {
            let is_bot_or_cheater = {
                let verdict = state.players.verdict(*steamid);
                verdict == Verdict::Bot || verdict == Verdict::Cheater
            };

            let out = if is_bot_or_cheater && result.is_err() {
                self.handle_players::<OM>(
                    state,
                    &state.players.connected,
                    state.settings.friends_api_usage(),
                    state.settings.steam_api_key(),
                    true,
                )
            } else {
                Handled::none()
            };

            self.in_progess.retain(|s| s != steamid);
            return out;
        }

        // Lookup any players that might need to be after a change to their verdicts
        if let Some(UserUpdates(users)) = try_get(message) {
            let policy = state.settings.friends_api_usage();
            let mut out = Vec::new();

            for (k, v) in users {
                if let Some(new_verdict) = v.local_verdict {
                    if !policy.lookup(new_verdict) {
                        continue;
                    }

                    // Lookup all if player was marked as a bot or cheater and we've already failed
                    // to get their info
                    let lookup_all_players = policy == FriendsAPIUsage::CheatersOnly
                        && state
                            .players
                            .friend_info
                            .get(k)
                            .is_some_and(|f| f.public.is_some_and(|i| !i));

                    if lookup_all_players {
                        out.push(self.handle_players(
                            state,
                            &state.players.connected,
                            state.settings.friends_api_usage(),
                            state.settings.steam_api_key(),
                            true,
                        ));
                    } else {
                        out.push(self.handle_players(
                            state,
                            &[*k],
                            state.settings.friends_api_usage(),
                            state.settings.steam_api_key(),
                            true,
                        ));
                    }
                }
            }

            return Handled::multiple(out);
        }

        // Do any lookups we might need to because of changing policy or steam API key
        if let Some(Preferences {
            internal: Some(internal),
            external: _,
        }) = try_get(message)
        {
            if internal.friends_api_usage.is_none() && internal.steam_api_key.is_none() {
                return Handled::none();
            }

            let policy = internal
                .friends_api_usage
                .unwrap_or_else(|| state.settings.friends_api_usage());
            let key = internal
                .steam_api_key
                .as_deref()
                .unwrap_or_else(|| state.settings.steam_api_key());

            return self.handle_players(state, &state.players.connected, policy, key, false);
        }

        Handled::none()
    }
}

// Utility ***************************************

/// Make a request to the Steam web API for the chosen player and return the
/// important steam info.
///
/// # Errors
/// Returns `Err` if the overall api request failed.
/// Individual elements in the Vec may be `Err` if specific accounts were not
/// found or failed to parse.
pub async fn request_steam_info(
    client: Arc<Steam>,
    playerids: &[SteamID],
    include_playtime: bool,
) -> Result<Vec<(SteamID, Result<SteamInfo, SteamAPIError>)>, SteamAPIError> {
    tracing::debug!("Requesting steam accounts: {:?}", playerids);

    let summaries = request_player_summary(&client, playerids).await?;
    let bans = request_account_bans(&client, playerids).await?;

    let playtimes = if include_playtime && !playerids.is_empty() {
        let mut join_handles: JoinSet<(SteamID, Result<u64, SteamAPIError>)> = JoinSet::new();

        for p in playerids {
            let client = client.clone();
            let p = *p;
            join_handles.spawn(async move {
                let playtime = request_game_playtime(&client, p).await;
                (p, playtime)
            });
        }

        let mut playtimes = Vec::new();
        while let Some(playtime) = join_handles.join_next().await {
            let Ok(playtime) = playtime else {
                continue;
            };
            playtimes.push(playtime);
        }

        playtimes
    } else {
        Vec::new()
    };

    let id_to_summary: HashMap<_, _> = summaries
        .into_iter()
        .map(|summary| (format!("{}", summary.steam_id.into_u64()), summary))
        .collect();
    let id_to_ban: HashMap<_, _> = bans
        .into_iter()
        .map(|ban| (ban.steam_id.clone(), ban))
        .collect();
    let id_to_playtime: HashMap<_, _> = playtimes
        .into_iter()
        .filter_map(|(s, r)| r.ok().map(|r| (s, r)))
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
                    account_name: summary.persona_name.clone(),
                    pfp_url: summary.avatar_full.clone(),
                    profile_url: summary.profile_url.clone(),
                    pfp_hash: summary.avatar_hash.clone(),
                    profile_visibility: summary.community_visibility_state.into(),
                    time_created: summary.time_created,
                    country_code: summary.loc_country_code.clone().map(Into::into),
                    vac_bans: ban.number_of_vac_bans,
                    game_bans: ban.number_of_game_bans,
                    days_since_last_ban: if ban.number_of_vac_bans > 0
                        || ban.number_of_game_bans > 0
                    {
                        Some(ban.days_since_last_ban)
                    } else {
                        None
                    },
                    playtime: id_to_playtime.get(&player).copied(),
                    fetched: Utc::now(),
                };
                Ok(steam_info)
            };

            (player, build_steam_info())
        })
        .collect())
}

async fn request_player_summary(
    client: &Steam,
    players: &[SteamID],
) -> Result<Vec<get_player_summaries::Player>, SteamAPIError> {
    let steamids = players
        .iter()
        .map(|s| steam_rs::steam_id::SteamId::new(u64::from(*s)))
        .collect();
    Ok(client.get_player_summaries(steamids).await?)
}

/// # Errors
/// If the API request failed, the account does not expose their friends list,
/// or the account does not exist.
pub async fn request_account_friends(
    client: &Steam,
    player: SteamID,
) -> Result<Vec<Friend>, SteamAPIError> {
    tracing::debug!(
        "Requesting friends list from Steam API for {}",
        u64::from(player)
    );
    let steamid = steam_rs::steam_id::SteamId::new(u64::from(player));
    let friends = client
        .get_friend_list(steamid, Some(get_friend_list::Relationship::All))
        .await?;

    Ok(friends
        .into_iter()
        .map(|f| Friend {
            steamid: SteamID::from(f.steam_id.into_u64()),
            #[allow(clippy::cast_lossless)]
            friend_since: f.friend_since as u64,
        })
        .collect())
}

async fn request_account_bans(
    client: &Steam,
    players: &[SteamID],
) -> Result<Vec<get_player_bans::Player>, SteamAPIError> {
    let steamids = players
        .iter()
        .map(|s| steam_rs::steam_id::SteamId::new(u64::from(*s)))
        .collect();

    let bans = client.get_player_bans(steamids).await?;

    Ok(bans)
}

async fn request_game_playtime(client: &Steam, player: SteamID) -> Result<u64, SteamAPIError> {
    let steamid = steam_rs::steam_id::SteamId::new(u64::from(player));
    let game = client
        .get_owned_games(steamid, false, false, vec![TF2_GAME_ID], false)
        .await?
        .games
        .into_iter()
        .find(|g| g.appid == TF2_GAME_ID);

    game.map(|g| g.playtime_forever)
        .ok_or(SteamAPIError::GameNotOwned)
}
