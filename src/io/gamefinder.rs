use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct GameFinder;

impl GameFinder {
    #[cfg(target_os = "windows")]
    fn find_default_lib(&mut self) -> PathBuf {
        let default_dir = "C:/Program Files (x86)/Steam/steamapps/";

        #[cfg(not(target_os = "windows"))]
        let default_dir = {
            use std::env::var_os;
            var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or("~".into())
                .join(".steam/steam/steamapps/")
        };

        return default_dir.into();
    }

    fn fetch_libraryfolders(&mut self) -> Vec<PathBuf> {
        log::debug!("Attempting to open libraryfolders.vdf");
        const LIBFILE: &str = "libraryfolders.vdf";
        let libraryfolders = fs::read_to_string(Path::join(&self.find_default_lib(), LIBFILE));

        match libraryfolders {
            Ok(content) => {
                let mut paths = Vec::new();
                let lines = content.lines();

                for line in lines {
                    if line.contains("path") {
                        if let Some(path_str) = line.split('"').nth(3) {
                            paths.push(PathBuf::from(path_str));
                        }
                    }
                }

                log::debug!("Successfully read libraryfolders");
                paths
            }
            Err(err) => {
                log::error!("Failed to read libraryfolders.vdf: {}", err);
                Vec::new()
            }
        }
    }

    fn verify_tf_location(&mut self, lib: &Path) -> bool {
        log::debug!("Start TF2 Verification of {:?}", lib.to_string_lossy());
        let gameinfo = "tf/gameinfo.txt";
        let mut path = lib.to_path_buf();
        path.push(gameinfo);

        if path.exists() {
            log::debug!("Passed Verification Check, Using.");
            return true;
        }
        log::debug!("Failed Verification Check");
        return false;
    }

    pub fn read_tf2_folder(&mut self) -> Option<PathBuf> {
        log::debug!("Fetching TF2 Folder");
        let tf2_folder: &str = "steamapps/common/Team Fortress 2";
        let libs: Vec<PathBuf> = self.fetch_libraryfolders();

        for lib in libs {
            let mut path = lib.to_path_buf();
            path.push(tf2_folder);

            if path.exists() && self.verify_tf_location(&path) {
                log::debug!("Found TF2 Folder: {:?}", path);
                println!("Using: {}", path.to_string_lossy());
                return Some(path);
            }
        }
        None
    }
}
