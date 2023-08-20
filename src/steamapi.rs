use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use steamid_ng::SteamID;
use tappet::{
    response_types::{
        GetFriendListResponseBase, GetPlayerBansResponseBase, GetPlayerSummariesResponseBase,
        PlayerBans, PlayerSummary,
    },
    Executor, SteamAPI,
};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::{Duration, MissedTickBehavior};

use crate::{
    player::{Friend, SteamInfo},
    state::State,
};

const BATCH_INTERVAL: Duration = Duration::from_millis(500);
const BATCH_SIZE: usize = 20; // adjust as needed

/// Enter a loop to wait for steam lookup requests, make those requests from the Steam web API,
/// and update the state to include that data. Intended to be run inside a new tokio::task
pub async fn steam_api_loop(mut requests: UnboundedReceiver<SteamID>, api_key: Arc<str>) {
    tracing::debug!("Entering steam api request loop");

    let mut client = SteamAPI::new(api_key);
    let mut buffer: VecDeque<SteamID> = VecDeque::new();
    let mut batch_timer = tokio::time::interval(BATCH_INTERVAL);
    batch_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            Some(request) = requests.recv() => {
                buffer.push_back(request);
                if buffer.len() >= BATCH_SIZE {
                    send_batch(&mut client, &mut buffer).await;
                    batch_timer.reset();  // Reset the timer
                }
            },
            _ = batch_timer.tick() => {
                if !buffer.is_empty() {
                    send_batch(&mut client, &mut buffer).await;
                }
            }
        }
    }
}

async fn send_batch(client: &mut SteamAPI, buffer: &mut VecDeque<SteamID>) {
    match request_steam_info(client, buffer.drain(..).collect()).await {
        Ok(steam_info_map) => {
            let mut state = State::write_state();
            for (id, steam_info) in steam_info_map {
                state.server.insert_steam_info(id, steam_info);
            }
        }
        Err(e) => {
            tracing::error!("Failed to get player info from SteamAPI: {:?}", e);
        }
    }
}

/// Make a request to the Steam web API for the chosen player and return the important steam info.
async fn request_steam_info(
    client: &mut SteamAPI,
    players: Vec<SteamID>,
) -> Result<Vec<(SteamID, SteamInfo)>> {
    tracing::debug!("Requesting steam accounts: {:?}", players);

    let summaries = request_player_summary(client, &players).await?;
    let bans = request_account_bans(client, &players).await?;

    // let friends = match request_account_friends(client, player).await {
    //     Ok(friends) => friends,
    //     Err(e) => {
    //         if summary.communityvisibilitystate == 3 {
    //             tracing::warn!(
    //                 "Friends could not be retrieved from public profile {}: {:?}",
    //                 u64::from(player),
    //                 e
    //             );
    //         }
    //         Vec::new()
    //     }
    // };
    let id_to_summary: HashMap<_, _> = summaries
        .into_iter()
        .map(|summary| (summary.steamid.clone(), summary))
        .collect();
    let id_to_ban: HashMap<_, _> = bans
        .into_iter()
        .map(|ban| (ban.steam_id.clone(), ban))
        .collect();

    let steam_infos = players
        .into_iter()
        .map(|player| {
            let id = format!("{}", u64::from(player));
            let summary = id_to_summary
                .get(&id)
                .ok_or(anyhow!("Missing summary for player {}", id))?;
            let ban = id_to_ban
                .get(&id)
                .ok_or(anyhow!("Missing ban info for player {}", id))?;

            let steam_info = SteamInfo {
                account_name: summary.personaname.clone().into(),
                pfp_url: summary.avatarfull.clone().into(),
                profile_url: summary.profileurl.clone().into(),
                pfp_hash: summary.avatarhash.clone().into(),
                profile_visibility: summary.communityvisibilitystate.into(),
                time_created: summary.timecreated,
                country_code: summary.loccountrycode.clone().map(|s| s.into()),
                vac_bans: ban.number_of_vac_bans,
                game_bans: ban.number_of_game_bans,
                days_since_last_ban: if ban.number_of_vac_bans > 0 || ban.number_of_game_bans > 0 {
                    Some(ban.days_since_last_ban)
                } else {
                    None
                },
            };
            Ok((player, steam_info))
        })
        .collect::<Result<_>>()?;

    Ok(steam_infos)
}

async fn request_player_summary(
    client: &mut SteamAPI,
    players: &[SteamID],
) -> Result<Vec<PlayerSummary>> {
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
        .await
        .context("Failed to get player summary from SteamAPI.")?;
    let summaries = serde_json::from_str::<GetPlayerSummariesResponseBase>(&summaries)
        .with_context(|| {
            format!(
                "Failed to parse player summary from SteamAPI: {}",
                &summaries
            )
        })?;
    Ok(summaries.response.players)
}

// Needs to be updated to accomodate batched SteamID requests
#[allow(dead_code)]
async fn request_account_friends(client: &mut SteamAPI, player: SteamID) -> Result<Vec<Friend>> {
    let friends = client
        .get()
        .ISteamUser()
        .GetFriendList(player.into(), "all".to_string())
        .execute()
        .await
        .context("Failed to get account friends from SteamAPI.")?;
    let friends =
        serde_json::from_str::<GetFriendListResponseBase>(&friends).with_context(|| {
            format!(
                "Failed to parse account friends from SteamAPI: {}",
                &friends
            )
        })?;
    Ok(friends
        .friendslist
        .map(|fl| fl.friends)
        .unwrap_or(Vec::new())
        .iter()
        .filter_map(|f| match f.steamid.parse::<u64>() {
            Err(_) => None,
            Ok(id) => Some(Friend {
                steamid: SteamID::from(id),
                friend_since: f.friend_since,
            }),
        })
        .collect())
}

async fn request_account_bans(
    client: &mut SteamAPI,
    players: &[SteamID],
) -> Result<Vec<PlayerBans>> {
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
        .await
        .context("Failed to get player bans from SteamAPI")?;
    let bans = serde_json::from_str::<GetPlayerBansResponseBase>(&bans)
        .with_context(|| format!("Failed to parse player bans from SteamAPI: {}", &bans))?;
    Ok(bans.players)
}
