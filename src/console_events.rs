use super::console::{ConsoleOutput, SerializableEvent};
use super::web::broadcast_event;
use crate::state::MACState;
use event_loop::{try_get, Handled, HandlerStruct, Is};

// Messages *********************

// Handlers *********************

#[allow(clippy::module_name_repetitions)]
pub struct SseBroadcastMessages;

/// Handler that parses through all `ConsoleOutput` events, taking the relevant messages and broadcasting them
/// via the SSE (Server Sent Events) channels that clients have subscribed to.
impl<IM, OM> HandlerStruct<MACState, IM, OM> for SseBroadcastMessages
where
    IM: Is<ConsoleOutput>,
{
    /// Given a `ConsoleOutput` 'message', and a set of players in the current context, broadcast the `ConsoleOutput` event to
    /// all subscribers (i.e. everyone thats grabbed an SSE stream channel). This also prunes closed channels from the subscribers
    /// list.
    ///
    /// This function is responsible for inserting the steam id fields into various `ConsoleOutput` events that don't have it populated
    /// due to them being constructed in a stateless manner.
    ///
    /// Note: this function is 'fire and forget'. It does not check that the messages were succesfully sent or recieved on any of
    ///       the channels, nor does it check anything about the channels beyond whether or not the sending side is currently open.
    ///
    /// # Panics
    /// May panic if the types wrapped by the `ConsoleOuput` type fail to serialise. These types are required to derive Serialise and
    /// Deserialise.
    fn handle_message(
        &mut self,
        state: &MACState,
        message: &IM,
    ) -> Option<event_loop::Handled<OM>> {
        let console_out: &ConsoleOutput = try_get(message)?;
        let cloned_co = console_out.clone();

        // We also set the steam_id fields in the events here before we serialise
        if let Some(event_json) = match cloned_co {
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
