use std::fmt::{Debug, Display, Write};

use futures::SinkExt;
use reqwest::{Client, RequestBuilder, Response};
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;
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

/// When the session was created, a task was opened to wait for a close request.
/// When the session is dropped, it's ID is sent to the task so that it can send
/// a close session request to the masterbase.
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

    host: String,
    key: String,
    http: bool,
}

impl DemoSession {
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
    pub async fn new(
        host: String,
        key: String,
        fake_ip: &str,
        map: &str,
        demo_name: &str,
        http: bool,
    ) -> Result<Self, Error> {
        let params: [(&str, &str); 4] = [
            ("api_key", &key),
            ("fake_ip", fake_ip),
            ("map", map),
            ("demo_name", demo_name),
        ];

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

        // Open Websocket
        let params: [(&str, &str); 2] =
            [("api_key", &key), ("session_id", &session_id.to_string())];
        let ws_endpoint = if http {
            format!("ws://{host}/demos")
        } else {
            format!("wss://{host}/demos")
        };
        let url = reqwest::Url::parse_with_params(&ws_endpoint, params)?;

        // Wait for the dropped `DemoSession` to tell it to close the session.
        // When it receives a message over the channel, it makes the appropriate
        // request.
        {
            let host = host.clone();
            let key = key.clone();
            let id = session_id.session_id;
            // let host = host.to_owned();
            // let key = key.to_owned();
            tokio::task::spawn(async move {
                rx.recv()
                    .await
                    .expect("Didn't get closing message from DemoSession.");

                match force_close_session(&host, &key, http).await {
                    Ok(_) => tracing::info!("Closed session {id}."),
                    Err(e) => tracing::error!("Failed to close session: {e:?}"),
                }
            });
        }

        let (ws_client, _) = tokio_tungstenite::connect_async(url).await?;

        Ok(Self {
            session_id,
            ws_client,
            host,
            key,
            http,
        })
    }

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

    /// # Errors
    /// If the web request failed for some reason
    pub async fn report_player(&mut self, player: SteamID) -> Result<Response, Error> {
        let params: [(&str, &str); 3] = [
            ("api_key", &self.key),
            ("target_steam_id", &format!("{}", u64::from(player))),
            ("session_id", &format!("{}", self.session_id)),
        ];

        let endpoint = if self.http {
            format!("http://{}/report", self.host)
        } else {
            format!("https://{}/report", self.host)
        };
        let url = reqwest::Url::parse_with_params(&endpoint, params)?;

        let client = reqwest::Client::builder().build()?;
        let resp = client.execute(client.post(url).build()?).await?;

        Ok(resp)
    }

    /// # Errors
    /// If the web request to send late bytes was unsuccessful
    pub async fn send_late_bytes(&self, bytes: Vec<u8>) -> Result<Response, Error> {
        #[derive(Serialize)]
        struct LateBytes {
            late_bytes: String,
        }

        let params = [("api_key", &self.key)];

        let endpoint = if self.http {
            format!("http://{}/late_bytes", self.host)
        } else {
            format!("https://{}/late_bytes", self.host)
        };

        let url = reqwest::Url::parse_with_params(&endpoint, params)?;

        let client = Client::new();
        let late_bytes_hex: String =
            bytes
                .iter()
                .fold(String::with_capacity(bytes.len() * 2), |mut s, byte| {
                    write!(&mut s, "{byte:02x}").expect("Couldn't write to string??");
                    s
                });

        let req: RequestBuilder = client.post(url).json(&LateBytes {
            late_bytes: late_bytes_hex,
        });

        Ok(req.send().await?)
    }
}

impl Debug for DemoSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DemoSession({})", self.session_id)
    }
}

/// Forcefully closes the active demo session if one was open.
/// If `response.status().is_success()`, then a session was closed.
/// If the response was not successful but response status is 403,
/// no session was probably open.
/// Any other errors are actual errors.
///
/// # Errors
/// * Fails to parse Url (usually indicating a bad host or key was provided)
/// * Web request failed
pub async fn force_close_session(host: &str, key: &str, http: bool) -> Result<Response, Error> {
    let params = [("api_key", key)];

    let endpoint = if http {
        format!("http://{host}/close_session")
    } else {
        format!("https://{host}/close_session")
    };
    let url = reqwest::Url::parse_with_params(&endpoint, params)?;

    Ok(reqwest::get(url).await?)
}
