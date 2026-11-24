//! Settings: the OpenAI-compatible endpoints (add, edit, enable, offer next to the pool or keep
//! as a fallback, test, reorder, delete) and the Weirdnet node addresses this device dials.
//! Everything is `Core`'s endpoints API; keys are typed masked and never shown or printed.

use bragi_core::{EndpointInput, EndpointMode, EndpointView, PoolView};

use crate::editor::Editor;

pub const FIELDS: [&str; 4] = ["Name", "Base URL", "Model (optional)", "API key (optional)"];
pub const KEY_FIELD: usize = 3;

/// The add or edit form.
#[derive(Default)]
pub struct Form {
    /// The endpoint being edited; `None` adds one.
    pub editing: Option<String>,
    pub fields: [Editor; 4],
    pub field: usize,
    /// Offered next to the pool (a chat can be sent to it) rather than only a fallback.
    pub alongside: bool,
    /// Editing: the key field was typed in (empty then clears the key).
    pub key_touched: bool,
}

impl Form {
    pub fn add() -> Self {
        Self { alongside: true, ..Self::default() }
    }

    pub fn edit(e: &EndpointView) -> Self {
        let mut f = Self { editing: Some(e.id.clone()), alongside: e.mode == EndpointMode::Alongside, ..Self::default() };
        f.fields[0].set(&e.name);
        f.fields[1].set(&e.base_url);
        f.fields[2].set(e.model.as_deref().unwrap_or(""));
        f
    }

    pub fn input(&self, enabled: bool) -> EndpointInput {
        let model = self.fields[2].text().trim();
        EndpointInput {
            name: self.fields[0].text().trim().to_string(),
            base_url: self.fields[1].text().trim().to_string(),
            model: (!model.is_empty()).then(|| model.to_string()),
            mode: if self.alongside { EndpointMode::Alongside } else { EndpointMode::Fallback },
            enabled,
        }
    }

    pub fn key(&self) -> Option<String> {
        Some(self.fields[KEY_FIELD].text().trim().to_string()).filter(|k| !k.is_empty())
    }
}

/// An endpoint as an input with one thing changed (the list's toggles).
pub fn input_of(e: &EndpointView) -> EndpointInput {
    EndpointInput { name: e.name.clone(), base_url: e.base_url.clone(), model: e.model.clone(), mode: e.mode, enabled: e.enabled }
}

#[derive(Default)]
pub struct Settings {
    pub endpoints: Vec<EndpointView>,
    /// Node addresses (`Core::links`), shown under the endpoints.
    pub links: Vec<bragi_core::LinkView>,
    /// Saved Ymir pools, shown above the endpoints.
    pub pools: Vec<PoolView>,
    pub sel: usize,
    pub form: Option<Form>,
    /// Endpoint id waiting for y to delete.
    pub confirm_delete: Option<String>,
    /// Pool id waiting for y to forget.
    pub confirm_delete_pool: Option<String>,
    /// The last test result per endpoint id.
    pub tests: std::collections::HashMap<String, String>,
}

impl Settings {
    pub fn reload(&mut self, core: &bragi_core::Core) {
        self.endpoints = core.endpoints();
        self.links = core.links();
        self.pools = core.pools();
        let n = self.pools.len() + self.endpoints.len();
        self.sel = self.sel.min(n.saturating_sub(1));
    }

    pub fn row_count(&self) -> usize {
        self.pools.len() + self.endpoints.len()
    }

    pub fn selected_pool(&self) -> Option<&PoolView> {
        self.pools.get(self.sel)
    }

    pub fn selected(&self) -> Option<&EndpointView> {
        self.sel.checked_sub(self.pools.len()).and_then(|i| self.endpoints.get(i))
    }
}

/// The masked form of a secret as typed: one dot per character.
pub fn mask(s: &str) -> String {
    "•".repeat(s.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_builds_inputs_and_masks_keys() {
        let mut f = Form::add();
        f.fields[0].set(" Local ");
        f.fields[1].set("http://127.0.0.1:8080/v1");
        f.fields[3].set("sk-test");
        let i = f.input(true);
        assert_eq!((i.name.as_str(), i.model.is_none(), i.mode), ("Local", true, EndpointMode::Alongside));
        assert_eq!(f.key().as_deref(), Some("sk-test"));
        assert_eq!(mask("abc"), "•••");
    }
}
