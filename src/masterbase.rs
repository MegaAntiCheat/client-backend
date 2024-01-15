use std::net::TcpStream;

use thiserror::Error;
use websocket::{sync::Client, url::ParseError, ClientBuilder};

pub struct Masterbase {
    key: String,
    remote: String,
}

pub struct Session {
    session_id: String,
    websocket: Client<TcpStream>,
}

#[derive(Debug, Error)]
pub enum MasterbaseError {
    #[error("Http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Websocket error: {0}")]
    Websocket(#[from] websocket::WebSocketError),
    #[error("Failed to parse websocket url: {0}")]
    WebsocketParseError(#[from] ParseError),
}

impl Masterbase {
    pub fn new(key: String, remote: String) -> Masterbase {
        Masterbase { key, remote }
    }

    pub fn new_session(&self, fake_ip: String, map: String) -> Result<Session, MasterbaseError> {
        let http = reqwest::blocking::Client::new();
        let response = http
            .get(format!("http://{}/session_id", self.remote))
            .query(&[("api_key", &self.key), ("fake_ip", &fake_ip), ("map", &map)])
            .send()?;

        let session_id = "todo".into();

        let ws =
            ClientBuilder::new("ws://{}/demos?api_key={}&session_id={}")?.connect_insecure()?;

        Ok(Session {
            session_id: session_id,
            websocket: ws,
        })
    }
}
