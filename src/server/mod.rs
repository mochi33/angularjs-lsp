mod progress;
pub mod workspace;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::analyzer::html::HtmlAngularJsAnalyzer;
use crate::analyzer::html::parser::HtmlParser;
use crate::analyzer::html::EmbeddedScript;
use crate::analyzer::js::AngularJsAnalyzer;
use crate::cache::{CacheLoader, CacheWriter};
use crate::config::{AjsConfig, DiagnosticsConfig, PathMatcher};
use crate::handler::{
    CodeLensHandler, CompletionHandler, DefinitionHandler, DiagnosticsHandler,
    DocumentSymbolHandler, HoverHandler, ReferencesHandler, RenameHandler,
    SemanticTokensHandler, SignatureHelpHandler, WorkspaceSymbolHandler,
};
use crate::index::Index;
use crate::ts_proxy::TsProxy;
use crate::util::{is_html_file, is_js_file};

use progress::{begin_progress, end_progress, report_progress};
use workspace::{collect_file_metadata, collect_files, find_tsconfig_root, get_service_prefix_at_cursor};

pub struct Backend {
    client: Client,
    analyzer: Arc<AngularJsAnalyzer>,
    html_analyzer: Arc<HtmlAngularJsAnalyzer>,
    index: Arc<Index>,
    root_uri: RwLock<Option<Url>>,
    ts_proxy: RwLock<Option<TsProxy>>,
    documents: Arc<DashMap<Url, String>>,
    ts_opened_files: DashMap<Url, bool>,
    path_matcher: RwLock<Option<PathMatcher>>,
    diagnostics_config: Arc<RwLock<DiagnosticsConfig>>,
    debounce_versions: Arc<DashMap<Url, u64>>,
}

async fn publish_html_diagnostics(
    client: &Client,
    index: &Arc<Index>,
    diagnostics_config: &Arc<RwLock<DiagnosticsConfig>>,
    uri: &Url,
) {
    let config = diagnostics_config.read().await.clone();
    let handler = DiagnosticsHandler::new(Arc::clone(index), config);
    let diagnostics = handler.diagnose_html(uri);
    client
        .publish_diagnostics(uri.clone(), diagnostics, None)
        .await;
}

async fn publish_js_diagnostics(
    client: &Client,
    index: &Arc<Index>,
    diagnostics_config: &Arc<RwLock<DiagnosticsConfig>>,
    uri: &Url,
) {
    let config = diagnostics_config.read().await.clone();
    let handler = DiagnosticsHandler::new(Arc::clone(index), config);
    let diagnostics = handler.diagnose_js(uri);
    client
        .publish_diagnostics(uri.clone(), diagnostics, None)
        .await;
}

async fn republish_all_js_diagnostics(
    client: &Client,
    index: &Arc<Index>,
    diagnostics_config: &Arc<RwLock<DiagnosticsConfig>>,
    documents: &Arc<DashMap<Url, String>>,
) {
    let js_uris: Vec<Url> = documents
        .iter()
        .filter(|entry| is_js_file(entry.key()))
        .map(|entry| entry.key().clone())
        .collect();

    for uri in js_uris {
        publish_js_diagnostics(client, index, diagnostics_config, &uri).await;
    }
}

impl Backend {
    pub fn new(client: Client) -> Self {
        let index = Arc::new(Index::new());
        let analyzer = Arc::new(AngularJsAnalyzer::new(Arc::clone(&index)));
        let html_analyzer = Arc::new(HtmlAngularJsAnalyzer::new(
            Arc::clone(&index),
            Arc::clone(&analyzer),
        ));

        Self {
            client,
            analyzer,
            html_analyzer,
            index,
            root_uri: RwLock::new(None),
            ts_proxy: RwLock::new(None),
            documents: Arc::new(DashMap::new()),
            ts_opened_files: DashMap::new(),
            path_matcher: RwLock::new(None),
            diagnostics_config: Arc::new(RwLock::new(DiagnosticsConfig::default())),
            debounce_versions: Arc::new(DashMap::new()),
        }
    }

    async fn publish_diagnostics_for_html(&self, uri: &Url) {
        publish_html_diagnostics(&self.client, &self.index, &self.diagnostics_config, uri).await;
    }

    async fn publish_diagnostics_for_js(&self, uri: &Url) {
        publish_js_diagnostics(&self.client, &self.index, &self.diagnostics_config, uri).await;
    }

    async fn republish_diagnostics_for_open_js_files(&self) {
        republish_all_js_diagnostics(
            &self.client,
            &self.index,
            &self.diagnostics_config,
            &self.documents,
        )
        .await;
    }

    async fn on_change(&self, uri: Url, text: String, version: i32) {
        self.documents.insert(uri.clone(), text.clone());

        if is_html_file(&uri) {
            // Increment version counter for debounce
            let ver = {
                let mut entry = self.debounce_versions.entry(uri.clone()).or_insert(0);
                *entry += 1;
                *entry
            };

            // Clone Arc handles (cheap reference count increment only)
            let client = self.client.clone();
            let analyzer = Arc::clone(&self.analyzer);
            let html_analyzer = Arc::clone(&self.html_analyzer);
            let index = Arc::clone(&self.index);
            let documents = Arc::clone(&self.documents);
            let diagnostics_config = Arc::clone(&self.diagnostics_config);
            let debounce_versions = Arc::clone(&self.debounce_versions);
            let spawn_uri = uri.clone();

            tokio::spawn(async move {
                let uri = spawn_uri;
                tokio::time::sleep(Duration::from_millis(200)).await;

                // Check version: skip if a newer keystroke has arrived
                if debounce_versions.get(&uri).map(|v| *v) != Some(ver) {
                    return;
                }

                // Clone Arc handles for spawn_blocking (outer scope keeps copies for diagnostics)
                let bl_uri = uri.clone();
                let bl_analyzer = Arc::clone(&analyzer);
                let bl_html_analyzer = Arc::clone(&html_analyzer);
                let bl_index = Arc::clone(&index);
                let bl_documents = Arc::clone(&documents);

                // Run CPU-intensive analysis on the blocking thread pool
                let analysis_done = tokio::task::spawn_blocking(move || {
                    let latest_text = match bl_documents.get(&bl_uri) {
                        Some(doc) => doc.value().clone(),
                        None => return false,
                    };

                    let scripts = bl_html_analyzer
                        .analyze_document_and_extract_scripts(&bl_uri, &latest_text);
                    bl_index.templates.mark_html_analyzed(&bl_uri);
                    for script in scripts {
                        bl_analyzer.analyze_embedded_script(
                            &bl_uri,
                            &script.source,
                            script.line_offset,
                        );
                    }

                    bl_index.remove_from_pending_reanalysis(&bl_uri);
                    let pending_uris = bl_index.take_pending_reanalysis();
                    for child_uri in pending_uris {
                        if child_uri == bl_uri {
                            continue;
                        }
                        if let Some(doc) = bl_documents.get(&child_uri) {
                            tracing::debug!(
                                "process_pending_reanalysis: reanalyzing {} (triggered by {})",
                                child_uri,
                                bl_uri
                            );
                            bl_html_analyzer.analyze_document(&child_uri, doc.value());
                        }
                    }
                    true
                })
                .await
                .unwrap_or(false);

                if analysis_done {
                    publish_html_diagnostics(&client, &index, &diagnostics_config, &uri).await;
                    republish_all_js_diagnostics(
                        &client,
                        &index,
                        &diagnostics_config,
                        &documents,
                    )
                    .await;
                    let _ = client.semantic_tokens_refresh().await;
                    let _ = client.code_lens_refresh().await;
                }
            });
        } else if is_js_file(&uri) {
            // Increment version counter for debounce
            let ver = {
                let mut entry = self.debounce_versions.entry(uri.clone()).or_insert(0);
                *entry += 1;
                *entry
            };

            let client = self.client.clone();
            let analyzer = Arc::clone(&self.analyzer);
            let index = Arc::clone(&self.index);
            let documents = Arc::clone(&self.documents);
            let diagnostics_config = Arc::clone(&self.diagnostics_config);
            let debounce_versions = Arc::clone(&self.debounce_versions);
            let spawn_uri = uri.clone();

            tokio::spawn(async move {
                let uri = spawn_uri;
                tokio::time::sleep(Duration::from_millis(200)).await;

                if debounce_versions.get(&uri).map(|v| *v) != Some(ver) {
                    return;
                }

                let bl_uri = uri.clone();
                let bl_analyzer = Arc::clone(&analyzer);
                let bl_documents = Arc::clone(&documents);

                let analysis_done = tokio::task::spawn_blocking(move || {
                    let latest_text = match bl_documents.get(&bl_uri) {
                        Some(doc) => doc.value().clone(),
                        None => return false,
                    };
                    bl_analyzer.analyze_document(&bl_uri, &latest_text);
                    true
                })
                .await
                .unwrap_or(false);

                if analysis_done {
                    publish_js_diagnostics(&client, &index, &diagnostics_config, &uri).await;
                    let _ = client.semantic_tokens_refresh().await;
                    let _ = client.code_lens_refresh().await;
                }
            });
        }

        if let Some(ref proxy) = *self.ts_proxy.read().await {
            proxy.did_change(&uri, &text, version).await;
        }
    }

    async fn on_open(&self, uri: Url, text: String) {
        self.documents.insert(uri.clone(), text.clone());

        if is_html_file(&uri) {
            self.debounce_versions.insert(uri.clone(), 0);

            let bl_uri = uri.clone();
            let bl_analyzer = Arc::clone(&self.analyzer);
            let bl_html_analyzer = Arc::clone(&self.html_analyzer);
            let bl_index = Arc::clone(&self.index);
            let bl_documents = Arc::clone(&self.documents);
            let bl_text = text.clone();
            tokio::task::spawn_blocking(move || {
                let scripts =
                    bl_html_analyzer.analyze_document_and_extract_scripts(&bl_uri, &bl_text);
                bl_index.templates.mark_html_analyzed(&bl_uri);
                for script in scripts {
                    bl_analyzer
                        .analyze_embedded_script(&bl_uri, &script.source, script.line_offset);
                }
                // process_pending_reanalysis inlined (&self cannot be sent to spawn_blocking)
                bl_index.remove_from_pending_reanalysis(&bl_uri);
                let pending = bl_index.take_pending_reanalysis();
                for child_uri in pending {
                    if child_uri == bl_uri {
                        continue;
                    }
                    if let Some(doc) = bl_documents.get(&child_uri) {
                        bl_html_analyzer.analyze_document(&child_uri, doc.value());
                    }
                }
            })
            .await
            .unwrap_or(());

            self.publish_diagnostics_for_html(&uri).await;
            self.republish_diagnostics_for_open_js_files().await;
        } else if is_js_file(&uri) {
            self.debounce_versions.insert(uri.clone(), 0);

            let bl_uri = uri.clone();
            let bl_analyzer = Arc::clone(&self.analyzer);
            let bl_text = text.clone();
            tokio::task::spawn_blocking(move || {
                bl_analyzer.analyze_document(&bl_uri, &bl_text);
            })
            .await
            .unwrap_or(());

            self.publish_diagnostics_for_js(&uri).await;
        }

        if let Some(ref proxy) = *self.ts_proxy.read().await {
            proxy.did_open(&uri, &text).await;
            self.ts_opened_files.insert(uri.clone(), true);
        }
    }

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

    fn process_pending_reanalysis(&self, current_uri: &Url) {
        self.index.remove_from_pending_reanalysis(current_uri);

        let pending_uris = self.index.take_pending_reanalysis();

        for child_uri in pending_uris {
            if &child_uri == current_uri {
                continue;
            }

            if let Some(doc) = self.documents.get(&child_uri) {
                tracing::debug!(
                    "process_pending_reanalysis: reanalyzing {} (triggered by {})",
                    child_uri,
                    current_uri
                );
                self.html_analyzer
                    .analyze_document(&child_uri, doc.value());
            }
        }
    }

    async fn scan_workspace(&self) {
        let root_uri = self.root_uri.read().await;
        let path_matcher = self.path_matcher.read().await;
        if let Some(ref uri) = *root_uri {
            if let Ok(path) = uri.to_file_path() {
                self.client
                    .log_message(MessageType::INFO, format!("Scanning workspace: {:?}", path))
                    .await;

                let token = begin_progress(
                    &self.client,
                    "angularjs-indexing",
                    "Indexing AngularJS",
                    Some("Collecting files...".to_string()),
                )
                .await;

                // Collect JS files
                let mut js_files: Vec<(Url, String)> = Vec::new();
                collect_files(&path, &path, path_matcher.as_ref(), &["js"], &mut js_files);
                let js_count = js_files.len();

                // Collect HTML files
                let mut html_files: Vec<(Url, String)> = Vec::new();
                collect_files(
                    &path,
                    &path,
                    path_matcher.as_ref(),
                    &["html", "htm"],
                    &mut html_files,
                );
                let html_count = html_files.len();

                // Extract embedded scripts from HTML
                let html_scripts: Vec<(Url, Vec<EmbeddedScript>)> = html_files
                    .iter()
                    .map(|(uri, content)| {
                        let scripts = HtmlAngularJsAnalyzer::extract_scripts(content);
                        (uri.clone(), scripts)
                    })
                    .filter(|(_, scripts)| !scripts.is_empty())
                    .collect();

                let html_script_count = html_scripts.len();

                // Pre-parse HTML files (HtmlParser is !Send, must be done on one thread)
                let mut parser = HtmlParser::new();
                let parsed_html_files: Vec<_> = html_files
                    .iter()
                    .filter_map(|(uri, content)| {
                        parser
                            .parse(content)
                            .map(|tree| (uri, content.as_str(), tree))
                    })
                    .collect();
                let parsed_count = parsed_html_files.len();

                // Phase 1: JS Pass 1 (definitions) ∥ HTML Pass 1 (ng-controller)
                report_progress(
                    &self.client,
                    &token,
                    "Phase 1: Indexing definitions + ng-controller".to_string(),
                    0,
                )
                .await;

                std::thread::scope(|s| {
                    s.spawn(|| {
                        // JS Pass 1: definitions
                        for (uri, content) in js_files.iter() {
                            self.analyzer
                                .analyze_document_with_options(uri, content, true);
                        }
                        for (uri, scripts) in html_scripts.iter() {
                            let mut first = true;
                            for script in scripts {
                                if first {
                                    self.index.clear_document(uri);
                                    first = false;
                                }
                                self.analyzer.analyze_embedded_script(
                                    uri,
                                    &script.source,
                                    script.line_offset,
                                );
                            }
                        }
                    });
                    s.spawn(|| {
                        // HTML Pass 1: ng-controller scopes
                        for (uri, content, tree) in &parsed_html_files {
                            self.html_analyzer
                                .collect_controller_scopes_only_with_tree(uri, content, tree);
                        }
                        // 全HTMLファイルを解析済みとして登録
                        for (uri, _content, _tree) in &parsed_html_files {
                            self.index.templates.mark_html_analyzed(uri);
                        }
                    });
                });

                report_progress(
                    &self.client,
                    &token,
                    "Phase 1 complete".to_string(),
                    40,
                )
                .await;

                // Phase 2: JS Pass 2 (references) ∥ HTML Pass 1.5 (ng-include)
                report_progress(
                    &self.client,
                    &token,
                    "Phase 2: Indexing references + ng-include".to_string(),
                    40,
                )
                .await;

                std::thread::scope(|s| {
                    s.spawn(|| {
                        // JS Pass 2: references
                        for (uri, content) in js_files.iter() {
                            self.analyzer
                                .analyze_document_with_options(uri, content, false);
                        }
                        for (uri, scripts) in html_scripts.iter() {
                            for script in scripts {
                                self.analyzer.analyze_embedded_script(
                                    uri,
                                    &script.source,
                                    script.line_offset,
                                );
                            }
                        }
                    });
                    s.spawn(|| {
                        // HTML Pass 1.5: ng-include bindings
                        for (uri, content, tree) in &parsed_html_files {
                            self.html_analyzer
                                .collect_ng_include_bindings_with_tree(uri, content, tree);
                        }
                    });
                });

                report_progress(
                    &self.client,
                    &token,
                    "Phase 2 complete".to_string(),
                    80,
                )
                .await;

                self.client
                    .log_message(
                        MessageType::INFO,
                        format!(
                            "Indexed {} JS files + {} HTML scripts",
                            js_count, html_script_count
                        ),
                    )
                    .await;

                // Phase 3: HTML Pass 1.6 (ng-view) + HTML Pass 2 (form bindings)
                report_progress(
                    &self.client,
                    &token,
                    format!("Phase 3: ng-view + form bindings (0/{} files)", parsed_count),
                    80,
                )
                .await;

                self.index.templates.apply_all_ng_view_inheritances();

                for (i, (uri, content, tree)) in parsed_html_files.iter().enumerate() {
                    self.html_analyzer
                        .collect_form_bindings_only_with_tree(uri, content, tree);
                    if i % 10 == 0 || i == parsed_count - 1 {
                        let pct = 80 + ((i + 1) * 10 / parsed_count.max(1)) as u32;
                        report_progress(
                            &self.client,
                            &token,
                            format!(
                                "Phase 3: ng-view + form bindings ({}/{} files)",
                                i + 1,
                                parsed_count
                            ),
                            pct,
                        )
                        .await;
                    }
                }

                // Phase 4: HTML Pass 3 (references)
                report_progress(
                    &self.client,
                    &token,
                    format!("Phase 4: HTML references (0/{} files)", parsed_count),
                    90,
                )
                .await;

                for (i, (uri, content, tree)) in parsed_html_files.iter().enumerate() {
                    self.html_analyzer
                        .analyze_document_references_only_with_tree(uri, content, tree);
                    if i % 10 == 0 || i == parsed_count - 1 {
                        let pct = 90 + ((i + 1) * 10 / parsed_count.max(1)) as u32;
                        report_progress(
                            &self.client,
                            &token,
                            format!(
                                "Phase 4: HTML references ({}/{} files)",
                                i + 1,
                                parsed_count
                            ),
                            pct,
                        )
                        .await;
                    }
                }

                self.client
                    .log_message(
                        MessageType::INFO,
                        format!("Indexed {} HTML files", html_count),
                    )
                    .await;

                self.republish_diagnostics_for_open_js_files().await;

                end_progress(
                    &self.client,
                    &token,
                    format!("Indexed {} JS + {} HTML files", js_count, html_count),
                )
                .await;
            }
        }
    }

    async fn scan_js_files_only(&self, files: &[PathBuf]) {
        for file_path in files {
            if let Ok(uri) = Url::from_file_path(file_path) {
                if let Ok(content) = fs::read_to_string(file_path) {
                    if is_js_file(&uri) {
                        self.analyzer.analyze_document(&uri, &content);
                    } else if is_html_file(&uri) {
                        let scripts = self
                            .html_analyzer
                            .analyze_document_and_extract_scripts(&uri, &content);
                        for script in scripts {
                            self.analyzer.analyze_embedded_script(
                                &uri,
                                &script.source,
                                script.line_offset,
                            );
                        }
                    }
                }
            }
        }
    }

    async fn scan_html_files_only(&self, files: &[PathBuf]) {
        let mut parser = HtmlParser::new();
        let mut html_files: Vec<(Url, String)> = Vec::new();

        for file_path in files {
            if let Ok(uri) = Url::from_file_path(file_path) {
                if is_html_file(&uri) {
                    if let Ok(content) = fs::read_to_string(file_path) {
                        html_files.push((uri, content));
                    }
                }
            }
        }

        if html_files.is_empty() {
            return;
        }

        let parsed_html_files: Vec<_> = html_files
            .iter()
            .filter_map(|(uri, content)| {
                parser
                    .parse(content)
                    .map(|tree| (uri, content.as_str(), tree))
            })
            .collect();

        // Pass 1 (ng-controller) ∥ Pass 1.5 (ng-include) — parallel
        std::thread::scope(|s| {
            s.spawn(|| {
                for (uri, content, tree) in &parsed_html_files {
                    self.html_analyzer
                        .collect_controller_scopes_only_with_tree(uri, content, tree);
                }
                // 全HTMLファイルを解析済みとして登録
                for (uri, _content, _tree) in &parsed_html_files {
                    self.index.templates.mark_html_analyzed(uri);
                }
            });
            s.spawn(|| {
                for (uri, content, tree) in &parsed_html_files {
                    self.html_analyzer
                        .collect_ng_include_bindings_with_tree(uri, content, tree);
                }
            });
        });

        // Pass 1.6
        self.index.templates.apply_all_ng_view_inheritances();

        // Pass 2
        for (uri, content, tree) in &parsed_html_files {
            self.html_analyzer
                .collect_form_bindings_only_with_tree(uri, content, tree);
        }

        // Pass 3
        for (uri, content, tree) in &parsed_html_files {
            self.html_analyzer
                .analyze_document_references_only_with_tree(uri, content, tree);
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let root = params
            .root_uri
            .or_else(|| {
                params
                    .workspace_folders
                    .as_ref()?
                    .first()
                    .map(|f| f.uri.clone())
            });

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
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec!["(".to_string(), ",".to_string()]),
                    retrigger_characters: None,
                    work_done_progress_options: Default::default(),
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: Default::default(),
                            legend: SemanticTokensHandler::legend(),
                            range: Some(false),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                        },
                    ),
                ),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec!["angularjs-lsp.refreshIndex".to_string()],
                    work_done_progress_options: Default::default(),
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(
                MessageType::INFO,
                "AngularJS Language Server initialized",
            )
            .await;

        // Load ajsconfig.json
        let root_uri = self.root_uri.read().await.clone();
        let mut cache_enabled = false;

        if let Some(ref uri) = root_uri {
            if let Ok(path) = uri.to_file_path() {
                let config = AjsConfig::load_from_dir(&path);
                cache_enabled = config.cache;

                self.html_analyzer
                    .set_interpolate_config(config.interpolate.clone());
                *self.diagnostics_config.write().await = config.diagnostics.clone();
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!(
                            "Interpolate symbols: {} ... {}",
                            config.interpolate.start_symbol, config.interpolate.end_symbol
                        ),
                    )
                    .await;

                if !config.include.is_empty() {
                    self.client
                        .log_message(
                            MessageType::INFO,
                            format!("Include patterns: {:?}", config.include),
                        )
                        .await;
                }
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!("Exclude patterns: {:?}", config.exclude),
                    )
                    .await;

                if cache_enabled {
                    self.client
                        .log_message(MessageType::INFO, "Cache enabled")
                        .await;
                }

                match config.create_path_matcher() {
                    Ok(matcher) => {
                        *self.path_matcher.write().await = Some(matcher);
                    }
                    Err(e) => {
                        self.client
                            .log_message(
                                MessageType::ERROR,
                                format!("Invalid path patterns: {}", e),
                            )
                            .await;
                    }
                }
            }
        }

        // Start typescript-language-server
        let ts_root_uri = find_tsconfig_root(&root_uri).or(root_uri.clone());

        if let Some(ref uri) = ts_root_uri {
            self.client
                .log_message(
                    MessageType::INFO,
                    format!("typescript-language-server root: {}", uri),
                )
                .await;
        }

        if let Some(proxy) = TsProxy::start(ts_root_uri.as_ref()).await {
            *self.ts_proxy.write().await = Some(proxy);
            self.client
                .log_message(
                    MessageType::INFO,
                    "typescript-language-server proxy started",
                )
                .await;
        } else {
            self.client
                .log_message(
                    MessageType::WARNING,
                    "typescript-language-server not found, fallback disabled",
                )
                .await;
        }

        // Cache handling
        if cache_enabled {
            if let Some(ref uri) = root_uri {
                if let Ok(root_path) = uri.to_file_path() {
                    let path_matcher = self.path_matcher.read().await;
                    let mut file_metadata = HashMap::new();
                    collect_file_metadata(
                        &root_path,
                        &root_path,
                        path_matcher.as_ref(),
                        &mut file_metadata,
                    );

                    let loader = CacheLoader::new(&root_path);
                    let files_for_validation: Vec<_> = file_metadata
                        .iter()
                        .map(|(p, m)| (p.clone(), m.mtime, m.size))
                        .collect();

                    match loader.validate(&files_for_validation) {
                        Ok(validation) => {
                            if !validation.valid_files.is_empty() {
                                let token = begin_progress(
                                    &self.client,
                                    "angularjs-lsp/cache",
                                    "Loading from cache",
                                    Some(format!(
                                        "Validating {} files...",
                                        validation.valid_files.len()
                                    )),
                                )
                                .await;

                                report_progress(
                                    &self.client,
                                    &token,
                                    "Loading cached data...".to_string(),
                                    20,
                                )
                                .await;

                                if let Err(e) =
                                    loader.load(&self.index, &validation.valid_files)
                                {
                                    end_progress(
                                        &self.client,
                                        &token,
                                        "Cache load failed, falling back to full scan"
                                            .to_string(),
                                    )
                                    .await;
                                    self.client
                                        .log_message(
                                            MessageType::WARNING,
                                            format!(
                                                "Cache load failed: {:?}, falling back to full scan",
                                                e
                                            ),
                                        )
                                        .await;
                                    drop(path_matcher);
                                    self.scan_workspace().await;
                                } else if !validation.invalid_files.is_empty() {
                                    let invalid_files: Vec<_> =
                                        validation.invalid_files.into_iter().collect();

                                    // Mark cached HTML files as analyzed
                                    let mut cached_html_count = 0;
                                    for path in &validation.valid_files {
                                        if path.extension().map_or(false, |e| {
                                            e == "html" || e == "htm"
                                        }) {
                                            cached_html_count += 1;
                                            if let Ok(uri) = Url::from_file_path(path) {
                                                self.index.mark_html_analyzed(&uri);
                                            }
                                        }
                                    }

                                    let definitions_count =
                                        self.index.definitions.get_all_definitions().len();
                                    report_progress(
                                        &self.client,
                                        &token,
                                        format!(
                                            "Loaded {} definitions, {} HTML; scanning {} changed...",
                                            definitions_count,
                                            cached_html_count,
                                            invalid_files.len()
                                        ),
                                        50,
                                    )
                                    .await;

                                    drop(path_matcher);
                                    self.scan_js_files_only(&invalid_files).await;

                                    report_progress(
                                        &self.client,
                                        &token,
                                        "Parsing changed HTML files...".to_string(),
                                        70,
                                    )
                                    .await;

                                    self.scan_html_files_only(&invalid_files).await;

                                    report_progress(
                                        &self.client,
                                        &token,
                                        "Saving cache...".to_string(),
                                        90,
                                    )
                                    .await;

                                    let writer = CacheWriter::new(&root_path);
                                    if let Err(e) = writer
                                        .save_full(&self.index, &file_metadata)
                                        .map_err(|e| e.to_string())
                                    {
                                        self.client
                                            .log_message(
                                                MessageType::WARNING,
                                                format!("Cache save failed: {}", e),
                                            )
                                            .await;
                                    }

                                    end_progress(
                                        &self.client,
                                        &token,
                                        format!(
                                            "Loaded {} definitions (scanned {} changed files)",
                                            definitions_count,
                                            invalid_files.len()
                                        ),
                                    )
                                    .await;
                                } else {
                                    // All files hit cache
                                    report_progress(
                                        &self.client,
                                        &token,
                                        "Restoring HTML data...".to_string(),
                                        80,
                                    )
                                    .await;

                                    let mut html_count = 0;
                                    for path in &validation.valid_files {
                                        if path.extension().map_or(false, |e| {
                                            e == "html" || e == "htm"
                                        }) {
                                            html_count += 1;
                                            if let Ok(uri) = Url::from_file_path(path) {
                                                self.index.mark_html_analyzed(&uri);
                                            }
                                        }
                                    }

                                    let definitions_count =
                                        self.index.definitions.get_all_definitions().len();

                                    end_progress(
                                        &self.client,
                                        &token,
                                        format!(
                                            "Loaded {} definitions, {} HTML files from cache",
                                            definitions_count, html_count
                                        ),
                                    )
                                    .await;

                                    drop(path_matcher);
                                }
                            } else {
                                drop(path_matcher);
                                self.scan_workspace().await;

                                let writer = CacheWriter::new(&root_path);
                                if let Err(e) = writer
                                    .save_full(&self.index, &file_metadata)
                                    .map_err(|e| e.to_string())
                                {
                                    self.client
                                        .log_message(
                                            MessageType::WARNING,
                                            format!("Cache save failed: {}", e),
                                        )
                                        .await;
                                } else {
                                    self.client
                                        .log_message(MessageType::INFO, "Cache saved")
                                        .await;
                                }
                            }
                        }
                        Err(e) => {
                            self.client
                                .log_message(
                                    MessageType::INFO,
                                    format!(
                                        "Cache not available: {:?}, performing full scan",
                                        e
                                    ),
                                )
                                .await;
                            drop(path_matcher);
                            self.scan_workspace().await;

                            let writer = CacheWriter::new(&root_path);
                            if let Err(e) = writer
                                .save_full(&self.index, &file_metadata)
                                .map_err(|e| e.to_string())
                            {
                                self.client
                                    .log_message(
                                        MessageType::WARNING,
                                        format!("Cache save failed: {}", e),
                                    )
                                    .await;
                            } else {
                                self.client
                                    .log_message(MessageType::INFO, "Cache saved")
                                    .await;
                            }
                        }
                    }
                } else {
                    self.scan_workspace().await;
                }
            } else {
                self.scan_workspace().await;
            }
        } else {
            self.scan_workspace().await;
        }
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> Result<Option<serde_json::Value>> {
        match params.command.as_str() {
            "angularjs-lsp.refreshIndex" => {
                self.client
                    .log_message(MessageType::INFO, "Refreshing AngularJS index...")
                    .await;

                self.index.clear_all();
                self.scan_workspace().await;

                // Save cache
                if let Some(ref uri) = *self.root_uri.read().await {
                    if let Ok(root_path) = uri.to_file_path() {
                        let config_path = root_path.join("ajsconfig.json");
                        let cache_enabled = if config_path.exists() {
                            fs::read_to_string(&config_path)
                                .ok()
                                .and_then(|s| serde_json::from_str::<AjsConfig>(&s).ok())
                                .map(|c| c.cache)
                                .unwrap_or(true)
                        } else {
                            true
                        };

                        if cache_enabled {
                            let path_matcher = self.path_matcher.read().await;
                            let mut file_metadata = HashMap::new();
                            collect_file_metadata(
                                &root_path,
                                &root_path,
                                path_matcher.as_ref(),
                                &mut file_metadata,
                            );

                            let cache_writer = CacheWriter::new(&root_path);
                            if let Err(e) = cache_writer
                                .save_full(&self.index, &file_metadata)
                                .map_err(|e| e.to_string())
                            {
                                self.client
                                    .log_message(
                                        MessageType::WARNING,
                                        format!("Failed to save cache: {}", e),
                                    )
                                    .await;
                            } else {
                                self.client
                                    .log_message(MessageType::INFO, "Cache saved")
                                    .await;
                            }
                        }
                    }
                }

                self.client
                    .log_message(MessageType::INFO, "AngularJS index refreshed")
                    .await;

                Ok(Some(serde_json::json!({ "success": true })))
            }
            _ => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("Unknown command: {}", params.command),
                    )
                    .await;
                Ok(None)
            }
        }
    }

    async fn shutdown(&self) -> Result<()> {
        // Save cache on shutdown
        if let Some(ref uri) = *self.root_uri.read().await {
            if let Ok(root_path) = uri.to_file_path() {
                let config_path = root_path.join("ajsconfig.json");
                let cache_enabled = if config_path.exists() {
                    fs::read_to_string(&config_path)
                        .ok()
                        .and_then(|s| serde_json::from_str::<AjsConfig>(&s).ok())
                        .map(|c| c.cache)
                        .unwrap_or(true)
                } else {
                    true
                };

                if cache_enabled {
                    let path_matcher = self.path_matcher.read().await;
                    let mut file_metadata = HashMap::new();
                    collect_file_metadata(
                        &root_path,
                        &root_path,
                        path_matcher.as_ref(),
                        &mut file_metadata,
                    );

                    let writer = CacheWriter::new(&root_path);
                    if let Err(e) = writer.save_full(&self.index, &file_metadata) {
                        tracing::warn!("Failed to save cache on shutdown: {}", e);
                    } else {
                        tracing::info!("Cache saved on shutdown");
                    }
                }
            }
        }

        // Shutdown ts_proxy
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
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            proxy.did_close(&params.text_document.uri).await;
        }
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;

        let handler = ReferencesHandler::new(Arc::clone(&self.index));
        if let Some(refs) = handler.find_references(params.clone()) {
            return Ok(Some(refs));
        }

        self.ensure_ts_file_opened(uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.references(&params).await);
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = &params.text_document_position_params.position;

        let handler = DefinitionHandler::new(Arc::clone(&self.index));
        let source = self.documents.get(uri).map(|s| s.value().clone());
        if let Some(def) = handler.goto_definition_with_source(params.clone(), source.as_deref())
        {
            self.client
                .log_message(
                    MessageType::INFO,
                    format!(
                        "AngularJS definition found at {}:{}:{}",
                        uri, pos.line, pos.character
                    ),
                )
                .await;
            return Ok(Some(def));
        }

        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "AngularJS definition NOT found at {}:{}:{}, falling back to tsserver",
                    uri, pos.line, pos.character
                ),
            )
            .await;

        self.ensure_ts_file_opened(uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.goto_definition(&params).await);
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;

        let handler = HoverHandler::new(Arc::clone(&self.index));
        if let Some(hover) = handler.hover(params.clone()) {
            return Ok(Some(hover));
        }

        self.ensure_ts_file_opened(uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.hover(&params).await);
        }

        Ok(None)
    }

    async fn signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> Result<Option<SignatureHelp>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = &params.text_document_position_params.position;

        let source = match self.documents.get(uri) {
            Some(doc) => doc.value().clone(),
            None => return Ok(None),
        };

        let handler = SignatureHelpHandler::new(Arc::clone(&self.index));
        if let Some(sig_help) =
            handler.signature_help(uri, position.line, position.character, &source)
        {
            return Ok(Some(sig_help));
        }

        self.ensure_ts_file_opened(uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.signature_help(&params).await);
        }

        Ok(None)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;

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

        // HTML file completion
        if is_html_file(uri) {
            if let Some(doc) = self.documents.get(uri) {
                let source = doc.value();

                // Directive completion context
                if let Some((prefix, is_tag_name)) =
                    self.html_analyzer
                        .get_directive_completion_context(source, line, col)
                {
                    let handler = CompletionHandler::new(Arc::clone(&self.index));
                    if let Some(completions) =
                        handler.complete_directives(&prefix, is_tag_name)
                    {
                        return Ok(Some(completions));
                    }
                }

                // Angular context completion
                if self.html_analyzer.is_in_angular_context(source, line, col) {
                    let controllers =
                        self.index.resolve_controllers_for_html(uri, line);
                    let handler = CompletionHandler::new(Arc::clone(&self.index));

                    let mut items: Vec<CompletionItem> = Vec::new();
                    let mut seen_labels: std::collections::HashSet<String> =
                        std::collections::HashSet::new();

                    // $scope completions
                    if controllers.is_empty() {
                        if let Some(CompletionResponse::Array(scope_items)) =
                            handler.complete_with_context(Some("$scope"), None, &[])
                        {
                            for item in scope_items {
                                if seen_labels.insert(item.label.clone()) {
                                    items.push(item);
                                }
                            }
                        }
                    } else {
                        for controller_name in &controllers {
                            if let Some(CompletionResponse::Array(scope_items)) =
                                handler.complete_with_context(
                                    Some("$scope"),
                                    Some(controller_name),
                                    &[],
                                )
                            {
                                for item in scope_items {
                                    if seen_labels.insert(item.label.clone()) {
                                        items.push(item);
                                    }
                                }
                            }
                        }
                    }

                    // Local variables
                    let local_vars = self.index.html.get_local_variables_at(uri, line);
                    for var in local_vars {
                        items.push(CompletionItem {
                            label: var.name.clone(),
                            kind: Some(CompletionItemKind::VARIABLE),
                            detail: Some(format!(
                                "local variable ({})",
                                var.source.as_str()
                            )),
                            ..Default::default()
                        });
                    }

                    // Inherited local variables
                    let inherited_vars = self
                        .index
                        .templates
                        .get_inherited_local_variables_for_template(uri);
                    for var in inherited_vars {
                        if items.iter().any(|item| item.label == var.name) {
                            continue;
                        }
                        items.push(CompletionItem {
                            label: var.name.clone(),
                            kind: Some(CompletionItemKind::VARIABLE),
                            detail: Some(format!(
                                "inherited variable ({})",
                                var.source.as_str()
                            )),
                            ..Default::default()
                        });
                    }

                    // Form bindings
                    let form_bindings = self.index.html.get_form_bindings_at(uri, line);
                    for binding in form_bindings {
                        if items.iter().any(|item| item.label == binding.name) {
                            continue;
                        }
                        items.push(CompletionItem {
                            label: binding.name.clone(),
                            kind: Some(CompletionItemKind::VARIABLE),
                            detail: Some("form binding ($scope)".to_string()),
                            ..Default::default()
                        });
                    }

                    // Inherited form bindings
                    let inherited_forms = self
                        .index
                        .templates
                        .get_inherited_form_bindings_for_template(uri);
                    for binding in inherited_forms {
                        if items.iter().any(|item| item.label == binding.name) {
                            continue;
                        }
                        items.push(CompletionItem {
                            label: binding.name.clone(),
                            kind: Some(CompletionItemKind::VARIABLE),
                            detail: Some("inherited form binding ($scope)".to_string()),
                            ..Default::default()
                        });
                    }

                    // Controller-as aliases
                    let alias_mappings =
                        self.index.controllers.get_html_alias_mappings(uri, line);
                    for (alias, controller_name) in alias_mappings {
                        if items.iter().any(|item| item.label == alias) {
                            continue;
                        }
                        items.push(CompletionItem {
                            label: alias,
                            kind: Some(CompletionItemKind::VARIABLE),
                            detail: Some(format!(
                                "controller alias ({})",
                                controller_name
                            )),
                            ..Default::default()
                        });
                    }

                    if !items.is_empty() {
                        return Ok(Some(CompletionResponse::Array(items)));
                    }
                }
            }
            return Ok(None);
        }

        // JS file completion
        let service_prefix = self
            .documents
            .get(uri)
            .and_then(|doc| get_service_prefix_at_cursor(doc.value(), line, col));

        // Non-AngularJS object pattern -> fallback to TypeScript
        if let Some(ref prefix) = service_prefix {
            if prefix != "$scope" && !self.index.definitions.is_service_or_factory(prefix) {
                self.ensure_ts_file_opened(uri).await;
                if let Some(ref proxy) = *self.ts_proxy.read().await {
                    return Ok(proxy.completion(&params).await);
                }
                return Ok(None);
            }
        }

        let controller_name = self.index.controllers.get_controller_at(uri, line);
        let injected_services = self.index.controllers.get_injected_services_at(uri, line);

        let handler = CompletionHandler::new(Arc::clone(&self.index));
        if let Some(completions) = handler.complete_with_context(
            service_prefix.as_deref(),
            controller_name.as_deref(),
            &injected_services,
        ) {
            return Ok(Some(completions));
        }

        self.ensure_ts_file_opened(uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.completion(&params).await);
        }

        Ok(None)
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = &params.text_document_position.text_document.uri;

        let handler = RenameHandler::new(Arc::clone(&self.index));
        if let Some(edit) = handler.rename(params.clone()) {
            return Ok(Some(edit));
        }

        self.ensure_ts_file_opened(uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.rename(&params).await);
        }

        Ok(None)
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
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

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        let handler = SemanticTokensHandler::new(Arc::clone(&self.index));
        if let Some(tokens) = handler.semantic_tokens_full(uri) {
            return Ok(Some(SemanticTokensResult::Tokens(tokens)));
        }
        Ok(None)
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let handler = WorkspaceSymbolHandler::new(Arc::clone(&self.index));
        let symbols = handler.handle(&params.query);
        if symbols.is_empty() {
            return Ok(None);
        }
        Ok(Some(symbols))
    }
}
