use std::sync::Arc;

use steamid_ng::SteamID;
use tappet::{errors::SteamAPIError, ExecutorResponse, SteamAPI};
use tokio::sync::mpsc::Receiver;

use crate::{
    player::{Friend, SteamInfo},
    state::State,
};

#[derive(Debug)]
pub enum Error {
    APIError(SteamAPIError),
    InvalidNumberOfResponses,
}

impl From<tappet::errors::SteamAPIError> for Error {
    fn from(value: tappet::errors::SteamAPIError) -> Self {
        Error::APIError(value)
    }
}

/// Enter a loop to wait for steam lookup requests, make those requests from the Steam web API,
/// and update the state to include that data. Intended to be run inside a new tokio::task
pub async fn steam_api_loop(mut requests: Receiver<SteamID>, api_key: Arc<str>) {
    log::debug!("Entering steam api request loop");

    let mut client = SteamAPI::new(api_key);
    loop {
        if let Some(request) = requests.recv().await {
            match request_steam_info(&mut client, request).await {
                Ok(steam_info) => State::write_state()
                    .server
                    .insert_steam_info(request, steam_info),
                Err(e) => {
                    log::error!("Can't request to steam API: {:?}", e);
                    panic!();
                }
            }
        }
    }
}

/// Make a request to the Steam web API for the chosen player and return the important steam info.
async fn request_steam_info(client: &mut SteamAPI, player: SteamID) -> Result<SteamInfo, Error> {
    log::debug!("Requesting steam account: {}", u64::from(player));

    let summary = client
        .get()
        .ISteamUser()
        .GetPlayerSummaries(vec![format!("{}", u64::from(player))])
        .execute_with_response()
        .await?;
    let summary = summary
        .response
        .players
        .get(0)
        .ok_or(Error::InvalidNumberOfResponses)?;

    let friends = client
        .get()
        .ISteamUser()
        .GetFriendList(player.into(), "all".to_string())
        .execute_with_response()
        .await?;
    let friends = friends
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
        .collect();

    let bans = client
        .get()
        .ISteamUser()
        .GetPlayerBans(vec![format!("{}", u64::from(player))])
        .execute_with_response()
        .await?;
    let bans = bans.players.get(0).ok_or(Error::InvalidNumberOfResponses)?;

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
