//! SSH runtime adapter: execute commands on a remote host.
//!
//! Implements `RuntimeAdapter` for remote command execution over SSH.
//! Uses the `russh` crate (already a dependency) for async SSH connections.

use super::traits::RuntimeAdapter;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// SSH runtime configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshRuntimeConfig {
    /// Remote hostname or IP address.
    #[serde(default = "default_ssh_host")]
    pub host: String,

    /// SSH port (default: 22).
    #[serde(default = "default_ssh_port")]
    pub port: u16,

    /// Username for SSH authentication.
    #[serde(default = "default_ssh_user")]
    pub user: String,

    /// Path to the SSH private key file.
    /// If empty, uses the default SSH key (~/.ssh/id_ed25519 or ~/.ssh/id_rsa).
    #[serde(default)]
    pub key_path: String,

    /// Remote working directory for command execution.
    #[serde(default = "default_remote_workdir")]
    pub remote_workdir: String,

    /// Remote storage path for persistent data.
    #[serde(default = "default_remote_storage")]
    pub remote_storage: String,

    /// Connection timeout in seconds.
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,

    /// Command execution timeout in seconds (0 = no limit).
    #[serde(default)]
    pub command_timeout_secs: u64,

    /// Whether to allocate a PTY for commands.
    #[serde(default)]
    pub allocate_pty: bool,

    /// SSH agent forwarding.
    #[serde(default)]
    pub agent_forwarding: bool,

    /// Known hosts file path (empty = disable strict checking).
    #[serde(default)]
    pub known_hosts_file: String,
}

fn default_ssh_host() -> String {
    "localhost".to_string()
}

fn default_ssh_port() -> u16 {
    22
}

fn default_ssh_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "root".to_string())
}

fn default_remote_workdir() -> String {
    "/tmp/rustyclaw-workspace".to_string()
}

fn default_remote_storage() -> String {
    "/tmp/rustyclaw-storage".to_string()
}

fn default_connect_timeout() -> u64 {
    30
}

impl Default for SshRuntimeConfig {
    fn default() -> Self {
        Self {
            host: default_ssh_host(),
            port: default_ssh_port(),
            user: default_ssh_user(),
            key_path: String::new(),
            remote_workdir: default_remote_workdir(),
            remote_storage: default_remote_storage(),
            connect_timeout_secs: default_connect_timeout(),
            command_timeout_secs: 0,
            allocate_pty: false,
            agent_forwarding: false,
            known_hosts_file: String::new(),
        }
    }
}

/// SSH runtime with remote command execution over SSH.
#[derive(Debug, Clone)]
pub struct SshRuntime {
    config: SshRuntimeConfig,
}

impl SshRuntime {
    pub fn new(config: SshRuntimeConfig) -> Self {
        Self { config }
    }

    /// Build the SSH command prefix for executing a remote command.
    fn ssh_command_args(&self, command: &str, workspace_dir: &Path) -> Vec<String> {
        let mut args = Vec::new();

        // Port
        if self.config.port != 22 {
            args.push("-p".to_string());
            args.push(self.config.port.to_string());
        }

        // Key file
        if !self.config.key_path.is_empty() {
            args.push("-i".to_string());
            args.push(self.config.key_path.clone());
        }

        // Disable strict host key checking if no known_hosts file specified
        if self.config.known_hosts_file.is_empty() {
            args.push("-o".to_string());
            args.push("StrictHostKeyChecking=no".to_string());
            args.push("-o".to_string());
            args.push("UserKnownHostsFile=/dev/null".to_string());
        } else {
            args.push("-o".to_string());
            args.push(format!(
                "UserKnownHostsFile={}",
                self.config.known_hosts_file
            ));
        }

        // Connection timeout
        args.push("-o".to_string());
        args.push(format!(
            "ConnectTimeout={}",
            self.config.connect_timeout_secs
        ));

        // Batch mode (no interactive prompts)
        args.push("-o".to_string());
        args.push("BatchMode=yes".to_string());

        // Agent forwarding
        if self.config.agent_forwarding {
            args.push("-A".to_string());
        }

        // PTY allocation
        if self.config.allocate_pty {
            args.push("-t".to_string());
        }

        // Target: user@host
        args.push(format!("{}@{}", self.config.user, self.config.host));

        // Remote command with cd to workspace
        let remote_dir = workspace_dir.to_string_lossy();
        let wrapped_command = format!("cd {} 2>/dev/null || true; {}", remote_dir, command);
        args.push(wrapped_command);

        args
    }
}

impl RuntimeAdapter for SshRuntime {
    fn name(&self) -> &str {
        "ssh"
    }

    fn has_shell_access(&self) -> bool {
        true
    }

    fn has_filesystem_access(&self) -> bool {
        // Remote filesystem access via SSH
        true
    }

    fn storage_path(&self) -> PathBuf {
        PathBuf::from(&self.config.remote_storage)
    }

    fn supports_long_running(&self) -> bool {
        // SSH can maintain persistent connections for long-running commands
        true
    }

    fn build_shell_command(
        &self,
        command: &str,
        workspace_dir: &Path,
    ) -> Result<tokio::process::Command> {
        let args = self.ssh_command_args(command, workspace_dir);

        let mut cmd = tokio::process::Command::new("ssh");
        cmd.args(&args);

        // Suppress SSH warnings on stderr for cleaner output
        cmd.env("SSH_AUTH_SOCK", std::env::var("SSH_AUTH_SOCK").unwrap_or_default());

        Ok(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_runtime_capabilities() {
        let config = SshRuntimeConfig {
            host: "example.com".to_string(),
            user: "deploy".to_string(),
            ..Default::default()
        };
        let runtime = SshRuntime::new(config);

        assert_eq!(runtime.name(), "ssh");
        assert!(runtime.has_shell_access());
        assert!(runtime.has_filesystem_access());
        assert!(runtime.supports_long_running());
        assert_eq!(runtime.memory_budget(), 0);
    }

    #[test]
    fn ssh_command_args_basic() {
        let config = SshRuntimeConfig {
            host: "remote.host".to_string(),
            user: "agent".to_string(),
            port: 22,
            ..Default::default()
        };
        let runtime = SshRuntime::new(config);

        let args = runtime.ssh_command_args("echo hello", Path::new("/workspace"));
        assert!(args.contains(&"agent@remote.host".to_string()));
        assert!(args.last().unwrap().contains("echo hello"));
    }

    #[test]
    fn ssh_command_args_custom_port() {
        let config = SshRuntimeConfig {
            host: "remote.host".to_string(),
            user: "agent".to_string(),
            port: 2222,
            ..Default::default()
        };
        let runtime = SshRuntime::new(config);

        let args = runtime.ssh_command_args("ls", Path::new("/workspace"));
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"2222".to_string()));
    }

    #[test]
    fn ssh_command_args_with_key() {
        let config = SshRuntimeConfig {
            host: "remote.host".to_string(),
            user: "agent".to_string(),
            key_path: "/home/user/.ssh/deploy_key".to_string(),
            ..Default::default()
        };
        let runtime = SshRuntime::new(config);

        let args = runtime.ssh_command_args("ls", Path::new("/workspace"));
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"/home/user/.ssh/deploy_key".to_string()));
    }

    #[tokio::test]
    async fn build_shell_command_creates_ssh_process() {
        let config = SshRuntimeConfig {
            host: "example.com".to_string(),
            user: "deploy".to_string(),
            ..Default::default()
        };
        let runtime = SshRuntime::new(config);

        let cmd = runtime
            .build_shell_command("echo test", Path::new("/workspace"))
            .unwrap();

        // Verify it's an ssh command
        assert_eq!(cmd.as_std().get_program(), "ssh");
    }

    #[test]
    fn default_config_values() {
        let config = SshRuntimeConfig::default();
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 22);
        assert_eq!(config.remote_workdir, "/tmp/rustyclaw-workspace");
        assert_eq!(config.remote_storage, "/tmp/rustyclaw-storage");
        assert_eq!(config.connect_timeout_secs, 30);
        assert!(!config.allocate_pty);
        assert!(!config.agent_forwarding);
    }
}
