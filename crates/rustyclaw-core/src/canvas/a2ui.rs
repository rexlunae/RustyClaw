//! A2UI (Agent-to-UI) protocol types.
//!
//! A2UI is a declarative UI protocol that allows agents to push
//! UI updates to a canvas without writing HTML/JS directly.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A2UI message types (v0.8 compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum A2UIMessage {
    /// Start rendering a surface
    BeginRendering { surface_id: String, root: String },

    /// Update surface components
    SurfaceUpdate {
        surface_id: String,
        components: Vec<A2UIComponentDef>,
    },

    /// Update data model
    DataModelUpdate {
        surface_id: String,
        data: serde_json::Value,
    },

    /// Delete a surface
    DeleteSurface { surface_id: String },
}

/// A2UI component definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2UIComponentDef {
    /// Component ID
    pub id: String,

    /// Component type and properties
    pub component: A2UIComponent,
}

/// A2UI component types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum A2UIComponent {
    /// Container column
    Column {
        children: A2UIChildren,
        #[serde(default)]
        spacing: Option<String>,
    },

    /// Container row
    Row {
        children: A2UIChildren,
        #[serde(default)]
        spacing: Option<String>,
    },

    /// Text element
    Text {
        text: A2UITextValue,
        #[serde(default)]
        usage_hint: Option<String>,
    },

    /// Button element
    Button {
        label: A2UITextValue,
        #[serde(default)]
        action: Option<String>,
    },

    /// Input field
    Input {
        #[serde(default)]
        placeholder: Option<String>,
        #[serde(default)]
        value: Option<A2UITextValue>,
        #[serde(default)]
        on_change: Option<String>,
    },

    /// Image element
    Image {
        src: String,
        #[serde(default)]
        alt: Option<String>,
    },

    /// Markdown content
    Markdown { content: A2UITextValue },

    /// Code block
    Code {
        content: A2UITextValue,
        #[serde(default)]
        language: Option<String>,
    },

    /// Progress indicator
    Progress {
        value: f64,
        #[serde(default)]
        max: Option<f64>,
    },

    /// Spacer
    Spacer {
        #[serde(default)]
        size: Option<String>,
    },

    /// Divider
    Divider,
}

/// A2UI children specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum A2UIChildren {
    /// Explicit list of child IDs
    ExplicitList(Vec<String>),

    /// Dynamic children from data binding
    DataBinding { source: String, template: String },
}

/// A2UI text value (literal or data binding).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum A2UITextValue {
    /// Literal string
    LiteralString(String),

    /// Data binding expression
    DataBinding(String),
}

impl A2UITextValue {
    /// Create a literal text value
    pub fn literal(s: impl Into<String>) -> Self {
        Self::LiteralString(s.into())
    }

    /// Create a data binding
    pub fn binding(expr: impl Into<String>) -> Self {
        Self::DataBinding(expr.into())
    }
}

/// A rendered A2UI surface.
#[derive(Debug, Clone, Default)]
pub struct A2UISurface {
    /// Surface ID
    pub id: String,

    /// Root component ID
    pub root: Option<String>,

    /// Components by ID
    pub components: HashMap<String, A2UIComponent>,

    /// Data model
    pub data: serde_json::Value,
}

impl A2UISurface {
    /// Create a new surface
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            root: None,
            components: HashMap::new(),
            data: serde_json::Value::Null,
        }
    }

    /// Apply an A2UI message to update this surface
    pub fn apply(&mut self, msg: &A2UIMessage) {
        match msg {
            A2UIMessage::BeginRendering { surface_id, root } => {
                if surface_id == &self.id {
                    self.root = Some(root.clone());
                }
            }
            A2UIMessage::SurfaceUpdate {
                surface_id,
                components,
            } => {
                if surface_id == &self.id {
                    for comp_def in components {
                        self.components
                            .insert(comp_def.id.clone(), comp_def.component.clone());
                    }
                }
            }
            A2UIMessage::DataModelUpdate { surface_id, data } => {
                if surface_id == &self.id {
                    self.data = data.clone();
                }
            }
            A2UIMessage::DeleteSurface { .. } => {
                // Handled at manager level
            }
        }
    }
}
