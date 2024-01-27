use std::{
    collections::{HashMap, HashSet, VecDeque},
    ops::{Deref, DerefMut},
    sync::Arc,
};

use serde::{Serialize, Serializer};
use steamid_ng::SteamID;

use crate::{
    io::{
        g15::{self, G15Player},
        regexes::StatusLine,
    },
    player_records::{default_custom_data, PlayerRecords, Verdict},
};

pub mod tags {
    pub const FRIEND: &str = "Friend";
}

const MAX_HISTORY_LEN: usize = 100;

pub struct Players {
    pub game_info: HashMap<SteamID, GameInfo>,
    pub steam_info: HashMap<SteamID, SteamInfo>,
    pub friend_info: HashMap<SteamID, FriendInfo>,
    pub records: PlayerRecords,
    pub tags: HashMap<SteamID, HashSet<Arc<str>>>,

    pub connected: Vec<SteamID>,
    pub history: VecDeque<SteamID>,

    pub user: Option<SteamID>,
}

#[allow(dead_code)]
impl Players {
    pub(crate) fn new(records: PlayerRecords) -> Players {
        Players {
            game_info: HashMap::new(),
            steam_info: HashMap::new(),
            friend_info: HashMap::new(),
            tags: HashMap::new(),
            records,

            connected: Vec::new(),
            history: VecDeque::with_capacity(MAX_HISTORY_LEN),
            user: None,
        }
    }

    /// Check if a player has a particular tag set
    pub fn has_tag(&self, steamid: SteamID, tag: &str) -> bool {
        self.tags
            .get(&steamid)
            .map(|t| t.contains(tag))
            .unwrap_or(false)
    }

    /// Set a particular tag on a player
    pub fn set_tag(&mut self, steamid: SteamID, tag: Arc<str>) {
        self.tags.entry(steamid).or_default().insert(tag);
    }

    /// Clear a particular tag on a player
    pub fn clear_tag(&mut self, steamid: SteamID, tag: &str) {
        if let Some(tags) = self.tags.get_mut(&steamid) {
            tags.remove(tag);
            if tags.is_empty() {
                self.tags.remove(&steamid);
            }
        }
    }

    /// Updates friends lists of a user
    /// Propagates to all other friends lists to ensure two-way lookup possible.
    /// Only call if friends list was obtained directly from Steam API (i.e.
    /// friends list is public)
    pub fn update_friends_list(&mut self, steamid: SteamID, friendslist: Vec<Friend>) {
        // Propagate to all other hashmap entries

        for friend in friendslist.iter() {
            self.propagate_friend(steamid, friend.clone());
        }

        let oldfriends: Vec<SteamID> = self.set_friends(steamid, friendslist);

        // If a player's friend has been unfriended, remove player from friend's hashmap
        for oldfriend in oldfriends {
            self.remove_from_friends_list(&oldfriend, &steamid);
        }

        self.update_user_friend_tag(steamid);
    }

    /// Sets the friends list and friends list visibility, returning any old
    /// friends that have been removed
    fn set_friends(&mut self, steamid: SteamID, friends: Vec<Friend>) -> Vec<SteamID> {
        let friend_info = self.friend_info.entry(steamid).or_default();

        friend_info.public = Some(true);

        let mut removed_friends = friends;
        friend_info.retain(|f1| !removed_friends.iter().any(|f2| f1.steamid == f2.steamid));
        std::mem::swap(&mut removed_friends, &mut friend_info.friends);

        removed_friends.into_iter().map(|f| f.steamid).collect()
    }

    /// Helper function to add a friend to a friends list
    fn propagate_friend(&mut self, steamid: SteamID, friend: Friend) {
        let friend_info = self.friend_info.entry(friend.steamid).or_default();

        friend_info.push(Friend {
            steamid,
            friend_since: friend.friend_since,
        });

        self.update_user_friend_tag(friend.steamid);
    }

    /// Helper function to remove a friend from a player's friendlist.
    fn remove_from_friends_list(&mut self, steamid: &SteamID, friend_to_remove: &SteamID) {
        if let Some(friends) = self.friend_info.get_mut(steamid) {
            friends.retain(|f| f.steamid != *friend_to_remove);
            if friends.len() == 0 && friends.public.is_none() {
                self.friend_info.remove(steamid);
            }
        }

        if let Some(friends) = self.friend_info.get_mut(friend_to_remove) {
            friends.retain(|f| f.steamid != *steamid);
            if friends.len() == 0 && friends.public.is_none() {
                self.friend_info.remove(friend_to_remove);
            }
        }
        self.update_user_friend_tag(*friend_to_remove);
    }

    /// Mark a friends list as being private, trim all now-stale information.
    pub fn mark_friends_list_private(&mut self, steamid: &SteamID) {
        let friends = self.friend_info.entry(*steamid).or_default();
        let old_vis_state = friends.public;
        if old_vis_state.is_some_and(|public| !public) {
            return;
        }

        friends.public = Some(false);

        let old_friendslist = friends.clone();

        for friend in old_friendslist {
            if let Some(friends_of_friend) = self.friend_info.get(&friend.steamid) {
                // If friend's friendlist is public, that information isn't stale.
                if friends_of_friend.public.is_some_and(|p| p) {
                    continue;
                }

                self.remove_from_friends_list(&friend.steamid, steamid);
            }
        }
    }

    fn update_user_friend_tag(&mut self, friend: SteamID) {
        let is_friends_with_user: Option<bool> = self.is_friends_with_user(&friend);
        if is_friends_with_user.is_some_and(|friends| friends) {
            self.set_tag(friend, tags::FRIEND.into());
        } else {
            self.clear_tag(friend, tags::FRIEND);
        }
    }

    /// Check if an account is friends with the user.
    /// Returns None if we don't have enough information to tell.
    pub fn is_friends_with_user(&self, friend: &SteamID) -> Option<bool> {
        if let Some(user) = self.user {
            self.are_friends(friend, &user)
        } else {
            None
        }
    }

    /// Check if two accounts are friends with each other.
    /// Returns None if we don't have enough information to tell.
    pub fn are_friends(&self, friend1: &SteamID, friend2: &SteamID) -> Option<bool> {
        if let Some(friends) = self.friend_info.get(friend1) {
            if friends.iter().any(|f| f.steamid == *friend2) {
                return Some(true);
            }

            // Friends list is public, so we should be able to see the other party
            // regardless
            if friends.public.is_some_and(|p| p) {
                return Some(false);
            }
        }

        // Other friends list is public, so 2-way lookup should have been possible
        if self
            .friend_info
            .get(friend2)
            .is_some_and(|f| f.public.is_some_and(|p| p))
        {
            return Some(false);
        }

        // Both are private :(
        None
    }

    /// Moves any old players from the server into history. Any console commands
    /// (status, g15_dumpplayer, etc) should be run before calling this
    /// function again to prevent removing all players from the player list.
    pub fn refresh(&mut self) {
        // Get old players
        let unaccounted_players: Vec<SteamID> = self
            .connected
            .iter()
            .filter(|&s| {
                self.game_info
                    .get(s)
                    .map(|gi| gi.should_prune())
                    .unwrap_or(true)
            })
            .cloned()
            .collect();

        self.connected.retain(|s| !unaccounted_players.contains(s));

        // Remove any of them from the history as they will be added more recently
        self.history
            .retain(|p| !unaccounted_players.iter().any(|up| up == p));

        // Shrink to not go past max number of players
        let num_players = self.history.len() + unaccounted_players.len();
        for _ in MAX_HISTORY_LEN..num_players {
            self.history.pop_front();
        }

        for p in unaccounted_players {
            self.history.push_back(p);
        }

        // Mark all remaining players as unaccounted, they will be marked as accounted
        // again when they show up in status or another console command.
        self.game_info.values_mut().for_each(GameInfo::next_cycle);
    }

    /// Gets a struct containing all the relevant data on a player in a
    /// serializable format
    pub fn get_serializable_player(&self, steamid: &SteamID) -> Option<Player> {
        let game_info = self.game_info.get(steamid)?;
        let tags: Vec<&str> = self
            .tags
            .get(steamid)
            .map(|tags| tags.iter().map(|t| t.as_ref()).collect())
            .unwrap_or_default();

        let record = self.records.get(steamid);
        let previous_names = record
            .as_ref()
            .map(|r| r.previous_names.iter().map(|n| n.as_ref()).collect())
            .unwrap_or_default();

        let friend_info = self.friend_info.get(steamid);
        let friends: Vec<&Friend> = friend_info
            .as_ref()
            .map(|fi| fi.friends.iter().collect())
            .unwrap_or_default();

        let local_verdict = record
            .as_ref()
            .map(|r| r.verdict)
            .unwrap_or(Verdict::Player);

        Some(Player {
            isSelf: self.user.is_some_and(|user| user == *steamid),
            name: game_info.name.as_ref(),
            steamID64: *steamid,
            localVerdict: local_verdict,
            steamInfo: self.steam_info.get(steamid),
            gameInfo: Some(game_info),
            customData: record
                .as_ref()
                .map(|r| r.custom_data.clone())
                .unwrap_or_else(default_custom_data),
            convicted: false,
            tags,
            previous_names,
            friends,
            friendsIsPublic: friend_info.and_then(|fi| fi.public),
        })
    }

    pub fn handle_g15(&mut self, players: Vec<g15::G15Player>) {
        for g15 in players {
            if g15.steamid.is_none() {
                continue;
            }
            let steamid = g15.steamid.unwrap();

            // Add to connected players if they aren't already
            if !self.connected.contains(&steamid) {
                self.connected.push(steamid);
            }

            // Update game info
            if let Some(game_info) = self.game_info.get_mut(&steamid) {
                if let Some(name) = g15.name.as_ref() {
                    if *name != game_info.name {
                        self.records.update_name(&steamid, name.clone());
                    }
                }
                game_info.update_from_g15(g15);
            } else if let Some(game_info) = GameInfo::new_from_g15(g15) {
                // Update name
                self.records.update_name(&steamid, game_info.name.clone());
                self.game_info.insert(steamid, game_info);
            }
        }
    }

    pub fn handle_status_line(&mut self, status: StatusLine) {
        let steamid = status.steamid;

        // Add to connected players if they aren't already
        if !self.connected.contains(&steamid) {
            self.connected.push(steamid);
        }

        if let Some(game_info) = self.game_info.get_mut(&steamid) {
            if status.name != game_info.name {
                self.records.update_name(&steamid, status.name.clone());
            }

            game_info.update_from_status(status);
        } else {
            let game_info = GameInfo::new_from_status(status);

            // Update name
            self.records.update_name(&steamid, game_info.name.clone());
            self.game_info.insert(steamid, game_info);
        }
    }
}

impl Serialize for Players {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let players: Vec<Player> = self
            .connected
            .iter()
            .flat_map(|s| self.get_serializable_player(s))
            .collect();
        players.serialize(serializer)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
pub enum PlayerState {
    Active,
    Spawning,
    Disconnected,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Team {
    Unassigned = 0,
    Spectators = 1,
    Red = 2,
    Blu = 3,
}

impl TryFrom<u32> for Team {
    type Error = &'static str;
    fn try_from(val: u32) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(Team::Unassigned),
            1 => Ok(Team::Spectators),
            2 => Ok(Team::Red),
            3 => Ok(Team::Blu),
            _ => Err("Not a valid team value"),
        }
    }
}

impl Serialize for Team {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        s.serialize_u32(*self as u32)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamInfo {
    #[serde(rename = "name")]
    pub account_name: Arc<str>,
    pub profile_url: Arc<str>,
    #[serde(rename = "pfp")]
    pub pfp_url: Arc<str>,
    pub pfp_hash: Arc<str>,
    pub profile_visibility: ProfileVisibility,
    pub time_created: Option<i64>,
    pub country_code: Option<Arc<str>>,
    pub vac_bans: i64,
    pub game_bans: i64,
    pub days_since_last_ban: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ProfileVisibility {
    Private = 1,
    FriendsOnly = 2,
    Public = 3,
}

impl From<i32> for ProfileVisibility {
    fn from(value: i32) -> Self {
        match value {
            1 => ProfileVisibility::Private,
            2 => ProfileVisibility::FriendsOnly,
            3 => ProfileVisibility::Public,
            _ => ProfileVisibility::Private,
        }
    }
}

// GameInfo

#[derive(Debug, Clone, Serialize)]
pub struct GameInfo {
    pub name: Arc<str>,
    pub userid: Arc<str>,
    pub team: Team,
    pub time: u32,
    pub ping: u32,
    pub loss: u32,
    pub state: PlayerState,
    pub kills: u32,
    pub deaths: u32,
    #[serde(skip)]
    /// How many cycles has passed since the player has been seen
    last_seen: u32,
}

impl Default for GameInfo {
    fn default() -> Self {
        GameInfo {
            name: "".into(),
            userid: "".into(),
            team: Team::Unassigned,
            time: 0,
            ping: 0,
            loss: 0,
            state: PlayerState::Active,
            kills: 0,
            deaths: 0,
            last_seen: 0,
        }
    }
}

impl GameInfo {
    pub(crate) fn new() -> GameInfo { Default::default() }

    pub(crate) fn new_from_g15(g15: G15Player) -> Option<GameInfo> {
        g15.userid.as_ref()?;

        let mut game_info = GameInfo::new();
        game_info.update_from_g15(g15);
        Some(game_info)
    }

    pub(crate) fn new_from_status(status: StatusLine) -> GameInfo {
        let mut game_info = GameInfo::new();
        game_info.update_from_status(status);
        game_info
    }

    pub(crate) fn update_from_g15(&mut self, g15: G15Player) {
        if let Some(name) = g15.name {
            self.name = name;
        }
        if let Some(userid) = g15.userid {
            self.userid = userid;
        }
        if let Some(team) = g15.team {
            self.team = team;
        }
        if let Some(ping) = g15.ping {
            self.ping = ping;
        }
        if let Some(kills) = g15.score {
            self.kills = kills;
        }
        if let Some(deaths) = g15.deaths {
            self.deaths = deaths;
        }

        self.acknowledge();
    }

    pub(crate) fn update_from_status(&mut self, status: StatusLine) {
        self.name = status.name;
        self.userid = status.userid;
        self.time = status.time;
        self.ping = status.ping;
        self.loss = status.loss;
        self.state = status.state;

        self.acknowledge();
    }

    pub(crate) fn next_cycle(&mut self) {
        const DISCONNECTED_THRESHOLD: u32 = 1;

        self.last_seen += 1;
        if self.last_seen > DISCONNECTED_THRESHOLD {
            self.state = PlayerState::Disconnected;
        }
    }

    pub(crate) fn should_prune(&self) -> bool {
        const CYCLE_LIMIT: u32 = 5;
        self.last_seen > CYCLE_LIMIT
    }

    fn acknowledge(&mut self) {
        self.last_seen = 0;

        if self.state == PlayerState::Disconnected {
            self.state = PlayerState::Active;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Friend {
    #[serde(rename = "steamID64", serialize_with = "serialize_steamid_as_string")]
    pub steamid: SteamID,
    #[serde(rename = "friendSince")]
    pub friend_since: u64,
}

#[derive(Debug, Serialize, Default)]
pub struct FriendInfo {
    pub public: Option<bool>,
    friends: Vec<Friend>,
}

impl Deref for FriendInfo {
    type Target = Vec<Friend>;

    fn deref(&self) -> &Self::Target { &self.friends }
}

impl DerefMut for FriendInfo {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.friends }
}

// Useful

fn serialize_steamid_as_string<S: Serializer>(steamid: &SteamID, s: S) -> Result<S::Ok, S::Error> {
    format!("{}", u64::from(*steamid)).serialize(s)
}

#[allow(non_snake_case)]
#[derive(Debug, Serialize)]
pub struct Player<'a> {
    pub isSelf: bool,
    pub name: &'a str,
    #[serde(serialize_with = "serialize_steamid_as_string")]
    pub steamID64: SteamID,

    pub steamInfo: Option<&'a SteamInfo>,
    pub gameInfo: Option<&'a GameInfo>,
    pub customData: serde_json::Value,
    pub localVerdict: Verdict,
    pub convicted: bool,
    pub tags: Vec<&'a str>,
    pub previous_names: Vec<&'a str>,

    pub friends: Vec<&'a Friend>,
    pub friendsIsPublic: Option<bool>,
}
