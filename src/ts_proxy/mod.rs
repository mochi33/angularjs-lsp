mod transport;

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use serde_json::{json, Value};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};
use tower_lsp::lsp_types::*;
use tracing::{error, info, warn};

use transport::{LspReader, LspWriter};

/// typescript-language-server へのプロキシ
pub struct TsProxy {
    writer: Arc<Mutex<LspWriter>>,
    pending_requests: Arc<DashMap<i64, oneshot::Sender<Value>>>,
    next_id: AtomicI64,
    _child: Child,
}

impl TsProxy {
    /// typescript-language-server を起動してプロキシを初期化
    pub async fn start(root_uri: Option<&Url>) -> Option<Self> {
        let tsserver_path = find_tsserver()?;

        info!("Starting typescript-language-server: {:?}", tsserver_path);

        let mut child = Command::new(&tsserver_path)
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        let stdin = child.stdin.take()?;
        let stdout = child.stdout.take()?;

        let writer = Arc::new(Mutex::new(LspWriter::new(stdin)));
        let mut reader = LspReader::new(stdout);
        let pending_requests: Arc<DashMap<i64, oneshot::Sender<Value>>> = Arc::new(DashMap::new());

        let proxy = Self {
            writer: Arc::clone(&writer),
            pending_requests: Arc::clone(&pending_requests),
            next_id: AtomicI64::new(1),
            _child: child,
        };

        // レスポンス受信タスクを起動
        let pending_clone = Arc::clone(&pending_requests);
        tokio::spawn(async move {
            loop {
                match reader.read_message().await {
                    Ok(response) => {
                        if let Some(id) = response.get("id").and_then(|v| v.as_i64()) {
                            if let Some((_, sender)) = pending_clone.remove(&id) {
                                let _ = sender.send(response);
                            }
                        }
                        // 通知メッセージ（idなし）は無視
                    }
                    Err(e) => {
                        error!("Error reading from tsserver: {}", e);
                        break;
                    }
                }
            }
        });

        // initialize リクエストを送信
        let root_uri_str = root_uri.map(|u| u.to_string());
        let init_params = json!({
            "processId": std::process::id(),
            "rootUri": root_uri_str,
            "capabilities": {
                "textDocument": {
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "definition": { "linkSupport": true },
                    "references": {}
                }
            }
        });

        let init_response = proxy.send_request("initialize", init_params).await;
        if init_response.is_none() {
            warn!("Failed to initialize typescript-language-server");
            return None;
        }

        // initialized 通知を送信
        proxy.send_notification("initialized", json!({})).await;

        info!("typescript-language-server initialized successfully");
        Some(proxy)
    }

    /// リクエストを送信してレスポンスを待機
    async fn send_request(&self, method: &str, params: Value) -> Option<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        let (tx, rx) = oneshot::channel();
        self.pending_requests.insert(id, tx);

        {
            let mut writer = self.writer.lock().await;
            if writer.write_message(&request).await.is_err() {
                self.pending_requests.remove(&id);
                return None;
            }
        }

        // タイムアウト付きで待機（大きなファイルでも対応できるよう30秒）
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(response)) => Some(response),
            _ => {
                self.pending_requests.remove(&id);
                None
            }
        }
    }

    /// 通知を送信
    async fn send_notification(&self, method: &str, params: Value) {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        let mut writer = self.writer.lock().await;
        let _ = writer.write_message(&notification).await;
    }

    /// ドキュメントを開いたことを通知
    pub async fn did_open(&self, uri: &Url, text: &str) {
        let params = json!({
            "textDocument": {
                "uri": uri.to_string(),
                "languageId": "javascript",
                "version": 1,
                "text": text
            }
        });
        self.send_notification("textDocument/didOpen", params).await;
    }

    /// ドキュメントの変更を通知
    pub async fn did_change(&self, uri: &Url, text: &str, version: i32) {
        let params = json!({
            "textDocument": {
                "uri": uri.to_string(),
                "version": version
            },
            "contentChanges": [{ "text": text }]
        });
        self.send_notification("textDocument/didChange", params).await;
    }

    /// ドキュメントを閉じたことを通知
    pub async fn did_close(&self, uri: &Url) {
        let params = json!({
            "textDocument": {
                "uri": uri.to_string()
            }
        });
        self.send_notification("textDocument/didClose", params).await;
    }

    /// ホバー情報を取得
    pub async fn hover(&self, params: &HoverParams) -> Option<Hover> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = &params.text_document_position_params.position;

        let request_params = json!({
            "textDocument": { "uri": uri.to_string() },
            "position": { "line": pos.line, "character": pos.character }
        });

        let response = self.send_request("textDocument/hover", request_params).await?;
        let result = response.get("result")?;

        if result.is_null() {
            return None;
        }

        serde_json::from_value(result.clone()).ok()
    }

    /// 定義へジャンプ
    pub async fn goto_definition(&self, params: &GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = &params.text_document_position_params.position;

        let request_params = json!({
            "textDocument": { "uri": uri.to_string() },
            "position": { "line": pos.line, "character": pos.character }
        });

        let response = self.send_request("textDocument/definition", request_params).await?;
        let result = response.get("result")?;

        if result.is_null() {
            return None;
        }

        serde_json::from_value(result.clone()).ok()
    }

    /// 参照を検索
    pub async fn references(&self, params: &ReferenceParams) -> Option<Vec<Location>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = &params.text_document_position.position;

        let request_params = json!({
            "textDocument": { "uri": uri.to_string() },
            "position": { "line": pos.line, "character": pos.character },
            "context": { "includeDeclaration": params.context.include_declaration }
        });

        let response = self.send_request("textDocument/references", request_params).await?;
        let result = response.get("result")?;

        if result.is_null() {
            return None;
        }

        serde_json::from_value(result.clone()).ok()
    }

    /// 補完候補を取得
    pub async fn completion(&self, params: &CompletionParams) -> Option<CompletionResponse> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = &params.text_document_position.position;

        let request_params = json!({
            "textDocument": { "uri": uri.to_string() },
            "position": { "line": pos.line, "character": pos.character }
        });

        let response = self.send_request("textDocument/completion", request_params).await?;
        let result = response.get("result")?;

        if result.is_null() {
            return None;
        }

        serde_json::from_value(result.clone()).ok()
    }

    /// シャットダウン
    pub async fn shutdown(&self) {
        let _ = self.send_request("shutdown", json!(null)).await;
        self.send_notification("exit", json!(null)).await;
    }
}

/// PATH から typescript-language-server を検索
fn find_tsserver() -> Option<PathBuf> {
    // which コマンドで検索
    let output = std::process::Command::new("which")
        .arg("typescript-language-server")
        .output()
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(PathBuf::from(path));
        }
    }

    warn!("typescript-language-server not found in PATH");
    None
}
