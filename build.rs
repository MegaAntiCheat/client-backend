use git2::{Repository, Error};
use std::{env, process::Command};
use execute::Execute;

pub const OFFICIAL_UI_REPO: &str = "https://github.com/MegaAntiCheat/MegaAntiCheat-UI";

pub const STANDARD_STAGING_DIR: &str = "./bundle/ui";

pub const STANDARD_WORKING_BRANCH: &str = "main";

pub const STANDARD_BUILD_CMD: &str = "pnpm i;pnpm exec webpack";

pub const STANDARD_BUILD_DIR: &str = "dist";

pub struct Config {
    /// Bundle and bundle with an unofficial UI, as provided by the given (git cloneable) URL
    pub ui_repo: String,
    /// The staging dir that the build script uses, by default is ./bundle/ui/, cannot be an existing UI.
    /// Note: The build script will remove all files from this directory during build.
    pub staging_dir: String,
    /// The branch to pull updates from
    pub working_branch: String,
    /// The command(s) to build the staging dir into a portable dir
    pub build_cmd: Vec<String>,
    /// The name of the resulting build folder
    pub build_dir: String,
}

impl Config {
    /// Build a `Config` struct by searching this procs Env Vars for specific keys.
    pub fn build() -> Config {
        let ui_repo = match env::var("UI_REPO") {
            Ok(url) => url,
            Err(_) => OFFICIAL_UI_REPO.to_string(),
        };

        let staging_dir = match env::var("STAGING_DIR") {
            Ok(dir) => dir,
            Err(_) => STANDARD_STAGING_DIR.to_string(),
        };

        let working_branch = match env::var("WORKING_BRANCH") {
            Ok(branch) => branch,
            Err(_) => STANDARD_WORKING_BRANCH.to_string(),
        };

        let build_cmd: Vec<String> = match env::var("BUILD_CMD") {
            Ok(cmd) => cmd.split(';').map(str::to_string).collect(),
            Err(_) => STANDARD_BUILD_CMD.split(';').map(str::to_string).collect(),
        };

        let build_dir = match env::var("BUILD_DIR") {
            Ok(dir) => dir,
            Err(_) => STANDARD_BUILD_DIR.to_string(),
        };

        Config {
            ui_repo,
            staging_dir,
            working_branch,
            build_cmd,
            build_dir
        }
    }

    fn fast_forward(&self, repo: &Repository) -> Result<(), Error> {
        repo.find_remote("origin")?
            .fetch(&[self.working_branch.as_str()], None, None)?;
    
        let fetch_head = repo.find_reference("FETCH_HEAD")?;
        let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;
        let analysis = repo.merge_analysis(&[&fetch_commit])?;
        if analysis.0.is_up_to_date() {
            Ok(())
        } else if analysis.0.is_fast_forward() {
            let refname = format!("refs/heads/{}", self.working_branch);
            let mut reference = repo.find_reference(&refname)?;
            reference.set_target(fetch_commit.id(), "Fast-Forward")?;
            repo.set_head(&refname)?;
            repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))
        } else {
            Err(Error::from_str("Fast-forward only!"))
        }
    }
}


pub fn main() {
    let configurations = Config::build();

    let _ = match Repository::open(configurations.staging_dir.as_str()) {
        Ok(repo) => {
            match configurations.fast_forward(&repo) {
                Ok(()) => (),
                Err(why) => panic!("Could not fast forward staging repo: {}", why),
            };
            repo
        },
        Err(_) => {
            match Repository::clone(
                configurations.ui_repo.as_str(),
                configurations.staging_dir.as_str()) {
                Ok(repo) => repo,
                Err(e) => panic!("Failed to clone UI repo: {}", e),
            }
        },
    };

    

} 