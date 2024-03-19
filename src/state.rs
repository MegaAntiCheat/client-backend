use crate::{console::ConsoleOutput, player::Players, server::Server, settings::Settings};

#[allow(clippy::module_name_repetitions)]
pub struct MACState {
    pub server: Server,
    pub settings: Settings,
    pub players: Players,
}

impl MACState {
    pub fn handle_console_output(&mut self, output: ConsoleOutput) {
        use ConsoleOutput::{Chat, Hostname, Kill, Map, PlayerCount, ServerIP, Status, G15, DemoStop};
        match output {
            Status(inner) => self.players.handle_status_line(inner),
            G15(inner) => self.players.handle_g15(inner),
            DemoStop(_) => {

            }
            Chat(_) | Kill(_) | Hostname(_) | ServerIP(_) | Map(_) | PlayerCount(_) => {
                self.server.handle_console_output(output);
            }
        }
    }
}
