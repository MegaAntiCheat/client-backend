use std::{fs::File, io::Read, path::PathBuf};

use anyhow::{Context, Result};
use keyvalues_parser::Vdf;
use steamid_ng::SteamID;
use tracing::Level;

use crate::gamefinder::{locate_steam_launch_configs, TF2_GAME_ID};

/// `-condebug` enables the console.log file to be written to by the game.
/// `-conclearlog` clears the console.log file on launch of TF2
/// `-usercon` allows local RCON to be used
/// `-g15` enables Logitech G15 keyboard support (used for the console command
/// `g15_dumpplayer`)
pub const TF2_REQUIRED_OPTS: [&str; 4] = ["-condebug", "-conclearlog", "-usercon", "-g15"];

/// Read the local steam library folders for data (stored in VDF/KeyValues
/// format) on the configured launch options for the given app ID.
/// Handles referencing the VDF store of a Steam app's launch options and
/// provides an interface to read and write launch options based on a set of
/// required options.
pub struct LaunchOptions {
    launch_options: Vec<String>,
}

/// Get the value of a nested key in a VDF object.
/// # Errors
/// Will raise `anyhow::Error` under the following conditions:
/// - The object is not a VDF object
/// - The key we are looking for does not exist
/// - The key does not have a corresponding value
fn get_nested_value<'a>(
    vdf: &'a Vdf,
    keys: &[&str],
) -> Result<&'a keyvalues_parser::Value<'a>, anyhow::Error> {
    let mut current = &vdf.value;

    for &key in keys {
        let obj = current.get_obj().context("Expected an object")?;

        let next = obj.get(key).context(format!("No key found for {key}"))?;

        current = next
            .first()
            .context(format!("No first key found for {key}"))?;
    }
    Ok(current)
}

impl LaunchOptions {
    /// Get the current configured launch options for the target app under the
    /// current logged in steam user.
    ///
    /// # Errors
    /// Will raise `anyhow::Error` under the following conditions:
    /// - No `localconfig.vdf` file found for the given user in the expected
    ///   Steam library
    /// - Could not read the `localconfig.vdf` file. (because of any
    ///   non-`ErrorKind::Interrupted` during read)
    /// - Failed to parse the `localconfig.vdf` file. (File is
    ///   corrupted/broken/incomplete)
    /// - The `LaunchOptions` key does not exist in the target app's data or
    ///  the value is not a string. (No user configured launch options)
    #[allow(clippy::missing_panics_doc)]
    pub fn new(user: SteamID) -> Result<Self, anyhow::Error> {
        let span = tracing::span!(Level::INFO, "LaunchOptions");
        let _enter = span.enter();

        let config_path: PathBuf = locate_steam_launch_configs(user)
            .context("Failed to find localconfig.vdf in Steam dir.")?;
        let mut data: Vec<u8> = Vec::new();
        tracing::debug!("Found Steam local app config.");

        if let Ok(mut f) = File::open(&config_path) {
            f.read_to_end(&mut data)
                .context(format!("Failed to read {config_path:?}"))?;
        }
        let binding = &String::from_utf8_lossy(&data);

        let localconfig = Vdf::parse(binding).context("Failed to parse localconfig.vdf")?;

        let keys = [
            "Software",
            "Valve",
            "Steam",
            "apps",
            &TF2_GAME_ID.to_string(),
            "LaunchOptions",
        ];
        let launch_options = get_nested_value(&localconfig, &keys)?;

        let launch_options_vec: Vec<String> = launch_options
            .get_str()
            .context("Expected a string for launch options")?
            .split_whitespace()
            .map(std::string::ToString::to_string)
            .collect();

        Ok(Self {
            launch_options: launch_options_vec,
        })
    }

    /// Returns a vector of the launch options NOT found in the target apps
    /// launch options, but are defined as required according to
    /// [`TF2_REQUIRED_OPTS`].
    #[must_use]
    pub fn check_missing_args(&self) -> std::vec::Vec<&str> {
        let missing_args: Vec<&str> = TF2_REQUIRED_OPTS
            .iter()
            .filter(|&opt| !self.launch_options.contains(&(*opt).to_string()))
            .copied()
            .collect();

        missing_args
    }
}
