use std::{fs::File, io::Read, path::PathBuf};

use anyhow::{Context, Result};
use regex::{Match, Regex};
use steamid_ng::SteamID;
use substring::Substring;

use crate::gamefinder::locate_steam_launch_configs;

pub const TF2_GAME_ID: &str = "440";
pub const CONDEBUG: &str = "-condebug";
pub const CONCLEARLOG: &str = "-conclearlog";
pub const USERCON: &str = "-usercon";
pub const G15: &str = "-g15";
pub fn get_required_args_vec() -> Vec<String> {
    let args: &[&str; 5] = &[TF2_GAME_ID, CONDEBUG, CONCLEARLOG, USERCON, G15];
    let mut buf: Vec<String> = Vec::new();
    args.iter().for_each(|str| buf.insert(0, str.to_string()));
    buf
}

pub struct LaunchOptionsV2 {
    local_config: PathBuf,
    target_app: String,
    launch_args_regex: Regex,
    app_data: Option<String>,
    new_app_data: Option<String>,
    required_options: Vec<String>,
}

/// TODO: Implement a 'write_new_app_data_to_file' fn to write the updated launch options back to the
/// file specified by 'local_config' struct element.
/// TODO: Add more tracing to this to be nicer to the user.
impl LaunchOptionsV2 {
    pub fn new(user: SteamID, target_app: String) -> Result<LaunchOptionsV2, anyhow::Error> {
        let required_opts = get_required_args_vec();
        let config_path: PathBuf = locate_steam_launch_configs(user)
            .context("Failed to find localconfig.vdf in Steam dir.")?;
        let mut data: Vec<u8> = Vec::new();
        tracing::debug!("Found Steam local app config.");

        if let Ok(mut f) = File::open(&config_path) {
            f.read_to_end(&mut data)
                .context(format!("Failed to read {:?}", config_path))?;
        }
        let binding = &String::from_utf8_lossy(&data);

        let apps_regex_raw = r#"\t{4}"apps"([\s\S]+)\t{5}}"#;
        let apps_regex =
            Regex::new(apps_regex_raw).context("Regex construction failed, good work.")?;

        let caps = apps_regex
            .captures(binding)
            .context("No capture groups found -  no apps list present in localconfig.vdf.")?;

        let mut matched_app_block: Option<String> = None;
        let open_match = format!("\t\t\t\t\t\"{}\"", target_app);

        // caps should be an iter of one element.
        caps.iter().for_each(|mat_opt: Option<Match<'_>>| {
            if let Some(mat) = mat_opt {
                let latter_portion = mat
                    .as_str()
                    .find(&open_match)
                    .context(
                        "Could not find specified app in localconfig.vdf for the current user.",
                    )
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
        });

        let launch_options_regex =
            Regex::new(r#"\t{6}"LaunchOptions"\t{2}"([(\-\w)\s]*)""#).unwrap();

        Ok(LaunchOptionsV2 {
            local_config: config_path,
            target_app,
            launch_args_regex: launch_options_regex,
            app_data: matched_app_block,
            new_app_data: None,
            required_options: required_opts,
        })
    }

    pub fn check_missing_args(&self) -> Result<Vec<&str>, anyhow::Error> {
        tracing::info!("Checking for missing launch arguments in specified app...");
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
        self.required_options.iter().for_each(|opt| {
            if !mat_str.contains(opt) {
                tracing::debug!("Launch Arguments: Missing argument identified -> {}", opt);
                missing_args.insert(0, opt);
            }
        });

        if !missing_args.is_empty() {
            tracing::warn!("Missing arguments found in launch options for specified app...");
            tracing::warn!("Missing arguments: {:?}", missing_args);
        } else {
            tracing::info!("All required launch arguments are present!");
        }
        Ok(missing_args)
    }

    pub fn print_app_data(&self) {
        println!("Old: {:#?}", self.app_data);
        println!("New: {:#?}", self.new_app_data);
    }

    pub fn add_opts_if_missing(&mut self) {
        let copied_app_data = self.app_data.clone();
        if let Some(mut prior) = copied_app_data {
            if !prior.contains("\"LaunchOptions\"") {
                prior += "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"\""
            }

            self.required_options.iter().for_each(|opt| {
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

            self.new_app_data = Some(prior);
        }
    }
}
