use event_loop::{try_get, Handled, HandlerStruct, Is, StateUpdater};
use steamid_ng::SteamID;

use crate::state::MACState;

use super::console::ConsoleOutput;

// Messages *********************

pub struct NewPlayers(pub Vec<SteamID>);
impl<S> StateUpdater<S> for NewPlayers {}

// Handlers *********************

pub struct ExtractNewPlayers;
impl<IM, OM> HandlerStruct<MACState, IM, OM> for ExtractNewPlayers
where
    IM: Is<ConsoleOutput>,
    OM: Is<NewPlayers>,
{
    fn handle_message(
        &mut self,
        state: &MACState,
        message: &IM,
    ) -> Option<event_loop::Handled<OM>> {
        let console_out: &ConsoleOutput = try_get(message)?;

        let mut steamids = Vec::new();

        match console_out {
            ConsoleOutput::Status(s) => {
                steamids.push(s.steamid);
            }
            ConsoleOutput::Chat(_) => {}
            ConsoleOutput::Kill(_) => {}
            ConsoleOutput::Hostname(_) => {}
            ConsoleOutput::ServerIP(_) => {}
            ConsoleOutput::Map(_) => {}
            ConsoleOutput::PlayerCount(_) => {}
            ConsoleOutput::G15(ps) => {
                steamids.extend(ps.iter().filter_map(|p| p.steamid));
            }
        }

        if steamids.is_empty() {
            return None;
        }

        steamids.retain(|s| !state.server.players().connected.contains(s));
        Handled::single(NewPlayers(steamids))
    }
}
