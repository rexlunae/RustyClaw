//! Runtime tool catalog — the registry the agent's toolset is built from.
//!
//! Phase 0 of the plugin architecture (`docs/PLUGIN_ARCHITECTURE.md` §5).
//! Until now the toolset was a hardcoded list of `&'static ToolDef`s, so
//! nothing could add or remove a tool while the gateway ran. The catalog
//! makes the set dynamic without changing how built-ins are written:
//!
//! - Built-in tools register at first use, grouped into named **in-tree
//!   groups** (the future in-tree plugins), all enabled by default. The
//!   group table ([`builtin_groups`]) is the single source of truth for
//!   which tools exist and which group each belongs to.
//! - Each registration resolves the tool's parameter schema, summary, and
//!   category **once**, so provider schema builders and the tool-config
//!   panel stop keying side tables on tool names — a runtime-registered
//!   tool is a first-class citizen of both.
//! - Plugins (once the dylib loader exists) register through
//!   [`ToolCatalog::register_plugin_tools`] and are torn down with
//!   [`ToolCatalog::unregister_source`]. Names are namespaced by enforced
//!   `<plugin>_` prefix and collide with nothing.
//! - Consumers take a [`ToolSnapshot`] — an immutable, cheaply-cloned view.
//!   The model's tool schema is built from a fresh snapshot every round, so
//!   a load/unload takes effect on the next turn without touching any
//!   session. [`ToolCatalog::generation`] lets callers skip rebuilds when
//!   nothing changed.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock};

use serde_json::Value;

use super::error::ToolResult;
use super::{SyncExecuteFn, ToolDef, ToolParam};

// ── Sources ─────────────────────────────────────────────────────────────────

/// Where a registered tool came from. Enabling/disabling operates on whole
/// sources: an in-tree group or a plugin, never an individual tool (per-tool
/// policy stays in `Config::tool_permissions`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolSource {
    /// A group of built-in tools compiled into this binary — the shape the
    /// future in-tree plugins will keep. Enabled by default.
    Builtin { group: &'static str },
    /// A runtime-registered plugin (Phase 1+). Also enabled by default once
    /// registered; whether to *load* it at all is the plugin manager's call.
    Plugin { name: String },
}

impl ToolSource {
    /// Stable string key, used in config (`disabled_tool_groups`) and the
    /// wire. Builtin groups are bare names ("files"); plugins are prefixed
    /// ("plugin:github-dash") so the two namespaces cannot collide.
    pub fn key(&self) -> String {
        match self {
            Self::Builtin { group } => (*group).to_string(),
            Self::Plugin { name } => format!("plugin:{name}"),
        }
    }
}

/// How a registered tool executes.
#[derive(Clone)]
pub enum ToolExec {
    /// Sync built-in: run on the blocking pool (the classic `ToolDef` path).
    Sync(SyncExecuteFn),
    /// Async-native built-in: dispatched by name in `execute_tool_streaming`'s
    /// async match. The catalog only records that the tool exists and is
    /// enabled; the dispatch table stays where the async fns live.
    AsyncNative,
    /// Runtime-registered executor carrying its own state (plugin tools).
    /// Runs on the blocking pool like `Sync`.
    Dynamic(Arc<dyn Fn(&Value, &Path) -> ToolResult + Send + Sync>),
}

impl std::fmt::Debug for ToolExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sync(_) => f.write_str("Sync"),
            Self::AsyncNative => f.write_str("AsyncNative"),
            Self::Dynamic(_) => f.write_str("Dynamic"),
        }
    }
}

/// A tool as the catalog holds it: everything a consumer needs, resolved at
/// registration time. Owned strings so runtime-registered tools are not
/// second-class.
#[derive(Debug)]
pub struct RegisteredTool {
    pub name: String,
    pub description: String,
    /// Parameter schema, resolved once (built-ins via the params tables).
    pub params: Vec<ToolParam>,
    /// Short user-facing summary for management UIs. Never empty: falls back
    /// to the first line of the description.
    pub summary: String,
    /// Panel grouping. For built-ins this is the group name; plugins get
    /// "plugins" unless they say otherwise.
    pub category: String,
    pub source: ToolSource,
    pub exec: ToolExec,
}

// ── Snapshot ────────────────────────────────────────────────────────────────

/// An immutable view of the enabled toolset at one instant. Cheap to clone
/// (everything is `Arc`), safe to hold across await points; a snapshot never
/// changes under its holder.
#[derive(Debug, Clone)]
pub struct ToolSnapshot {
    generation: u64,
    tools: Vec<Arc<RegisteredTool>>,
    by_name: HashMap<String, usize>,
}

impl ToolSnapshot {
    /// The catalog generation this snapshot was taken at.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn get(&self, name: &str) -> Option<&Arc<RegisteredTool>> {
        self.by_name.get(name).map(|&i| &self.tools[i])
    }

    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<RegisteredTool>> {
        self.tools.iter()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

/// One source as the management surfaces see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    pub key: String,
    pub enabled: bool,
    pub tool_count: usize,
}

// ── Catalog ─────────────────────────────────────────────────────────────────

struct Inner {
    /// All registered tools in registration order, enabled or not.
    tools: Vec<Arc<RegisteredTool>>,
    /// Source keys currently disabled.
    disabled: HashSet<String>,
    /// Cached snapshot of the enabled set; rebuilt on every mutation so
    /// readers never pay for filtering.
    snapshot: Arc<ToolSnapshot>,
}

/// The runtime tool registry. One per process, reached via [`catalog`].
pub struct ToolCatalog {
    inner: RwLock<Inner>,
    generation: AtomicU64,
}

impl ToolCatalog {
    fn new() -> Self {
        Self {
            inner: RwLock::new(Inner {
                tools: Vec::new(),
                disabled: HashSet::new(),
                snapshot: Arc::new(ToolSnapshot {
                    generation: 0,
                    tools: Vec::new(),
                    by_name: HashMap::new(),
                }),
            }),
            generation: AtomicU64::new(0),
        }
    }

    /// Current generation; bumped on every mutation. Consumers that cache a
    /// derived form (provider schemas) can compare before rebuilding.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// The current enabled toolset.
    pub fn snapshot(&self) -> Arc<ToolSnapshot> {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone()
    }

    /// Every source with its enabled state, builtins first in registration
    /// order.
    pub fn sources(&self) -> Vec<SourceInfo> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let mut order: Vec<String> = Vec::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for tool in &inner.tools {
            let key = tool.source.key();
            if !counts.contains_key(&key) {
                order.push(key.clone());
            }
            *counts.entry(key).or_insert(0) += 1;
        }
        order
            .into_iter()
            .map(|key| SourceInfo {
                enabled: !inner.disabled.contains(&key),
                tool_count: counts[&key],
                key,
            })
            .collect()
    }

    /// Register tools for a source. Rejects (whole batch, atomically) any
    /// name that collides with an already-registered tool.
    fn register(
        &self,
        source: ToolSource,
        tools: Vec<Arc<RegisteredTool>>,
    ) -> Result<(), CatalogError> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let mut seen: HashSet<&str> = inner.tools.iter().map(|t| t.name.as_str()).collect();
        for tool in &tools {
            debug_assert_eq!(tool.source, source, "tool registered under wrong source");
            if !seen.insert(tool.name.as_str()) {
                return Err(CatalogError::NameTaken(tool.name.clone()));
            }
        }
        inner.tools.extend(tools);
        self.rebuild(&mut inner);
        Ok(())
    }

    /// Register a plugin's tools. Every name must start with `<plugin>_` —
    /// the namespacing rule that keeps plugin tools out of the built-in
    /// namespace and makes their origin legible in transcripts.
    pub fn register_plugin_tools(
        &self,
        plugin: &str,
        tools: Vec<PluginToolSpec>,
    ) -> Result<(), CatalogError> {
        let prefix = format!("{plugin}_");
        let source = ToolSource::Plugin {
            name: plugin.to_string(),
        };
        let registered = tools
            .into_iter()
            .map(|spec| {
                if !spec.name.starts_with(&prefix) {
                    return Err(CatalogError::BadPrefix {
                        tool: spec.name,
                        expected: prefix.clone(),
                    });
                }
                Ok(Arc::new(RegisteredTool {
                    summary: if spec.summary.is_empty() {
                        first_line(&spec.description)
                    } else {
                        spec.summary
                    },
                    name: spec.name,
                    description: spec.description,
                    params: spec.params,
                    category: "plugins".to_string(),
                    source: source.clone(),
                    exec: ToolExec::Dynamic(spec.exec),
                }))
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.register(source, registered)
    }

    /// Remove every tool a source registered. Idempotent.
    pub fn unregister_source(&self, source: &ToolSource) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let before = inner.tools.len();
        inner.tools.retain(|t| t.source != *source);
        inner.disabled.remove(&source.key());
        if inner.tools.len() != before {
            self.rebuild(&mut inner);
        }
    }

    /// Enable or disable a whole source by key. Disabled tools vanish from
    /// snapshots — not advertised to the model, not executable. Unknown keys
    /// are an error so a typo in config is loud.
    pub fn set_source_enabled(&self, key: &str, enabled: bool) -> Result<(), CatalogError> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if !inner.tools.iter().any(|t| t.source.key() == key) {
            return Err(CatalogError::UnknownSource(key.to_string()));
        }
        let changed = if enabled {
            inner.disabled.remove(key)
        } else {
            inner.disabled.insert(key.to_string())
        };
        if changed {
            self.rebuild(&mut inner);
        }
        Ok(())
    }

    fn rebuild(&self, inner: &mut Inner) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let tools: Vec<Arc<RegisteredTool>> = inner
            .tools
            .iter()
            .filter(|t| !inner.disabled.contains(&t.source.key()))
            .cloned()
            .collect();
        let by_name = tools
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i))
            .collect();
        inner.snapshot = Arc::new(ToolSnapshot {
            generation,
            tools,
            by_name,
        });
    }
}

/// A tool as a plugin hands it to the catalog. (The dylib ABI will produce
/// this shape; until then the SDK tests and the declarative-plugin executor
/// are the only producers.)
pub struct PluginToolSpec {
    pub name: String,
    pub description: String,
    pub params: Vec<ToolParam>,
    /// Optional short summary; defaults to the description's first line.
    pub summary: String,
    pub exec: Arc<dyn Fn(&Value, &Path) -> ToolResult + Send + Sync>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("tool name '{0}' is already registered")]
    NameTaken(String),
    #[error("plugin tool '{tool}' must be prefixed '{expected}'")]
    BadPrefix { tool: String, expected: String },
    #[error("unknown tool source '{0}'")]
    UnknownSource(String),
}

// ── Built-in registration ───────────────────────────────────────────────────

/// The in-tree tool groups: the single source of truth for which built-in
/// tools exist and how they are packaged. Each group is a future in-tree
/// plugin (`docs/PLUGIN_ARCHITECTURE.md` §12, open question 5); group names
/// double as the tool-config panel's categories, so they are user-visible.
///
/// `all_tools()` flattens this table, so a tool absent here does not exist
/// anywhere — a test asserts the flattening stays duplicate-free.
pub(crate) fn builtin_groups() -> Vec<(&'static str, Vec<&'static ToolDef>)> {
    use super::definitions::*;
    vec![
        (
            "files",
            vec![
                &READ_FILE,
                &WRITE_FILE,
                &EDIT_FILE,
                &LIST_DIRECTORY,
                &SEARCH_FILES,
                &FIND_FILES,
                &APPLY_PATCH,
                #[cfg(feature = "office-docs")]
                &DOCUMENT,
            ],
        ),
        ("runtime", vec![&EXECUTE_COMMAND, &PROCESS]),
        (
            "web",
            vec![&WEB_FETCH, &WEB_SEARCH, &HTTP_REQUEST, &WEB_EXTRACT],
        ),
        (
            "memory",
            vec![
                #[cfg(feature = "semantic-memory")]
                &MEMORY_SEARCH,
                &MEMORY_GET,
                &SAVE_MEMORY,
                &SEARCH_HISTORY,
                #[cfg(feature = "semantic-memory")]
                &ADD_MEMORY,
            ],
        ),
        ("scheduling", vec![&CRON]),
        (
            "sessions",
            vec![
                &SESSIONS_LIST,
                &SESSIONS_SPAWN,
                &SESSIONS_KILL,
                &SESSIONS_SEND,
                &SESSIONS_HISTORY,
                &SESSION_STATUS,
                &SESSION_SEARCH,
                &AGENTS_LIST,
                &AGENTS_CREATE,
                &AGENTS_DELETE,
                &SUBAGENT_LIST,
                &SUBAGENT_CREATE,
                &SUBAGENT_DELETE,
                &SUBAGENT_RUN,
                &TRIGGERS_CREATE,
                &TRIGGERS_LIST,
                &TRIGGERS_UPDATE,
                &TRIGGERS_DELETE,
                &TRIGGERS_SET_ENABLED,
            ],
        ),
        (
            "secrets",
            vec![
                &SECRETS_LIST,
                &SECRETS_GET,
                &SECRETS_STORE,
                &SECRETS_SET_POLICY,
                &SECRETS_LINK_TRIGGER,
            ],
        ),
        ("gateway", vec![&GATEWAY]),
        ("messaging", vec![&MESSAGE, &TTS]),
        (
            "media",
            vec![
                &IMAGE,
                #[cfg(feature = "image-gen")]
                &IMAGE_GENERATE,
            ],
        ),
        ("devices", vec![&NODES, &CANVAS]),
        ("browser", vec![&BROWSER]),
        (
            "skills",
            vec![
                &SKILL_LIST,
                &SKILL_SEARCH,
                &SKILL_INSTALL,
                &SKILL_INFO,
                &SKILL_ENABLE,
                &SKILL_LINK_SECRET,
                &SKILL_CREATE,
                &SKILL_CURATOR,
            ],
        ),
        ("mcp", vec![&MCP_LIST, &MCP_CONNECT, &MCP_DISCONNECT]),
        (
            "tasks",
            vec![
                &TASK_LIST,
                &TASK_STATUS,
                &TASK_FOREGROUND,
                &TASK_BACKGROUND,
                &TASK_CANCEL,
                &TASK_PAUSE,
                &TASK_RESUME,
                &TASK_INPUT,
                &TASK_DESCRIBE,
            ],
        ),
        (
            "threads",
            vec![&THREAD_DESCRIBE, &SET_THREAD_CAPTION, &THREADS_LIST],
        ),
        (
            "models",
            vec![
                &MODEL_LIST,
                &MODEL_ENABLE,
                &MODEL_DISABLE,
                &MODEL_SET,
                &MODEL_RECOMMEND,
            ],
        ),
        (
            "system",
            vec![
                &HOST_INFO,
                &LOAD_STATUS,
                &DISK_USAGE,
                &CLASSIFY_FILES,
                &SYSTEM_MONITOR,
                &BATTERY_HEALTH,
                &APP_INDEX,
                &CLOUD_BROWSE,
                &BROWSER_CACHE,
                &SCREENSHOT,
                &CLIPBOARD,
                &AUDIT_SENSITIVE,
                &SECURE_DELETE,
                &SUMMARIZE_FILE,
            ],
        ),
        (
            "services",
            vec![
                &SERVICE_LIST,
                &SERVICE_START,
                &SERVICE_STOP,
                &SERVICE_RESTART,
                &SERVICE_LOGS,
            ],
        ),
        (
            "sysadmin",
            vec![
                &PKG_MANAGE,
                &NET_INFO,
                &NET_SCAN,
                &SERVICE_MANAGE,
                &USER_MANAGE,
                &FIREWALL,
            ],
        ),
        ("engines", vec![&OLLAMA_MANAGE, &EXO_MANAGE, &AGENT_SETUP]),
        ("code", vec![&AST_GREP_MANAGE, &UV_MANAGE, &NPM_MANAGE]),
        ("documents", vec![&PDF, &CHART]),
        (
            "swarm",
            vec![
                &SWARM_CREATE,
                &SWARM_LIST,
                &SWARM_STATUS,
                &SWARM_SEND,
                &SWARM_STOP,
                &SWARM_DELETE,
                &SWARM_TEMPLATES,
            ],
        ),
        ("planning", vec![&TODO]),
        ("freenet", vec![&FREENET, &RIVER, &ATLAS]),
        (
            "plugins",
            vec![
                &PLUGIN_LIST,
                &PLUGIN_STATE_GET,
                &PLUGIN_STATE_SET,
                &PLUGIN_STATE_PATCH,
                &PLUGIN_CREATE,
            ],
        ),
        ("interactive", vec![&ASK_USER, &CLIENT_DOM_QUERY]),
    ]
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or_default().trim().to_string()
}

/// Build the registration record for one built-in.
fn register_builtin(group: &'static str, def: &'static ToolDef) -> Arc<RegisteredTool> {
    let summary = match super::tool_summary(def.name) {
        "Unknown tool" => first_line(def.description),
        s => s.to_string(),
    };
    let exec = if super::ASYNC_NATIVE_TOOLS.contains(&def.name) {
        ToolExec::AsyncNative
    } else {
        ToolExec::Sync(def.execute)
    };
    Arc::new(RegisteredTool {
        name: def.name.to_string(),
        description: def.description.to_string(),
        params: super::schema::resolve_params(def),
        summary,
        category: group.to_string(),
        source: ToolSource::Builtin { group },
        exec,
    })
}

/// A fresh catalog with every built-in group registered. The global goes
/// through this; tests use it directly so mutations (disabling groups,
/// registering fake plugins) never leak into other tests via the global.
fn with_builtins() -> ToolCatalog {
    let catalog = ToolCatalog::new();
    for (group, defs) in builtin_groups() {
        let tools = defs
            .into_iter()
            .map(|def| register_builtin(group, def))
            .collect();
        catalog
            .register(ToolSource::Builtin { group }, tools)
            .expect("builtin group table contains a duplicate tool name");
    }
    catalog
}

static CATALOG: LazyLock<ToolCatalog> = LazyLock::new(with_builtins);

/// The process-wide tool catalog. Built-ins are registered on first access.
pub fn catalog() -> &'static ToolCatalog {
    &CATALOG
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_exec(_args: &Value, _dir: &Path) -> ToolResult {
        Ok("ran".to_string())
    }

    fn plugin_tool(name: &str) -> PluginToolSpec {
        PluginToolSpec {
            name: name.to_string(),
            description: "A test plugin tool.\nSecond line.".to_string(),
            params: vec![],
            summary: String::new(),
            exec: Arc::new(fake_exec),
        }
    }

    #[test]
    fn builtin_groups_are_duplicate_free_and_fully_registered() {
        // `with_builtins` panics on a duplicate; this pins the rest: every
        // group non-empty, every tool present in the snapshot, every
        // registration carrying a category, summary, and resolved params
        // where the params tables define them.
        let catalog = with_builtins();
        let snapshot = catalog.snapshot();
        let mut count = 0;
        for (group, defs) in builtin_groups() {
            assert!(!defs.is_empty(), "group '{group}' is empty");
            for def in defs {
                count += 1;
                let tool = snapshot
                    .get(def.name)
                    .unwrap_or_else(|| panic!("'{}' missing from snapshot", def.name));
                assert_eq!(tool.category, group);
                assert!(!tool.summary.is_empty(), "'{}' has no summary", def.name);
            }
        }
        assert_eq!(snapshot.len(), count);
    }

    #[test]
    fn read_file_registration_resolved_its_params() {
        // The params tables used to be consulted per schema build; now they
        // are read once at registration. If this resolution breaks, every
        // provider schema silently degrades to zero-argument tools.
        let catalog = with_builtins();
        let snapshot = catalog.snapshot();
        let read_file = snapshot.get("read_file").expect("read_file registered");
        assert!(
            read_file.params.iter().any(|p| p.name == "path"),
            "read_file lost its 'path' param at registration"
        );
    }

    #[test]
    fn disabling_a_group_hides_its_tools_and_bumps_generation() {
        let catalog = with_builtins();
        let before = catalog.generation();
        assert!(catalog.snapshot().contains("freenet"));

        catalog.set_source_enabled("freenet", false).unwrap();
        let snapshot = catalog.snapshot();
        assert!(!snapshot.contains("freenet"));
        assert!(!snapshot.contains("river"));
        assert!(!snapshot.contains("atlas"));
        assert!(catalog.generation() > before, "generation must move");
        // Other groups are untouched.
        assert!(snapshot.contains("read_file"));

        catalog.set_source_enabled("freenet", true).unwrap();
        assert!(catalog.snapshot().contains("river"));
    }

    #[test]
    fn disabling_an_unknown_group_is_a_loud_error() {
        let catalog = with_builtins();
        assert_eq!(
            catalog.set_source_enabled("no-such-group", false),
            Err(CatalogError::UnknownSource("no-such-group".to_string()))
        );
    }

    #[test]
    fn plugin_tools_register_under_their_prefix_and_unregister_cleanly() {
        let catalog = with_builtins();
        catalog
            .register_plugin_tools("demo", vec![plugin_tool("demo_hello")])
            .unwrap();

        let snapshot = catalog.snapshot();
        let tool = snapshot.get("demo_hello").expect("plugin tool registered");
        assert_eq!(tool.category, "plugins");
        // Empty summary falls back to the description's first line.
        assert_eq!(tool.summary, "A test plugin tool.");
        assert_eq!(
            tool.source,
            ToolSource::Plugin {
                name: "demo".to_string()
            }
        );

        let source = ToolSource::Plugin {
            name: "demo".to_string(),
        };
        catalog.unregister_source(&source);
        assert!(!catalog.snapshot().contains("demo_hello"));
        // Idempotent, and does not bump generation when nothing changed.
        let generation = catalog.generation();
        catalog.unregister_source(&source);
        assert_eq!(catalog.generation(), generation);
    }

    #[test]
    fn plugin_tool_without_prefix_is_rejected() {
        let catalog = with_builtins();
        let err = catalog
            .register_plugin_tools("demo", vec![plugin_tool("hello")])
            .unwrap_err();
        assert_eq!(
            err,
            CatalogError::BadPrefix {
                tool: "hello".to_string(),
                expected: "demo_".to_string(),
            }
        );
        // The rejection is atomic: nothing from the batch registered.
        assert!(!catalog.snapshot().contains("hello"));
    }

    #[test]
    fn plugin_tool_colliding_with_existing_name_is_rejected() {
        let catalog = with_builtins();
        catalog
            .register_plugin_tools("demo", vec![plugin_tool("demo_hello")])
            .unwrap();
        // A second plugin whose prefix happens to nest ("demo" vs "demo_hello"
        // is impossible, but the same plugin re-registering is the real case).
        let err = catalog
            .register_plugin_tools("demo", vec![plugin_tool("demo_hello")])
            .unwrap_err();
        assert_eq!(err, CatalogError::NameTaken("demo_hello".to_string()));
    }

    #[test]
    fn snapshots_are_immutable_views() {
        let catalog = with_builtins();
        let held = catalog.snapshot();
        catalog.set_source_enabled("freenet", false).unwrap();
        // The held snapshot still sees the old world; a fresh one does not.
        assert!(held.contains("freenet"));
        assert!(!catalog.snapshot().contains("freenet"));
    }

    #[tokio::test]
    async fn dynamic_tool_executes_through_the_global_catalog() {
        // The one test that touches the global: a uniquely-named plugin tool
        // registers, executes through the real dispatch path, and vanishes
        // after unregistration. Unique names keep this invisible to every
        // other test ("at least N tools" assertions are unaffected by one
        // extra).
        let name = "cattest_echo";
        catalog()
            .register_plugin_tools("cattest", vec![plugin_tool(name)])
            .unwrap();

        let result =
            super::super::execute_tool(name, &serde_json::json!({}), Path::new("/tmp")).await;
        assert_eq!(result.unwrap(), "ran");

        catalog().unregister_source(&ToolSource::Plugin {
            name: "cattest".to_string(),
        });
        let result =
            super::super::execute_tool(name, &serde_json::json!({}), Path::new("/tmp")).await;
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Unknown tool"),
            "unregistered tool must be unknown, got: {err}"
        );
    }
}
