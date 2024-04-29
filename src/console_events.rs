use super::console::ConsoleOutput;
use super::web::broadcast_event;
use crate::state::MACState;
use event_loop::{try_get, Handled, HandlerStruct, Is};

// Messages *********************

// pub struct NewConsoleMessage(pub Option<String>);

// Handlers *********************


#[allow(clippy::module_name_repetitions)]
pub struct SseBroadcastMessages;

/// Handler that parses through all ConsoleOutput events, taking the relevant messages and broadcasting them
/// via the SSE (Server Sent Events) channels that clients have subscribed to.
impl<IM, OM> HandlerStruct<MACState, IM, OM> for SseBroadcastMessages
where
    IM: Is<ConsoleOutput>,
{
    /// Takes a clone of the message and throws it off to the broadcast_event method in web.rs.
    /// Returns a Handled future for the event loop infrastructure to await. Makes no modifications to state,
    /// does not add any new messages to the queue.
    /// 
    /// The current state is used to construct a map of player names to SteamID64s, useful for injecting
    /// SteamID64 values into ConsoleOutput messaages that only have a name. 
    fn handle_message(
        &mut self,
        state: &MACState,
        message: &IM,
    ) -> Option<event_loop::Handled<OM>> {
        let console_out: &ConsoleOutput = try_get(message)?;
        let cloned_co = console_out.clone();

        let name_map = state.players.get_name_to_steam_ids_map();
        Handled::<OM>::future(async move {
            broadcast_event(cloned_co, name_map).await;
            None
        })
    }
}
