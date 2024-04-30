use crate::{
    demo::{DemoEvent, DemoMessage, VoteCastEventWrapped, VoteRelatedEvent},
    state::MACState,
    console::{ConsoleOutput, SerializableEvent},
    web::broadcast_event,
};
use event_loop::{try_get, Handled, HandlerStruct, Is};
use std::collections::HashMap;

/// This struct will house the relevant vars used by various message handlers when broadcasting
/// Encapsulates a set of functions to invoke the `web::broadcast_event` function with the 
/// appropriately serialised JSON messages.
pub struct BroadcastableEvent {
    /// Used when handling certain `DemoMessage` messages (I.e. `VoteStarted`)
    votes: HashMap<u32, Vec<String>>,
}

impl BroadcastableEvent {
    #[must_use]
    pub fn new() -> Self {
        Self {
            votes: HashMap::new(),
        }
    }
}

impl Default for BroadcastableEvent {
    fn default() -> Self {
        Self::new()
    }
}

/// We 'broadcast' events to subscribers using SSE (Server Sent Events) publishing to push events to
/// clients without them having to poll us. This is essentially a reverse API, where we expose the
/// endpoint, but data is shipped when _we_ want and the clients have to respond.
/// 
/// See `broadcast_event` in `crate::web` for more info 
impl<IM, OM> HandlerStruct<MACState, IM, OM> for BroadcastableEvent
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
        let event_json = if try_get::<DemoMessage>(message).is_some() {
            let demo_msg = try_get::<DemoMessage>(message)?;
            self.handle_demo_message(state, demo_msg)
        } else if try_get::<ConsoleOutput>(message).is_some() {
            let con_msg = try_get::<ConsoleOutput>(message)?;
            self.handle_console_message(state, con_msg)
        } else {
            None
        };
        if let Some(json) = event_json {
            Handled::<OM>::future(async move {
                broadcast_event(json).await;
                None
            })
        } else {
            None
        }
    }
}

impl BroadcastableEvent {
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
                    .unwrap_or("Someone");

                let vote: &str = self
                    .votes
                    .get(&event.voteidx)
                    .and_then(|v| v.get(event.vote_option as usize))
                    .map_or::<&str, _>("Invalid vote", |s| s);

                let mut wrapper = VoteCastEventWrapped::from_vote_cast_event(event, steamid);
                wrapper.set_choice(vote.to_string());
                wrapper.set_voter(steamid);
                wrapper.set_vote_name(name.to_string());
                let res = VoteRelatedEvent::make_from(wrapper);
                Some(serde_json::to_string(&res).expect("Serialization failure"))
            }
            DemoEvent::LastestTick => None,
        }
    }
}
