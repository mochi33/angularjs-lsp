use std::fs;
use std::path::Path;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::analyzer::{AngularJsAnalyzer, HtmlAngularJsAnalyzer};
use crate::config::AjsConfig;
use crate::handlers::{CodeLensHandler, CompletionHandler, DocumentSymbolHandler, HoverHandler, ReferencesHandler, RenameHandler};
use crate::index::SymbolIndex;
use crate::ts_proxy::TsProxy;

pub struct Backend {
    client: Client,
    analyzer: Arc<AngularJsAnalyzer>,
    html_analyzer: Arc<HtmlAngularJsAnalyzer>,
    index: Arc<SymbolIndex>,
    root_uri: RwLock<Option<Url>>,
    ts_proxy: RwLock<Option<TsProxy>>,
    documents: DashMap<Url, String>,
    /// tsserverに開かれたファイルを追跡
    ts_opened_files: DashMap<Url, bool>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        let index = Arc::new(SymbolIndex::new());
        let analyzer = Arc::new(AngularJsAnalyzer::new(Arc::clone(&index)));
        let html_analyzer = Arc::new(HtmlAngularJsAnalyzer::new(Arc::clone(&index), Arc::clone(&analyzer)));

        Self {
            client,
            analyzer,
            html_analyzer,
            index,
            root_uri: RwLock::new(None),
            ts_proxy: RwLock::new(None),
            documents: DashMap::new(),
            ts_opened_files: DashMap::new(),
        }
    }

    /// ファイルがHTMLかどうか判定
    fn is_html_file(uri: &Url) -> bool {
        let path = uri.path().to_lowercase();
        path.ends_with(".html") || path.ends_with(".htm")
    }

    /// ファイルがJSかどうか判定
    fn is_js_file(uri: &Url) -> bool {
        uri.path().ends_with(".js")
    }

    async fn on_change(&self, uri: Url, text: String, version: i32) {
        self.documents.insert(uri.clone(), text.clone());

        // ファイルタイプに応じた解析
        if Self::is_html_file(&uri) {
            self.html_analyzer.analyze_document(&uri, &text);
        } else if Self::is_js_file(&uri) {
            self.analyzer.analyze_document(&uri, &text);
        }

        // ts_proxyにも変更を通知
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            proxy.did_change(&uri, &text, version).await;
        }
    }

    async fn on_open(&self, uri: Url, text: String) {
        self.documents.insert(uri.clone(), text.clone());

        // ファイルタイプに応じた解析
        if Self::is_html_file(&uri) {
            self.html_analyzer.analyze_document(&uri, &text);
        } else if Self::is_js_file(&uri) {
            self.analyzer.analyze_document(&uri, &text);
        }

        // ts_proxyにも通知
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            proxy.did_open(&uri, &text).await;
            self.ts_opened_files.insert(uri, true);
        }
        // ts_proxyがまだNoneの場合は、後で必要時に開く
    }

    /// tsserverにファイルが開かれていることを確認し、開かれていなければ開く
    async fn ensure_ts_file_opened(&self, uri: &Url) {
        if self.ts_opened_files.contains_key(uri) {
            return;
        }

        if let Some(ref proxy) = *self.ts_proxy.read().await {
            if let Some(doc) = self.documents.get(uri) {
                proxy.did_open(uri, doc.value()).await;
                self.ts_opened_files.insert(uri.clone(), true);
            }
        }
    }

    async fn scan_workspace(&self) {
        let root_uri = self.root_uri.read().await;
        if let Some(ref uri) = *root_uri {
            if let Ok(path) = uri.to_file_path() {
                self.client
                    .log_message(MessageType::INFO, format!("Scanning workspace: {:?}", path))
                    .await;

                // 進捗トークンを作成
                let token = NumberOrString::String("angularjs-indexing".to_string());
                let _ = self.client.send_request::<request::WorkDoneProgressCreate>(
                    WorkDoneProgressCreateParams { token: token.clone() }
                ).await;

                // 進捗開始を通知
                self.send_progress(&token, WorkDoneProgress::Begin(WorkDoneProgressBegin {
                    title: "Indexing AngularJS".to_string(),
                    cancellable: Some(false),
                    message: Some("Collecting files...".to_string()),
                    percentage: Some(0),
                })).await;

                // Collect all JS files
                let mut js_files: Vec<(Url, String)> = Vec::new();
                self.collect_js_files(&path, &mut js_files);

                let js_count = js_files.len();

                // Collect HTML files
                let mut html_files: Vec<(Url, String)> = Vec::new();
                self.collect_html_files(&path, &mut html_files);

                let html_count = html_files.len();

                // Pass 1: Index definitions
                self.send_progress(&token, WorkDoneProgress::Report(WorkDoneProgressReport {
                    cancellable: Some(false),
                    message: Some(format!("Pass 1: Indexing definitions (0/{} files)", js_count)),
                    percentage: Some(0),
                })).await;

                for (i, (uri, content)) in js_files.iter().enumerate() {
                    self.analyzer.analyze_document_with_options(uri, content, true);

                    // 10ファイルごとに進捗更新（パフォーマンスのため）
                    if i % 10 == 0 || i == js_count - 1 {
                        let pct = ((i + 1) * 40 / js_count.max(1)) as u32; // 0-40%
                        self.send_progress(&token, WorkDoneProgress::Report(WorkDoneProgressReport {
                            cancellable: Some(false),
                            message: Some(format!("Pass 1: Indexing definitions ({}/{} files)", i + 1, js_count)),
                            percentage: Some(pct),
                        })).await;
                    }
                }

                // Pass 2: Index references (now that all definitions are known)
                self.send_progress(&token, WorkDoneProgress::Report(WorkDoneProgressReport {
                    cancellable: Some(false),
                    message: Some(format!("Pass 2: Indexing references (0/{} files)", js_count)),
                    percentage: Some(40),
                })).await;

                for (i, (uri, content)) in js_files.iter().enumerate() {
                    self.analyzer.analyze_document_with_options(uri, content, false);

                    if i % 10 == 0 || i == js_count - 1 {
                        let pct = 40 + ((i + 1) * 40 / js_count.max(1)) as u32; // 40-80%
                        self.send_progress(&token, WorkDoneProgress::Report(WorkDoneProgressReport {
                            cancellable: Some(false),
                            message: Some(format!("Pass 2: Indexing references ({}/{} files)", i + 1, js_count)),
                            percentage: Some(pct),
                        })).await;
                    }
                }

                self.client
                    .log_message(MessageType::INFO, format!("Indexed {} JavaScript files", js_count))
                    .await;

                // Index HTML files
                self.send_progress(&token, WorkDoneProgress::Report(WorkDoneProgressReport {
                    cancellable: Some(false),
                    message: Some(format!("Indexing HTML (0/{} files)", html_count)),
                    percentage: Some(80),
                })).await;

                for (i, (uri, content)) in html_files.iter().enumerate() {
                    self.html_analyzer.analyze_document(uri, content);

                    if i % 10 == 0 || i == html_count - 1 {
                        let pct = 80 + ((i + 1) * 20 / html_count.max(1)) as u32; // 80-100%
                        self.send_progress(&token, WorkDoneProgress::Report(WorkDoneProgressReport {
                            cancellable: Some(false),
                            message: Some(format!("Indexing HTML ({}/{} files)", i + 1, html_count)),
                            percentage: Some(pct),
                        })).await;
                    }
                }

                self.client
                    .log_message(MessageType::INFO, format!("Indexed {} HTML files", html_count))
                    .await;

                // 進捗完了を通知
                self.send_progress(&token, WorkDoneProgress::End(WorkDoneProgressEnd {
                    message: Some(format!("Indexed {} JS + {} HTML files", js_count, html_count)),
                })).await;
            }
        }
    }

    /// 進捗通知を送信するヘルパー
    async fn send_progress(&self, token: &NumberOrString, value: WorkDoneProgress) {
        let params = ProgressParams {
            token: token.clone(),
            value: ProgressParamsValue::WorkDone(value),
        };
        // JSON-RPC通知として送信
        self.client.send_notification::<notification::Progress>(params).await;
    }

    fn collect_js_files(&self, dir: &Path, files: &mut Vec<(Url, String)>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Skip node_modules and hidden directories
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') || name == "node_modules" || name == "dist" || name == "build" {
                        continue;
                    }
                }

                if path.is_dir() {
                    self.collect_js_files(&path, files);
                } else if path.extension().map_or(false, |ext| ext == "js") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(uri) = Url::from_file_path(&path) {
                            files.push((uri, content));
                        }
                    }
                }
            }
        }
    }

    fn collect_html_files(&self, dir: &Path, files: &mut Vec<(Url, String)>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                // Skip node_modules and hidden directories
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') || name == "node_modules" || name == "dist" || name == "build" {
                        continue;
                    }
                }

                if path.is_dir() {
                    self.collect_html_files(&path, files);
                } else if path.extension().map_or(false, |ext| ext == "html" || ext == "htm") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(uri) = Url::from_file_path(&path) {
                            files.push((uri, content));
                        }
                    }
                }
            }
        }
    }

    fn get_service_prefix_at_cursor(&self, params: &CompletionParams) -> Option<String> {
        let uri = &params.text_document_position.text_document.uri;
        let position = &params.text_document_position.position;

        let doc = self.documents.get(uri)?;
        let text = doc.value();

        // カーソル位置までのテキストを取得
        let lines: Vec<&str> = text.lines().collect();
        if position.line as usize >= lines.len() {
            return None;
        }

        let line = lines[position.line as usize];
        let col = position.character as usize;
        if col > line.len() {
            return None;
        }

        let before_cursor = &line[..col];

        // "ServiceName." パターンを検出
        // 末尾が "." で終わっている場合、その前の識別子を取得
        if before_cursor.ends_with('.') {
            let without_dot = &before_cursor[..before_cursor.len() - 1];
            // 識別子を逆方向に抽出
            let service_name: String = without_dot
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                .collect::<String>()
                .chars()
                .rev()
                .collect();

            if !service_name.is_empty() {
                return Some(service_name);
            }
        }

        None
    }

    /// tsconfig.json を探して、そのディレクトリのURIを返す
    fn find_tsconfig_root(&self, root_uri: &Option<Url>) -> Option<Url> {
        let root_uri = root_uri.as_ref()?;
        let root_path = root_uri.to_file_path().ok()?;

        // 再帰的にtsconfig.jsonを探す
        self.find_tsconfig_in_dir(&root_path)
    }

    fn find_tsconfig_in_dir(&self, dir: &Path) -> Option<Url> {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();

                // tsconfig.json が見つかったらそのディレクトリを返す
                if path.is_file() && path.file_name().map_or(false, |n| n == "tsconfig.json") {
                    return Url::from_file_path(dir).ok();
                }

                // サブディレクトリを探索（node_modules等はスキップ）
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with('.') || name == "node_modules" || name == "dist" || name == "build" {
                            continue;
                        }
                    }
                    if let Some(found) = self.find_tsconfig_in_dir(&path) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // Store root URI
        let root = params
            .root_uri
            .or_else(|| params.workspace_folders.as_ref()?.first().map(|f| f.uri.clone()));

        *self.root_uri.write().await = root;

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "angularjs-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                references_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "AngularJS Language Server initialized")
            .await;

        // ajsconfig.json を読み込む
        let root_uri = self.root_uri.read().await.clone();
        if let Some(ref uri) = root_uri {
            if let Ok(path) = uri.to_file_path() {
                let config = AjsConfig::load_from_dir(&path);
                self.html_analyzer.set_interpolate_config(config.interpolate.clone());
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!(
                            "Interpolate symbols: {} ... {}",
                            config.interpolate.start_symbol,
                            config.interpolate.end_symbol
                        ),
                    )
                    .await;
            }
        }

        // typescript-language-server を起動
        // tsconfig.json を探して、そのディレクトリをルートにする
        let ts_root_uri = self.find_tsconfig_root(&root_uri).or(root_uri.clone());

        if let Some(ref uri) = ts_root_uri {
            self.client
                .log_message(MessageType::INFO, format!("typescript-language-server root: {}", uri))
                .await;
        }

        if let Some(proxy) = TsProxy::start(ts_root_uri.as_ref()).await {
            *self.ts_proxy.write().await = Some(proxy);
            self.client
                .log_message(MessageType::INFO, "typescript-language-server proxy started")
                .await;
        } else {
            self.client
                .log_message(MessageType::WARNING, "typescript-language-server not found, fallback disabled")
                .await;
        }

        // Scan workspace for JS files
        self.scan_workspace().await;
    }

    async fn shutdown(&self) -> Result<()> {
        // ts_proxyをシャットダウン
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            proxy.shutdown().await;
        }
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.on_open(uri, text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        if let Some(change) = params.content_changes.into_iter().next() {
            self.on_change(uri, change.text, version).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if let Some(text) = params.text {
            self.on_change(params.text_document.uri, text, 0).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        // Don't remove from index - keep for cross-file references

        // ts_proxyにも通知
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            proxy.did_close(&params.text_document.uri).await;
        }
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;

        // 1. AngularJS解析を試行
        let handler = ReferencesHandler::new(Arc::clone(&self.index));
        if let Some(refs) = handler.find_references(params.clone()) {
            return Ok(Some(refs));
        }

        // 2. フォールバック: typescript-language-server
        // ファイルがtsserverに開かれていることを確認
        self.ensure_ts_file_opened(uri).await;

        if let Some(ref proxy) = *self.ts_proxy.read().await {
            let result = proxy.references(&params).await;
            return Ok(result);
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = &params.text_document_position_params.position;

        // 1. AngularJS解析を試行
        let handler = ReferencesHandler::new(Arc::clone(&self.index));
        if let Some(def) = handler.goto_definition(params.clone()) {
            self.client
                .log_message(
                    MessageType::INFO,
                    format!("AngularJS definition found at {}:{}:{}",
                            uri, pos.line, pos.character),
                )
                .await;
            return Ok(Some(def));
        }

        self.client
            .log_message(
                MessageType::INFO,
                format!("AngularJS definition NOT found at {}:{}:{}, falling back to tsserver",
                        uri, pos.line, pos.character),
            )
            .await;

        // 2. フォールバック: typescript-language-server
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.goto_definition(&params).await);
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        // 1. AngularJS解析を試行
        let handler = HoverHandler::new(Arc::clone(&self.index));
        if let Some(hover) = handler.hover(params.clone()) {
            return Ok(Some(hover));
        }

        // 2. フォールバック: typescript-language-server
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.hover(&params).await);
        }

        Ok(None)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;

        // AngularJS解析
        let handler = DocumentSymbolHandler::new(Arc::clone(&self.index));
        if let Some(symbols) = handler.document_symbols(uri) {
            return Ok(Some(symbols));
        }

        Ok(None)
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let line = params.text_document_position.position.line;
        let col = params.text_document_position.position.character;

        // HTMLファイルの場合、Angularコンテキスト内かチェック
        if Self::is_html_file(uri) {
            if let Some(doc) = self.documents.get(uri) {
                let source = doc.value();
                if self.html_analyzer.is_in_angular_context(source, line, col) {
                    // Angularコンテキスト内 → $scope補完を返す
                    let controller_name = self.index.resolve_controller_for_html(uri, line);
                    let handler = CompletionHandler::new(Arc::clone(&self.index));
                    if let Some(completions) = handler.complete_with_context(Some("$scope"), controller_name.as_deref()) {
                        return Ok(Some(completions));
                    }
                }
            }
            // HTMLファイルでAngularコンテキスト外の場合は補完なし
            return Ok(None);
        }

        // JSファイルの場合
        // カーソル前のサービス名を取得（"ServiceName." パターン）
        let service_prefix = self.get_service_prefix_at_cursor(&params);

        // $scope. の場合、カーソル位置のコントローラー名を取得
        let controller_name = if service_prefix.as_deref() == Some("$scope") {
            self.index.get_controller_at(uri, line)
        } else {
            None
        };

        // 1. AngularJS解析を試行
        let handler = CompletionHandler::new(Arc::clone(&self.index));
        if let Some(completions) = handler.complete_with_context(service_prefix.as_deref(), controller_name.as_deref()) {
            return Ok(Some(completions));
        }

        // 2. フォールバック: typescript-language-server
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.completion(&params).await);
        }

        Ok(None)
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = &params.text_document_position.text_document.uri;

        // 1. AngularJS解析を試行
        let handler = RenameHandler::new(Arc::clone(&self.index));
        if let Some(edit) = handler.rename(params.clone()) {
            return Ok(Some(edit));
        }

        // 2. フォールバック: typescript-language-server
        self.ensure_ts_file_opened(uri).await;

        if let Some(ref proxy) = *self.ts_proxy.read().await {
            let result = proxy.rename(&params).await;
            return Ok(result);
        }

        Ok(None)
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        // AngularJS解析を試行
        let handler = RenameHandler::new(Arc::clone(&self.index));
        if let Some(response) = handler.prepare_rename(params) {
            return Ok(Some(response));
        }

        Ok(None)
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let uri = &params.text_document.uri;
        let handler = CodeLensHandler::new(Arc::clone(&self.index));
        Ok(handler.code_lens(uri))
    }
}
