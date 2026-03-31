use subxt::{
    backend::rpc::{RawValue, RpcClient},
    ext::subxt_rpcs::client::RpcParams,
};

use crate::url_to_string;
use anyhow::{Result, anyhow, bail};

pub struct RawParams(Option<Box<RawValue>>);

impl RawParams {
    /// Creates a new `RawParams` instance from a slice of string parameters.
    pub fn new(params: &[String]) -> Result<Self> {
        if params.is_empty() {
            return Ok(Self(None));
        }

        let value_params: Vec<serde_json::Value> = params
            .iter()
            .map(|e| {
                // Try parsing as JSON first, fall back to string
                serde_json::from_str(e).unwrap_or_else(|_| serde_json::Value::String(e.clone()))
            })
            .collect();

        let mut rpc_params = RpcParams::new();
        for param in &value_params {
            rpc_params
                .push(param)
                .map_err(|e| anyhow!("Building method parameters failed: {e}"))?;
        }

        Ok(Self(rpc_params.build()))
    }
}

pub struct RpcRequest(RpcClient);

impl RpcRequest {
    /// Creates a new `RpcRequest` instance.
    pub async fn new(url: &url::Url) -> Result<Self> {
        let rpc = RpcClient::from_url(url_to_string(url)).await?;
        Ok(Self(rpc))
    }

    /// Performs a raw RPC call with the specified method and parameters.
    pub async fn raw_call<'a>(
        &'a self,
        method: &'a str,
        params: RawParams,
    ) -> Result<Box<RawValue>> {
        let methods = self.get_supported_methods().await?;
        if !methods.iter().any(|e| e == method) {
            bail!(
                "Method not found, supported methods: {}",
                methods.join(", ")
            );
        }
        self.0
            .request_raw(method, params.0)
            .await
            .map_err(|e| anyhow!("Raw RPC call failed: {e}"))
    }

    /// Retrieves the supported RPC methods.
    async fn get_supported_methods(&self) -> Result<Vec<String>> {
        let result = self
            .0
            .request_raw("rpc_methods", None)
            .await
            .map_err(|e| anyhow!("Rpc call 'rpc_methods' failed: {e}"))?;

        let result_value: serde_json::Value = serde_json::from_str(result.get())?;

        let methods = result_value
            .get("methods")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow!("Methods field parsing failed!"))?;

        let patterns = ["watch", "unstable", "subscribe"];
        let filtered_methods: Vec<String> = methods
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .filter(|s| {
                patterns
                    .iter()
                    .all(|&pattern| !s.to_lowercase().contains(pattern))
            })
            .collect();

        Ok(filtered_methods)
    }
}
