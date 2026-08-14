//! Runtime registry of messenger kinds — which backends this gateway can
//! connect, and how to build each one.
//!
//! The static [`setup::KINDS`] table stays what it always was: the complete
//! schema vocabulary, listing every kind this codebase has ever been taught
//! so a client can explain "this build cannot do Matrix" instead of silently
//! omitting it. What used to be *implicit* next to it — a hardcoded factory
//! `match` in the gateway — is now this registry: the set of kinds that are
//! actually constructible in this process, each carrying its schema and its
//! factory. In-tree kinds register on first access (the compiled-feature
//! subset of `KINDS`); plugins register theirs at load time through
//! [`MessengerRegistry::register_plugin_kind`] and are torn down with
//! [`MessengerRegistry::unregister_source`], mirroring the tool catalog's
//! plugin surface (`docs/PLUGIN_ARCHITECTURE.md`).

use std::borrow::Cow;
use std::sync::{Arc, LazyLock, RwLock};

use super::setup::{self, FieldSpec, KindSpec};
use super::{Messenger, factory};
use crate::config::MessengerConfig;

/// Builds an uninitialized messenger from its account config. Construction is
/// synchronous; the caller owns the async `initialize()` — a factory that
/// needs IO to *construct* is doing initialization in the wrong place.
pub type MessengerFactory =
    Arc<dyn Fn(&MessengerConfig) -> anyhow::Result<Box<dyn Messenger>> + Send + Sync>;

/// Where a registered kind came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KindSource {
    /// Compiled into this binary.
    Builtin,
    /// Registered by the named plugin.
    Plugin(String),
}

struct RegisteredKind {
    spec: KindSpec,
    source: KindSource,
    factory: MessengerFactory,
}

struct Inner {
    kinds: Vec<RegisteredKind>,
    /// Bumped on every registration change, so consumers holding derived
    /// state (a kinds list sent to a client, say) can cheaply notice staleness.
    generation: u64,
}

/// The process-wide messenger-kind registry. Obtain it via
/// [`messenger_registry`].
pub struct MessengerRegistry {
    inner: RwLock<Inner>,
}

impl MessengerRegistry {
    fn new_with_builtins() -> Self {
        let mut kinds = Vec::new();
        // Every KINDS entry whose cargo feature is compiled in is
        // constructible, and they all construct through the one in-tree
        // decision tree. Feature-gated entries simply never register in a
        // build without the feature — "registered" and "available" are the
        // same thing here, where the old code kept a separate cfg! filter.
        for spec in setup::KINDS {
            if !builtin_feature_compiled(spec.feature.as_deref()) {
                continue;
            }
            kinds.push(RegisteredKind {
                spec: spec.clone(),
                source: KindSource::Builtin,
                factory: Arc::new(factory::construct),
            });
        }
        Self {
            inner: RwLock::new(Inner {
                kinds,
                generation: 0,
            }),
        }
    }

    /// The registered kinds' schemas, in registration order (builtins first).
    pub fn kinds(&self) -> Vec<KindSpec> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.kinds.iter().map(|k| k.spec.clone()).collect()
    }

    /// The registered kind ids — what `available_kinds` means now.
    pub fn kind_ids(&self) -> Vec<String> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.kinds.iter().map(|k| k.spec.id.to_string()).collect()
    }

    /// One registered kind's schema.
    pub fn spec(&self, id: &str) -> Option<KindSpec> {
        let id = canonical_id(id);
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner
            .kinds
            .iter()
            .find(|k| k.spec.id == id)
            .map(|k| k.spec.clone())
    }

    /// One field of one registered kind — the registry-aware version of
    /// [`setup::field_spec`], which only knows the static table. Validation
    /// paths use this so a plugin kind's fields validate like a builtin's.
    pub fn field(&self, kind: &str, field: &str) -> Option<FieldSpec> {
        self.spec(kind)
            .and_then(|s| s.fields.iter().find(|f| f.name == field).cloned())
    }

    /// Construct (but do not initialize) a messenger for this account.
    ///
    /// Unknown kinds fail with the most useful sentence available: a kind in
    /// the static table but not registered names the missing cargo feature,
    /// the one retired kind names its replacement, and everything else is
    /// plainly unknown.
    pub fn create(&self, config: &MessengerConfig) -> anyhow::Result<Box<dyn Messenger>> {
        let id = canonical_id(&config.messenger_type);
        let factory = {
            let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
            inner
                .kinds
                .iter()
                .find(|k| k.spec.id == id)
                .map(|k| Arc::clone(&k.factory))
        };
        // The factory runs outside the lock: plugin factories are arbitrary
        // code, and holding a registry lock across them invites deadlock.
        match factory {
            Some(f) => f(config),
            None => Err(unknown_kind_error(&config.messenger_type)),
        }
    }

    /// Register a plugin-provided messenger kind.
    ///
    /// Collisions are rejected against both registered kinds *and* the whole
    /// static table: a plugin may not shadow an in-tree kind even in a build
    /// where that kind's feature is off — the id would silently change
    /// meaning the moment the feature is compiled back in.
    pub fn register_plugin_kind(
        &self,
        plugin_name: &str,
        spec: KindSpec,
        factory: MessengerFactory,
    ) -> Result<(), String> {
        if spec.id.trim().is_empty() {
            return Err("Messenger kind id must not be empty".to_string());
        }
        if setup::KINDS.iter().any(|k| k.id == spec.id) {
            return Err(format!(
                "Messenger kind '{}' is an in-tree kind and cannot be replaced by plugin '{}'",
                spec.id, plugin_name
            ));
        }
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = inner.kinds.iter().find(|k| k.spec.id == spec.id) {
            return Err(match &existing.source {
                KindSource::Builtin => format!(
                    "Messenger kind '{}' is built in and cannot be replaced by plugin '{}'",
                    spec.id, plugin_name
                ),
                KindSource::Plugin(other) => format!(
                    "Messenger kind '{}' is already registered by plugin '{}'",
                    spec.id, other
                ),
            });
        }
        inner.kinds.push(RegisteredKind {
            spec,
            source: KindSource::Plugin(plugin_name.to_string()),
            factory,
        });
        inner.generation += 1;
        Ok(())
    }

    /// Remove every kind the named plugin registered. Returns how many.
    pub fn unregister_source(&self, plugin_name: &str) -> usize {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let before = inner.kinds.len();
        inner
            .kinds
            .retain(|k| k.source != KindSource::Plugin(plugin_name.to_string()));
        let removed = before - inner.kinds.len();
        if removed > 0 {
            inner.generation += 1;
        }
        removed
    }

    /// Current registration generation; changes whenever the kind set does.
    pub fn generation(&self) -> u64 {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .generation
    }
}

/// `signal-cli` was the id's earlier spelling; both name the same backend.
fn canonical_id(id: &str) -> Cow<'_, str> {
    if id == "signal-cli" {
        Cow::Borrowed("signal")
    } else {
        Cow::Borrowed(id)
    }
}

fn unknown_kind_error(requested: &str) -> anyhow::Error {
    if requested == "matrix-cli" {
        return anyhow::anyhow!(
            "matrix-cli messenger type is deprecated. Use 'matrix' type instead."
        );
    }
    if let Some(spec) = setup::kind_spec(&canonical_id(requested)) {
        if let Some(feature) = spec.feature.as_deref() {
            return anyhow::anyhow!(
                "{} messenger not compiled in. Rebuild with --features {}",
                spec.label,
                feature
            );
        }
    }
    anyhow::anyhow!("Unknown messenger type: {}", requested)
}

/// Whether a `KindSpec::feature` gate is satisfied by this build.
fn builtin_feature_compiled(feature: Option<&str>) -> bool {
    match feature {
        None => true,
        Some("matrix") => cfg!(feature = "matrix"),
        Some("whatsapp") => cfg!(feature = "whatsapp"),
        Some("signal-cli") => cfg!(feature = "signal-cli"),
        // An unrecognised feature name is a schema entry this build was never
        // taught about; call it unavailable rather than promising it works.
        Some(_) => false,
    }
}

static REGISTRY: LazyLock<MessengerRegistry> = LazyLock::new(MessengerRegistry::new_with_builtins);

/// The process-wide messenger-kind registry, in-tree kinds pre-registered.
pub fn messenger_registry() -> &'static MessengerRegistry {
    &REGISTRY
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messengers::ConsoleMessenger;

    fn plugin_spec(id: &str) -> KindSpec {
        KindSpec {
            id: Cow::Owned(id.to_string()),
            label: Cow::Owned(format!("{id} (plugin)")),
            icon: Cow::Borrowed("🔌"),
            summary: Cow::Owned(format!("Test kind {id}")),
            feature: None,
            fields: Cow::Owned(vec![FieldSpec {
                name: Cow::Borrowed("token"),
                label: Cow::Borrowed("Token"),
                kind: setup::FieldKind::Secret,
                requirement: setup::Requirement::Required,
                help: Cow::Borrowed("A test token"),
            }]),
        }
    }

    /// `Result<Box<dyn Messenger>, _>` has no `Debug`, so `unwrap_err` can't
    /// be used; take the error by hand.
    fn create_err(reg: &MessengerRegistry, config: &MessengerConfig) -> String {
        match reg.create(config) {
            Ok(_) => panic!("expected {} to fail", config.messenger_type),
            Err(e) => e.to_string(),
        }
    }

    fn console_factory() -> MessengerFactory {
        Arc::new(|config: &MessengerConfig| {
            Ok(Box::new(ConsoleMessenger::new(config.name.clone())) as Box<dyn Messenger>)
        })
    }

    #[test]
    fn builtins_cover_every_compiled_kind() {
        let ids = messenger_registry().kind_ids();
        for spec in setup::KINDS {
            let compiled = builtin_feature_compiled(spec.feature.as_deref());
            assert_eq!(
                ids.iter().any(|id| *id == spec.id),
                compiled,
                "kind {} registered={} but compiled={}",
                spec.id,
                !compiled,
                compiled
            );
        }
        // And nothing beyond the table pre-registers itself.
        assert!(ids.len() <= setup::KINDS.len());
    }

    #[test]
    fn create_dispatches_console_builtin() {
        let config = MessengerConfig {
            messenger_type: "console".into(),
            name: "test-console".into(),
            ..Default::default()
        };
        let messenger = messenger_registry()
            .create(&config)
            .unwrap_or_else(|e| panic!("console builds: {e}"));
        assert_eq!(messenger.name(), "test-console");
    }

    #[test]
    fn unknown_kind_errors_name_the_problem() {
        let reg = MessengerRegistry::new_with_builtins();
        let mut config = MessengerConfig {
            messenger_type: "nonesuch".into(),
            name: "x".into(),
            ..Default::default()
        };
        let err = create_err(&reg, &config);
        assert!(err.contains("Unknown messenger type"), "{err}");

        config.messenger_type = "matrix-cli".into();
        let err = create_err(&reg, &config);
        assert!(err.contains("deprecated"), "{err}");

        #[cfg(not(feature = "matrix"))]
        {
            config.messenger_type = "matrix".into();
            let err = create_err(&reg, &config);
            assert!(err.contains("--features matrix"), "{err}");
        }
    }

    #[test]
    fn plugin_kind_registers_creates_and_unregisters() {
        let reg = MessengerRegistry::new_with_builtins();
        let gen_before = reg.generation();
        reg.register_plugin_kind("acme", plugin_spec("acme_chat"), console_factory())
            .expect("registers");
        assert!(reg.generation() > gen_before);
        assert!(reg.kind_ids().iter().any(|id| id == "acme_chat"));
        assert!(reg.field("acme_chat", "token").is_some());

        let config = MessengerConfig {
            messenger_type: "acme_chat".into(),
            name: "acme-1".into(),
            ..Default::default()
        };
        let messenger = reg
            .create(&config)
            .unwrap_or_else(|e| panic!("plugin kind builds: {e}"));
        assert_eq!(messenger.name(), "acme-1");

        assert_eq!(reg.unregister_source("acme"), 1);
        assert!(!reg.kind_ids().iter().any(|id| id == "acme_chat"));
        assert!(reg.create(&config).is_err());
    }

    #[test]
    fn plugin_kind_cannot_shadow_in_tree_ids() {
        let reg = MessengerRegistry::new_with_builtins();
        // A registered builtin.
        let err = reg
            .register_plugin_kind("acme", plugin_spec("telegram"), console_factory())
            .unwrap_err();
        assert!(err.contains("in-tree"), "{err}");
        // In the static table even when its feature is not compiled.
        let err = reg
            .register_plugin_kind("acme", plugin_spec("matrix"), console_factory())
            .unwrap_err();
        assert!(err.contains("in-tree"), "{err}");
        // And two plugins cannot fight over one id.
        reg.register_plugin_kind("acme", plugin_spec("shared_kind"), console_factory())
            .expect("first registration");
        let err = reg
            .register_plugin_kind("other", plugin_spec("shared_kind"), console_factory())
            .unwrap_err();
        assert!(err.contains("already registered by plugin 'acme'"), "{err}");
    }

    #[test]
    fn signal_cli_alias_resolves() {
        let reg = MessengerRegistry::new_with_builtins();
        #[cfg(feature = "signal-cli")]
        assert!(reg.spec("signal-cli").is_some());
        #[cfg(not(feature = "signal-cli"))]
        {
            let config = MessengerConfig {
                messenger_type: "signal-cli".into(),
                name: "s".into(),
                ..Default::default()
            };
            let err = create_err(&reg, &config);
            assert!(err.contains("--features signal-cli"), "{err}");
        }
    }
}
