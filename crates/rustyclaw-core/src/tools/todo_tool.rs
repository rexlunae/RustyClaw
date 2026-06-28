//! Todo planning tool: in-session checklist for multi-step task planning.
//!
//! Gives the agent a lightweight todo/checklist that persists within the
//! session. Distinct from `task_*` tools which manage OS processes.

use serde_json::{Value, json};
use std::path::Path;
use std::sync::Mutex;
use tracing::{debug, instrument};

use super::ToolParam;

// ── In-memory session state ─────────────────────────────────────────────────

/// Status of a todo item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Done,
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Done => write!(f, "done"),
        }
    }
}

impl TodoStatus {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "done" => Some(Self::Done),
            _ => None,
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            Self::Pending => "○",
            Self::InProgress => "◉",
            Self::Done => "✓",
        }
    }
}

/// A single todo item.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TodoItem {
    pub id: u32,
    pub content: String,
    pub status: TodoStatus,
}

/// In-memory todo list (session-scoped).
static TODO_LIST: Mutex<Option<Vec<TodoItem>>> = Mutex::new(None);
static NEXT_ID: Mutex<u32> = Mutex::new(1);

fn with_list<F, R>(f: F) -> R
where
    F: FnOnce(&mut Vec<TodoItem>) -> R,
{
    let mut guard = TODO_LIST.lock().unwrap();
    let list = guard.get_or_insert_with(Vec::new);
    f(list)
}

fn next_id() -> u32 {
    let mut id = NEXT_ID.lock().unwrap();
    let current = *id;
    *id += 1;
    current
}

// ── Tool executor ───────────────────────────────────────────────────────────

/// Execute the `todo` tool.
#[instrument(skip(args, _workspace_dir), fields(action))]
pub fn exec_todo(args: &Value, _workspace_dir: &Path) -> Result<String, String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required parameter: action".to_string())?;

    tracing::Span::current().record("action", action);
    debug!("Executing todo tool");

    match action {
        "add" => {
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: content")?;

            let status_str = args
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending");

            let status = TodoStatus::from_str(status_str).ok_or_else(|| {
                format!(
                    "Invalid status: '{}'. Use: pending, in_progress, done",
                    status_str
                )
            })?;

            let id = next_id();
            let item = TodoItem {
                id,
                content: content.to_string(),
                status,
            };

            with_list(|list| list.push(item.clone()));

            debug!(id, content, "Added todo item");
            Ok(json!({
                "action": "add",
                "item": {
                    "id": id,
                    "content": content,
                    "status": status.to_string(),
                }
            })
            .to_string())
        }

        "update_status" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .ok_or("Missing required parameter: id")?;

            let status_str = args
                .get("status")
                .and_then(|v| v.as_str())
                .ok_or("Missing required parameter: status")?;

            let status = TodoStatus::from_str(status_str).ok_or_else(|| {
                format!(
                    "Invalid status: '{}'. Use: pending, in_progress, done",
                    status_str
                )
            })?;

            let updated = with_list(|list| {
                if let Some(item) = list.iter_mut().find(|i| i.id == id) {
                    item.status = status;
                    Some(item.clone())
                } else {
                    None
                }
            });

            match updated {
                Some(item) => {
                    debug!(id, status = %status, "Updated todo status");
                    Ok(json!({
                        "action": "update_status",
                        "item": {
                            "id": item.id,
                            "content": item.content,
                            "status": item.status.to_string(),
                        }
                    })
                    .to_string())
                }
                None => Err(format!("Todo item not found: {}", id)),
            }
        }

        "remove" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .ok_or("Missing required parameter: id")?;

            let removed = with_list(|list| {
                let pos = list.iter().position(|i| i.id == id);
                pos.map(|p| list.remove(p))
            });

            match removed {
                Some(item) => {
                    debug!(id, "Removed todo item");
                    Ok(json!({
                        "action": "remove",
                        "removed": {
                            "id": item.id,
                            "content": item.content,
                        }
                    })
                    .to_string())
                }
                None => Err(format!("Todo item not found: {}", id)),
            }
        }

        "list" => {
            let items = with_list(|list| list.clone());

            if items.is_empty() {
                return Ok("No todo items. Use action='add' to create one.".to_string());
            }

            let mut output = String::from("Todo list:\n\n");
            for item in &items {
                output.push_str(&format!(
                    "{} [{}] #{} — {}\n",
                    item.status.symbol(),
                    item.status,
                    item.id,
                    item.content
                ));
            }

            let pending = items
                .iter()
                .filter(|i| i.status == TodoStatus::Pending)
                .count();
            let in_progress = items
                .iter()
                .filter(|i| i.status == TodoStatus::InProgress)
                .count();
            let done = items
                .iter()
                .filter(|i| i.status == TodoStatus::Done)
                .count();
            output.push_str(&format!(
                "\nSummary: {} pending, {} in progress, {} done ({} total)",
                pending,
                in_progress,
                done,
                items.len()
            ));

            Ok(output)
        }

        "clear" => {
            let count = with_list(|list| {
                let count = list.len();
                list.clear();
                count
            });

            debug!(count, "Cleared todo list");
            Ok(json!({
                "action": "clear",
                "removed_count": count,
            })
            .to_string())
        }

        _ => Err(format!(
            "Unknown action: '{}'. Valid actions: add, update_status, remove, list, clear",
            action
        )),
    }
}

// ── Parameter definitions ───────────────────────────────────────────────────

pub fn todo_params() -> Vec<ToolParam> {
    vec![
        ToolParam {
            name: "action".into(),
            description:
                "Action to perform: 'add' (create item), 'update_status' (change status), \
                          'remove' (delete item), 'list' (show all), 'clear' (remove all)."
                    .into(),
            param_type: "string".into(),
            required: true,
        },
        ToolParam {
            name: "content".into(),
            description: "Text content of the todo item (required for 'add').".into(),
            param_type: "string".into(),
            required: false,
        },
        ToolParam {
            name: "id".into(),
            description: "Item ID (required for 'update_status' and 'remove').".into(),
            param_type: "integer".into(),
            required: false,
        },
        ToolParam {
            name: "status".into(),
            description: "Status value: 'pending', 'in_progress', or 'done'. \
                          Used with 'add' (default: pending) and 'update_status' (required)."
                .into(),
            param_type: "string".into(),
            required: false,
        },
    ]
}

// ── Public access for UI rendering ──────────────────────────────────────────

/// Get a snapshot of the current todo list for UI rendering.
pub fn get_todo_snapshot() -> Vec<TodoItem> {
    with_list(|list| list.clone())
}
