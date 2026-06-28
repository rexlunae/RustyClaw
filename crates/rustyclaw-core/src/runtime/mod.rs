//! Runtime subsystem for platform abstraction.
//!
//! This module provides the [`RuntimeAdapter`] trait and implementations for
//! different execution environments. The runtime abstraction allows RustyClaw
//! to run on native systems, Docker containers, and (in future) serverless
//! platforms with appropriate capability detection.
//!
//! Adapted from ZeroClaw (MIT OR Apache-2.0 licensed).

pub mod docker;
pub mod native;
pub mod ssh;
pub mod traits;

pub use docker::{DockerRuntime, DockerRuntimeConfig};
pub use native::NativeRuntime;
pub use ssh::{SshRuntime, SshRuntimeConfig};
pub use traits::RuntimeAdapter;

use serde::{Deserialize, Serialize};

/// Runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Runtime kind: "native", "docker", "ssh", etc.
    #[serde(default = "default_runtime_kind")]
    pub kind: String,
    /// Docker-specific configuration.
    #[serde(default)]
    pub docker: DockerRuntimeConfig,
    /// SSH-specific configuration.
    #[serde(default)]
    pub ssh: SshRuntimeConfig,
}

fn default_runtime_kind() -> String {
    "native".to_string()
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            kind: default_runtime_kind(),
            docker: DockerRuntimeConfig::default(),
            ssh: SshRuntimeConfig::default(),
        }
    }
}

/// Factory: create the right runtime from config
pub fn create_runtime(config: &RuntimeConfig) -> anyhow::Result<Box<dyn RuntimeAdapter>> {
    match config.kind.as_str() {
        "native" => Ok(Box::new(NativeRuntime::new())),
        "docker" => Ok(Box::new(DockerRuntime::new(config.docker.clone()))),
        "ssh" => Ok(Box::new(SshRuntime::new(config.ssh.clone()))),
        other if other.trim().is_empty() => {
            anyhow::bail!("runtime.kind cannot be empty. Supported values: native, docker, ssh")
        }
        other => {
            anyhow::bail!("Unknown runtime kind '{other}'. Supported values: native, docker, ssh")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_native() {
        let cfg = RuntimeConfig {
            kind: "native".into(),
            ..RuntimeConfig::default()
        };
        let rt = create_runtime(&cfg).unwrap();
        assert_eq!(rt.name(), "native");
        assert!(rt.has_shell_access());
    }

    #[test]
    fn factory_docker() {
        let cfg = RuntimeConfig {
            kind: "docker".into(),
            ..RuntimeConfig::default()
        };
        let rt = create_runtime(&cfg).unwrap();
        assert_eq!(rt.name(), "docker");
        assert!(rt.has_shell_access());
    }

    #[test]
    fn factory_ssh() {
        let cfg = RuntimeConfig {
            kind: "ssh".into(),
            ..RuntimeConfig::default()
        };
        let rt = create_runtime(&cfg).unwrap();
        assert_eq!(rt.name(), "ssh");
        assert!(rt.has_shell_access());
    }

    #[test]
    fn factory_unknown_errors() {
        let cfg = RuntimeConfig {
            kind: "wasm-edge-unknown".into(),
            ..RuntimeConfig::default()
        };
        match create_runtime(&cfg) {
            Err(err) => assert!(err.to_string().contains("Unknown runtime kind")),
            Ok(_) => panic!("unknown runtime should error"),
        }
    }

    #[test]
    fn factory_empty_errors() {
        let cfg = RuntimeConfig {
            kind: String::new(),
            ..RuntimeConfig::default()
        };
        match create_runtime(&cfg) {
            Err(err) => assert!(err.to_string().contains("cannot be empty")),
            Ok(_) => panic!("empty runtime should error"),
        }
    }
}
