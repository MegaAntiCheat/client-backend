use std::{
    collections::{HashMap, HashSet, VecDeque},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize, Serializer};
use steamid_ng::SteamID;

use crate::{
    io::{
        g15::{self, G15Player},
        regexes::StatusLine,
    },
    parties::Parties,
    player_records::{default_custom_data, PlayerRecord, PlayerRecords, Verdict},
    settings::{ConfigFilesError, Settings},
};

pub mod tags {
    pub const FRIEND: &str = "Friend";
}

// const MAX_HISTORY_LEN: usize = 100;

pub struct Players {
    pub game_info: HashMap<SteamID, GameInfo>,
    pub steam_info: HashMap<SteamID, SteamInfo>,
    pub friend_info: HashMap<SteamID, FriendInfo>,
    pub records: PlayerRecords,
    pub tags: HashMap<SteamID, HashSet<String>>,
    pub parties: Parties,

    pub connected: Vec<SteamID>,
    pub history: VecDeque<SteamID>,

    pub user: Option<SteamID>,

    parties_needs_update: bool,
}

#[allow(dead_code)]
impl Players {
    #[must_use]
    pub fn new(records: PlayerRecords, user: Option<SteamID>) -> Self {
        let mut players = Self {
            game_info: HashMap::new(),
            steam_info: HashMap::new(),
            friend_info: HashMap::new(),
            tags: HashMap::new(),
            records,
            parties: Parties::new(),

            connected: Vec::new(),
            history: VecDeque::new(),
            user,

            parties_needs_update: false,
        };

        match players.load_steam_info() {
            Ok(()) => tracing::info!(
                "Loaded steam info cache with {} entries.",
                players.steam_info.len()
            ),
            Err(ConfigFilesError::IO(_, e)) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!("No steam info cache was found, creating a new one.");
            }
            Err(e) => tracing::error!("Failed to load steam info cache: {e}"),
        }

        players
    }

    /// Retrieve the local verdict for a player
    #[must_use]
    pub fn verdict(&self, steamid: SteamID) -> Verdict {
        self.records
            .get(&steamid)
            .map_or(Verdict::Player, PlayerRecord::verdict)
    }

    /// Check if a player has a particular tag set
    #[must_use]
    pub fn has_tag(&self, steamid: SteamID, tag: &str) -> bool {
        self.tags.get(&steamid).is_some_and(|t| t.contains(tag))
    }

    /// Set a particular tag on a player
    pub fn set_tag(&mut self, steamid: SteamID, tag: String) {
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

        for friend in &friendslist {
            self.propagate_friend(steamid, friend);
        }

        let oldfriends: Vec<SteamID> = self.set_friends(steamid, friendslist);

        // If a player's friend has been unfriended, remove player from friend's hashmap
        for oldfriend in oldfriends {
            self.remove_from_friends_list(oldfriend, steamid);
        }

        self.update_user_friend_tag(steamid);
    }

    /// Sets the friends list and friends list visibility, returning any old
    /// friends that have been removed
    fn set_friends(&mut self, steamid: SteamID, friends: Vec<Friend>) -> Vec<SteamID> {
        self.parties_needs_update = true;

        let friend_info = self.friend_info.entry(steamid).or_default();

        friend_info.public = Some(true);

        let mut removed_friends = friends;
        friend_info.retain(|f1| !removed_friends.iter().any(|f2| f1.steamid == f2.steamid));
        std::mem::swap(&mut removed_friends, &mut friend_info.friends);

        removed_friends.into_iter().map(|f| f.steamid).collect()
    }

    /// Helper function to add a friend to a friends list
    fn propagate_friend(&mut self, steamid: SteamID, friend: &Friend) {
        let friend_info = self.friend_info.entry(friend.steamid).or_default();

        friend_info.push(Friend {
            steamid,
            friend_since: friend.friend_since,
        });

        self.update_user_friend_tag(friend.steamid);
    }

    /// Helper function to remove a friend from a player's friendlist.
    fn remove_from_friends_list(&mut self, steamid: SteamID, friend_to_remove: SteamID) {
        if let Some(friends) = self.friend_info.get_mut(&steamid) {
            friends.retain(|f| f.steamid != friend_to_remove);
            if friends.len() == 0 && friends.public.is_none() {
                self.friend_info.remove(&steamid);
            }
        }

        if let Some(friends) = self.friend_info.get_mut(&friend_to_remove) {
            friends.retain(|f| f.steamid != steamid);
            if friends.len() == 0 && friends.public.is_none() {
                self.friend_info.remove(&friend_to_remove);
            }
        }
        self.update_user_friend_tag(friend_to_remove);
    }

    /// Mark a friends list as being private, trim all now-stale information.
    pub fn mark_friends_list_private(&mut self, steamid: SteamID) {
        let friends = self.friend_info.entry(steamid).or_default();
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

                self.remove_from_friends_list(friend.steamid, steamid);
            }
        }
    }

    fn update_user_friend_tag(&mut self, friend: SteamID) {
        let is_friends_with_user: Option<bool> = self.is_friends_with_user(friend);
        if is_friends_with_user.is_some_and(|friends| friends) {
            self.set_tag(friend, tags::FRIEND.into());
        } else {
            self.clear_tag(friend, tags::FRIEND);
        }
    }

    /// Check if an account is friends with the user.
    /// Returns None if we don't have enough information to tell.
    #[must_use]
    pub fn is_friends_with_user(&self, friend: SteamID) -> Option<bool> {
        self.user.and_then(|user| self.are_friends(friend, user))
    }

    /// Check if two accounts are friends with each other.
    /// Returns None if we don't have enough information to tell.
    #[must_use]
    pub fn are_friends(&self, friend1: SteamID, friend2: SteamID) -> Option<bool> {
        if let Some(friends) = self.friend_info.get(&friend1) {
            if friends.iter().any(|f| f.steamid == friend2) {
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
            .get(&friend2)
            .is_some_and(|f| f.public.is_some_and(|p| p))
        {
            return Some(false);
        }

        // Both are private :(
        None
    }

    /// Moves any old players from the server into history. Any console commands
    /// (status, `g15_dumpplayer`, etc) should be run before calling this
    /// function again to prevent removing all players from the player list.
    pub fn refresh(&mut self) {
        // Get old players
        let unaccounted_players: Vec<SteamID> = self
            .connected
            .iter()
            .filter(|&s| self.game_info.get(s).map_or(true, GameInfo::should_prune))
            .copied()
            .collect();

        if !unaccounted_players.is_empty() {
            self.parties_needs_update = true;
        }

        self.connected.retain(|s| !unaccounted_players.contains(s));

        // Remove any of them from the history as they will be added more recently
        self.history
            .retain(|p| !unaccounted_players.iter().any(|up| up == p));

        // Shrink to not go past max number of players
        // let num_players = self.history.len() + unaccounted_players.len();
        // for _ in MAX_HISTORY_LEN..num_players {
        //     self.history.pop_front();
        // }

        for p in unaccounted_players {
            self.history.push_back(p);
        }

        // Mark all remaining players as unaccounted, they will be marked as accounted
        // again when they show up in status or another console command.
        self.game_info.values_mut().for_each(GameInfo::next_cycle);

        if self.parties_needs_update {
            self.parties
                .find_parties(&self.friend_info, &self.connected);
            self.parties_needs_update = false;
        }
    }

    /// Gets a struct containing all the relevant data on a player in a
    /// serializable format
    pub fn get_serializable_player(&self, steamid: SteamID) -> Player {
        let game_info = self.game_info.get(&steamid);
        let steam_info = self.steam_info.get(&steamid);
        let name = game_info.map_or_else(
            || steam_info.map_or("", |si| &si.account_name),
            |gi| &gi.name,
        );
        let tags: Vec<&str> = self
            .tags
            .get(&steamid)
            .map(|tags| tags.iter().map(AsRef::as_ref).collect())
            .unwrap_or_default();

        let record = self.records.get(&steamid);
        let previous_names = record
            .as_ref()
            .map(|r| r.previous_names().iter().map(AsRef::as_ref).collect())
            .unwrap_or_default();

        let friend_info = self.friend_info.get(&steamid);
        let friends: Vec<&Friend> = friend_info
            .as_ref()
            .map(|fi| fi.friends.iter().collect())
            .unwrap_or_default();

        let local_verdict = record.as_ref().map_or(Verdict::Player, |r| r.verdict());

        Player {
            isSelf: self.user.is_some_and(|user| user == steamid),
            name,
            steamID64: steamid,
            localVerdict: local_verdict,
            steamInfo: steam_info,
            gameInfo: game_info,
            customData: record
                .as_ref()
                .map_or_else(default_custom_data, |r| r.custom_data().clone()),
            convicted: false,
            tags,
            previous_names,
            friends,
            friendsIsPublic: friend_info.and_then(|fi| fi.public),
        }
    }

    pub fn handle_g15(&mut self, players: Vec<g15::G15Player>) {
        for g15 in players {
            let Some(steamid) = g15.steamid else {
                continue;
            };

            if let Some(r) = self.records.get_mut(&steamid) {
                r.mark_seen();
            }

            // Add to connected players if they aren't already
            if !self.connected.contains(&steamid) {
                self.connected.push(steamid);
                self.parties_needs_update = true;
            }

            // Update game info
            if let Some(game_info) = self.game_info.get_mut(&steamid) {
                if let Some(name) = g15.name.as_ref() {
                    self.records.update_name(steamid, name);
                }
                game_info.update_from_g15(g15);
            } else if let Some(game_info) = GameInfo::new_from_g15(g15) {
                // Update name
                self.records.update_name(steamid, &game_info.name);
                self.game_info.insert(steamid, game_info);
            }
        }
    }

    pub fn handle_status_line(&mut self, status: StatusLine) {
        let steamid = status.steamid;

        if let Some(r) = self.records.get_mut(&steamid) {
            r.mark_seen();
        }

        // Add to connected players if they aren't already
        if !self.connected.contains(&steamid) {
            self.connected.push(steamid);
            self.parties_needs_update = true;
        }

        if let Some(game_info) = self.game_info.get_mut(&steamid) {
            if status.name != game_info.name {
                self.records.update_name(steamid, &status.name);
            }

            game_info.update_from_status(status);
        } else {
            let game_info = GameInfo::new_from_status(status);

            // Update name
            self.records.update_name(steamid, &game_info.name);
            self.game_info.insert(steamid, game_info);
        }
    }

    #[must_use]
    pub fn get_name(&self, steamid: SteamID) -> Option<&str> {
        if let Some(gi) = self.game_info.get(&steamid) {
            return Some(&gi.name);
        } else if let Some(si) = self.steam_info.get(&steamid) {
            return Some(&si.account_name);
        } else if let Some(last_name) = self
            .records
            .get(&steamid)
            .map(|r| r.previous_names().first())
        {
            return last_name.map(String::as_str);
        }

        None
    }

    #[must_use]
    pub fn get_steamid_from_name(&self, name: &str) -> Option<SteamID> {
        self.connected
            .iter()
            .find(|&s| self.game_info.get(s).is_some_and(|gi| gi.name == name))
            .copied()
    }

    #[must_use]
    pub fn get_name_to_steam_ids_map(&self) -> HashMap<String, SteamID> {
        self.connected
            .iter()
            .filter_map(|s| self.game_info.get(s).map(|gi| (gi.name.clone(), *s)))
            .collect()
    }

    fn locate_steam_info_cache_path() -> Result<PathBuf, ConfigFilesError> {
        Settings::locate_config_directory().map(|p| p.join("steam_cache.bin"))
    }

    /// # Errors
    /// If the file could not be read from disk or the data could not be deserialized
    pub fn load_steam_info(&mut self) -> Result<(), ConfigFilesError> {
        let path = Self::locate_steam_info_cache_path()?;
        self.load_steam_info_from(&path)
    }

    /// # Errors
    /// If the data could not be serialized or the file could not be written back to disk
    pub fn save_steam_info(&self) -> Result<(), ConfigFilesError> {
        let path = Self::locate_steam_info_cache_path()?;
        self.save_steam_info_to(&path)
    }

    pub fn save_steam_info_ok(&self) {
        if let Err(e) = self.save_steam_info() {
            tracing::error!("Failed to save steam info cache: {e}");
        } else {
            tracing::debug!("Saved steam info cache.");
        }
    }

    fn load_steam_info_from(&mut self, path: &Path) -> Result<(), ConfigFilesError> {
        let contents = std::fs::read(path)
            .map_err(|e| ConfigFilesError::IO(path.to_string_lossy().into(), e))?;
        let steam_info = pot::from_slice(&contents)
            .map_err(|e| ConfigFilesError::Pot(path.to_string_lossy().into(), e))?;

        self.steam_info = steam_info;
        Ok(())
    }

    fn save_steam_info_to(&self, path: &Path) -> Result<(), ConfigFilesError> {
        let contents = pot::to_vec(&self.steam_info)
            .map_err(|e| ConfigFilesError::Pot(path.to_string_lossy().into(), e))?;
        std::fs::write(path, contents)
            .map_err(|e| ConfigFilesError::IO(path.to_string_lossy().into(), e))?;

        Ok(())
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
            .map(|&s| self.get_serializable_player(s))
            .collect();
        players.serialize(serializer)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[allow(clippy::module_name_repetitions)]
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
            0 => Ok(Self::Unassigned),
            1 => Ok(Self::Spectators),
            2 => Ok(Self::Red),
            3 => Ok(Self::Blu),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamInfo {
    #[serde(rename = "name")]
    pub account_name: String,
    pub profile_url: String,
    #[serde(rename = "pfp")]
    pub pfp_url: String,
    pub pfp_hash: String,
    pub profile_visibility: ProfileVisibility,
    pub time_created: Option<i64>,
    pub country_code: Option<String>,
    pub vac_bans: i64,
    pub game_bans: i64,
    pub days_since_last_ban: Option<i64>,
    pub fetched: DateTime<Utc>,
}

impl SteamInfo {
    #[must_use]
    pub fn expired(&self) -> bool {
        Utc::now().signed_duration_since(self.fetched).num_hours() > 3
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileVisibility {
    Private = 1,
    FriendsOnly = 2,
    Public = 3,
}

impl From<i32> for ProfileVisibility {
    fn from(value: i32) -> Self {
        match value {
            2 => Self::FriendsOnly,
            3 => Self::Public,
            _ => Self::Private,
        }
    }
}

// GameInfo

#[derive(Debug, Clone, Serialize)]
pub struct GameInfo {
    pub name: String,
    pub userid: String,
    pub team: Team,
    pub time: u32,
    pub ping: u32,
    pub loss: u32,
    pub state: PlayerState,
    pub kills: u32,
    pub deaths: u32,
    pub alive: bool,
    #[serde(skip)]
    /// How many cycles has passed since the player has been seen
    last_seen: u32,
}

impl Default for GameInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            userid: String::new(),
            team: Team::Unassigned,
            time: 0,
            ping: 0,
            loss: 0,
            state: PlayerState::Active,
            kills: 0,
            deaths: 0,
            last_seen: 0,
            alive: false,
        }
    }
}

impl GameInfo {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn new_from_g15(g15: G15Player) -> Option<Self> {
        g15.userid.as_ref()?;

        let mut game_info = Self::new();
        game_info.update_from_g15(g15);
        Some(game_info)
    }

    pub(crate) fn new_from_status(status: StatusLine) -> Self {
        let mut game_info = Self::new();
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
        if let Some(alive) = g15.alive {
            self.alive = alive;
        }

        self.acknowledge();
    }

    pub(crate) fn update_from_status(&mut self, status: StatusLine) {
        self.name = status.name;
        self.userid = status.userid;
        self.time = status.time;
        self.ping = status.ping;
        self.loss = status.loss;

        // Attach the spawning flag manually as it can be easily missed by the parsers due to timing.
        if status.time > 0 && status.time < 30 && self.team == Team::Unassigned {
            self.state = PlayerState::Spawning;
        }
        // Make the Spawning flag "sticky" until they either pick a class or join spectator.
        // Makes it easy to spot bots taking up a player slot that can't be kicked.
        else if self.state != PlayerState::Spawning
            || status.state != PlayerState::Active
            || self.alive
            || self.team == Team::Spectators
        {
            self.state = status.state;
        }

        self.acknowledge();
    }

    pub(crate) fn next_cycle(&mut self) {
        const DISCONNECTED_THRESHOLD: u32 = 2;

        self.last_seen += 1;
        if self.last_seen > DISCONNECTED_THRESHOLD {
            self.state = PlayerState::Disconnected;
        }
    }

    pub(crate) const fn should_prune(&self) -> bool {
        const CYCLE_LIMIT: u32 = 6;
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
    pub friends: Vec<Friend>,
}

impl FriendInfo {
    #[must_use]
    pub fn friends(&self) -> &[Friend] {
        &self.friends
    }
}

impl Deref for FriendInfo {
    type Target = Vec<Friend>;

    fn deref(&self) -> &Self::Target {
        &self.friends
    }
}

impl DerefMut for FriendInfo {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.friends
    }
}

// Useful

#[allow(clippy::trivially_copy_pass_by_ref, clippy::missing_errors_doc)]
pub fn serialize_steamid_as_string<S: Serializer>(
    steamid: &SteamID,
    s: S,
) -> Result<S::Ok, S::Error> {
    format!("{}", u64::from(*steamid)).serialize(s)
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::missing_errors_doc)]
pub fn serialize_maybe_steamid_as_string<S: Serializer>(
    steamid: &Option<SteamID>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match steamid {
        Some(steamid) => format!("{}", u64::from(*steamid)).serialize(s),
        None => s.serialize_none(),
    }
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
