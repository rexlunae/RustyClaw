//! WebSocket transport for the RustyClaw gateway protocol.
//!
//! Provides a browser-compatible transport using the WebSocket API,
//! sending frames as binary messages (bincode-encoded).

use wasm_bindgen::prelude::*;
use web_sys::{MessageEvent, WebSocket};

/// Connection state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsState {
    Connecting,
    Open,
    Closing,
    Closed,
    Error(String),
}

/// WebSocket transport for the gateway protocol.
pub struct WsTransport {
    ws: WebSocket,
    state: WsState,
}

impl WsTransport {
    /// Create a new WebSocket connection to the gateway.
    pub fn connect(url: &str) -> Result<Self, JsValue> {
        let ws = WebSocket::new(url)?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        Ok(Self {
            ws,
            state: WsState::Connecting,
        })
    }

    /// Get the current connection state.
    pub fn state(&self) -> &WsState {
        &self.state
    }

    /// Send a binary frame.
    pub fn send_binary(&self, data: &[u8]) -> Result<(), JsValue> {
        self.ws.send_with_u8_array(data)
    }

    /// Close the WebSocket connection.
    pub fn close(&self) -> Result<(), JsValue> {
        self.ws.close()
    }

    /// Set up event callbacks.
    pub fn set_onopen(&self, callback: impl FnMut() + 'static) {
        let closure = Closure::wrap(Box::new(callback) as Box<dyn FnMut()>);
        self.ws.set_onopen(Some(closure.as_ref().unchecked_ref()));
        closure.forget();
    }

    /// Set up message callback.
    pub fn set_onmessage(&self, callback: impl FnMut(Vec<u8>) + 'static) {
        let closure = Closure::wrap(Box::new(move |e: MessageEvent| {
            if let Ok(abuf) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                let array = js_sys::Uint8Array::new(&abuf);
                let mut data = vec![0u8; array.length() as usize];
                array.copy_to(&mut data);
                callback(data);
            }
        }) as Box<dyn FnMut(MessageEvent)>);
        self.ws
            .set_onmessage(Some(closure.as_ref().unchecked_ref()));
        closure.forget();
    }

    /// Set up error callback.
    pub fn set_onerror(&self, callback: impl FnMut(String) + 'static) {
        let closure = Closure::wrap(Box::new(move |_e: web_sys::ErrorEvent| {
            callback("WebSocket error".to_string());
        }) as Box<dyn FnMut(web_sys::ErrorEvent)>);
        self.ws.set_onerror(Some(closure.as_ref().unchecked_ref()));
        closure.forget();
    }

    /// Set up close callback.
    pub fn set_onclose(&self, callback: impl FnMut(u16, String) + 'static) {
        let closure = Closure::wrap(Box::new(move |e: web_sys::CloseEvent| {
            callback(e.code(), e.reason());
        }) as Box<dyn FnMut(web_sys::CloseEvent)>);
        self.ws
            .set_onclose(Some(closure.as_ref().unchecked_ref()));
        closure.forget();
    }
}

/// APIs gated behind `#[cfg(not(target_arch = "wasm32"))]` in other crates:
///
/// The following ~39 APIs are desktop-only and not available in the web client:
/// - File system operations (std::fs, dirs, walkdir)
/// - Process spawning (std::process::Command, tokio::process)
/// - Native TLS/SSH (openssl, ssh-key, russh)
/// - Terminal I/O (crossterm, rpassword)
/// - System info queries (sysinfo, which)
/// - Native audio/media playback
/// - MCP child process transport
/// - Clipboard via native APIs
/// - Local socket/pipe listeners
///
/// These are accessible only through the gateway via protocol frames.
pub const DESKTOP_ONLY_API_COUNT: usize = 39;
