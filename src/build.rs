
#[allow(dead_code)]
pub async fn is_ui_updated(update_url: &str, hash_str: &str) -> Result<(), Box<dyn std::error::Error>> {

    // Fetch the current HEAD commit hash from the remote repository
    // url example"https://api.github.com/repos/MegaAntiCheat/MegaAntiCheat-UI/git/refs/heads/main";
    let client = reqwest::Client::new();
    let resp = client
        .get(update_url)
        .header("User-Agent", "mac-client")
        .send()
        .await?;

    if resp.status().is_success() {
        let text = resp.text().await?;
        let json_resp: serde_json::Value = serde_json::from_str(&text)?;

        if let Some(current_commit_hash) = json_resp["object"]["sha"].as_str() {
            if current_commit_hash != hash_str {
                tracing::info!("New UI update available.");
            }
        } else {
            tracing::debug!("Failed to get the current commit hash.");
        }
    } else {
        tracing::debug!(
            "Failed to fetch data from repo. Status: {}",
            resp.status()
        );
    }

    Ok(())
}
