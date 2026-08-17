//! Minimal MCP (Model Context Protocol) client for the external-source protocol
//! (P36 外部源协议 / B-2 检查点).
//!
//! B-2 结论：不引入重型 MCP SDK（`rmcp` 等）。MCP 就是 **JSON-RPC 2.0 over
//! newline-delimited stdio**，用一个最小的客户端即可实现 `initialize` /
//! `tools/list` / `tools/call`，完全复用既有 `serde_json` + `tokio`。
//!
//! 外部 MCP 服务器是**外部代码**，按 A4（P32 安全沙箱）纪律，在 `run_mcp_tool`
//! 中由 `SkillSandbox` 提供工作目录与策略 env 后启动，落入沙箱约束而非裸跑。
//!
//! 该模块在默认 `cargo test --lib` 下编译（纯 tokio + serde_json，无新依赖），
//! 端到端行为由 `tests` 借助一个 mock stdio 服务器验证。

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use crate::skill::schema::{McpRef, Skill};
use crate::skill::executor::sandbox::SkillSandbox;

/// An MCP tool descriptor returned by `tools/list`.
#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

/// Errors from the MCP client / transport.
#[derive(Debug)]
pub struct McpError(pub String);

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mcp error: {}", self.0)
    }
}

impl std::error::Error for McpError {}

impl From<std::io::Error> for McpError {
    fn from(e: std::io::Error) -> Self {
        McpError(format!("io: {e}"))
    }
}

impl From<serde_json::Error> for McpError {
    fn from(e: serde_json::Error) -> Self {
        McpError(format!("json: {e}"))
    }
}

/// A live MCP session: owns the server subprocess and its stdio channels.
pub struct McpClient {
    child: Option<tokio::process::Child>,
    stdin_tx: mpsc::UnboundedSender<String>,
    rx: Mutex<mpsc::UnboundedReceiver<Value>>,
    next_id: AtomicU64,
}

impl McpClient {
    /// Launch the server and complete the `initialize` handshake.
    ///
    /// `sandbox_dir`, when provided, is set as the server's working directory and
    /// the `CASPIAN_SANDBOX` policy env is applied — keeping external code inside
    /// the P32 sandbox (A4).
    pub async fn start(
        server_command: &[String],
        sandbox_dir: Option<&Path>,
    ) -> Result<Self, McpError> {
        if server_command.is_empty() {
            return Err(McpError("empty MCP server command".into()));
        }
        let mut cmd = TokioCommand::new(&server_command[0]);
        cmd.args(&server_command[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        if let Some(dir) = sandbox_dir {
            cmd.current_dir(dir);
            cmd.env("CASPIAN_SANDBOX", "1");
            cmd.env(
                "CASPIAN_SKILL_DIR",
                dir.to_string_lossy().to_string(),
            );
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| McpError(format!("spawn {}: {e}", server_command[0])))?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError("server has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError("server has no stdout".into()))?;

        // Reader task: parse newline-delimited JSON-RPC messages → channel.
        let (tx, rx) = mpsc::unbounded_channel::<Value>();
        let mut reader = BufReader::new(stdout).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = reader.next_line().await {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    let _ = tx.send(v);
                }
            }
        });

        // Writer task: flush outgoing request lines.
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = stdin_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                let _ = stdin.write_all(b"\n").await;
                let _ = stdin.flush().await;
            }
        });

        let client = Self {
            child: Some(child),
            stdin_tx,
            rx: Mutex::new(rx),
            next_id: AtomicU64::new(1),
        };
        client.handshake().await?;
        Ok(client)
    }

    /// Send a request and await its matching response (correlated by `id`).
    async fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let line = serde_json::to_string(&msg)?;
        self.stdin_tx
            .send(line)
            .map_err(|_| McpError("stdin closed".into()))?;

        let mut rx = self.rx.lock().await;
        loop {
            let v = tokio::time::timeout(Duration::from_secs(30), rx.recv())
                .await
                .map_err(|_| McpError("request timeout".into()))?
                .ok_or_else(|| McpError("channel closed".into()))?;
            // Responses carry the request id; notifications (no id) are ignored.
            if let Some(msg_id) = v.get("id").and_then(|x| x.as_u64()) {
                if msg_id == id {
                    if let Some(err) = v.get("error") {
                        return Err(McpError(format!("server error: {err}")));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
            }
        }
    }

    /// `initialize` request + `notifications/initialized` (no response expected).
    async fn handshake(&self) -> Result<(), McpError> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "caspian-flow", "version": "0.1.0"},
        });
        self.request("initialize", params).await?;
        let notif = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        self.stdin_tx
            .send(serde_json::to_string(&notif)?)
            .map_err(|_| McpError("stdin closed".into()))?;
        Ok(())
    }

    /// List tools exposed by the server (`tools/list`).
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, McpError> {
        let res = self.request("tools/list", json!({})).await?;
        let tools = res
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .into_iter()
            .map(|t| McpTool {
                name: t
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                description: t.get("description").and_then(|x| x.as_str()).map(str::to_string),
                input_schema: t.get("inputSchema").cloned().unwrap_or(Value::Null),
            })
            .collect())
    }

    /// Call a tool (`tools/call`). Returns the raw JSON-RPC `result`.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        self.request("tools/call", json!({"name": name, "arguments": arguments }))
            .await
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }
}

/// Execute one MCP tool call as a skill execution, inside the P32 sandbox.
pub async fn run_mcp_tool(
    server_command: &[String],
    tool: &str,
    input: &Value,
) -> Result<Value, McpError> {
    // External code runs sandboxed (A4): private disposable CWD + policy env.
    let sandbox = SkillSandbox::new().map_err(|e| McpError(e.to_string()))?;
    let client = McpClient::start(server_command, Some(&sandbox.dir)).await?;
    let arguments = if input.is_object() {
        input.clone()
    } else {
        json!({})
    };
    client.call_tool(tool, arguments).await
}

/// Convert an MCP server's tool list into in-memory `Skill` structs bound to it.
///
/// These are *virtual* skills: they carry an [`McpRef`] so the executor routes
/// their execution to `run_mcp_tool` instead of spawning a local entry script.
/// `server_command` is shared by all tools of the same server.
pub fn tools_to_skills(server_command: &[String], tools: &[McpTool]) -> Vec<Skill> {
    tools
        .iter()
        .map(|t| Skill {
            schema_version: crate::skill::schema::SKILL_SCHEMA_VERSION.to_string(),
            name: t.name.clone(),
            display_name: t.name.clone(),
            version: "0.0.0".to_string(),
            description: t.description.clone().unwrap_or_default(),
            category: "mcp".to_string(),
            trigger_phrases: vec![],
            runtime: crate::skill::schema::SkillRuntime::default(),
            input_schema: t.input_schema.clone(),
            output_schema: Value::Null,
            permissions: Default::default(),
            tags: vec!["mcp".to_string()],
            author: "mcp".to_string(),
            license: "MIT".to_string(),
            enabled: true,
            path: Path::new("").to_path_buf(),
            mcp: Some(McpRef {
                server_command: server_command.to_vec(),
                tool: t.name.clone(),
            }),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write a tiny mock MCP stdio server (Python) that implements
    /// initialize / tools/list / tools/call for a single `echo` tool.
    fn write_mock_server(dir: &Path) -> String {
        let script = r#"
import sys, json
def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n"); sys.stdout.flush()
def handle(req):
    mid = req.get("id")
    method = req.get("method")
    if method == "initialize":
        return {"jsonrpc":"2.0","id":mid,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"mock","version":"0.0.0"}}}
    if method == "tools/list":
        return {"jsonrpc":"2.0","id":mid,"result":{"tools":[{"name":"echo","description":"echo a text","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}]}}
    if method == "tools/call":
        name = req["params"]["name"]
        args = req["params"].get("arguments", {})
        if name == "echo":
            text = args.get("text", "")
            return {"jsonrpc":"2.0","id":mid,"result":{"content":[{"type":"text","text":"echo:"+str(text)}],"isError":False}}
        return {"jsonrpc":"2.0","id":mid,"error":{"code":-32601,"message":"unknown tool "+name}}
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except Exception:
        continue
    if "method" in req and req.get("id") is not None and not req["method"].startswith("notifications/"):
        send(handle(req))
"#;
        let path = dir.join("mock_mcp_server.py");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(script.as_bytes()).unwrap();
        path.to_string_lossy().to_string()
    }

    #[tokio::test]
    async fn test_mcp_client_echo_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let server = write_mock_server(dir.path());
        let cmd = vec!["python3".to_string(), server];

        let client = McpClient::start(&cmd, None).await.expect("connect");
        let tools = client.list_tools().await.expect("list tools");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");

        let res = client
            .call_tool("echo", json!({"text": "hi"}))
            .await
            .expect("call");
        let content = res.get("content").and_then(|c| c.as_array()).unwrap();
        let text = content[0].get("text").and_then(|t| t.as_str()).unwrap();
        assert_eq!(text, "echo:hi");
    }

    #[tokio::test]
    async fn test_run_mcp_tool_sandboxed() {
        let dir = tempfile::tempdir().unwrap();
        let server = write_mock_server(dir.path());
        let cmd = vec!["python3".to_string(), server];
        let res = run_mcp_tool(&cmd, "echo", &json!({"text": "boxed"})).await.unwrap();
        let text = res["content"][0]["text"].as_str().unwrap();
        assert_eq!(text, "echo:boxed");
    }

    #[test]
    fn test_tools_to_skills_carries_mcp_ref() {
        let tools = vec![McpTool {
            name: "echo".to_string(),
            description: Some("echo".to_string()),
            input_schema: json!({"type": "object"}),
        }];
        let cmd = vec!["python3".to_string(), "server.py".to_string()];
        let skills = tools_to_skills(&cmd, &tools);
        assert_eq!(skills.len(), 1);
        let mcp = skills[0].mcp.as_ref().unwrap();
        assert_eq!(mcp.tool, "echo");
        assert_eq!(mcp.server_command, cmd);
        assert_eq!(skills[0].category, "mcp");
    }
}
