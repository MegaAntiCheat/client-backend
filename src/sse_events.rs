use crate::{
    console::ConsoleOutput,
    demo::{DemoEvent, DemoMessage},
    io::regexes::{ChatMessage, DemoStop, PlayerKill},
    state::MACState,
    web::broadcast_event,
};
use chrono::{DateTime, Utc};
use event_loop::{try_get, Handled, HandlerStruct, Is};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use steamid_ng::SteamID;
use tf_demo_parser::demo::gameevent_gen::{VoteCastEvent, VoteOptionsEvent, VoteStartedEvent};
use uuid::Uuid;

/// This is useful, but not implemented well. This needs to be either reworked and reimplemented, or utilised better.
/// TODO: fix this
pub trait SerializableConsoleOutput {
    /// Just returns a string name for the child wrapped by the `ConsoleOutput` type
    /// i.e. `ConsoleOutput(ChatMessage)` -> `"ChatMessage"`
    fn get_type(&self) -> String;
}

impl SerializableConsoleOutput for ChatMessage {
    fn get_type(&self) -> String {
        "ChatMessage".to_string()
    }
}
impl SerializableConsoleOutput for PlayerKill {
    fn get_type(&self) -> String {
        "PlayerKill".to_string()
    }
}
impl SerializableConsoleOutput for DemoStop {
    fn get_type(&self) -> String {
        "DemoStop".to_string()
    }
}

/// Wraps the type (that is wrapped by `ConsoleOutput`) with use json data, such as the event type,
/// timestamp and even a uuid.  
#[derive(Serialize, Deserialize)]
pub struct SerializableEvent<T: SerializableConsoleOutput> {
    #[serde(rename = "type")]
    event_type: String,
    uuid: Uuid,
    time: DateTime<Utc>,
    event: T,
}

impl<T> SerializableEvent<T>
where
    T: SerializableConsoleOutput,
{
    /// Make a `SerializableEvent` from the type wrapped by `ConsoleOutput`, only if that type
    /// implements the `SerializableConsoleOutput` trait
    pub fn make_from(console_output_child: T) -> Self {
        SerializableEvent {
            event_type: console_output_child.get_type(),
            uuid: Uuid::new_v4(),
            time: Utc::now(),
            event: console_output_child,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct VoteCastEventWrapped {
    pub voter: Option<SteamID>,
    pub voter_name: Option<String>,
    pub choice: Option<String>,
    event: VoteCastEvent,
}

impl VoteCastEventWrapped {
    #[must_use]
    pub fn from_vote_cast_event(event: VoteCastEvent, voter: Option<SteamID>) -> Self {
        Self {
            voter,
            voter_name: None,
            choice: None,
            event,
        }
    }
}

pub trait SerializableVoteEventContent {
    fn event_name(&self) -> String;
}

impl SerializableVoteEventContent for Box<VoteOptionsEvent> {
    fn event_name(&self) -> String {
        "VoteStarted".to_string()
    }
}

impl SerializableVoteEventContent for VoteCastEventWrapped {
    fn event_name(&self) -> String {
        "VoteCast".to_string()
    }
}

impl SerializableVoteEventContent for VoteStartedEvent {
    fn event_name(&self) -> String {
        "VoteCreated".to_string()
    }
}

#[derive(Serialize, Deserialize)]
pub struct VoteRelatedEvent<T: SerializableVoteEventContent> {
    #[serde(rename = "type")]
    event_type: String,
    content: T,
    time: DateTime<Utc>,
    uuid: Uuid,
}

impl<T> VoteRelatedEvent<T>
where
    T: SerializableVoteEventContent,
{
    pub fn make_from(vote_event: T) -> Self {
        Self {
            event_type: vote_event.event_name(),
            content: vote_event,
            time: Utc::now(),
            uuid: Uuid::new_v4(),
        }
    }
}

/// This struct will house the relevant vars used by various message handlers when broadcasting
/// Encapsulates a set of functions to invoke the `web::broadcast_event` function with the
/// appropriately serialised JSON messages.
pub struct SseEventBroadcaster {
    /// Used when handling certain `DemoMessage` messages (I.e. `VoteStarted`)
    votes: HashMap<u32, Vec<String>>,
}

impl SseEventBroadcaster {
    #[must_use]
    pub fn new() -> Self {
        Self {
            votes: HashMap::new(),
        }
    }
}

impl Default for SseEventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

/// We 'broadcast' events to subscribers using SSE (Server Sent Events) publishing to push events to
/// clients without them having to poll us. This is essentially a reverse API, where we expose the
/// endpoint, but data is shipped when _we_ want and the clients have to respond.
///
/// See `broadcast_event` in `crate::web` for more info
impl<IM, OM> HandlerStruct<MACState, IM, OM> for SseEventBroadcaster
where
    IM: Is<DemoMessage> + Is<ConsoleOutput>,
{
    /// `IM` can be a message that is either a `DemoMessage` or a `ConsoleOutput` message
    /// These are then dispatched to the appropriate discrete message handler, which take the
    /// direct message (i.e. not wrapped in Is<> or as a generic), then return `Option<String>` for
    /// the message serialisation.
    /// If a `String` was returned, we broadcast that message to all subscribers.
    #[allow(clippy::cognitive_complexity)]
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        let event_json = if let Some(demo_msg) = try_get::<DemoMessage>(message) {
            self.handle_demo_message(state, demo_msg)
        } else if let Some(con_msg) = try_get::<ConsoleOutput>(message) {
            self.handle_console_message(state, con_msg)
        } else {
            None
        };
        if let Some(json) = event_json {
            return Handled::<OM>::future(async move {
                broadcast_event(json).await;
                None
            });
        } 
        None
    }
}

impl SseEventBroadcaster {
    /// Handling the console message requires injecting certain variables into the `ConsoleOutput` wrapped values,
    /// as not all of them are populated when instantiated. We use the current `MACState` to pull out the relevant
    /// values, then insert in the necessary place on a case-by-case basis.
    /// Allow unused self (non-static method that can be static) as self may be used in future when we add more
    /// `ConsoleOutput` types to handle.
    #[allow(clippy::unused_self)]
    fn handle_console_message(&self, state: &MACState, message: &ConsoleOutput) -> Option<String> {
        let cloned_co = message.clone();

        // We also set the steam_id fields in the events here before we serialise
        match cloned_co {
            ConsoleOutput::Chat(mut m) => {
                let players = state.players.get_name_to_steam_ids_map();
                if let Some(id) = players.get(&m.player_name) {
                    m.set_steam_id(*id);
                }
                let event = SerializableEvent::make_from(m);
                Some(serde_json::to_string(&event).expect("Serialisation failure"))
            }
            ConsoleOutput::Kill(mut m) => {
                let players = state.players.get_name_to_steam_ids_map();
                if let Some(id) = players.get(&m.killer_name) {
                    m.set_steam_id_killer(*id);
                }
                if let Some(id) = players.get(&m.victim_name) {
                    m.set_steam_id_victim(*id);
                }
                let event = SerializableEvent::make_from(m);
                Some(serde_json::to_string(&event).expect("Serialisation failure"))
            }
            ConsoleOutput::DemoStop(m) => {
                let event = SerializableEvent::make_from(m);
                Some(serde_json::to_string(&event).expect("Serialisation failure"))
            }
            _ => None,
        }
    }

    /// Handling `DemoMessages` often requires inspecting or modifying the list of votes that have been
    /// cast. I.e. a `VoteCast` event wont contain information about what the vote options were. So we
    /// have to keep this context ourselves.
    fn handle_demo_message(&mut self, state: &MACState, message: &DemoMessage) -> Option<String> {
        let cloned_msg = message.clone();
        match cloned_msg.event {
            DemoEvent::VoteStarted(options) => {
                let mut values = Vec::new();
                for i in 0..options.count {
                    let opt = match i {
                        0 => options.option_1.to_string(),
                        1 => options.option_2.to_string(),
                        2 => options.option_3.to_string(),
                        3 => options.option_4.to_string(),
                        4 => options.option_5.to_string(),
                        _ => String::new(),
                    };
                    values.push(opt);
                }
                self.votes.insert(options.voteidx, values);

                let res = VoteRelatedEvent::make_from(options);
                Some(serde_json::to_string(&res).expect("Serialisation failure"))
            }
            DemoEvent::VoteCast(event, steamid) => {
                let name = steamid
                    .as_ref()
                    .and_then(|&id| state.players.get_name(id))
                    .map(std::string::ToString::to_string);

                let vote = self
                    .votes
                    .get(&event.voteidx)
                    .and_then(|v| v.get(event.vote_option as usize))
                    .map(std::string::ToString::to_string);

                let mut wrapper = VoteCastEventWrapped::from_vote_cast_event(event, steamid);
                wrapper.voter = steamid;
                wrapper.voter_name = name;
                wrapper.choice = vote;
                let res = VoteRelatedEvent::make_from(wrapper);
                Some(serde_json::to_string(&res).expect("Serialization failure"))
            }
            DemoEvent::VoteCreated(event) => {
                let res = VoteRelatedEvent::make_from(event);
                Some(serde_json::to_string(&res).expect("Serialisation failure"))
            }
            DemoEvent::LastestTick => None,
        }
    }
}
