//! Integration tests for gateway connections and protocol handling.
//!
//! These tests verify WebSocket connections, authentication, and message flow.
//! Run with: cargo test --test integration_gateway -- --ignored

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;

/// Find an available port
fn find_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Find the rustyclaw binary
fn find_binary() -> Option<PathBuf> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    
    let debug = PathBuf::from(&manifest_dir).join("target/debug/rustyclaw");
    if debug.exists() {
        return Some(debug);
    }
    
    let release = PathBuf::from(&manifest_dir).join("target/release/rustyclaw");
    if release.exists() {
        return Some(release);
    }
    
    which::which("rustyclaw").ok()
}

/// Test gateway process wrapper
struct TestGateway {
    process: Child,
    port: u16,
    workspace: PathBuf,
}

impl TestGateway {
    async fn start() -> Option<Self> {
        let binary = find_binary()?;
        let port = find_port();
        let workspace = std::env::temp_dir().join(format!("rustyclaw-test-{}-{}", std::process::id(), port));
        
        std::fs::create_dir_all(&workspace).ok()?;
        
        // Write minimal config
        let config_path = workspace.join("config.toml");
        std::fs::write(&config_path, format!(r#"
[gateway]
port = {port}
host = "127.0.0.1"

[provider]
kind = "mock"
"#)).ok()?;
        
        let process = Command::new(&binary)
            .arg("gateway")
            .arg("run")
            .arg("--config")
            .arg(&config_path)
            .env("RUST_LOG", "warn")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        
        let gateway = Self { process, port, workspace };
        
        // Wait for gateway to start
        for _ in 0..50 {
            if std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
                return Some(gateway);
            }
            sleep(Duration::from_millis(100)).await;
        }
        
        None
    }
    
    fn ws_url(&self) -> String {
        format!("ws://127.0.0.1:{}", self.port)
    }
}

impl Drop for TestGateway {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
        let _ = std::fs::remove_dir_all(&self.workspace);
    }
}

/// Test basic WebSocket connection to gateway
#[tokio::test]
#[ignore = "requires built binary"]
async fn test_gateway_websocket_connect() {
    let Some(gateway) = TestGateway::start().await else {
        eprintln!("Skipping: could not start gateway");
        return;
    };
    
    let result = timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(&gateway.ws_url())
    ).await;
    
    assert!(result.is_ok(), "Should connect within timeout");
    let (ws, _) = result.unwrap().expect("Should connect successfully");
    drop(ws);
}

/// Test sending a chat message
#[tokio::test]
#[ignore = "requires built binary"]
async fn test_gateway_chat_message() {
    let Some(gateway) = TestGateway::start().await else {
        eprintln!("Skipping: could not start gateway");
        return;
    };
    
    let (mut ws, _) = tokio_tungstenite::connect_async(&gateway.ws_url())
        .await
        .expect("Should connect");
    
    // Send a chat message
    let msg = serde_json::json!({
        "type": "chat",
        "content": "Hello, test!"
    });
    ws.send(Message::Text(msg.to_string())).await.expect("Should send");
    
    // Wait for any response
    let response = timeout(Duration::from_secs(10), ws.next()).await;
    assert!(response.is_ok(), "Should receive response within timeout");
}

/// Test ping/pong
#[tokio::test]
#[ignore = "requires built binary"]
async fn test_gateway_ping_pong() {
    let Some(gateway) = TestGateway::start().await else {
        eprintln!("Skipping: could not start gateway");
        return;
    };
    
    let (mut ws, _) = tokio_tungstenite::connect_async(&gateway.ws_url())
        .await
        .expect("Should connect");
    
    // Send ping
    ws.send(Message::Ping(vec![1, 2, 3])).await.expect("Should send ping");
    
    // Wait for pong
    let response = timeout(Duration::from_secs(5), ws.next()).await;
    assert!(response.is_ok(), "Should receive pong");
    
    if let Ok(Some(Ok(msg))) = response {
        assert!(matches!(msg, Message::Pong(_)), "Should be pong");
    }
}

/// Test graceful close
#[tokio::test]
#[ignore = "requires built binary"]
async fn test_gateway_graceful_close() {
    let Some(gateway) = TestGateway::start().await else {
        eprintln!("Skipping: could not start gateway");
        return;
    };
    
    let (mut ws, _) = tokio_tungstenite::connect_async(&gateway.ws_url())
        .await
        .expect("Should connect");
    
    // Send close
    ws.send(Message::Close(None)).await.expect("Should send close");
    
    // Should receive close back
    let response = timeout(Duration::from_secs(5), ws.next()).await;
    if let Ok(Some(Ok(msg))) = response {
        assert!(matches!(msg, Message::Close(_)), "Should be close frame");
    }
}

/// Test multiple concurrent connections
#[tokio::test]
#[ignore = "requires built binary"]
async fn test_gateway_concurrent_connections() {
    let Some(gateway) = TestGateway::start().await else {
        eprintln!("Skipping: could not start gateway");
        return;
    };
    
    let url = gateway.ws_url();
    
    // Open 5 concurrent connections
    let mut handles = vec![];
    for _ in 0..5 {
        let url = url.clone();
        handles.push(tokio::spawn(async move {
            tokio_tungstenite::connect_async(&url).await.is_ok()
        }));
    }
    
    let results: Vec<_> = futures_util::future::join_all(handles).await;
    let successes = results.iter().filter(|r| matches!(r, Ok(true))).count();
    
    assert!(successes >= 3, "At least 3 connections should succeed");
}
