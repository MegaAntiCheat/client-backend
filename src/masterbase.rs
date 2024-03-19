use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use futures::SinkExt;
use reqwest::{Response, RequestBuilder, Client};
use serde::Deserialize;
use thiserror::Error;
use tokio::{net::TcpStream, sync::mpsc::Sender};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Request failed: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("Failed to parse url: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("Websocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::error::Error),
    #[error("Request failed: {0}")]
    Failed(String),
}

#[derive(Deserialize)]
struct SessionID {
    session_id: u128,
    #[serde(skip)]
    close: Option<Sender<()>>,
}

impl Drop for SessionID {
    fn drop(&mut self) {
        self.close.as_ref().map(|c| {
            c.try_send(()).map_err(|e| {
                tracing::error!(
                    "Failed to request closing of session {}: {e}",
                    self.session_id
                );
            })
        });
    }
}

impl Debug for SessionID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SessionID({})", self.session_id)
    }
}

impl Display for SessionID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.session_id)
    }
}

pub type WebSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;
pub struct DemoSession {
    session_id: SessionID,
    ws_client: WebSocket,
}

impl DemoSession {
    /// # Errors
    /// If the client failed to send the bytes
    pub async fn send_bytes(
        &mut self,
        bytes: Vec<u8>,
    ) -> Result<(), tokio_tungstenite::tungstenite::Error> {
        self.ws_client
            .send(tokio_tungstenite::tungstenite::Message::Binary(bytes))
            .await
    }
}

impl Debug for DemoSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DemoSession({})", self.session_id)
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
    fake_ip: &str,
    map: &str,
    http: bool,
) -> Result<DemoSession, Error> {
    let params: [(&str, &str); 3] = [("api_key", &key), ("fake_ip", &fake_ip), ("map", &map)];

    // Request to start session

    let endpoint = if http {
        format!("http://{host}/session_id")
    } else {
        format!("https://{host}/session_id")
    };
    let url = reqwest::Url::parse_with_params(&endpoint, params)?;
    let response = reqwest::get(url).await?;

    if !response.status().is_success() {
        return Err(Error::Failed(response.text().await?));
    }

    let mut session_id: SessionID = response.json().await?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    session_id.close = Some(tx);

    // Wait for the dropped `DemoSession` to tell it to close the session.
    // When it receives a message over the channel, it makes the appropriate request.
    {
        let id = session_id.session_id;
        let host = host.clone();
        let key = key.clone();
        tokio::task::spawn(async move {
            rx.recv()
                .await
                .expect("Didn't get closing message from DemoSession.");

            match force_close_session(host, key, http).await {
                Ok(_) => tracing::info!("Closed session {id}."),
                Err(e) => tracing::error!("Failed to close session: {e:?}"),
            }
        });
    }

    // Open Websocket
    let params: [(&str, &str); 2] = [("api_key", &key), ("session_id", &session_id.to_string())];
    let url = reqwest::Url::parse_with_params(&format!("ws://{host}/demos"), params)?;
    let (ws_client, _) = tokio_tungstenite::connect_async(url).await?;

    Ok(DemoSession {
        session_id,
        ws_client,
    })
}

/// Forcefully closes the demo session if one was open.
/// If `response.status().is_success()`, then a session was closed.
/// If the reponse was unsuccessful but `response.status().is_server_error()`
/// is false, there was probably no session open to start with.
///
/// # Errors
/// * Fails to parse Url (usually indicating a bad host or key was provided)
/// * Web request failed
pub async fn force_close_session(
    host: Arc<str>,
    key: Arc<str>,
    http: bool,
) -> Result<Response, Error> {
    let params = [("api_key", &key)];

    let endpoint = if http {
        format!("http://{host}/close_session")
    } else {
        format!("https://{host}/close_session")
    };
    let url = reqwest::Url::parse_with_params(&endpoint, params)?;

    Ok(reqwest::get(url).await?)
}

pub async fn send_late_bytes(
    host: Arc<str>,
    key: Arc<str>,
    http: bool,
    bytes: Vec<u8>
) -> Result<Response, Error> {
    let params = [("api_key", &key)];

    let endpoint = if http {
        format!("http://{host}/late_bytes")
    } else {
        format!("https://{host}/late_bytes")
    };

    let url = reqwest::Url::parse_with_params(&endpoint, params)?;

    let client = Client::new();
    let req: RequestBuilder = client.post(url).body(bytes);
    
    Ok(req.send().await?)
}
