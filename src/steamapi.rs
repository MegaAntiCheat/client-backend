use std::collections::HashMap;
use std::collections::VecDeque;

use anyhow::{anyhow, Context, Result};
use steamid_ng::SteamID;
use tappet::{
    response_types::{
        GetFriendListResponseBase, GetPlayerBansResponseBase, GetPlayerSummariesResponseBase,
        PlayerBans, PlayerSummary,
    },
    Executor, SteamAPI,
};

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::{Duration, MissedTickBehavior};

use crate::player::{Friend, SteamInfo};

const BATCH_INTERVAL: Duration = Duration::from_millis(500);
const BATCH_SIZE: usize = 20; // adjust as needed

enum SteamAPIRequest {
    Lookup(SteamID),
    SetAPIKey(String),
}

pub struct SteamAPIManager {
    sender: UnboundedSender<SteamAPIRequest>,
    receiver: UnboundedReceiver<(SteamID, SteamInfo)>,
}

impl SteamAPIManager {
    pub async fn new(api_key: String) -> SteamAPIManager {
        let (req_tx, resp_rx) = SteamAPIManagerInner::new(api_key).await;

        SteamAPIManager {
            sender: req_tx,
            receiver: resp_rx,
        }
    }

    pub fn request_lookup(&self, steamid: SteamID) {
        self.sender
            .send(SteamAPIRequest::Lookup(steamid))
            .expect("API thread ded");
    }

    pub fn set_api_key(&self, api_key: String) {
        self.sender
            .send(SteamAPIRequest::SetAPIKey(api_key))
            .expect("API thread ded");
    }

    /// Attempts to receive the next response. Returns [None] if there is none ready
    pub fn try_next_response(&mut self) -> Option<(SteamID, SteamInfo)> {
        match self.receiver.try_recv() {
            Ok(response) => Some(response),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => panic!("Steam API loop ded"),
        }
    }

    /// Receives the next response. Since this just reading from a [tokio::mpsc::UnboundedReceiver]
    /// it is cancellation safe.
    pub async fn next_response(&mut self) -> (SteamID, SteamInfo) {
        self.receiver.recv().await.expect("Steam API loop ded")
    }
}

struct SteamAPIManagerInner {
    client: SteamAPI,
    batch_buffer: VecDeque<SteamID>,
    batch_timer: tokio::time::Interval,

    request_recv: UnboundedReceiver<SteamAPIRequest>,
    response_send: UnboundedSender<(SteamID, SteamInfo)>,
}

impl SteamAPIManagerInner {
    async fn new(
        api_key: String,
    ) -> (
        UnboundedSender<SteamAPIRequest>,
        UnboundedReceiver<(SteamID, SteamInfo)>,
    ) {
        let (req_tx, req_rx) = unbounded_channel();
        let (resp_tx, resp_rx) = unbounded_channel();

        let mut api_manager = SteamAPIManagerInner {
            client: SteamAPI::new(api_key),
            batch_buffer: VecDeque::with_capacity(BATCH_SIZE),
            batch_timer: tokio::time::interval(BATCH_INTERVAL),

            request_recv: req_rx,
            response_send: resp_tx,
        };
        api_manager
            .batch_timer
            .set_missed_tick_behavior(MissedTickBehavior::Delay);

        tokio::task::spawn(async move {
            api_manager.api_loop().await;
        });

        (req_tx, resp_rx)
    }

    fn set_api_key(&mut self, api_key: String) {
        self.client = SteamAPI::new(api_key);
    }

    /// Enter a loop to wait for steam lookup requests, make those requests from the Steam web API,
    /// and update the state to include that data. Intended to be run inside a new tokio::task
    async fn api_loop(&mut self) {
        loop {
            tokio::select! {
                Some(request) = self.request_recv.recv() => {
                    match request {
                        SteamAPIRequest::SetAPIKey(key) => {
                            self.set_api_key(key);
                        },
                        SteamAPIRequest::Lookup(steamid) => {
                            self.batch_buffer.push_back(steamid);
                            if self.batch_buffer.len() >= BATCH_SIZE {
                                self.send_batch().await;
                                self.batch_timer.reset();  // Reset the timer
                            }
                        }
                    }
                },
                _ = self.batch_timer.tick() => {
                    if !self.batch_buffer.is_empty() {
                        self.send_batch().await;
                    }
                }
            }
        }
    }

    async fn send_batch(&mut self) {
        match request_steam_info(&mut self.client, self.batch_buffer.drain(..).collect()).await {
            Ok(steam_info_map) => {
                for response in steam_info_map {
                    self.response_send
                        .send(response)
                        .expect("Lost connection to main thread.");
                }
            }
            Err(e) => {
                tracing::error!("Failed to get player info from SteamAPI: {:?}", e);
            }
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

pub async fn request_account_friends(
    client: &mut SteamAPI,
    player: SteamID,
) -> Result<Vec<Friend>> {
    let friends = client
        .get()
        .ISteamUser()
        .GetFriendList(player.into(), "all".to_string())
        .execute()
        .await
        .context("Failed to get account friends from SteamAPI, profile may be private.")?;
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
