use rbx_control::studio::mcp_client::StudioMcpClient;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Handle an `mcp.call` RPC request.
///
/// The mcp_studio_id is already resolved by the reconciliation loop in studio_state.
/// This forwards the tool call targeted at that Studio: StudioMCP tools that act
/// on a Studio take its id as a `studio_id` argument.
pub async fn handle_mcp_call(
    studio_mcp: Arc<Mutex<Option<StudioMcpClient>>>,
    mcp_studio_id: &str,
    tool: &str,
    arguments: &Value,
) -> Result<String, String> {
    let mcp_short = &mcp_studio_id[..8.min(mcp_studio_id.len())];
    let wait_start = std::time::Instant::now();
    tracing::info!(mcp_studio = mcp_short, tool, "mcp.call wait_lock");
    let mut mcp_guard = studio_mcp.lock().await;
    tracing::info!(mcp_studio = mcp_short, tool, wait_ms = wait_start.elapsed().as_millis() as u64, "mcp.call locked");
    let mcp = match mcp_guard.as_mut() {
        Some(mcp) => mcp,
        None => {
            return Err(
                "StudioMCP not connected yet. Enable MCP Server in Studio AI Assistant settings."
                    .into(),
            );
        }
    };

    // list_roblox_studios is the one tool that targets no Studio.
    let mut arguments = arguments.clone();
    if tool != "list_roblox_studios" {
        if let Value::Object(ref mut map) = arguments {
            map.entry("studio_id").or_insert_with(|| Value::String(mcp_studio_id.to_string()));
        }
    }

    let call_start = std::time::Instant::now();
    let call_res = mcp.call_tool(tool, &arguments).await;
    tracing::info!(
        mcp_studio = mcp_short,
        tool,
        ok = call_res.is_ok(),
        elapsed_ms = call_start.elapsed().as_millis() as u64,
        "mcp.call_tool done"
    );
    call_res
}
