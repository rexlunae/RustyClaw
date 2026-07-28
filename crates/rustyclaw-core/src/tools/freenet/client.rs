//! Minimal client for a local Freenet node's WebSocket command API.
//!
//! Freenet is a decentralized key-value store where keys are WebAssembly
//! contracts and values are the state those contracts govern. A node runs
//! locally and exposes a WebSocket endpoint; every operation here is a
//! request/response round trip against that endpoint.
//!
//! Modelled on the reference clients (`atlasctl`, `riverctl`), which both talk
//! to the node through `freenet-stdlib`'s [`WebApi`].

use std::sync::Arc;
use std::time::Duration;

use freenet_stdlib::client_api::{
    ClientRequest, ContractRequest, ContractResponse, HostResponse, WebApi,
};
use freenet_stdlib::prelude::*;
use tokio_tungstenite::connect_async;

use crate::tools::error::{ToolError, ToolResult};

/// Where a node listens for the WebSocket command API when nothing overrides it.
///
/// Freenet originally used 50509; current builds default to 7509 because
/// Windows reserves the 50000-50059 range. [`node_url`] surfaces both in its
/// error text so a connection failure against the wrong one is diagnosable.
pub const DEFAULT_NODE_URL: &str =
    "ws://127.0.0.1:7509/v1/contract/command?encodingProtocol=native";

/// The port used by Freenet releases before the 7509 change, offered as a
/// hint when a connection is refused.
pub const LEGACY_NODE_PORT: u16 = 50509;

/// Environment variable that overrides [`DEFAULT_NODE_URL`].
pub const NODE_URL_ENV: &str = "FREENET_NODE_URL";

/// How long to wait for the node to answer a single request.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// Resolve the node URL: an explicit tool argument wins, then
/// `$FREENET_NODE_URL`, then [`DEFAULT_NODE_URL`].
pub fn node_url(explicit: Option<&str>) -> String {
    if let Some(url) = explicit.filter(|u| !u.trim().is_empty()) {
        return url.trim().to_string();
    }
    std::env::var(NODE_URL_ENV)
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_NODE_URL.to_string())
}

/// Parse a base58 contract instance id, the 43-44 character identifier that
/// addresses one contract instance on the network.
pub fn parse_instance_id(id: &str) -> ToolResult<ContractInstanceId> {
    let trimmed = id.trim();
    // Accept the `freenet:<id><path>` locator form Atlas publishes, and plain
    // ids alike, so an id copied out of a search result works unchanged.
    let trimmed = trimmed.strip_prefix("freenet:").unwrap_or(trimmed);
    let bare = trimmed.split(['/', '?', '#']).next().unwrap_or(trimmed);
    bare.parse::<ContractInstanceId>().map_err(|e| {
        ToolError::from(format!(
            "'{bare}' is not a valid Freenet contract instance id \
             (expected 43-44 base58 characters): {e}"
        ))
    })
}

/// A connection to one Freenet node, good for a single tool invocation.
pub struct NodeClient {
    api: WebApi,
    url: String,
}

impl NodeClient {
    /// Open a WebSocket connection to the node.
    ///
    /// A refused connection is by far the most common failure, so the error
    /// spells out what to check rather than surfacing a bare transport error.
    pub async fn connect(url: &str) -> ToolResult<Self> {
        let (ws, _) = connect_async(url).await.map_err(|e| {
            ToolError::from(format!(
                "Could not reach a Freenet node at {url}: {e}\n\
                 Check that `freenet` is running locally. If your node predates \
                 the move off the Windows-reserved port range, it listens on \
                 {LEGACY_NODE_PORT} instead — set {NODE_URL_ENV} (or pass \
                 `node_url`) to point at it."
            ))
        })?;
        Ok(Self {
            api: WebApi::start(ws),
            url: url.to_string(),
        })
    }

    /// Send one request and wait for the node's reply.
    async fn roundtrip(&mut self, req: ClientRequest<'static>) -> ToolResult<HostResponse> {
        self.api
            .send(req)
            .await
            .map_err(|e| ToolError::from(format!("Sending request to the node failed: {e}")))?;
        tokio::time::timeout(RESPONSE_TIMEOUT, self.api.recv())
            .await
            .map_err(|_| {
                ToolError::from(format!(
                    "The node at {} did not respond within {}s. A GET for a \
                     contract no peer is hosting can hang until it times out.",
                    self.url,
                    RESPONSE_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| ToolError::from(format!("The node returned an error: {e}")))
    }

    /// Fetch a contract's state, and optionally the contract itself (whose
    /// parameters carry app-level identity — the room owner for River, the
    /// index root key for Atlas).
    pub async fn get(
        &mut self,
        id: ContractInstanceId,
        return_contract_code: bool,
    ) -> ToolResult<ContractFetch> {
        let req = ContractRequest::Get {
            key: id,
            return_contract_code,
            subscribe: false,
            blocking_subscribe: false,
        };
        match self.roundtrip(ClientRequest::ContractOp(req)).await? {
            HostResponse::ContractResponse(ContractResponse::GetResponse {
                state,
                contract,
                ..
            }) => Ok(ContractFetch {
                state: state.as_ref().to_vec(),
                parameters: contract.as_ref().map(|c| c.params().as_ref().to_vec()),
                key: contract.as_ref().map(|c| c.key()),
            }),
            other => Err(unexpected("GET", &other)),
        }
    }

    /// Publish a contract and its initial state, returning the instance id the
    /// node assigned. `subscribe` makes the node host the contract so remote
    /// GETs can find it.
    pub async fn put(
        &mut self,
        code: Vec<u8>,
        params: Vec<u8>,
        state: Vec<u8>,
        subscribe: bool,
    ) -> ToolResult<ContractKey> {
        let contract_code = ContractCode::from(code);
        let parameters = Parameters::from(params);
        let key = ContractKey::from_params_and_code(parameters.clone(), &contract_code);
        let container = ContractContainer::from(ContractWasmAPIVersion::V1(WrappedContract::new(
            Arc::new(contract_code),
            parameters,
        )));
        let req = ContractRequest::Put {
            contract: container,
            state: WrappedState::new(state),
            related_contracts: Default::default(),
            subscribe,
            blocking_subscribe: subscribe,
        };
        match self.roundtrip(ClientRequest::ContractOp(req)).await? {
            HostResponse::ContractResponse(ContractResponse::PutResponse { key }) => Ok(key),
            // A PUT over a contract that already exists is applied as an
            // update, which the node acknowledges differently.
            HostResponse::ContractResponse(ContractResponse::UpdateResponse { .. })
            | HostResponse::ContractResponse(ContractResponse::UpdateNotification { .. })
            | HostResponse::Ok => Ok(key),
            other => Err(unexpected("PUT", &other)),
        }
    }

    /// Fetch a contract's full key, which an update needs.
    ///
    /// A [`ContractKey`] is an instance id *plus* the code hash, and the hash
    /// cannot be recovered from the id — so an update has to ask the node for
    /// the contract before it can address one. Fetching by id alone and
    /// fabricating a key would produce a request the node silently routes
    /// nowhere.
    pub async fn key_for(&mut self, id: ContractInstanceId) -> ToolResult<ContractKey> {
        self.get(id, true).await?.key.ok_or_else(|| {
            ToolError::from(format!(
                "The node returned state for {id} without the contract itself, \
                 so its code hash — required to address an update — is unknown."
            ))
        })
    }

    /// Apply a delta to an existing contract. The contract decides whether the
    /// delta is admissible; a rejection comes back as a node error.
    pub async fn update_delta(&mut self, key: ContractKey, delta: Vec<u8>) -> ToolResult<()> {
        let req = ContractRequest::Update {
            key,
            data: UpdateData::Delta(StateDelta::from(delta)),
        };
        match self.roundtrip(ClientRequest::ContractOp(req)).await? {
            HostResponse::ContractResponse(ContractResponse::UpdateResponse { .. })
            | HostResponse::Ok => Ok(()),
            other => Err(unexpected("UPDATE", &other)),
        }
    }

    /// Ask the node about itself — currently its connected peers.
    pub async fn connected_peers(&mut self) -> ToolResult<Vec<(String, String)>> {
        let req = ClientRequest::NodeQueries(freenet_stdlib::client_api::NodeQuery::ConnectedPeers);
        match self.roundtrip(req).await? {
            HostResponse::QueryResponse(
                freenet_stdlib::client_api::QueryResponse::ConnectedPeers { peers },
            ) => Ok(peers
                .into_iter()
                .map(|(id, addr)| (id.to_string(), addr.to_string()))
                .collect()),
            other => Err(unexpected("node query", &other)),
        }
    }
}

/// A contract's state, plus what the contract itself carries when the code was
/// requested.
pub struct ContractFetch {
    /// Raw state bytes. Interpretation is entirely up to the contract.
    pub state: Vec<u8>,
    /// The contract's parameters, present only when `return_contract_code` was set.
    pub parameters: Option<Vec<u8>>,
    /// The contract's full key (instance id + code hash), same condition.
    pub key: Option<ContractKey>,
}

/// Build the error for a response the node was not supposed to send here.
///
/// Kept explicit rather than silently treating an unknown response as success:
/// an unrecognised reply means the operation's outcome is genuinely unknown.
fn unexpected(op: &str, response: &HostResponse) -> ToolError {
    ToolError::from(format!(
        "The node answered a {op} with an unexpected response: {response:?}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_url_wins_over_the_environment() {
        assert_eq!(node_url(Some("ws://elsewhere/")), "ws://elsewhere/");
    }

    #[test]
    fn a_blank_url_falls_back_to_the_default() {
        // Only asserts the argument path; the env var is process-global and
        // reading it from a test would race the rest of the suite.
        assert_eq!(node_url(Some("   ")), node_url(None));
    }

    #[test]
    fn an_id_is_accepted_with_or_without_a_locator_wrapper() {
        // 43-44 base58 chars; the exact value is arbitrary.
        let raw = "771DvtPMwt2PumPyrFvsz7fpvU1gogcmb5qtS1yYEEH9";
        let plain = parse_instance_id(raw).expect("plain id should parse");
        let wrapped =
            parse_instance_id(&format!("freenet:{raw}/index.html?q=1")).expect("locator form");
        assert_eq!(plain, wrapped);
    }

    #[test]
    fn a_malformed_id_names_what_was_expected() {
        let err = parse_instance_id("not-an-id").expect_err("should reject");
        assert!(err.to_string().contains("43-44 base58"), "{err}");
    }
}
