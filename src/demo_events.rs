use super::web::broadcast_event;
use crate::{
    demo::{DemoEvent, DemoMessage, VoteCastEventWrapped, VoteRelatedEvent},
    state::MACState,
};
use event_loop::{try_get, Handled, HandlerStruct, Is};
use std::collections::HashMap;

pub struct SseBroadcastVotes {
    votes: HashMap<u32, Vec<String>>,
}

impl SseBroadcastVotes {
    #[must_use]
    pub fn new() -> Self {
        Self {
            votes: HashMap::new(),
        }
    }
}

impl Default for SseBroadcastVotes {
    fn default() -> Self {
        Self::new()
    }
}

impl<IM, OM> HandlerStruct<MACState, IM, OM> for SseBroadcastVotes
where
    IM: Is<DemoMessage>,
{
    #[allow(clippy::cognitive_complexity)]
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        let msg = try_get(message)?;
        let cloned_msg = msg.clone();
        if let Some(event_json) = match cloned_msg.event {
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
        } {
            Handled::<OM>::future(async move {
                broadcast_event(event_json).await;
                None
            })
        } else {
            None
        }
    }
}
