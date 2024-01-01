use serde::{Serialize, Serializer};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use steamid_ng::SteamID;

use crate::{
    io::{
        g15,
        regexes::{self, ChatMessage, PlayerKill, StatusLine},
        IOOutput,
    },
    player::{Friend, GameInfo, Player, SteamInfo},
    player_records::{PlayerRecord, PlayerRecordLock, PlayerRecords},
};

const MAX_HISTORY_LEN: usize = 100;

// Server

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    user: Option<SteamID>,
    map: Option<Arc<str>>,
    ip: Option<Arc<str>>,
    hostname: Option<Arc<str>>,
    max_players: Option<u32>,
    num_players: Option<u32>,
    players: Players,

    gamemode: Option<Gamemode>,

    #[serde(skip)]
    player_records: PlayerRecords,
    #[serde(skip)]
    friends_lists: HashMap<SteamID, Vec<Friend>>,
    #[serde(skip)]
    friends_is_public: HashMap<SteamID, bool>, // True if public, False if private, no entry if we haven't checked yet.
}

pub struct Players {
    all: HashMap<SteamID, Player>,
    connected: Vec<SteamID>,
    history: VecDeque<SteamID>,
}

#[allow(dead_code)]
impl Players {
    fn new() -> Players {
        Players {
            all: HashMap::new(),
            connected: Vec::new(),
            history: VecDeque::with_capacity(MAX_HISTORY_LEN),
        }
    }

    pub fn all(&self) -> &HashMap<SteamID, Player> {
        &self.all
    }

    pub fn get(&self, steamid: &SteamID) -> Option<&Player> {
        self.all.get(&steamid)
    }

    fn get_mut(&mut self, steamid: &SteamID) -> Option<&mut Player> {
        self.all.get_mut(steamid)
    }

    pub fn connected(&self) -> impl Iterator<Item = &Player> {
        self.connected.iter().flat_map(|s| self.all.get(s))
    }

    fn connected_mut(&mut self) -> impl Iterator<Item = &mut Player> {
        self.all
            .iter_mut()
            .filter(|(s, _)| self.connected.contains(s))
            .map(&|(_, p)| p)
    }

    pub fn history(&self) -> impl Iterator<Item = &Player> {
        self.history.iter().flat_map(|s| self.all.get(s))
    }
}

impl Serialize for Players {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let connected: Vec<&Player> = self.connected().collect();
        connected.serialize(serializer)
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct Gamemode {
    pub matchmaking: bool,
    #[serde(rename = "type")]
    pub game_type: Arc<str>,
    pub vanilla: bool,
}

#[allow(dead_code)]
impl Server {
    pub fn new(playerlist: PlayerRecords) -> Server {
        Server {
            user: None,
            map: None,
            ip: None,
            hostname: None,
            max_players: None,
            num_players: None,
            players: Players::new(),

            gamemode: None,
            player_records: playerlist,
            friends_lists: HashMap::new(),
            friends_is_public: HashMap::new(),
        }
    }

    // **** Getters / Setters ****

    pub fn user(&self) -> Option<SteamID> {
        self.user
    }

    pub fn set_user(&mut self, user_id: Option<SteamID>) {
        self.user = user_id;
    }

    pub fn map(&self) -> Option<Arc<str>> {
        self.map.clone()
    }

    pub fn ip(&self) -> Option<Arc<str>> {
        self.ip.clone()
    }

    pub fn hostname(&self) -> Option<Arc<str>> {
        self.hostname.clone()
    }

    pub fn max_players(&self) -> Option<u32> {
        self.max_players
    }

    pub fn num_players(&self) -> Option<u32> {
        self.num_players
    }

    pub fn players(&self) -> &Players {
        &self.players
    }

    pub fn gamemode(&self) -> Option<&Gamemode> {
        self.gamemode.as_ref()
    }
}

impl Server {
    /// Moves any old players from the server into history. Any console commands (status, g15_dumpplayer, etc)
    /// should be run before calling this function again to prevent removing all players from the player list.
    pub fn refresh(&mut self) {
        // Get old players
        let unaccounted_players: Vec<SteamID> = self
            .players
            .connected()
            .filter(|p| p.game_info.is_some() && p.game_info.as_ref().unwrap().should_prune())
            .map(|p| p.steamid)
            .collect();

        self.players
            .connected
            .retain(|s| !unaccounted_players.contains(s));

        // Remove any of them from the history as they will be added more recently
        self.players
            .history
            .retain(|p| !unaccounted_players.iter().any(|up| up == p));

        // Shrink to not go past max number of players
        let num_players = self.players.history.len() + unaccounted_players.len();
        for _ in MAX_HISTORY_LEN..num_players {
            self.players.history.pop_front();
        }

        for p in unaccounted_players {
            self.players.history.push_back(p);
        }

        // Mark all remaining players as unaccounted, they will be marked as accounted again
        // when they show up in status or another console command.
        self.players
            .connected_mut()
            .flat_map(|p| p.game_info.as_mut())
            .for_each(GameInfo::next_cycle);
    }

    /// Add the provided SteamInfo to the given player. Returns true if that player was
    /// found in the server.
    pub fn insert_steam_info(&mut self, player: SteamID, info: SteamInfo) -> bool {
        if let Some(player) = self.players.get_mut(&player) {
            player.steam_info = Some(info.clone());
            true
        } else {
            false
        }
    }

    /// Updates friends lists of a user
    /// Propagates to all other friends lists to ensure two-way lookup possible.
    /// Only call if friends list was obtained directly from Steam API (i.e. friends list is public)
    pub fn update_friends_list(&mut self, steamid: SteamID, friendslist: Vec<Friend>) {
        self.friends_is_public.insert(steamid, true);

        let oldfriends = self.friends_lists.insert(steamid, friendslist.clone());

        // Propagate to all other hashmap entries
        for friend in friendslist.clone() {
            match self.friends_lists.get_mut(&friend.steamid) {
                // Friend's friendlist in memory
                Some(friends_of_friend) => {
                    match friends_of_friend.iter().position(|f| f.steamid == steamid) {
                        Some(friend_index) => {
                            // player already in friend's friends list, update friend_since in case it changed.
                            friends_of_friend[friend_index].friend_since = friend.friend_since;
                        }
                        None => {
                            friends_of_friend.push(Friend {
                                steamid,
                                friend_since: friend.friend_since,
                            });
                        }
                    }
                }
                // Friend's friendlist isn't in memory yet; create a new vector with player.
                None => {
                    let mut friends_of_friend = Vec::new();
                    friends_of_friend.push(Friend {
                        steamid,
                        friend_since: friend.friend_since,
                    });
                    self.friends_lists.insert(friend.steamid, friends_of_friend);
                }
            }
            self.update_friends_playerobj(&friend.steamid, None);
        }

        // If a player's friend has been unfriended, remove player from friend's hashmap
        match oldfriends {
            Some(oldfriends) => {
                let oldfriends_ids = oldfriends
                    .iter()
                    .map(|f| f.steamid)
                    .filter(|fid| friendslist.iter().find(|f| f.steamid == *fid).is_none());
                for oldfriend_id in oldfriends_ids {
                    self.remove_from_friends_list(&oldfriend_id, &steamid);
                }
            }
            None => {}
        }
        self.update_friends_playerobj(&steamid, None);
    }

    /// Mark a friends list as being private, trim all now-stale information.
    pub fn private_friends_list(&mut self, steamid: &SteamID) {
        let old_vis_state = self.friends_is_public.insert(*steamid, false);
        let old_friendslist = self.friends_lists.get(steamid).cloned();

        match (old_vis_state, old_friendslist) {
            (Some(old_vis_state), Some(old_friendslist)) => {
                // Already processed, this function is the only one that sets friends lists to private.
                if old_vis_state == false {
                    return;
                }

                for friend in old_friendslist {
                    let friends_of_friend = self.friends_lists.get(&friend.steamid);
                    match (
                        self.friends_is_public.get(&friend.steamid),
                        friends_of_friend,
                    ) {
                        // If friend doesn't have any friends on record, nothing to remove.
                        (_, None) => {
                            continue;
                        }
                        (is_public, Some(_)) => {
                            // If friend's friendlist is public, that information isn't stale.
                            if is_public.is_some_and(|p| *p) {
                                continue;
                            }
                            self.remove_from_friends_list(&friend.steamid, &steamid);
                        }
                    }
                }
            }
            _ => {}
        }
        self.update_friends_playerobj(steamid, None);
    }

    /// Helper function to remove a friend from a player's friendlist.
    fn remove_from_friends_list(&mut self, steamid: &SteamID, friend_to_remove: &SteamID) {
        match self.friends_lists.get_mut(steamid) {
            Some(friends) => match friends.iter().position(|f| f.steamid == *friend_to_remove) {
                Some(i) => {
                    friends.remove(i);
                    self.update_friends_playerobj(&steamid, None);
                }
                None => {}
            },
            None => {}
        }
    }

    /// Helper function to update the player object with the friends information we have on them.
    fn update_friends_playerobj(
        &mut self,
        steamid: &SteamID,
        existing_player: Option<&mut Player>,
    ) {
        let friends = self.friends_lists.get(steamid);
        let friends_is_public = self.friends_is_public.get(steamid);

        let mut player = self.players.get_mut(&steamid);

        if existing_player.is_some() {
            player = existing_player;
        }

        if player.is_some() && friends.is_some() {
            player.unwrap().update_friends(
                friends.unwrap().to_vec(),
                friends_is_public.copied(),
                self.user,
            );
        }
    }

    /// Return all known friends of a user, as well as their friend's list visibility as a bool.
    pub fn get_friends_list(&self, steamid: &SteamID) -> (Option<&Vec<Friend>>, Option<&bool>) {
        let friends = self.friends_lists.get(steamid);
        let is_public = self.friends_is_public.get(steamid);

        return (friends, is_public);
    }

    /// Check if an account is friends with the user.
    /// Returns None if we don't have enough information to tell.
    pub fn is_friends_with_user(&self, friend: &SteamID) -> Option<bool> {
        if self.user.is_none() {
            return None;
        }
        let user = self.user.unwrap();
        return self.is_friends(friend, &user);
    }

    /// Check if two accounts are friends with each other.
    /// Returns None if we don't have enough information to tell.
    pub fn is_friends(&self, friend1: &SteamID, friend2: &SteamID) -> Option<bool> {
        let ispublic_1 = self.friends_is_public.get(friend1);
        let ispublic_2 = self.friends_is_public.get(friend2);

        // If both friends lists are private, we can't say for sure.
        if !ispublic_1.is_some_and(|p| *p) && !ispublic_2.is_some_and(|p| *p) {
            return None;
        }

        let friends_list_1 = self.friends_lists.get(friend1);
        let friends_list_2 = self.friends_lists.get(friend2);

        match (friends_list_1, friends_list_2) {
            (Some(friends_list_1), _) => {
                return Some(friends_list_1.iter().any(|f| f.steamid == *friend2));
            }
            (_, Some(friends_list_2)) => {
                return Some(friends_list_2.iter().any(|f| f.steamid == *friend1));
            }
            _ => {}
        }
        return Some(false);
    }

    // Player records

    pub fn has_player_record(&self, steamid: SteamID) -> bool {
        self.player_records.get_records().contains_key(&steamid)
    }

    pub fn get_player_records(&self) -> &PlayerRecords {
        &self.player_records
    }

    pub fn insert_player_record(&mut self, record: PlayerRecord) {
        self.player_records.insert_record(record);
    }

    pub fn get_player_record(&self, steamid: SteamID) -> Option<&PlayerRecord> {
        self.player_records.get_record(steamid)
    }

    pub fn get_player_record_mut(&mut self, steamid: SteamID) -> Option<PlayerRecordLock> {
        self.player_records
            .get_record_mut(steamid, &mut self.players.all)
    }

    // **** Message handling ****

    /// Handles any io output from running commands / reading the console log file.
    /// Returns:
    /// * Some<[SteamID]> of a player if they have been newly added to the server.
    pub fn handle_io_output(&mut self, response: IOOutput) -> Vec<SteamID> {
        use IOOutput::*;
        match response {
            G15(players) => return self.handle_g15_parse(players).into(),
            Status(status) => {
                return self
                    .handle_status_line(status)
                    .map(|s| vec![s])
                    .unwrap_or_default();
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

        Vec::new()
    }

    fn handle_g15_parse(&mut self, players: Vec<g15::G15Player>) -> Vec<SteamID> {
        let mut new_players = Vec::new();
        let mut name_updates: Vec<(SteamID, Arc<str>)> = Vec::new();
        for g15 in players {
            if g15.steamid.is_none() {
                continue;
            }
            let steamid = g15.steamid.unwrap();

            // Add to connected players if they aren't already
            if !self.players.connected.contains(&steamid) {
                self.players.connected.push(steamid);
            }

            if let Some(player) = self.players.get_mut(&steamid) {
                if player.game_info.is_none() {
                    player.game_info = GameInfo::new_from_g15(&g15);
                }

                if let Some(name) = g15.name {
                    player.name = name;

                    if self.player_records.get_records().contains_key(&steamid)
                        && !player.previous_names.contains(&player.name)
                    {
                        name_updates.push((steamid, player.name.clone()));
                    }
                }

                if let Some(game_info) = &mut player.game_info {
                    // Update existing player
                    if let Some(scr) = g15.score {
                        game_info.kills = scr;
                    }
                    if let Some(dth) = g15.deaths {
                        game_info.deaths = dth;
                    }
                    if let Some(png) = g15.ping {
                        game_info.ping = png;
                    }
                    if let Some(tm) = g15.team {
                        game_info.team = tm;
                    }
                    if let Some(uid) = g15.userid {
                        game_info.userid = uid;
                    }

                    game_info.acknowledge();
                }
            } else {
                // Create player data if they don't exist yet
                if let Some(mut player) = Player::new_from_g15(&g15, self.user) {
                    if let Some(mut record) = self.get_player_record_mut(steamid) {
                        if !record.previous_names.contains(&player.name) {
                            record.previous_names.push(player.name.clone());
                        }

                        player.update_from_record(record.clone());
                    }

                    if self.is_friends_with_user(&steamid).is_some_and(|f| f) {
                        player.tags.push(Arc::from("Friend"));
                    }

                    self.update_friends_playerobj(&steamid, Some(&mut player));
                    self.players.all.insert(steamid, player);
                    new_players.push(steamid);
                }
            }
        }

        // Update any recorded names
        for (steamid, name) in name_updates {
            if let Some(mut record) = self.get_player_record_mut(steamid) {
                record.previous_names.push(name);
            }
        }

        new_players
    }

    /// Given a status line, update an existing or add a new one to the server.
    /// Returns the SteamID if a new player was created.
    fn handle_status_line(&mut self, status: StatusLine) -> Option<SteamID> {
        // Add to connected players if they aren't already
        if !self.players.connected.contains(&status.steamid) {
            self.players.connected.push(status.steamid);
        }

        // Update existing player or insert new player
        if let Some(player) = self.players.get_mut(&status.steamid) {
            // Update existing player
            if player.game_info.is_none() {
                player.game_info = Some(GameInfo::new_from_status(&status));
            }

            let game_info = player.game_info.as_mut().unwrap();
            player.name = status.name;
            game_info.userid = status.userid;
            game_info.ping = status.ping;
            game_info.loss = status.loss;
            game_info.state = status.state;
            game_info.time = status.time;
            game_info.acknowledge();

            // Update previous names
            if self
                .player_records
                .get_records()
                .contains_key(&status.steamid)
                && !player.previous_names.contains(&player.name)
            {
                let new_name = player.name.clone();
                if let Some(mut record) = self.get_player_record_mut(status.steamid) {
                    record.previous_names.push(new_name);
                }
            }

            None
        } else {
            // Create and insert new player
            let mut player = Player::new_from_status(&status, self.user);

            if let Some(mut record) = self.get_player_record_mut(status.steamid) {
                if !record.previous_names.contains(&player.name) {
                    record.previous_names.push(player.name.clone());
                }

                player.update_from_record(record.clone());
            }

            if self
                .is_friends_with_user(&status.steamid)
                .is_some_and(|f| f)
            {
                player.tags.push(Arc::from("Friend"));
            }

            self.update_friends_playerobj(&status.steamid, Some(&mut player));
            self.players.all.insert(status.steamid, player);
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
