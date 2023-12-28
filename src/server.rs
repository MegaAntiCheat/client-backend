use serde::{Serialize, Serializer};
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    sync::Arc,
};
use steamid_ng::SteamID;

use crate::{
    io::{
        g15,
        regexes::{self, ChatMessage, PlayerKill, StatusLine},
        IOOutput,
    },
    player::{Friend, Player, SteamInfo},
    player_records::{PlayerRecord, PlayerRecordLock, PlayerRecords},
};

const MAX_HISTORY_LEN: usize = 100;

// Server

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    user_id: Option<SteamID>,
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
    #[serde(skip)]
    friends_lists: HashMap<SteamID, Vec<Friend>>,
    #[serde(skip)]
    friends_is_public: HashMap<SteamID, bool>, // True if public, False if private, no entry if we haven't checked yet.
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
            user_id: None,
            map: None,
            ip: None,
            hostname: None,
            max_players: None,
            num_players: None,
            players: HashMap::new(),
            player_history: VecDeque::with_capacity(MAX_HISTORY_LEN),
            gamemode: None,
            player_records: playerlist,
            friends_lists: HashMap::new(),
            friends_is_public: HashMap::new()
        }
    }

    pub fn set_steam_user(&mut self, user_id: &Option<SteamID>) {
        self.user_id = user_id.clone();
    }

    /// Handles any io output from running commands / reading the console log file.
    /// Returns:
    /// * Some<[SteamID]> of a player if they have been newly added to the server.
    pub fn handle_io_output(&mut self, response: IOOutput, user: Option<SteamID>) -> Vec<SteamID> {
        use IOOutput::*;
        match response {
            G15(players) => return self.handle_g15_parse(players, user).into(),
            Status(status) => {
                return self
                    .handle_status_line(status, user)
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

    /// Moves any old players from the server into history. Any console commands (status, g15_dumpplayer, etc)
    /// should be run before calling this function again to prevent removing all players from the player list.
    pub fn refresh(&mut self) {
        // Get old players
        let mut unaccounted_players = Vec::new();
        for (steamid, player) in &self.players {
            if player.game_info.should_prune() {
                unaccounted_players.push(*steamid);
            }
        }

        let unaccounted_players: Vec<Player> = unaccounted_players
            .into_iter()
            .flat_map(|sid| self.players.remove(&sid))
            .collect();

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
            p.game_info.next_cycle();
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
                        },
                        None => {
                            friends_of_friend.push(Friend { steamid, friend_since: friend.friend_since });
                        }
                    }
                }
                // Friend's friendlist isn't in memory yet; create a new vector with player.
                None => {
                    let mut friends_of_friend = Vec::new();
                    friends_of_friend.push(Friend { steamid, friend_since: friend.friend_since });
                    self.friends_lists.insert(friend.steamid, friends_of_friend);
                }
            }
            self.update_friends_playerobj(&friend.steamid, None);
        }

        // If a player's friend has been unfriended, remove player from friend's hashmap
        match oldfriends {
            Some(oldfriends) => {
                let oldfriends_ids = oldfriends.iter().map(|f| {
                    f.steamid
                }).filter(|fid| {
                    friendslist.iter().find(|f| f.steamid == *fid).is_none()
                });
                for oldfriend_id in oldfriends_ids {
                    self.remove_from_friends_list(&oldfriend_id, &steamid);
                }
            },
            None => {}
        }
        self.update_friends_playerobj(&steamid, None);
    }

    /// Mark a friends list as being private, trim all now-stale information.
    pub fn private_friends_list (&mut self, steamid: &SteamID) {
        let old_vis_state = self.friends_is_public.insert(*steamid, false);
        let old_friendslist = self.friends_lists.get(steamid).cloned();
        
        match (old_vis_state, old_friendslist) {
            (Some(old_vis_state), Some(old_friendslist)) => {
                // Already processed, this function is the only one that sets friends lists to private.
                if old_vis_state == false { return; } 

                for friend in old_friendslist {
                    let friends_of_friend = self.friends_lists.get(&friend.steamid);
                    match (self.friends_is_public.get(&friend.steamid), friends_of_friend){
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
            Some(friends) => {
                match friends.iter().position(|f| f.steamid == *friend_to_remove) {
                    Some(i) => {
                        friends.remove(i);
                        self.update_friends_playerobj(&steamid, None);
                    },
                    None => {}
                }
            }
            None => {}
        }
    }

    /// Helper function to update the player object with the friends information we have on them.
    fn update_friends_playerobj(&mut self, steamid: &SteamID, existing_player: Option<&mut Player>) {
        let friends = self.friends_lists.get(steamid);
        let friends_is_public = self.friends_is_public.get(steamid);

        let mut player = self.players.get_mut(&steamid);

        if existing_player.is_some() {
            player = existing_player;
        }

        if player.is_some() && friends.is_some() {
            player.unwrap().update_friends(friends.unwrap().to_vec(), friends_is_public.copied(), self.user_id);
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
        if self.user_id.is_none() {
            return None;
        }
        let user = self.user_id.unwrap();
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
                return Some(friends_list_1.iter().any(|f| {
                    f.steamid == *friend2
                }));
            }
            (_, Some(friends_list_2)) => {
                return Some(friends_list_2.iter().any(|f| {
                    f.steamid == *friend1
                }));
            }
            _ => {}
        }
        return Some(false);
    }

    /// Retrieve the player history somewhere in the range 0..100
    pub fn get_player_history(&self, range: Range<usize>) -> Vec<&Player> {
        self.player_history
            .iter()
            .rev()
            .skip(range.start)
            .take(range.end - range.start)
            .collect()
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
            .get_record_mut(steamid, &mut self.players, &mut self.player_history)
    }

    pub fn get_players(&self) -> &HashMap<SteamID, Player> {
        &self.players
    }

    // Other

    fn handle_g15_parse(
        &mut self,
        players: Vec<g15::G15Player>,
        user: Option<SteamID>,
    ) -> Vec<SteamID> {
        let mut new_players = Vec::new();
        let mut name_updates: Vec<(SteamID, Arc<str>)> = Vec::new();
        for pl in players {
            if let Some(steamid) = pl.steamid {
                // Update existing player
                if let Some(player) = self.players.get_mut(&steamid) {
                    if let Some(scr) = pl.score {
                        player.game_info.kills = scr;
                    }
                    if let Some(name) = pl.name {
                        player.name = name;

                        if self.player_records.get_records().contains_key(&steamid)
                            && !player.previous_names.contains(&player.name)
                        {
                            name_updates.push((steamid, player.name.clone()));
                        }
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

                    player.game_info.acknowledge();
                } else if let Some(mut player) = Player::new_from_g15(&pl, user) {
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

                    self.players.insert(steamid, player);
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
            player.game_info.acknowledge();

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
            let mut player = Player::new_from_status(&status, user);

            if let Some(mut record) = self.get_player_record_mut(status.steamid) {
                if !record.previous_names.contains(&player.name) {
                    record.previous_names.push(player.name.clone());
                }

                player.update_from_record(record.clone());
            }

            if self.is_friends_with_user(&status.steamid).is_some_and(|f| f) {
                player.tags.push(Arc::from("Friend"));
            }

            self.update_friends_playerobj(&status.steamid, Some(&mut player));

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

// Useful

fn serialize_player_map<S, K, V>(map: &HashMap<K, V>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    V: Serialize,
{
    let values: Vec<_> = map.values().collect();
    values.serialize(s)
}
