use std::sync::Arc;

use reqwest::Response;
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Request failed: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("Failed to parse url: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("Request failed: {0}")]
    Failed(String),
}

#[derive(Debug, Clone, Deserialize)]
struct SessionID {
    session_id: Arc<str>,
}

#[derive(Debug)]
pub struct DemoSession {
    close: Sender<()>,
    session_id: Arc<str>,
}

impl Drop for DemoSession {
    fn drop(&mut self) {
        self.close
            .blocking_send(())
            .map_err(|e| {
                tracing::error!(
                    "Failed to request closing of session {}: {e}",
                    self.session_id
                );
            })
            .ok();
    }
}

/// # Errors
/// * Url failed to parse (likely indicating an invalid `endpoint`, `key`,
///   `fake_ip`, or `map`)
/// * Network error
/// * Invalid response
///
/// # Panics
/// This function spawns a task responsible for closing the session out when the
/// resulting `DemoSession` is dropped. This task can panic if the `DemoSession`
/// fails to notify it, which should only be possible if the async runtime stops
/// before the session is dropped, meaning this shouldn't be able to occur.
pub async fn new_demo_session(
    host: Arc<str>,
    key: Arc<str>,
    fake_ip: Arc<str>,
    map: Arc<str>,
) -> Result<DemoSession, Error> {
    let params = [("api_key", &key), ("fake_ip", &fake_ip), ("map", &map)];

    let url = reqwest::Url::parse_with_params(&format!("{host}/session_id"), params)?;
    let response = reqwest::get(url).await?;

    if !response.status().is_success() {
        return Err(Error::Failed(response.text().await?));
    }

    let session_id: SessionID = response.json().await?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    tokio::task::spawn(async move {
        rx.recv()
            .await
            .expect("Didn't get closing message from DemoSession.");

        match force_close_session(host, key).await {
            Ok(_) => tracing::info!("Closed session."),
            Err(e) => tracing::error!("Failed to close session: {e:?}"),
        }
    });

    Ok(DemoSession {
        close: tx,
        session_id: session_id.session_id,
    })
}

/// # Errors
/// * Fails to parse Url (usually indicating a bad host or key was provided)
/// * Web request failed
pub async fn force_close_session(host: Arc<str>, key: Arc<str>) -> Result<Response, Error> {
    let params = [("api_key", &key)];

    let url = reqwest::Url::parse_with_params(&format!("{host}/close_session"), params)?;

    Ok(reqwest::get(url).await?)
}
