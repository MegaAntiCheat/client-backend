#![allow(non_upper_case_globals)]
#![allow(unused_variables)]

use regex::{Regex, Captures};
use std::{fs};

// A RegMatch struct contains a Regex and the corresponding function to add the result to the G15Player vec
// if the regex results in a match.
pub struct RegMatch(Regex, fn(caps: Captures, players: &mut Vec<G15Player>));
impl RegMatch {
    pub fn new(reg_str: &str, func: fn(caps: Captures, players: &mut Vec<G15Player>)) -> RegMatch {
        let /*p*/regers = Regex::new(reg_str).unwrap();
        RegMatch (regers, func)
    }
}

// m_iAmmo[1] integer (32)      --match--> (player idx) (ammo count)
pub const REGEX_I_AMMO: &str = r#"^m_iAmmo\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub fn parse_ammo(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let ammo = caps[2].parse::<u32>().unwrap();
    players[idx].ammo = ammo;
}

// m_szName[1] string (Lust)    --match--> (player idx) (string name)
pub const REGEX_SZ_NAME: &str = r#"^m_szName\[(\d+)\]\s+string\s+\((.+)\)$"#;
pub fn parse_name(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let name = caps[2].to_string();
    players[idx].name = name;
}

// m_iPing[2] integer (15)      --match--> (player idx) (ping in ms)
pub const REGEX_I_PING: &str = r#"^m_iPing\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub fn parse_ping(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let ping = caps[2].parse::<u32>().unwrap();
    players[idx].ping = ping;
}

// m_iScore[4] integer (7)      --match--> (player idx) (score (num of kills))
pub const REGEX_I_SCORE: &str = r#"^m_iScore\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub fn parse_score(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let score = caps[2].parse::<u32>().unwrap();
    players[idx].score = score;
}

// m_iDeaths[17] integer (4)    --match--> (player idx) (deaths)
pub const REGEX_I_DEATHS: &str = r#"^m_iDeaths\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub fn parse_deaths(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let deaths = caps[2].parse::<u32>().unwrap();
    players[idx].deaths = deaths;
}

// m_bConnected[0] bool (false) --match--> (player idx) (connected status (true/false))
pub const REGEX_B_CONNECTED: &str = r#"^m_bConnected\[(\d+)\]\s+bool\s+\((false|true)\)$"#;
pub fn parse_connected(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let connected_str = &caps[2];
    let truth_value = matches!(connected_str, "true");
    players[idx].connected = truth_value;
}

// m_iTeam[18] integer (3)      --match--> (player idx) (team (0=unconnected, 1=spectator, 2=blu, 3=red))
pub const REGEX_I_TEAM: &str = r#"^m_iTeam\[(\d+)\]\s+integer\s+\(([0-3])\)$"#;
pub fn parse_team(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let team = caps[2].parse::<u32>().unwrap();
    players[idx].team = team;
}


// m_bAlive[0] bool (false)     --match--> (player idx) (alive status (true/false))
pub const REGEX_B_ALIVE: &str = r#"^m_bAlive\[(\d+)\]\s+bool\s+\((false|true)\)$"#;
pub fn parse_alive(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let alive_str = &caps[2];
    let truth_value = matches!(alive_str, "true");
    players[idx].alive = truth_value;
}

// m_iHealth[3] integer (125)   --match--> (player idx) (current health)
pub const REGEX_I_HEALTH: &str = r#"^m_iHealth\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub fn parse_health(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let health = caps[2].parse::<u32>().unwrap();
    players[idx].health = health;
}

// m_iAccountID[3] integer (1505713148) --match--> (player idx) (variable component of a steamID3 ([U:1:1505713148]))
pub const REGEX_I_SID3: &str = r#"^m_iAccountID\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub fn parse_sid3(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let sid3 = caps[2].parse::<u64>().unwrap();
    players[idx].sid3 = format!("[U:1:{}]", sid3);
}

// m_bValid[24] bool (true)     --match--> (player idx) (valid status (true/false))
// I'm not sure what valid means - perhaps to with whether this is a current/in-game player idx
// and to display this row in the scoreboard or not? Can someone look pls
pub const REGEX_B_VALID: &str = r#"^m_bValid\[(\d+)\]\s+bool\s+\((false|true)\)$"#;
pub fn parse_valid(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let valid_str = &caps[2];
    let truth_value = matches!(valid_str, "true");
    players[idx].valid = truth_value;
}

// m_iUserID[2] integer (68)    --match--> (player idx) (server id (as it appears in status))
pub const REGEX_I_USERID: &str = r#"^m_iUserID\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub fn parse_userid(caps: Captures, players: &mut Vec<G15Player>) {
    let idx = caps[1].parse::<usize>().unwrap();
    let userid = caps[2].to_string();
    players[idx].userid = userid;
}

// The G15Player struct contains all the data per player searched.
#[derive(Debug, Clone)]
pub struct G15Player {
    name: String,     // eg "Lilith"
    ping: u32,        // eg 21
    score: u32,       // eg 16
    deaths: u32,      // eg 5
    sid3: String,     // eg [U:1:111216987]
    team: u32,        // eg 3
    health: u32,      // eg 125
    ammo: u32,        // eg 6
    connected: bool,  // eg true
    valid: bool,      // eg true
    alive: bool,      // eg true
    userid: String,   // eg "301"
}
impl G15Player {
    fn new() -> G15Player {
        G15Player {
            name: String::from("none"), 
            ping: 0, 
            score: 0, 
            deaths: 0, 
            sid3: String::from("none"), 
            team: 0, 
            health: 0, 
            ammo: 0,
            alive: false,
            connected: false,
            userid: String::from("0"),
            valid: false,
        }
    }
}

/*
  Parse a g15_dumpplayer string via Regex search.
  We only extract useful data here, so drop most data.
*/
pub fn parse_g15(g15_log: String) -> Vec<G15Player> {
    let mut players: Vec<G15Player> = vec![G15Player::new(); 34];
    let patterns = [
        RegMatch::new(REGEX_I_AMMO, parse_ammo), 
        RegMatch::new(REGEX_SZ_NAME, parse_name),
        RegMatch::new(REGEX_I_PING, parse_ping),
        RegMatch::new(REGEX_I_SCORE, parse_score),
        RegMatch::new(REGEX_I_DEATHS, parse_deaths),
        RegMatch::new(REGEX_B_CONNECTED, parse_connected),
        RegMatch::new(REGEX_I_TEAM, parse_team),
        RegMatch::new(REGEX_B_ALIVE, parse_alive),
        RegMatch::new(REGEX_I_HEALTH, parse_health),
        RegMatch::new(REGEX_I_SID3, parse_sid3),
        RegMatch::new(REGEX_B_VALID, parse_valid),
        RegMatch::new(REGEX_I_USERID, parse_userid),
        ];
    let lines = g15_log.split("\n"); 

    for line in lines {
        for pat in &patterns {
            // Perform a regex search on the current line using the stored regex in the current RegMatch struct
            let pat_caps = pat.0.captures(line.trim());
            if !pat_caps.is_none() {
                // Call the corresponding function to store the capture groups, as we have found a match
                pat.1(pat_caps.unwrap(), &mut players);
            }
        }
    }
    players.retain(|x| x.valid);

    return players
}


fn main() {
    println!("Reading test_g15.log file...");
    let g15_log = fs::read_to_string("./test_g15.log")
        .expect("No g15 log file found?");

    let players = parse_g15(g15_log);

    for pl in players {
        println!("{:?}", pl);
    }
}