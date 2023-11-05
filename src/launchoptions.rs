use std::{
    fs,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

use anyhow::{Context, Result};
use regex::Regex;
use steamid_ng::SteamID;
use substring::Substring;
use tracing::Level;

use crate::gamefinder::locate_steam_launch_configs;

/// -condebug enables the console.log file to be written to by the game.
/// -conclearlog clears the console.log file on launch of TF2
/// -usercon allows local RCON to be used
/// -g15 enables Logitech G15 keyboard support (used for the console command `g15_dumpplayer`)
pub const TF2_REQUIRED_OPTS: [&str; 4] = ["-condebug", "-conclearlog", "-usercon", "-g15"];

/// Read the local steam library folders for data (stored in VDF/KeyValues format) on the configured launch options for the given app
/// ID.
/// Handles referencing the VDF store of a Steam app's launch options and provides an interface to read
/// and write launch options based on a set of required options.
pub struct LaunchOptions {
    local_config: PathBuf,
    launch_args_regex: Regex,
    app_data: Option<String>,
    new_app_data: Option<String>,
}

impl LaunchOptions {
    /// Get the current configured launch options for the target app under the current logged in steam user.
    ///
    /// # Errors
    /// Will raise `anyhow::Error` under the following conditions:
    /// - No `localconfig.vdf` file found for the given user in the expected Steam library
    /// - Could not read the `localconfig.vdf` file. (because of any non-`ErrorKind::Interrupted` during read)
    /// - Failed to parse the `localconfig.vdf` file. (File is corrupted/broken/incomplete)
    /// - Target app ID does not exist in `localconfig.vdf` file or the object is corrupted.
    pub fn new(user: SteamID, target_app: String) -> Result<LaunchOptions, anyhow::Error> {
        let span = tracing::span!(Level::INFO, "LaunchOptions");
        let _enter = span.enter();

        let config_path: PathBuf = locate_steam_launch_configs(user)
            .context("Failed to find localconfig.vdf in Steam dir.")?;
        let mut data: Vec<u8> = Vec::new();
        tracing::debug!("Found Steam local app config.");

        if let Ok(mut f) = File::open(&config_path) {
            f.read_to_end(&mut data)
                .context(format!("Failed to read {:?}", config_path))?;
        }
        let binding = &String::from_utf8_lossy(&data);

        let apps_regex =
            Regex::new(r#"\t{4}"[aA]pps"([\s\S]+)\t{5}}"#).expect("Apps regex construction");

        let caps = apps_regex
            .captures(binding)
            .context("No capture groups found -  no apps list present in localconfig.vdf.")?;

        let mut matched_app_block: Option<String> = None;
        let open_match = format!("\t\t\t\t\t\"{}\"", target_app);

        let mat_opt = caps.get(1);
        if let Some(mat) = mat_opt {
            let latter_portion = mat
                .as_str()
                .find(&open_match)
                .context("Could not find specified app in localconfig.vdf for the current user.")?;
            let latter = mat.as_str().substring(latter_portion, mat.end());
            let first_app_close = latter
                .find("\n\t\t\t\t\t}")
                .context("Failed to find object closing statement for the matched app block.")?;
            let app_match = latter.substring(0, first_app_close);

            if matched_app_block.is_none() {
                matched_app_block = Some(app_match.to_string());
            }
        }

        let launch_options_regex = Regex::new(r#"\t{6}"LaunchOptions"\t{2}"([(\-\w)\s]*)""#)
            .expect("Constructing LaunchOptions regex");

        Ok(LaunchOptions {
            local_config: config_path,
            launch_args_regex: launch_options_regex,
            app_data: matched_app_block,
            new_app_data: None,
        })
    }

    /// Returns a vector of the launch options NOT found in the target apps launch options,
    /// but are defined as required according to [`TF2_REQUIRED_OPTS`].
    ///
    /// # Errors
    /// Will raise anyhow::Error under the following conditions:
    /// - Target app exists but has no 'LaunchOptions' key (no user configured launch options).
    /// - No app data is stored in this object (`self.app_data` is None).
    pub fn check_missing_args(&self) -> Result<Vec<&str>, anyhow::Error> {
        let span = tracing::span!(Level::TRACE, "MissingLaunchOptions");
        let _enter = span.enter();
        tracing::debug!("Checking for missing launch arguments in specified app...");
        let mut missing_args: Vec<&str> = Vec::new();
        let data_ref = match self.new_app_data {
            Some(_) => &self.new_app_data,
            None => &self.app_data,
        };
        let app_data = data_ref.clone().context("No data currently stored.")?;
        let current_args = match self.launch_args_regex.find(&app_data) {
            Some(current_args) => current_args,
            None => {
                missing_args.extend(TF2_REQUIRED_OPTS.iter());
                return Ok(missing_args);
            }
        };

        let mat_str = current_args.as_str();
        TF2_REQUIRED_OPTS.iter().for_each(|opt| {
            if !mat_str.contains(opt) {
                tracing::debug!("Launch Arguments: Missing argument identified -> {}", opt);
                missing_args.insert(0, opt);
            }
        });

        Ok(missing_args)
    }

    /// Identifies any missing required launch options, and writes them into the
    /// `localconfig.vdf` file as well as the [`new_app_data`](Self::new_app_data) var.
    ///
    /// Return value is the return value of [`write_changes_to_file`](Self::write_changes_to_file)
    pub fn write_corrected_args_to_file(&mut self) -> Result<(), anyhow::Error> {
        self.add_opts_if_missing();
        self.write_changes_to_file()
    }

    /// Writes any changes to the launch options present in [`new_app_data`](Self::new_app_data)
    /// into the `localconfig.vdf` file.
    ///
    /// # Errors
    /// Will raise anyhow::Error if:
    /// - The `localconfig.vdf` file could not be opened to write into (potentially if Steam happens to also be writing the file simultanesouly).
    /// - An error was encountered during writing to the file.
    fn write_changes_to_file(&self) -> Result<(), anyhow::Error> {
        let span = tracing::span!(Level::INFO, "WriteLaunchOptions");
        let _enter = span.enter();
        let old_app = self.app_data.clone().context("No data is loaded.")?;
        let new_app = self
            .new_app_data
            .clone()
            .context("No updated app data, assuming configuration correct.")?;

        if old_app == new_app {
            tracing::info!("Launch configuration correct, no changes required.");
            return Ok(());
        }
        let span2 = tracing::span!(Level::INFO, "RewriteMissingLaunchOptions");
        let _enter2 = span2.enter();

        tracing::debug!("Reading all data from disk...");
        let file_contents: Vec<u8> =
            fs::read(self.local_config.as_path()).context("Failed to read localconfig.vdf.")?;

        // Unsafe because we don't want to break non-utf8 byte sequences that may be contained in the "friends" object.
        unsafe {
            let mut f_str = String::from_utf8_unchecked(file_contents);
            f_str = f_str.replace(&old_app, &new_app);
            tracing::debug!("Replaced old app data with new app data");

            let mut f = OpenOptions::new()
                .write(true)
                .open(self.local_config.as_path())
                .context("Failed to open localconfig.vdf in write mode.")?;

            f.write_all(f_str.as_bytes())
                .context("Failed to write in localconfig.vdf.")?;
            tracing::debug!("Wrote new app data to disk...");
        }

        Ok(())
    }

    /// Clones [`app_data`](Self::app_data) into [`new_app_data`](Self::new_app_data) and modifies the
    /// LaunchOptions key to contain the updated list of launch opts.
    ///
    /// If there are no missing required launch options, this is a no-op.
    fn add_opts_if_missing(&mut self) {
        let copied_app_data = self.app_data.clone();
        if let Some(mut prior) = copied_app_data {
            if !prior.contains("\"LaunchOptions\"") {
                prior += "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"\""
            }

            TF2_REQUIRED_OPTS.iter().for_each(|opt| {
                let curr_opts = self.launch_args_regex.find(&prior);
                if let Some(mat) = curr_opts {
                    let mat_str = mat.as_str();
                    if !mat_str.contains(opt) {
                        let new_opts = mat_str.replace(
                            "\"LaunchOptions\"\t\t\"",
                            &format!("\"LaunchOptions\"\t\t\"{} ", opt),
                        );
                        prior = prior.replace(mat_str, &new_opts);
                    }
                }
            });

            if let Some(old_app) = self.app_data.clone() {
                if prior == old_app {
                    tracing::debug!("No changes detected to launch options.");
                } else {
                    tracing::debug!("Tracking config to correct missing launch options... call `write_changes_to_file` now...");
                    self.new_app_data = Some(prior);
                }
            }
        }
    }
}
