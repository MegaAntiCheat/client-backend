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
use tokio::sync::mpsc::Receiver;

use crate::{
    player::{Friend, SteamInfo},
    state::State,
};

/// Enter a loop to wait for steam lookup requests, make those requests from the Steam web API,
/// and update the state to include that data. Intended to be run inside a new tokio::task
pub async fn steam_api_loop(mut requests: Receiver<SteamID>, api_key: Arc<str>) {
    tracing::debug!("Entering steam api request loop");

    let mut client = SteamAPI::new(api_key);
    loop {
        if let Some(request) = requests.recv().await {
            match request_steam_info(&mut client, request).await {
                Ok(steam_info) => State::write_state()
                    .server
                    .insert_steam_info(request, steam_info),
                Err(e) => {
                    tracing::error!("Failed to get player info from SteamAPI: {:?}", e);
                }
            }
        }
    }
}

/// Make a request to the Steam web API for the chosen player and return the important steam info.
async fn request_steam_info(client: &mut SteamAPI, player: SteamID) -> Result<SteamInfo> {
    tracing::debug!("Requesting steam account: {}", u64::from(player));

    let summary = request_player_summary(client, player).await?;
    let friends = match request_account_friends(client, player).await {
        Ok(friends) => friends,
        Err(e) => {
            if summary.communityvisibilitystate == 3 {
                tracing::warn!(
                    "Friends could not be retrieved from public profile {}: {:?}",
                    u64::from(player),
                    e
                );
            }
            Vec::new()
        }
    };
    let bans = request_account_bans(client, player).await?;

    Ok(SteamInfo {
        account_name: summary.personaname.clone().into(),
        pfp_url: summary.avatarfull.clone().into(),
        profile_url: summary.profileurl.clone().into(),
        pfp_hash: summary.avatarhash.clone().into(),
        profile_visibility: summary.communityvisibilitystate.into(),
        time_created: summary.timecreated,
        country_code: summary.loccountrycode.clone().map(|s| s.into()),

        vac_bans: bans.number_of_vac_bans,
        game_bans: bans.number_of_game_bans,
        days_since_last_ban: if bans.number_of_vac_bans > 0 || bans.number_of_game_bans > 0 {
            Some(bans.days_since_last_ban)
        } else {
            None
        },

        friends,
    })
}

async fn request_player_summary(client: &mut SteamAPI, player: SteamID) -> Result<PlayerSummary> {
    let summary = client
        .get()
        .ISteamUser()
        .GetPlayerSummaries(vec![format!("{}", u64::from(player))])
        .execute()
        .await
        .context("Failed to get player summary from SteamAPI.")?;
    let mut summary = serde_json::from_str::<GetPlayerSummariesResponseBase>(&summary)
        .with_context(|| format!("Failed to parse player summary from SteamAPI: {}", &summary))?;
    if summary.response.players.is_empty() {
        return Err(anyhow!(
            "Invalid number of responses from player summary request"
        ));
    }
    Ok(summary.response.players.remove(0))
}

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

async fn request_account_bans(client: &mut SteamAPI, player: SteamID) -> Result<PlayerBans> {
    let bans = client
        .get()
        .ISteamUser()
        .GetPlayerBans(vec![format!("{}", u64::from(player))])
        .execute()
        .await
        .context("Failed to get player bans from SteamAPI")?;
    let mut bans = serde_json::from_str::<GetPlayerBansResponseBase>(&bans)
        .with_context(|| format!("Failed to parse player bans from SteamAPI: {}", &bans))?;
    if bans.players.is_empty() {
        return Err(anyhow!(
            "Invalid number of responses from account bans request"
        ));
    }
    Ok(bans.players.remove(0))
}
