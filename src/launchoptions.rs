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

/// Handles referencing the VDF store of a Steam app's launch options and provides an interface to read
/// and write launch options based on a set of required options.
pub struct LaunchOptionsV2 {
    local_config: PathBuf,
    launch_args_regex: Regex,
    app_data: Option<String>,
    new_app_data: Option<String>,
}

/// Read the local steam library folders for data (stored in VDF/KeyValues format) on the configured launch options for the give app
/// ID. If necessary, warn about the missing launch options.
impl LaunchOptionsV2 {
    /// Get the Launch options of the given app for the given steam user (should be the user provided in loginusers.vdf)
    /// This can fail if the `localconfig.vdf` file is not present in the expected path, or if the given app is not present
    /// in the users list of launched apps (either they don't own it or have never played it).
    ///
    /// The Source VDF file format is just a wrapped 'KeyValues' format, which is essentially the cursed brother of JSON.
    /// This format separates keys and values via '\t\t' rather than ':', and is (obviously) white space sensitive. This
    /// allows us to make the guarenteed assumption that for as long as the specification of `localconfig.vdf` does not change,
    /// the app block we are looking for will start with `\t\t\t\t\t"<app id>"`, and as such all of the required app data is at
    /// least 6 tabs deep, and the closing '}' character is the very next '}' found only 5 tabs deep.
    pub fn new(user: SteamID, target_app: String) -> Result<LaunchOptionsV2, anyhow::Error> {
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
            Regex::new(r#"\t{4}"apps"([\s\S]+)\t{5}}"#).expect("Apps regex construction");

        let caps = apps_regex
            .captures(binding)
            .context("No capture groups found -  no apps list present in localconfig.vdf.")?;

        let mut matched_app_block: Option<String> = None;
        let open_match = format!("\t\t\t\t\t\"{}\"", target_app);

        let mat_opt = caps.get(0);
        if let Some(mat) = mat_opt {
            let latter_portion = mat
                .as_str()
                .find(&open_match)
                .context("Could not find specified app in localconfig.vdf for the current user.")
                .unwrap();
            let latter = mat.as_str().substring(latter_portion, mat.end());
            let first_app_close = latter
                .find("\n\t\t\t\t\t}")
                .context("Failed to find object closing statement for the matched app block.")
                .unwrap();
            let app_match = latter.substring(0, first_app_close);

            if matched_app_block.is_none() {
                matched_app_block = Some(app_match.to_string());
            }
        }

        let launch_options_regex = Regex::new(r#"\t{6}"LaunchOptions"\t{2}"([(\-\w)\s]*)""#)
            .expect("Constructing LaunchOptions regex");

        Ok(LaunchOptionsV2 {
            local_config: config_path,
            launch_args_regex: launch_options_regex,
            app_data: matched_app_block,
            new_app_data: None,
        })
    }

    /// Regex searches the acquired app data for the launch options Key-Value pair, and cross references
    /// the present launch options against the const list of required options. Can fail if there is no
    /// 'LaunchOptions' key present in that app object. This is usually because the user has never configured any
    /// launch options, and so this is highly likely. Error should be handled gracefully.
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
        let current_args = self
            .launch_args_regex
            .find(&app_data)
            .context("Failed to find launch args object.")?;

        let mat_str = current_args.as_str();
        TF2_REQUIRED_OPTS.iter().for_each(|opt| {
            if !mat_str.contains(opt) {
                tracing::debug!("Launch Arguments: Missing argument identified -> {}", opt);
                missing_args.insert(0, opt);
            }
        });

        Ok(missing_args)
    }

    /// Writes the changes made by `Self::add_opts_if_missing()` into the `localconfig.vdf` file via String
    /// search and replace, as its cheaper than VDF parsing the whole file, serialising and deserialising the
    /// updated objects back into the file. This can fail if `Self::add_opts_if_missing()` has not been called
    /// prior to this function, or `self.new_app_data` is None. IO failures (such as OS write locks encountered
    /// when steam is updating the file itself) can also occur and should be handled gracefully.
    ///
    /// In its present state, this functionality is only guarenteed to work if Steam is CLOSED when this runs,
    /// as Steam keeps a copy of this data cached in memory, and intermitantly writes it out to this file,
    /// overwriting these changes. It also writes out when Steam is closed. But if you write to this file when
    /// Steam is closed, it will load the changes upon launch.
    pub fn write_changes_to_file(&self) -> Result<(), anyhow::Error> {
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

    /// Similar to `self.check_missing_args` except will clone the current app data into `self.new_app_data` and
    /// modify the LaunchOptions key (creating one if necessary) to add the required launch options if not present.
    /// If `self.app_data` is None, this is a no-op.
    pub fn add_opts_if_missing(&mut self) {
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
