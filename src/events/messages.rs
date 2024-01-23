use crate::state::MACState;
use event_loop::StateUpdater;

pub struct Refresh;
impl StateUpdater<MACState> for Refresh {
    fn update_state(self, state: &mut MACState) {
        state.server.players_mut().refresh();
    }
}
