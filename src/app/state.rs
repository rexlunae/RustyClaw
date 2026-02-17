use crate::config::Config;
use crate::gateway::ChatMessage;
use crate::panes::{DisplayMessage, GatewayStatus, InputMode, PaneState};
use crate::secrets::SecretsManager;
use crate::skills::SkillManager;
use crate::soul::SoulManager;

pub struct SharedState {
    pub config: Config,
    pub messages: Vec<DisplayMessage>,
    pub conversation_history: Vec<ChatMessage>,
    pub input_mode: InputMode,
    pub secrets_manager: SecretsManager,
    pub skill_manager: SkillManager,
    pub soul_manager: SoulManager,
    pub gateway_status: GatewayStatus,
    pub loading_line: Option<String>,
    pub streaming_started: Option<std::time::Instant>,
}

impl SharedState {
    pub fn pane_state(&mut self) -> PaneState<'_> {
        PaneState {
            config: &self.config,
            secrets_manager: &mut self.secrets_manager,
            skill_manager: &mut self.skill_manager,
            soul_manager: &self.soul_manager,
            messages: &mut self.messages,
            input_mode: self.input_mode,
            gateway_status: self.gateway_status,
            loading_line: self.loading_line.clone(),
            streaming_started: self.streaming_started,
        }
    }
}
