use crate::action::Action;
use crate::app::App;
use crate::dialogs;
use crate::panes::DisplayMessage;
use crossterm::event::KeyCode;

impl App {
    pub fn handle_api_key_dialog_key(&mut self, code: KeyCode) -> Action {
        let Some(dialog) = self.api_key_dialog.take() else {
            return Action::Noop;
        };
        let (new_state, action) =
            dialogs::handle_api_key_dialog_key(dialog, code, &mut self.state.messages);
        self.api_key_dialog = new_state;
        action
    }

    pub fn handle_provider_selector_key(&mut self, code: KeyCode) -> Action {
        let Some(sel) = self.provider_selector.take() else {
            return Action::Noop;
        };
        let (new_state, action) =
            dialogs::handle_provider_selector_key(sel, code, &mut self.state.messages);
        self.provider_selector = new_state;
        action
    }

    pub fn handle_model_selector_key(&mut self, code: KeyCode) -> Action {
        let Some(sel) = self.model_selector.take() else {
            return Action::Noop;
        };
        let save_fn = || -> Result<(), String> { Ok(()) };
        let (new_state, action) = dialogs::handle_model_selector_key(
            sel,
            code,
            &mut self.state.config.model,
            &mut self.state.messages,
            save_fn,
        );
        if matches!(action, Action::RestartGateway) {
            if let Err(e) = self.state.config.save(None) {
                if let Some(msg) = self.state.messages.last_mut() {
                    *msg = DisplayMessage::error(format!("Failed to save config: {}", e));
                }
            }
        }
        self.model_selector = new_state;
        action
    }

    pub fn handle_policy_picker_key(&mut self, code: KeyCode) -> Action {
        let Some(picker) = self.policy_picker.take() else {
            return Action::Noop;
        };
        let (new_state, action) =
            dialogs::handle_policy_picker_key_gateway(picker, code, &mut self.state.messages);
        self.policy_picker = new_state;
        action
    }

    pub fn handle_secret_viewer_key(&mut self, code: KeyCode) -> Action {
        let Some(viewer) = self.secret_viewer.take() else {
            return Action::Noop;
        };
        let (new_state, action) = dialogs::handle_secret_viewer_key(viewer, code);
        self.secret_viewer = new_state;
        action
    }

    pub fn handle_credential_dialog_key(&mut self, code: KeyCode) -> Action {
        let Some(dlg) = self.credential_dialog.take() else {
            return Action::Noop;
        };
        let (new_dlg, new_picker, action) =
            dialogs::handle_credential_dialog_key_gateway(dlg, code, &mut self.state.messages);
        self.credential_dialog = new_dlg;
        if new_picker.is_some() {
            self.policy_picker = new_picker;
        }
        action
    }

    pub fn handle_totp_dialog_key(&mut self, code: KeyCode) -> Action {
        let Some(dlg) = self.totp_dialog.take() else {
            return Action::Noop;
        };
        let (new_state, action) =
            dialogs::handle_totp_dialog_key_gateway(dlg, code, &mut self.state.messages);
        self.totp_dialog = new_state;
        action
    }

    pub fn handle_auth_prompt_key(&mut self, code: KeyCode) -> Action {
        let Some(prompt) = self.auth_prompt.take() else {
            return Action::Noop;
        };
        let (new_state, action) = dialogs::handle_auth_prompt_key(
            prompt,
            code,
            &mut self.state.messages,
            &mut self.state.gateway_status,
        );
        self.auth_prompt = new_state;
        action
    }

    pub fn handle_vault_unlock_prompt_key(&mut self, code: KeyCode) -> Action {
        let Some(prompt) = self.vault_unlock_prompt.take() else {
            return Action::Noop;
        };
        let (new_state, action) =
            dialogs::handle_vault_unlock_prompt_key(prompt, code, &mut self.state.messages);
        self.vault_unlock_prompt = new_state;
        action
    }
}
