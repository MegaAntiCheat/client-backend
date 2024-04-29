use std::collections::HashMap;

use event_loop::{try_get, Handled, HandlerStruct, Is};
use steamid_ng::SteamID;
use super::console::ConsoleOutput;
use crate::state::MACState;
use super::web::broadcast_event;

// Messages *********************

// pub struct NewConsoleMessage(pub Option<String>);

// Handlers *********************

#[allow(clippy::module_name_repetitions)]
pub struct SseBroadcastMessages;
impl<IM, OM> HandlerStruct<MACState, IM, OM> for SseBroadcastMessages
where
    IM: Is<ConsoleOutput>
{
    fn handle_message(
        &mut self,
        state: &MACState,
        message: &IM,
    ) -> Option<event_loop::Handled<OM>> {
        let console_out: &ConsoleOutput = try_get(message)?;
        let mut cloned_co = console_out.clone();

        let mut player_names_to_ids: HashMap<String, SteamID> = HashMap::new();
        for (player_id, info) in &state.players.steam_info {
            player_names_to_ids.insert(info.account_name.clone(), player_id.clone());
        }
        
        return Handled::<OM>::future(async move {
            broadcast_event(&mut cloned_co, player_names_to_ids).await;
            None
        });
    }
}
