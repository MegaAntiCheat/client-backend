use std::{path::PathBuf, sync::Arc};

use crate::io::gamefinder::GameFinder;

pub struct Settings {
    pub tf2_directory: PathBuf,
    pub rcon_password: Arc<str>,
}

impl Default for Settings {
    fn default() -> Self {
        let mut game_finder = GameFinder;
        let tf2_directory = game_finder.read_tf2_folder();

        match tf2_directory {
            Some(path) => {
                println!("TF2 Folder found at {:?}", path);
                Settings {
                    tf2_directory: path,
                    rcon_password: "mac_rcon".into(),
                }
            }
            None => {
                println!("FATAL ERROR: TF2 Folder not found.");
                // Return a default Settings with a placeholder value for tf2_directory
                Settings {
                    tf2_directory: PathBuf::new(),
                    rcon_password: Arc::from(""),
                }
            }
        }
    }
}
