use serde::{Serialize, Serializer};
use std::{
    collections::{HashMap, VecDeque},
    ops::{Deref, DerefMut, Range},
    sync::Arc,
};
use steamid_ng::SteamID;

use crate::{
    io::{
        g15,
        regexes::{self, ChatMessage, PlayerKill, StatusLine},
        IOOutput,
    },
    player::{Player, SteamInfo},
    player_records::{PlayerRecord, PlayerRecords},
};

const MAX_HISTORY_LEN: usize = 100;

// Server

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    map: Option<Arc<str>>,
    ip: Option<Arc<str>>,
    hostname: Option<Arc<str>>,
    max_players: Option<u32>,
    num_players: Option<u32>,
    #[serde(serialize_with = "serialize_player_map")]
    players: HashMap<SteamID, Player>,
    #[serde(skip)]
    player_history: VecDeque<Player>,
    gamemode: Option<Gamemode>,

    #[serde(skip)]
    player_records: PlayerRecords,
}

#[derive(Debug, Serialize)]
pub struct Gamemode {
    pub matchmaking: bool,
    #[serde(rename = "type")]
    pub game_type: Arc<str>,
    pub vanilla: bool,
}

impl Server {
    pub fn new(playerlist: PlayerRecords) -> Server {
        Server {
            map: None,
            ip: None,
            hostname: None,
            max_players: None,
            num_players: None,
            players: HashMap::new(),
            player_history: VecDeque::with_capacity(MAX_HISTORY_LEN),
            gamemode: None,

            player_records: playerlist,
        }
    }

    /// Handles any io output from running commands / reading the console log file.
    /// Returns:
    /// * Some<[SteamID]> of a player if they have been newly added to the server.
    pub fn handle_io_output(&mut self, response: IOOutput, user: Option<SteamID>) -> NewPlayers {
        use IOOutput::*;
        match response {
            NoOutput => {}
            G15(players) => return self.handle_g15_parse(players, user).into(),
            Status(status) => {
                return self.handle_status_line(status, user).into();
            }
            MultiStatus(status_lines) => {
                let mut new_players = Vec::new();
                for status in status_lines {
                    if let Some(new_player) = self.handle_status_line(status, user) {
                        new_players.push(new_player);
                    }
                }
                return new_players.into();
            }
            Chat(chat) => self.handle_chat(chat),
            Kill(kill) => self.handle_kill(kill),
            Hostname(regexes::Hostname(hostname)) => {
                self.hostname = Some(hostname);
            }
            ServerIP(regexes::ServerIP(ip)) => {
                self.ip = Some(ip);
            }
            Map(regexes::Map(map)) => {
                self.map = Some(map);
            }
            PlayerCount(playercount) => {
                self.max_players = Some(playercount.max);
                self.num_players = Some(playercount.players);
            }
        }

        NewPlayers::None
    }

    /// Moves any old players from the server into history. Any console commands (status, g15_dumpplayer, etc)
    /// should be run before calling this function again to prevent removing all players from the player list.
    pub fn refresh(&mut self) {
        // Get old players
        let unaccounted_players: Vec<Player> = self
            .players
            .values()
            .filter(|p| !p.game_info.accounted)
            .cloned()
            .collect();

        // Remove old players from server
        for p in &unaccounted_players {
            self.players.remove(&p.steamid);
        }

        // Remove any of them from the history as they will be added more recently
        self.player_history
            .retain(|p| !unaccounted_players.iter().any(|up| up.steamid == p.steamid));

        // Shrink to not go past max number of players
        let num_players = self.player_history.len() + unaccounted_players.len();
        for _ in MAX_HISTORY_LEN..num_players {
            self.player_history.pop_front();
        }

        for p in unaccounted_players {
            self.player_history.push_back(p);
        }

        // Mark all remaining players as unaccounted, they will be marked as accounted again
        // when they show up in status or another console command.
        for p in self.players.values_mut() {
            p.game_info.accounted = false;
        }
    }

    /// Add the provided SteamInfo to the given player. Returns true if that player was
    /// found in the server.
    pub fn insert_steam_info(&mut self, player: SteamID, info: SteamInfo) -> bool {
        let mut found = false;

        if let Some(player) = self.players.get_mut(&player) {
            player.steam_info = Some(info.clone());
            found = true;
        }

        if let Some(player) = self.player_history.iter_mut().find(|p| p.steamid == player) {
            player.steam_info = Some(info);
            found = true;
        }

        found
    }

    pub fn get_history(&self, range: Range<usize>) -> Vec<&Player> {
        self.player_history
            .iter()
            .rev()
            .skip(range.start)
            .take(range.end - range.start)
            .collect()
    }

    // Player records

    pub fn has_player_record(&self, steamid: &SteamID) -> bool {
        self.player_records.records.contains_key(steamid)
    }

    pub fn insert_player_record(&mut self, record: PlayerRecord) {
        self.player_records.records.insert(record.steamid, record);
    }

    #[allow(dead_code)]
    pub fn get_player_record(&self, steamid: &SteamID) -> Option<&PlayerRecord> {
        self.player_records.records.get(steamid)
    }

    pub fn get_player_record_mut(&mut self, steamid: &SteamID) -> Option<PlayerRecordLock> {
        if self.player_records.records.contains_key(steamid) {
            Some(PlayerRecordLock {
                steamid: *steamid,
                players: &mut self.players,
                history: &mut self.player_history,
                playerlist: &mut self.player_records,
            })
        } else {
            None
        }
    }

    // Other

    fn handle_g15_parse(
        &mut self,
        players: Vec<g15::G15Player>,
        user: Option<SteamID>,
    ) -> Vec<SteamID> {
        let mut new_players = Vec::new();
        for pl in players {
            if let Some(steamid) = &pl.steamid {
                // Update existing player
                if let Some(player) = self.players.get_mut(steamid) {
                    if let Some(scr) = pl.score {
                        player.game_info.kills = scr;
                    }
                    if let Some(dth) = pl.deaths {
                        player.game_info.deaths = dth;
                    }
                    if let Some(png) = pl.ping {
                        player.game_info.ping = png;
                    }
                    if let Some(tm) = pl.team {
                        player.game_info.team = tm;
                    }
                    if let Some(uid) = pl.userid {
                        player.game_info.userid = uid;
                    }
                    player.game_info.accounted = true;
                } else if let Some(mut player) = Player::new_from_g15(&pl, user) {
                    if let Some(record) = self.player_records.records.get(steamid) {
                        player.update_from_record(record.clone());
                    }

                    self.players.insert(*steamid, player);
                    new_players.push(*steamid);
                }
            }
        }

        new_players
    }

    /// Given a status line, update an existing or add a new one to the server.
    /// Returns the SteamID if a new player was created.
    fn handle_status_line(&mut self, status: StatusLine, user: Option<SteamID>) -> Option<SteamID> {
        // Update existing player or insert new player
        if let Some(player) = self.players.get_mut(&status.steamid) {
            // Update existing player
            player.name = status.name;
            player.game_info.userid = status.userid;
            player.game_info.ping = status.ping;
            player.game_info.loss = status.loss;
            player.game_info.state = status.state;
            player.game_info.time = status.time;
            player.game_info.accounted = true;
            None
        } else {
            // Create and insert new player
            let mut player = Player::new_from_status(&status, user);

            if let Some(record) = self.player_records.records.get(&status.steamid) {
                player.update_from_record(record.clone());
            }

            self.players.insert(status.steamid, player);
            Some(status.steamid)
        }
    }

    fn handle_chat(&mut self, chat: ChatMessage) {
        // TODO
        tracing::debug!("Chat: {:?}", chat);
    }

    fn handle_kill(&mut self, kill: PlayerKill) {
        // TODO
        tracing::debug!("Kill: {:?}", kill);
    }
}

pub struct PlayerRecordLock<'a> {
    playerlist: &'a mut PlayerRecords,
    players: &'a mut HashMap<SteamID, Player>,
    history: &'a mut VecDeque<Player>,
    steamid: SteamID,
}

impl Deref for PlayerRecordLock<'_> {
    type Target = PlayerRecord;

    fn deref(&self) -> &Self::Target {
        self.playerlist
            .records
            .get(&self.steamid)
            .expect("Mutating player record.")
    }
}

impl DerefMut for PlayerRecordLock<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.playerlist
            .records
            .get_mut(&self.steamid)
            .expect("Reading player record.")
    }
}

// Update all players the server has with the updated record
impl Drop for PlayerRecordLock<'_> {
    fn drop(&mut self) {
        let record = self
            .playerlist
            .records
            .get(&self.steamid)
            .expect("Reading player record");

        // Update server players and history
        if let Some(p) = self.players.get_mut(&self.steamid) {
            p.update_from_record(record.clone());
        }

        if let Some(p) = self.history.iter_mut().find(|p| p.steamid == self.steamid) {
            p.update_from_record(record.clone());
        }

        // Update playerlist
        if record.is_empty() {
            self.playerlist.records.remove(&self.steamid);
        }

        if let Err(e) = self.playerlist.save() {
            tracing::error!("Failed to save playerlist: {:?}", e);
        }
    }
}

pub enum NewPlayers {
    Single(SteamID),
    Multiple(Vec<SteamID>),
    None,
}

impl From<Option<SteamID>> for NewPlayers {
    fn from(value: Option<SteamID>) -> Self {
        if let Some(steamid) = value {
            NewPlayers::Single(steamid)
        } else {
            NewPlayers::None
        }
    }
}

impl From<Vec<SteamID>> for NewPlayers {
    fn from(value: Vec<SteamID>) -> Self {
        NewPlayers::Multiple(value)
    }
}

pub struct NewPlayersIterator<'a> {
    players: &'a NewPlayers,
    index: usize,
}

impl Iterator for NewPlayersIterator<'_> {
    type Item = SteamID;

    fn next(&mut self) -> Option<Self::Item> {
        match &self.players {
            NewPlayers::Single(s) => {
                if self.index == 0 {
                    self.index += 1;
                    Some(*s)
                } else {
                    None
                }
            }
            NewPlayers::Multiple(s) => {
                self.index += 1;
                if self.index <= s.len() {
                    Some(s[self.index - 1])
                } else {
                    None
                }
            }
            NewPlayers::None => None,
        }
    }
}

impl<'a> IntoIterator for &'a NewPlayers {
    type Item = SteamID;

    type IntoIter = NewPlayersIterator<'a>;

    fn into_iter(self) -> Self::IntoIter {
        NewPlayersIterator {
            players: self,
            index: 0,
        }
    }
}

// Useful

fn serialize_player_map<S, K, V>(map: &HashMap<K, V>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    V: Serialize,
{
    let values: Vec<_> = map.values().collect();
    values.serialize(s)
}
