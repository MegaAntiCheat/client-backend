use std::collections::HashMap;

use super::console::ConsoleOutput;
use super::web::broadcast_event;
use crate::state::MACState;
use event_loop::{try_get, Handled, HandlerStruct, Is};
use steamid_ng::SteamID;

// Messages *********************

// pub struct NewConsoleMessage(pub Option<String>);

// Handlers *********************

#[allow(clippy::module_name_repetitions)]
pub struct SseBroadcastMessages;
impl<IM, OM> HandlerStruct<MACState, IM, OM> for SseBroadcastMessages
where
    IM: Is<ConsoleOutput>,
{
    fn handle_message(
        &mut self,
        state: &MACState,
        message: &IM,
    ) -> Option<event_loop::Handled<OM>> {
        let console_out: &ConsoleOutput = try_get(message)?;
        let cloned_co = console_out.clone();

        let player_names_to_ids: HashMap<String, SteamID> = state
            .players
            .connected
            .iter()
            .filter_map(|s| {
                state
                    .players
                    .game_info
                    .get(s)
                    .map(|gi| (gi.name.clone(), *s))
            })
            .collect();

        Handled::<OM>::future(async move {
            broadcast_event(cloned_co, player_names_to_ids).await;
            None
        })
    }
}
