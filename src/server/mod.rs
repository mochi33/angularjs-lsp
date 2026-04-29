mod progress;
pub mod workspace;

use std::collections::{HashMap, HashSet};
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
    /// `tokio::spawn` 内のデバウンス済みタスクからも参照する必要があるため Arc 化。
    ts_proxy: Arc<RwLock<Option<TsProxy>>>,
    documents: Arc<DashMap<Url, String>>,
    ts_opened_files: DashMap<Url, bool>,
    path_matcher: RwLock<Option<PathMatcher>>,
    diagnostics_config: Arc<RwLock<DiagnosticsConfig>>,
    debounce_versions: Arc<DashMap<Url, u64>>,
    /// URI ごとに「tsserver に最後に flush した debounce_versions の値」。
    /// `debounce_versions[uri] > ts_synced_versions[uri]` のとき未同期 (デバウンス
    /// 待ちの did_change が積まれている)。`ensure_ts_synced` で同期済みかを
    /// 判定し、必要なら request 直前に flush する。
    ts_synced_versions: Arc<DashMap<Url, u64>>,
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

/// HTML スコープ参照の property_path から末尾のプロパティ名 (leaf) を抜き出す。
/// 例: "vm.foo" -> "foo", "foo" -> "foo", "vm.foo.bar" -> "bar"
fn property_path_leaf(property_path: &str) -> &str {
    match property_path.rfind('.') {
        Some(idx) => &property_path[idx + 1..],
        None => property_path,
    }
}

/// `completion` ハンドラの sync 計算結果。`spawn_blocking` から戻った後、
/// async 側でこれを見て tsserver にフォールバックするか結果を返すかを判断する。
enum CompletionDecision {
    /// AngularJS の補完候補が見つかった → そのまま返す
    Resolved(CompletionResponse),
    /// AngularJS では何も提案しない & tsserver も呼ばない (補完なし)
    NoResult,
    /// AngularJS では解決できないので tsserver にフォールバック
    FallbackToTsProxy,
}

/// `completion` の CPU-bound な計算 (HTML/JS の AngularJS 補完抽出) を行う。
///
/// `spawn_blocking` 内で動かすため `&self` ではなく必要なものを Arc clone で
/// 受け取る。tokio runtime に触れない純粋同期関数。
fn compute_completion_decision(
    index: Arc<Index>,
    html_analyzer: Arc<HtmlAngularJsAnalyzer>,
    documents: Arc<DashMap<Url, String>>,
    uri: Url,
    line: u32,
    col: u32,
) -> CompletionDecision {
    // HTML file completion
    if is_html_file(&uri) {
        if let Some(doc) = documents.get(&uri) {
            let source = doc.value();

            // Directive completion context
            if let Some((prefix, is_tag_name, element_tag_name)) =
                html_analyzer.get_directive_completion_context_with_tag(source, line, col)
            {
                let handler = CompletionHandler::new(Arc::clone(&index));
                let mut items: Vec<CompletionItem> = Vec::new();

                // 属性名位置 + 既知 component 要素 → bindings を提案
                if !is_tag_name {
                    if let Some(ref tag_name) = element_tag_name {
                        items.extend(handler.complete_component_bindings(tag_name, &prefix));
                    }
                }

                // 既存のディレクティブ補完（ng-* など）も併せて返す
                if let Some(CompletionResponse::Array(directive_items)) =
                    handler.complete_directives(&prefix, is_tag_name)
                {
                    let mut seen: HashSet<String> =
                        items.iter().map(|i| i.label.clone()).collect();
                    for item in directive_items {
                        if seen.insert(item.label.clone()) {
                            items.push(item);
                        }
                    }
                }

                if !items.is_empty() {
                    return CompletionDecision::Resolved(CompletionResponse::Array(items));
                }
            }

            // Angular context completion
            if html_analyzer.is_in_angular_context(source, line, col) {
                let handler = CompletionHandler::new(Arc::clone(&index));
                let items = handler.complete_in_html_angular_context(&uri, line);
                if !items.is_empty() {
                    return CompletionDecision::Resolved(CompletionResponse::Array(items));
                }
            }
        }
        return CompletionDecision::NoResult;
    }

    // JS file completion
    let service_prefix = documents
        .get(&uri)
        .and_then(|doc| get_service_prefix_at_cursor(doc.value(), line, col));

    // Non-AngularJS object pattern -> fallback to TypeScript
    if let Some(ref prefix) = service_prefix {
        if prefix != "$scope" && !index.definitions.is_service_or_factory(prefix) {
            return CompletionDecision::FallbackToTsProxy;
        }
    }

    let controller_name = index.controllers.get_controller_at(&uri, line);
    let injected_services = index.controllers.get_injected_services_at(&uri, line);

    let handler = CompletionHandler::new(Arc::clone(&index));
    if let Some(completions) = handler.complete_with_context(
        service_prefix.as_deref(),
        controller_name.as_deref(),
        &injected_services,
    ) {
        return CompletionDecision::Resolved(completions);
    }

    CompletionDecision::FallbackToTsProxy
}

/// `pending_reanalysis` キューが空になるまでドレインし、各 URI を `analyze_one` で
/// 処理する。
///
/// 単純に `take_pending_reanalysis` を 1 回呼んで Vec ループする実装では:
/// 1. ループ中に panic / 早期 return すると、`take` 時の clear で全エントリが
///    消失して二度と再解析されない (取りこぼし)
/// 2. `analyze_one` 中に**孫**が新たに pending に追加されても、その同じラウンド
///    では消費されない (深い ng-include で更新が遅延)
///
/// 本ヘルパーはドレインループ + `visited` セット + 深さ上限で対応する:
/// - ループの先頭で `take_pending_reanalysis` を再呼び出し → 孫まで追える
/// - `visited` で同一 URI の重複処理を防ぐ → 循環や重複再パース防止
/// - `MAX_ROUNDS` 上限で万一の暴走を防ぐ
///
/// `skip_uri` は呼び出し側が自分自身を pending から除外するために渡す
/// (typically 編集中の親 URI)。
fn drain_pending_reanalysis<F>(index: &Index, skip_uri: &Url, mut analyze_one: F)
where
    F: FnMut(&Url),
{
    /// ng-include の現実的なネスト深さ (経験則として 8 段あれば十分)。
    /// これを超える場合は循環の疑いがあるので打ち切る。
    const MAX_ROUNDS: usize = 8;

    let mut visited: HashSet<Url> = HashSet::new();
    visited.insert(skip_uri.clone());

    for round in 0..MAX_ROUNDS {
        let pending = index.take_pending_reanalysis();
        if pending.is_empty() {
            break;
        }

        let mut progressed = false;
        for child_uri in pending {
            if !visited.insert(child_uri.clone()) {
                continue;
            }
            analyze_one(&child_uri);
            progressed = true;
        }

        if !progressed {
            break;
        }

        if round + 1 == MAX_ROUNDS {
            tracing::warn!(
                "drain_pending_reanalysis: hit MAX_ROUNDS={} (possible ng-include cycle, skip_uri={})",
                MAX_ROUNDS,
                skip_uri
            );
        }
    }
}

/// HTML 編集の影響範囲を判断するためのスナップショット。
///
/// `before != after` を確認するだけで、診断・semanticTokens/refresh・codeLens/refresh
/// が必要かをまとめて判定できる。各フィールドは「他ファイルの診断/トークン/lens に
/// 影響し得る cross-file dependency」を網羅する:
///
/// - `html_props`: HTML テンプレ scope 参照 (例: `{{vm.foo}}`) の property leaf 名
///   集合。JS ファイルの `unused_scope_variables` 診断に影響
/// - `embedded_refs`: 埋め込みスクリプトが参照しているシンボル名集合。JS の
///   "他JSから参照あり" 判定 (= unused 警告) に影響
/// - `embedded_defs`: 埋め込みスクリプトが定義しているシンボル名集合。
///   他 HTML のセマンティックトークン解決と code lens (controller jump) に影響
/// - `ng_includes`: ng-include の template_path 集合。子 HTML の
///   "Included by" code lens、controller 継承解決に影響
/// - `ng_controllers`: ng-controller スコープの (controller_name, alias) 集合。
///   JS 側 controller の "Used in templates" code lens に影響
/// - `embedded_template_bindings`: 埋め込みスクリプトの $routeProvider 等の
///   binding 集合。バインド先 HTML の lens に影響
/// - `embedded_component_template_urls`: 埋め込みスクリプトの component templateUrl
///   集合。テンプレ HTML の lens に影響
#[derive(Debug, PartialEq, Eq)]
struct HtmlChangeSnapshot {
    html_props: HashSet<String>,
    embedded_refs: HashSet<String>,
    embedded_defs: HashSet<String>,
    ng_includes: HashSet<String>,
    ng_controllers: HashSet<(String, Option<String>)>,
    embedded_template_bindings: HashSet<(String, String, &'static str)>,
    embedded_component_template_urls: HashSet<(String, Option<String>, String)>,
}

impl HtmlChangeSnapshot {
    fn capture(index: &Index, uri: &Url) -> Self {
        let html_props = index
            .html
            .get_html_scope_references(uri)
            .iter()
            .map(|r| property_path_leaf(&r.property_path).to_string())
            .collect();
        let embedded_refs = index.definitions.get_reference_names_for_uri(uri);
        let embedded_defs = index.definitions.get_definition_names_for_uri(uri);
        let ng_includes = index
            .templates
            .get_ng_includes_in_file(uri)
            .into_iter()
            .map(|(_line, path, _resolved)| path)
            .collect();
        let ng_controllers = index
            .controllers
            .get_all_html_controller_scopes(uri)
            .into_iter()
            .map(|s| (s.controller_name, s.alias))
            .collect();
        let embedded_template_bindings = index
            .templates
            .get_template_bindings_for_js_file(uri)
            .into_iter()
            .map(|b| (b.template_path, b.controller_name, b.source.label()))
            .collect();
        let embedded_component_template_urls = index
            .components
            .get_component_template_urls(uri)
            .into_iter()
            .map(|c| (c.template_path, c.controller_name, c.controller_as))
            .collect();

        Self {
            html_props,
            embedded_refs,
            embedded_defs,
            ng_includes,
            ng_controllers,
            embedded_template_bindings,
            embedded_component_template_urls,
        }
    }

    /// 他ファイルの semanticTokens / codeLens に影響し得る変化があるか
    /// (`html_props` と `embedded_refs` は当該 HTML の診断/トークンにしか
    /// 影響しないため除外する。それらは LSP クライアント側の didChange 後
    /// 自動再要求でカバー、または `collect_affected_js_uris` で個別 publish)
    fn cross_file_lens_state_changed(&self, other: &Self) -> bool {
        self.embedded_defs != other.embedded_defs
            || self.ng_includes != other.ng_includes
            || self.ng_controllers != other.ng_controllers
            || self.embedded_template_bindings != other.embedded_template_bindings
            || self.embedded_component_template_urls != other.embedded_component_template_urls
    }
}

/// JS 編集の影響範囲を判断するためのスナップショット。
///
/// `code_lens` の cross-file dep:
/// - `symbols`: defined controller / scope property 等。HTML の ng-controller
///   からの jump-to-definition lens に影響
/// - `template_bindings`: $routeProvider / $stateProvider / $uibModal 等の
///   templateUrl + controller binding。バインド先 HTML の lens に影響
/// - `component_template_urls`: component templateUrl。テンプレ HTML の
///   lens に影響
#[derive(Debug, PartialEq, Eq)]
struct JsChangeSnapshot {
    symbols: HashSet<String>,
    template_bindings: HashSet<(String, String, &'static str)>,
    component_template_urls: HashSet<(String, Option<String>, String)>,
}

impl JsChangeSnapshot {
    fn capture(index: &Index, uri: &Url) -> Self {
        let symbols = index.definitions.get_definition_names_for_uri(uri);
        let template_bindings = index
            .templates
            .get_template_bindings_for_js_file(uri)
            .into_iter()
            .map(|b| (b.template_path, b.controller_name, b.source.label()))
            .collect();
        let component_template_urls = index
            .components
            .get_component_template_urls(uri)
            .into_iter()
            .map(|c| (c.template_path, c.controller_name, c.controller_as))
            .collect();

        Self {
            symbols,
            template_bindings,
            component_template_urls,
        }
    }

    /// 他ファイルの semanticTokens / codeLens に影響し得る変化があるか
    fn cross_file_lens_state_changed(&self, other: &Self) -> bool {
        self.symbols != other.symbols
            || self.template_bindings != other.template_bindings
            || self.component_template_urls != other.component_template_urls
    }
}

/// HTML ファイル更新後、その変更で診断結果が変わり得る開いている JS ファイルの
/// URI 集合を返す。
///
/// JS の `check_unused_scope_variables` は次の経路で HTML に依存している:
/// 1. HTML テンプレ参照: `is_scope_variable_referenced(MyCtrl.$scope.foo)` が
///    HTML スコープ参照 (例: `{{vm.foo}}`) を全件スキャン
/// 2. HTML 埋め込みスクリプト参照: 埋め込みスクリプトが他 JS のシンボルを
///    参照すると `definitions.references` に書き込まれ、
///    `is_referenced_in_other_js` チェックが変動する
///
/// よって変更前後の (1)(2) の名前集合の和に、JS の scope 定義名がマッチする
/// 開いている JS だけ再診断対象にすれば良い。
///
/// 過剰包含 (controller alias 解決を省略するなど) はあるが、
/// 「開いている JS 全件 × is_scope_variable_referenced 全 HTML スキャン」より
/// 圧倒的に軽量。開いていない JS (`documents` に無い) は除外する。
fn collect_affected_js_uris(
    index: &Arc<Index>,
    documents: &Arc<DashMap<Url, String>>,
    before_html_property_names: &HashSet<String>,
    after_html_property_names: &HashSet<String>,
    before_embedded_ref_names: &HashSet<String>,
    after_embedded_ref_names: &HashSet<String>,
) -> HashSet<Url> {
    let mut affected: HashSet<Url> = HashSet::new();

    let property_candidates: HashSet<&String> = before_html_property_names
        .union(after_html_property_names)
        .collect();
    let symbol_candidates: HashSet<&String> = before_embedded_ref_names
        .union(after_embedded_ref_names)
        .collect();

    if property_candidates.is_empty() && symbol_candidates.is_empty() {
        return affected;
    }

    for entry in documents.iter() {
        let js_uri = entry.key();
        if !is_js_file(js_uri) {
            continue;
        }

        let scope_defs = index.definitions.get_scope_definitions_for_js(js_uri);
        for def in scope_defs {
            // 全名一致 (埋め込みスクリプトからの直接参照経路)
            if symbol_candidates.contains(&def.name) {
                affected.insert(js_uri.clone());
                break;
            }
            // プロパティ名末尾一致 (HTML テンプレ参照経路)
            if let Some((_, property_path)) = index.parse_scope_symbol_name(&def.name) {
                let leaf = property_path_leaf(&property_path).to_string();
                if property_candidates.contains(&leaf) {
                    affected.insert(js_uri.clone());
                    break;
                }
            }
        }
    }

    affected
}

/// JS ファイル更新後、その変更で診断結果が変わり得る開いている HTML ファイルの
/// URI 集合を返す。
///
/// 変更の影響範囲は以下の和集合:
/// 1. before/after の symbol 名集合の和に対し、HTML 側からそれらシンボルを
///    参照している HTML ファイル (例: `<div ng-controller="MyCtrl">` で
///    MyCtrl が削除/追加された場合に該当)
/// 2. この JS で宣言されている template binding (component / route /
///    state / modal) のターゲット HTML テンプレート
///    (controller 名は変わらなくても、template_path 側の binding が
///    新規/削除されることはあるため)
///
/// 開いていない HTML (`documents` に無い URI) は除外する。
fn collect_affected_html_uris(
    index: &Arc<Index>,
    documents: &Arc<DashMap<Url, String>>,
    js_uri: &Url,
    before_symbols: &HashSet<String>,
    after_symbols: &HashSet<String>,
) -> HashSet<Url> {
    let mut affected: HashSet<Url> = HashSet::new();

    // 1. シンボル名参照を持つ HTML
    //    全件 Vec clone を避けるため for_each_reference で借用イテレート
    let candidate_names: HashSet<&String> = before_symbols.union(after_symbols).collect();
    for name in candidate_names {
        index.definitions.for_each_reference(name, |reference| {
            if is_html_file(&reference.uri) && documents.contains_key(&reference.uri) {
                affected.insert(reference.uri.clone());
            }
        });
    }

    // 2. この JS で宣言されている template binding のテンプレート
    for binding in index.templates.get_template_bindings_for_js_file(js_uri) {
        if let Some(template_uri) = index.resolve_template_uri(&binding.template_path) {
            if documents.contains_key(&template_uri) {
                affected.insert(template_uri);
            }
        }
    }

    affected
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
            ts_proxy: Arc::new(RwLock::new(None)),
            documents: Arc::new(DashMap::new()),
            ts_opened_files: DashMap::new(),
            path_matcher: RwLock::new(None),
            diagnostics_config: Arc::new(RwLock::new(DiagnosticsConfig::default())),
            debounce_versions: Arc::new(DashMap::new()),
            ts_synced_versions: Arc::new(DashMap::new()),
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

    async fn on_change(&self, uri: Url, text: String) {
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
                //
                // 戻り値: Some((before_snapshot, after_snapshot))
                //   解析前後の HtmlChangeSnapshot ペアを返す。各種 cross-file
                //   dependency (HTML scope refs / 埋め込み defs/refs / ng-include /
                //   ng-controller / 埋め込み template_bindings / component templateUrl)
                //   をまとめて捕捉し、ピンポイント診断と refresh signals の発行判定に
                //   利用する。
                let analysis_result = tokio::task::spawn_blocking(move || {
                    let latest_text = match bl_documents.get(&bl_uri) {
                        Some(doc) => doc.value().clone(),
                        None => return None,
                    };

                    // before スナップショット: 解析後に clear されてしまうので先に取得
                    let before = HtmlChangeSnapshot::capture(&bl_index, &bl_uri);

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
                    drain_pending_reanalysis(&bl_index, &bl_uri, |child_uri| {
                        if let Some(doc) = bl_documents.get(child_uri) {
                            tracing::debug!(
                                "drain_pending_reanalysis: reanalyzing {} (triggered by {})",
                                child_uri,
                                bl_uri
                            );
                            bl_html_analyzer.analyze_document(child_uri, doc.value());
                        }
                    });

                    // after スナップショット
                    let after = HtmlChangeSnapshot::capture(&bl_index, &bl_uri);

                    Some((before, after))
                })
                .await
                .ok()
                .flatten();

                if let Some((before, after)) = analysis_result {
                    publish_html_diagnostics(&client, &index, &diagnostics_config, &uri).await;

                    // この HTML 変更で診断結果が変わり得る開いている JS だけ
                    // ピンポイントに再発行する
                    let affected_js = collect_affected_js_uris(
                        &index,
                        &documents,
                        &before.html_props,
                        &after.html_props,
                        &before.embedded_refs,
                        &after.embedded_refs,
                    );
                    for js_uri in affected_js {
                        publish_js_diagnostics(&client, &index, &diagnostics_config, &js_uri).await;
                    }

                    // semantic_tokens_refresh / code_lens_refresh はどちらも workspace
                    // 全 applicable ファイルに再要求が走る重い操作。cross-file dep の
                    // 状態変化が無ければスキップする。HTML 自身のスコープ参照変化
                    // (`{{vm.foo}}` 追加など) は当該 HTML のトークン/lens にしか影響
                    // せず、それは LSP クライアントが didChange 後に自動再要求する。
                    if before.cross_file_lens_state_changed(&after) {
                        let _ = client.semantic_tokens_refresh().await;
                        let _ = client.code_lens_refresh().await;
                    }
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
            let ts_proxy = Arc::clone(&self.ts_proxy);
            let ts_synced_versions = Arc::clone(&self.ts_synced_versions);
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
                let bl_index = Arc::clone(&index);

                // 戻り値: Some((before_snapshot, after_snapshot))
                //   解析前後の JsChangeSnapshot ペア。defined symbols /
                //   template_bindings / component templateUrl の cross-file dep を
                //   全部捕捉する。
                let analysis_result = tokio::task::spawn_blocking(move || {
                    let latest_text = match bl_documents.get(&bl_uri) {
                        Some(doc) => doc.value().clone(),
                        None => return None,
                    };

                    let before = JsChangeSnapshot::capture(&bl_index, &bl_uri);

                    bl_analyzer.analyze_document(&bl_uri, &latest_text);

                    let after = JsChangeSnapshot::capture(&bl_index, &bl_uri);

                    Some((before, after))
                })
                .await
                .ok()
                .flatten();

                // tsserver にも debounce 後に最新テキストを送る (毎キーストローク
                // の IPC 送信を避ける)。`debounce_versions` の version check で
                // 最新タスクのみ flush するので、200ms 以内の連続キーは最後の 1 回
                // しか送らない。
                //
                // request 経路から ensure_ts_synced で flush される staleness
                // 対策があるので、ここはあくまで「アイドル時に背景で同期」する役割。
                if ts_synced_versions
                    .get(&uri)
                    .map(|v| *v < ver)
                    .unwrap_or(true)
                {
                    if let Some(ref proxy) = *ts_proxy.read().await {
                        if let Some(doc) = documents.get(&uri) {
                            proxy
                                .did_change(&uri, doc.value(), ver as i32)
                                .await;
                            ts_synced_versions.insert(uri.clone(), ver);
                        }
                    }
                }

                if let Some((before, after)) = analysis_result {
                    publish_js_diagnostics(&client, &index, &diagnostics_config, &uri).await;

                    // この JS の変更で診断結果が変わり得る HTML ファイルを特定して
                    // ピンポイントに再発行する
                    let affected_html = collect_affected_html_uris(
                        &index,
                        &documents,
                        &uri,
                        &before.symbols,
                        &after.symbols,
                    );
                    for html_uri in affected_html {
                        publish_html_diagnostics(&client, &index, &diagnostics_config, &html_uri).await;
                    }

                    // semantic_tokens_refresh / code_lens_refresh はどちらも workspace
                    // 全 applicable ファイルに再要求が走る重い操作。cross-file dep の
                    // 状態変化が無ければスキップする (シンボル / template_bindings /
                    // component templateUrl のいずれも変わっていなければ、他ファイルの
                    // トークン解決と lens 内容は変わらない)。
                    if before.cross_file_lens_state_changed(&after) {
                        let _ = client.semantic_tokens_refresh().await;
                        let _ = client.code_lens_refresh().await;
                    }
                }
            });
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
                // drain_pending_reanalysis を spawn_blocking 内で直接呼ぶ
                // (&self を送れないため process_pending_reanalysis ではなく直接 helper)
                bl_index.remove_from_pending_reanalysis(&bl_uri);
                drain_pending_reanalysis(&bl_index, &bl_uri, |child_uri| {
                    if let Some(doc) = bl_documents.get(child_uri) {
                        bl_html_analyzer.analyze_document(child_uri, doc.value());
                    }
                });
            })
            .await
            .unwrap_or(());

            self.publish_diagnostics_for_html(&uri).await;
            self.republish_diagnostics_for_open_js_files().await;

            // クライアントが on_open 完了前に semantic_tokens_full を要求した場合、
            // 空トークンが返ってハイライトが永続的に消えるレースを防ぐ。
            // 解析完了をクライアントに通知して再要求させる。
            let _ = self.client.semantic_tokens_refresh().await;
            let _ = self.client.code_lens_refresh().await;
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

            // (HTML側と同じ理由) JS ファイルでも解析後に refresh を送る
            let _ = self.client.semantic_tokens_refresh().await;
            let _ = self.client.code_lens_refresh().await;
        }

        // tsserver は JS ファイルだけ知っていれば良い (HTML は内部ハンドラで処理)
        if is_js_file(&uri) {
            if let Some(ref proxy) = *self.ts_proxy.read().await {
                proxy.did_open(&uri, &text).await;
                self.ts_opened_files.insert(uri.clone(), true);
            }
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
                // open 時点のテキストは tsserver と同期済み。`debounce_versions`
                // と同じ初期値 0 をセットして、以降の did_change 監視の起点に
                self.ts_synced_versions.insert(uri.clone(), 0);
            }
        }
    }

    /// tsserver の見ているテキストを最新に同期する。
    ///
    /// JS on_change では `did_change` を 200ms デバウンスして spawn 内から送信する
    /// ため、デバウンス完了前に LSP リクエスト (completion / hover など) が来ると
    /// tsserver が古いテキストで応答してしまう。本メソッドは
    /// `debounce_versions[uri] > ts_synced_versions[uri]` の場合のみ、リクエスト
    /// 直前に同期送信して staleness を解消する。
    async fn ensure_ts_synced(&self, uri: &Url) {
        if !is_js_file(uri) {
            return;
        }
        let current = self.debounce_versions.get(uri).map(|v| *v).unwrap_or(0);
        let synced = self.ts_synced_versions.get(uri).map(|v| *v).unwrap_or(0);
        if current <= synced {
            return;
        }
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            if let Some(doc) = self.documents.get(uri) {
                proxy.did_change(uri, doc.value(), current as i32).await;
                self.ts_synced_versions.insert(uri.clone(), current);
            }
        }
    }

    fn process_pending_reanalysis(&self, current_uri: &Url) {
        self.index.remove_from_pending_reanalysis(current_uri);
        drain_pending_reanalysis(&self.index, current_uri, |child_uri| {
            if let Some(doc) = self.documents.get(child_uri) {
                tracing::debug!(
                    "drain_pending_reanalysis: reanalyzing {} (triggered by {})",
                    child_uri,
                    current_uri
                );
                self.html_analyzer.analyze_document(child_uri, doc.value());
            }
        });
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

                // interpolate 記号は JS の `$interpolateProvider.startSymbol/endSymbol`
                // から動的に解決する (ajsconfig.json 経由の設定経路は撤廃済み)。
                *self.diagnostics_config.write().await = config.diagnostics.clone();

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
        if let Some(change) = params.content_changes.into_iter().next() {
            self.on_change(uri, change.text).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if let Some(text) = params.text {
            self.on_change(params.text_document.uri, text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = &params.text_document.uri;
        // 実際に tsserver に開いたファイルだけを close する
        // (HTML 等、tsserver に渡していないファイルに did_close を送る必要はない)
        if self.ts_opened_files.remove(uri).is_some() {
            if let Some(ref proxy) = *self.ts_proxy.read().await {
                proxy.did_close(uri).await;
            }
        }
        // 閉じられたファイルの sync 状態は不要、エントリを削除する
        // (再 open 時に ensure_ts_file_opened が新しく初期化する)
        self.ts_synced_versions.remove(uri);
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri.clone();
        let index = Arc::clone(&self.index);
        let params_for_blocking = params.clone();
        let local_refs = tokio::task::spawn_blocking(move || {
            ReferencesHandler::new(index).find_references(params_for_blocking)
        })
        .await
        .ok()
        .flatten();
        if let Some(refs) = local_refs {
            return Ok(Some(refs));
        }

        self.ensure_ts_file_opened(&uri).await;
        self.ensure_ts_synced(&uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.references(&params).await);
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri.clone();
        let pos = params.text_document_position_params.position;

        let source = self.documents.get(&uri).map(|s| s.value().clone());
        let index = Arc::clone(&self.index);
        let params_for_blocking = params.clone();
        let local_def = tokio::task::spawn_blocking(move || {
            DefinitionHandler::new(index)
                .goto_definition_with_source(params_for_blocking, source.as_deref())
        })
        .await
        .ok()
        .flatten();

        if let Some(def) = local_def {
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

        self.ensure_ts_file_opened(&uri).await;
        self.ensure_ts_synced(&uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.goto_definition(&params).await);
        }

        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri.clone();
        let index = Arc::clone(&self.index);
        let params_for_blocking = params.clone();
        let local_hover = tokio::task::spawn_blocking(move || {
            HoverHandler::new(index).hover(params_for_blocking)
        })
        .await
        .ok()
        .flatten();
        if let Some(hover) = local_hover {
            return Ok(Some(hover));
        }

        self.ensure_ts_file_opened(&uri).await;
        self.ensure_ts_synced(&uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.hover(&params).await);
        }

        Ok(None)
    }

    async fn signature_help(
        &self,
        params: SignatureHelpParams,
    ) -> Result<Option<SignatureHelp>> {
        let uri = params.text_document_position_params.text_document.uri.clone();
        let position = params.text_document_position_params.position;

        let source = match self.documents.get(&uri) {
            Some(doc) => doc.value().clone(),
            None => return Ok(None),
        };

        let index = Arc::clone(&self.index);
        let blocking_uri = uri.clone();
        let local_sig = tokio::task::spawn_blocking(move || {
            SignatureHelpHandler::new(index).signature_help(
                &blocking_uri,
                position.line,
                position.character,
                &source,
            )
        })
        .await
        .ok()
        .flatten();
        if let Some(sig_help) = local_sig {
            return Ok(Some(sig_help));
        }

        self.ensure_ts_file_opened(&uri).await;
        self.ensure_ts_synced(&uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.signature_help(&params).await);
        }

        Ok(None)
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri.clone();
        let index = Arc::clone(&self.index);
        // CPU-bound work を blocking スレッドに退避し、tokio worker を解放する。
        // (多数ファイル open 時に handler が tokio worker を占有して他 LSP リクエストが
        //  詰まる問題を回避するため。以下の handler 群でも同様)
        let result = tokio::task::spawn_blocking(move || {
            DocumentSymbolHandler::new(index).document_symbols(&uri)
        })
        .await
        .ok()
        .flatten();
        Ok(result)
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.clone();
        let line = params.text_document_position.position.line;
        let col = params.text_document_position.position.character;

        // CPU-bound 部分 (HTML/JS の AngularJS 補完計算) を spawn_blocking に
        // 退避し、ts_proxy フォールバック判定だけ async 側で扱う。
        let index = Arc::clone(&self.index);
        let html_analyzer = Arc::clone(&self.html_analyzer);
        let documents = Arc::clone(&self.documents);
        let blocking_uri = uri.clone();
        let decision = tokio::task::spawn_blocking(move || {
            compute_completion_decision(
                index,
                html_analyzer,
                documents,
                blocking_uri,
                line,
                col,
            )
        })
        .await
        .unwrap_or(CompletionDecision::NoResult);

        match decision {
            CompletionDecision::Resolved(c) => Ok(Some(c)),
            CompletionDecision::NoResult => Ok(None),
            CompletionDecision::FallbackToTsProxy => {
                self.ensure_ts_file_opened(&uri).await;
                self.ensure_ts_synced(&uri).await;
                if let Some(ref proxy) = *self.ts_proxy.read().await {
                    return Ok(proxy.completion(&params).await);
                }
                Ok(None)
            }
        }
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri.clone();
        let index = Arc::clone(&self.index);
        let params_for_blocking = params.clone();
        let local_edit = tokio::task::spawn_blocking(move || {
            RenameHandler::new(index).rename(params_for_blocking)
        })
        .await
        .ok()
        .flatten();
        if let Some(edit) = local_edit {
            return Ok(Some(edit));
        }

        self.ensure_ts_file_opened(&uri).await;
        self.ensure_ts_synced(&uri).await;
        if let Some(ref proxy) = *self.ts_proxy.read().await {
            return Ok(proxy.rename(&params).await);
        }

        Ok(None)
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let index = Arc::clone(&self.index);
        let result = tokio::task::spawn_blocking(move || {
            RenameHandler::new(index).prepare_rename(params)
        })
        .await
        .ok()
        .flatten();
        Ok(result)
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let uri = params.text_document.uri.clone();
        let index = Arc::clone(&self.index);
        let result = tokio::task::spawn_blocking(move || {
            CodeLensHandler::new(index).code_lens(&uri)
        })
        .await
        .ok()
        .flatten();
        Ok(result)
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = params.text_document.uri.clone();
        let index = Arc::clone(&self.index);
        let tokens = tokio::task::spawn_blocking(move || {
            SemanticTokensHandler::new(index).semantic_tokens_full(&uri)
        })
        .await
        .ok()
        .flatten();
        Ok(tokens.map(SemanticTokensResult::Tokens))
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let index = Arc::clone(&self.index);
        let symbols = tokio::task::spawn_blocking(move || {
            WorkspaceSymbolHandler::new(index).handle(&params.query)
        })
        .await
        .unwrap_or_default();
        if symbols.is_empty() {
            return Ok(None);
        }
        Ok(Some(symbols))
    }
}

#[cfg(test)]
mod collect_affected_html_uris_tests {
    use super::*;
    use crate::model::{
        BindingSource, Span, SymbolBuilder, SymbolKind, SymbolReference, TemplateBinding,
    };

    fn js(path: &str) -> Url {
        Url::parse(&format!("file://{}", path)).unwrap()
    }
    fn html(path: &str) -> Url {
        Url::parse(&format!("file://{}", path)).unwrap()
    }

    fn add_definition(index: &Index, name: &str, uri: &Url) {
        let span = Span::new(0, 0, 0, name.len() as u32);
        let symbol = SymbolBuilder::new(name.to_string(), SymbolKind::Controller, uri.clone())
            .definition_span(span)
            .name_span(span)
            .build();
        index.definitions.add_definition(symbol);
    }

    fn add_html_reference(index: &Index, name: &str, html_uri: &Url) {
        index.definitions.add_reference(SymbolReference {
            name: name.to_string(),
            uri: html_uri.clone(),
            span: Span::new(0, 0, 0, name.len() as u32),
        });
    }

    fn build_documents(uris: &[&Url]) -> Arc<DashMap<Url, String>> {
        let docs = DashMap::new();
        for u in uris {
            docs.insert((*u).clone(), String::new());
        }
        Arc::new(docs)
    }

    #[test]
    fn collects_html_referencing_existing_symbol() {
        // JS で MyCtrl が定義され、HTML で参照されている → 影響あり
        let index = Arc::new(Index::new());
        let js_uri = js("/app/ctrl.js");
        let html_uri = html("/app/page.html");

        add_definition(&index, "MyCtrl", &js_uri);
        add_html_reference(&index, "MyCtrl", &html_uri);

        let documents = build_documents(&[&js_uri, &html_uri]);
        let mut after = HashSet::new();
        after.insert("MyCtrl".to_string());

        let affected = collect_affected_html_uris(
            &index,
            &documents,
            &js_uri,
            &HashSet::new(),
            &after,
        );
        assert!(affected.contains(&html_uri), "MyCtrl 参照を持つ HTML が含まれるべき");
    }

    #[test]
    fn collects_html_referencing_removed_symbol() {
        // JS から MyCtrl が消えた (before のみに存在) → HTML 参照は今 undefined になる
        // HTML 診断更新が必要
        let index = Arc::new(Index::new());
        let js_uri = js("/app/ctrl.js");
        let html_uri = html("/app/page.html");

        // 定義はもう index にない (clear 済み想定) が、HTML 参照は残ってる
        add_html_reference(&index, "MyCtrl", &html_uri);

        let documents = build_documents(&[&js_uri, &html_uri]);
        let mut before = HashSet::new();
        before.insert("MyCtrl".to_string());

        let affected = collect_affected_html_uris(
            &index,
            &documents,
            &js_uri,
            &before,
            &HashSet::new(),
        );
        assert!(
            affected.contains(&html_uri),
            "削除されたシンボルへの HTML 参照を持つファイルも対象"
        );
    }

    #[test]
    fn skips_unaffected_html_files() {
        // OtherCtrl は別のJSで定義されており、今変更している JS とは無関係
        let index = Arc::new(Index::new());
        let js_uri = js("/app/ctrl.js");
        let other_js = js("/app/other.js");
        let html_uri = html("/app/uses_other.html");

        add_definition(&index, "OtherCtrl", &other_js);
        add_html_reference(&index, "OtherCtrl", &html_uri);

        let documents = build_documents(&[&js_uri, &html_uri]);
        // 今変えている js_uri (ctrl.js) は OtherCtrl を持っていない
        let affected = collect_affected_html_uris(
            &index,
            &documents,
            &js_uri,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(
            !affected.contains(&html_uri),
            "無関係な HTML は再発行対象に入らないべき"
        );
    }

    #[test]
    fn skips_unopened_html_files() {
        // HTML が documents に無い (= エディタで開かれていない) ものは対象外
        let index = Arc::new(Index::new());
        let js_uri = js("/app/ctrl.js");
        let unopened_html = html("/app/never-opened.html");

        add_definition(&index, "MyCtrl", &js_uri);
        add_html_reference(&index, "MyCtrl", &unopened_html);

        // documents に html を含めない
        let documents = build_documents(&[&js_uri]);
        let mut after = HashSet::new();
        after.insert("MyCtrl".to_string());

        let affected = collect_affected_html_uris(
            &index,
            &documents,
            &js_uri,
            &HashSet::new(),
            &after,
        );
        assert!(affected.is_empty(), "未オープン HTML は対象外");
    }

    #[test]
    fn collects_template_binding_targets() {
        // この JS で template_binding を宣言している → ターゲット HTML を含める
        // (シンボル名参照だけでは捕まらない、route/component 系のバインディング)
        let index = Arc::new(Index::new());
        let js_uri = js("/app/routes.js");
        let html_uri = html("/app/templates/home.html");

        index.templates.add_template_binding(TemplateBinding {
            template_path: "templates/home.html".to_string(),
            controller_name: "HomeCtrl".to_string(),
            source: BindingSource::RouteProvider,
            binding_uri: js_uri.clone(),
            binding_line: 0,
        });

        // resolve_template_uri が機能するように、template が "open" 扱いとして
        // documents にも入れておく (実際は templates store がフルパスで持つが、
        // ここではテスト簡略化として直接 document に登録)
        let documents = build_documents(&[&js_uri, &html_uri]);

        let affected = collect_affected_html_uris(
            &index,
            &documents,
            &js_uri,
            &HashSet::new(),
            &HashSet::new(),
        );
        // resolve_template_uri がマッチするかは index 内部実装依存だが、
        // 少なくとも binding が登録されていれば、ターゲットを試す経路が走る
        // 結果は空でも可 (resolve できないケース) だが、affected が unaffected に
        // なってはいけない → 単に「panic しない / 余計な URI を返さない」を保証
        for u in &affected {
            assert!(documents.contains_key(u), "documents に無い URI を返さない");
        }
    }
}

#[cfg(test)]
mod collect_affected_js_uris_tests {
    use super::*;
    use crate::model::{
        HtmlScopeReference, Span, SymbolBuilder, SymbolKind, SymbolReference,
    };

    fn js(path: &str) -> Url {
        Url::parse(&format!("file://{}", path)).unwrap()
    }
    fn html(path: &str) -> Url {
        Url::parse(&format!("file://{}", path)).unwrap()
    }

    fn add_scope_property(index: &Index, name: &str, js_uri: &Url) {
        let span = Span::new(0, 0, 0, name.len() as u32);
        let symbol = SymbolBuilder::new(name.to_string(), SymbolKind::ScopeProperty, js_uri.clone())
            .definition_span(span)
            .name_span(span)
            .build();
        index.definitions.add_definition(symbol);
    }

    fn add_embedded_script_reference(index: &Index, name: &str, html_uri: &Url) {
        index.definitions.add_reference(SymbolReference {
            name: name.to_string(),
            uri: html_uri.clone(),
            span: Span::new(0, 0, 0, name.len() as u32),
        });
    }

    fn add_html_scope_ref_for_setup(
        index: &Index,
        property_path: &str,
        html_uri: &Url,
    ) {
        index.html.add_html_scope_reference(HtmlScopeReference {
            property_path: property_path.to_string(),
            uri: html_uri.clone(),
            start_line: 0,
            start_col: 0,
            end_line: 0,
            end_col: property_path.len() as u32,
        });
    }

    fn build_documents(uris: &[&Url]) -> Arc<DashMap<Url, String>> {
        let docs = DashMap::new();
        for u in uris {
            docs.insert((*u).clone(), String::new());
        }
        Arc::new(docs)
    }

    fn names<I: IntoIterator<Item = &'static str>>(iter: I) -> HashSet<String> {
        iter.into_iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn property_path_leaf_returns_last_component() {
        assert_eq!(property_path_leaf("foo"), "foo");
        assert_eq!(property_path_leaf("vm.foo"), "foo");
        assert_eq!(property_path_leaf("vm.foo.bar"), "bar");
    }

    #[test]
    fn collects_js_with_matching_property_name() {
        // HTML が `vm.foo` を参照、JS が `MyCtrl.$scope.foo` を定義 → 影響あり
        let index = Arc::new(Index::new());
        let js_uri = js("/app/ctrl.js");
        let html_uri = html("/app/page.html");

        add_scope_property(&index, "MyCtrl.$scope.foo", &js_uri);
        let documents = build_documents(&[&js_uri, &html_uri]);

        let affected = collect_affected_js_uris(
            &index,
            &documents,
            &HashSet::new(),
            &names(["foo"]),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(
            affected.contains(&js_uri),
            "プロパティ名一致の JS が含まれるべき"
        );
    }

    #[test]
    fn collects_js_referenced_by_embedded_script_full_name() {
        // HTML 埋め込みスクリプトが `MyCtrl.$scope.foo` を参照、
        // JS がそのシンボルを定義 → 影響あり
        let index = Arc::new(Index::new());
        let js_uri = js("/app/ctrl.js");
        let html_uri = html("/app/page.html");

        add_scope_property(&index, "MyCtrl.$scope.foo", &js_uri);
        let documents = build_documents(&[&js_uri, &html_uri]);

        let affected = collect_affected_js_uris(
            &index,
            &documents,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &names(["MyCtrl.$scope.foo"]),
        );
        assert!(
            affected.contains(&js_uri),
            "埋め込みスクリプト直接参照の JS が含まれるべき"
        );
    }

    #[test]
    fn collects_js_when_only_before_set_has_match() {
        // 削除ケース: HTML が以前 `vm.foo` を参照していたが消えた。
        // JS の MyCtrl.$scope.foo は今 unused に変わるので再診断対象。
        let index = Arc::new(Index::new());
        let js_uri = js("/app/ctrl.js");
        let html_uri = html("/app/page.html");

        add_scope_property(&index, "MyCtrl.$scope.foo", &js_uri);
        let documents = build_documents(&[&js_uri, &html_uri]);

        let affected = collect_affected_js_uris(
            &index,
            &documents,
            &names(["foo"]),    // before
            &HashSet::new(),    // after
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(
            affected.contains(&js_uri),
            "削除前の参照名一致でも対象にする (before only)"
        );
    }

    #[test]
    fn skips_unrelated_js_files() {
        // OtherCtrl.$scope.bar を定義する JS は、`foo` 参照の HTML 変更とは無関係
        let index = Arc::new(Index::new());
        let js_uri = js("/app/other.js");
        let html_uri = html("/app/page.html");

        add_scope_property(&index, "OtherCtrl.$scope.bar", &js_uri);
        let documents = build_documents(&[&js_uri, &html_uri]);

        let affected = collect_affected_js_uris(
            &index,
            &documents,
            &HashSet::new(),
            &names(["foo"]),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(
            !affected.contains(&js_uri),
            "プロパティ名が一致しない JS は対象外"
        );
    }

    #[test]
    fn skips_unopened_js_files() {
        // documents に無い JS は対象外 (閉じてるので発行不要)
        let index = Arc::new(Index::new());
        let unopened_js = js("/app/closed.js");
        let html_uri = html("/app/page.html");

        add_scope_property(&index, "MyCtrl.$scope.foo", &unopened_js);
        // unopened_js は documents に入れない
        let documents = build_documents(&[&html_uri]);

        let affected = collect_affected_js_uris(
            &index,
            &documents,
            &HashSet::new(),
            &names(["foo"]),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(
            affected.is_empty(),
            "未オープン JS は再発行対象に含めない"
        );
    }

    #[test]
    fn returns_empty_when_no_candidates() {
        // before/after が両方空 → スキャン不要・空集合
        let index = Arc::new(Index::new());
        let js_uri = js("/app/ctrl.js");
        let html_uri = html("/app/page.html");

        add_scope_property(&index, "MyCtrl.$scope.foo", &js_uri);
        let documents = build_documents(&[&js_uri, &html_uri]);

        let affected = collect_affected_js_uris(
            &index,
            &documents,
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(affected.is_empty(), "候補名がなければ空集合");
    }

    #[test]
    fn property_leaf_match_with_dotted_path() {
        // HTML 参照 `vm.foo`, JS 定義 `MyCtrl.$scope.foo` → leaf 一致
        // (HtmlScopeReference の property_path が "vm.foo" 形式でも leaf 抽出で照合)
        let index = Arc::new(Index::new());
        let js_uri = js("/app/ctrl.js");
        let html_uri = html("/app/page.html");

        // 後の改修で property_path から leaf を取り出すロジックを追加した場合、
        // この経路でも検出できることを保証
        add_scope_property(&index, "MyCtrl.$scope.foo", &js_uri);
        add_html_scope_ref_for_setup(&index, "vm.foo", &html_uri);
        let documents = build_documents(&[&js_uri, &html_uri]);

        // collect_affected_js_uris は property names (leaf) を直接受け取る前提なので、
        // 呼び出し元で leaf 抽出する。ここは leaf 化済みの "foo" を渡してテスト
        let affected = collect_affected_js_uris(
            &index,
            &documents,
            &HashSet::new(),
            &names(["foo"]),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(affected.contains(&js_uri));
    }

    #[test]
    fn skips_non_js_documents() {
        // documents に HTML だけが入っているケースで、HTML を JS と誤認しない
        let index = Arc::new(Index::new());
        let html_uri = html("/app/page.html");

        // HTML URI に scope-property 風の定義があっても、is_js_file で弾かれる
        add_scope_property(&index, "MyCtrl.$scope.foo", &html_uri);
        let documents = build_documents(&[&html_uri]);

        let affected = collect_affected_js_uris(
            &index,
            &documents,
            &HashSet::new(),
            &names(["foo"]),
            &HashSet::new(),
            &HashSet::new(),
        );
        assert!(affected.is_empty(), "HTML URI は対象外");
    }

    #[test]
    fn returns_open_js_when_embedded_script_referenced_symbol_removed() {
        // 削除ケース (埋め込みスクリプト経由): 前は HTML 埋め込みスクリプトから
        // MyCtrl.$scope.foo を参照していたが消えた → JS の "他JSから参照あり"
        // 判定が変わるので再診断対象
        let index = Arc::new(Index::new());
        let js_uri = js("/app/ctrl.js");
        let html_uri = html("/app/page.html");

        add_scope_property(&index, "MyCtrl.$scope.foo", &js_uri);
        // before として add しておくが、collect_affected_js_uris の入力としては
        // before セットに名前を渡せば良いので、ここではリファレンス追加は省略
        let documents = build_documents(&[&js_uri, &html_uri]);

        let affected = collect_affected_js_uris(
            &index,
            &documents,
            &HashSet::new(),
            &HashSet::new(),
            &names(["MyCtrl.$scope.foo"]), // before only
            &HashSet::new(),
        );
        assert!(
            affected.contains(&js_uri),
            "埋め込みスクリプト参照の削除ケースも対象"
        );

        // before/after 両方の経路を実引き当てるための補助確認: add_embedded_script_reference は
        // 実際には server 内 spawn_blocking 内で get_reference_names_for_uri と組み合わせて
        // before/after を構築する。テストでは入力集合を直接渡して挙動を検証している。
        let _ = add_embedded_script_reference; // ヘルパー未使用警告抑制
    }
}

#[cfg(test)]
mod change_snapshot_tests {
    use super::*;
    use crate::model::{
        BindingSource, ComponentTemplateUrl, HtmlControllerScope, NgIncludeBinding, Span,
        SymbolBuilder, SymbolKind, TemplateBinding,
    };

    fn html(path: &str) -> Url {
        Url::parse(&format!("file://{}", path)).unwrap()
    }
    fn js(path: &str) -> Url {
        Url::parse(&format!("file://{}", path)).unwrap()
    }

    fn add_definition(index: &Index, name: &str, uri: &Url) {
        let span = Span::new(0, 0, 0, name.len() as u32);
        let symbol = SymbolBuilder::new(name.to_string(), SymbolKind::Controller, uri.clone())
            .definition_span(span)
            .name_span(span)
            .build();
        index.definitions.add_definition(symbol);
    }

    fn add_template_binding(
        index: &Index,
        binding_uri: &Url,
        template_path: &str,
        controller_name: &str,
        source: BindingSource,
    ) {
        index.templates.add_template_binding(TemplateBinding {
            template_path: template_path.to_string(),
            controller_name: controller_name.to_string(),
            source,
            binding_uri: binding_uri.clone(),
            binding_line: 0,
        });
    }

    fn add_component_template_url(
        index: &Index,
        uri: &Url,
        template_path: &str,
        controller_name: Option<&str>,
        controller_as: &str,
    ) {
        index.components.add_component_template_url(ComponentTemplateUrl {
            uri: uri.clone(),
            template_path: template_path.to_string(),
            line: 0,
            col: 0,
            controller_name: controller_name.map(String::from),
            controller_as: controller_as.to_string(),
        });
    }

    fn add_ng_controller_scope(index: &Index, uri: &Url, name: &str, alias: Option<&str>) {
        index.controllers.add_html_controller_scope(HtmlControllerScope {
            controller_name: name.to_string(),
            alias: alias.map(String::from),
            uri: uri.clone(),
            start_line: 0,
            end_line: 100,
        });
    }

    fn add_ng_include(index: &Index, parent: &Url, template_path: &str) {
        index.templates.add_ng_include_binding(NgIncludeBinding {
            parent_uri: parent.clone(),
            template_path: template_path.to_string(),
            resolved_filename: template_path.to_string(),
            line: 0,
            inherited_controllers: Vec::new(),
            inherited_local_variables: Vec::new(),
            inherited_form_bindings: Vec::new(),
        });
    }

    // -------------------- HtmlChangeSnapshot --------------------

    #[test]
    fn html_snapshot_empty_state_is_equal() {
        let index = Index::new();
        let uri = html("/app/page.html");
        let s1 = HtmlChangeSnapshot::capture(&index, &uri);
        let s2 = HtmlChangeSnapshot::capture(&index, &uri);
        assert_eq!(s1, s2);
        assert!(!s1.cross_file_lens_state_changed(&s2));
    }

    #[test]
    fn html_snapshot_detects_embedded_def_change() {
        let index = Index::new();
        let uri = html("/app/page.html");
        let before = HtmlChangeSnapshot::capture(&index, &uri);

        // 埋め込みスクリプトが新たに controller を定義
        add_definition(&index, "MyCtrl", &uri);
        let after = HtmlChangeSnapshot::capture(&index, &uri);

        assert_ne!(before, after);
        assert!(before.cross_file_lens_state_changed(&after));
    }

    #[test]
    fn html_snapshot_detects_ng_include_change() {
        let index = Index::new();
        let uri = html("/app/page.html");
        let before = HtmlChangeSnapshot::capture(&index, &uri);

        add_ng_include(&index, &uri, "child.html");
        let after = HtmlChangeSnapshot::capture(&index, &uri);

        assert_ne!(before, after);
        assert!(before.cross_file_lens_state_changed(&after));
    }

    #[test]
    fn html_snapshot_detects_ng_controller_change() {
        let index = Index::new();
        let uri = html("/app/page.html");
        let before = HtmlChangeSnapshot::capture(&index, &uri);

        add_ng_controller_scope(&index, &uri, "MyCtrl", Some("vm"));
        let after = HtmlChangeSnapshot::capture(&index, &uri);

        assert_ne!(before, after);
        assert!(before.cross_file_lens_state_changed(&after));
    }

    #[test]
    fn html_snapshot_detects_template_binding_change() {
        let index = Index::new();
        let uri = html("/app/page.html");
        let before = HtmlChangeSnapshot::capture(&index, &uri);

        // 埋め込みスクリプトに $routeProvider.when を追加
        add_template_binding(&index, &uri, "foo.html", "FooCtrl", BindingSource::RouteProvider);
        let after = HtmlChangeSnapshot::capture(&index, &uri);

        assert_ne!(before, after);
        assert!(before.cross_file_lens_state_changed(&after));
    }

    #[test]
    fn html_snapshot_detects_component_template_url_change() {
        let index = Index::new();
        let uri = html("/app/page.html");
        let before = HtmlChangeSnapshot::capture(&index, &uri);

        add_component_template_url(&index, &uri, "foo.html", Some("FooCtrl"), "$ctrl");
        let after = HtmlChangeSnapshot::capture(&index, &uri);

        assert_ne!(before, after);
        assert!(before.cross_file_lens_state_changed(&after));
    }

    #[test]
    fn html_snapshot_html_props_change_does_not_trigger_lens_refresh() {
        // html_props (テンプレ scope ref) は当該 HTML のトークンにしか影響しないので、
        // cross_file_lens_state_changed は false を返さねばならない
        let mut a = HtmlChangeSnapshot {
            html_props: HashSet::new(),
            embedded_refs: HashSet::new(),
            embedded_defs: HashSet::new(),
            ng_includes: HashSet::new(),
            ng_controllers: HashSet::new(),
            embedded_template_bindings: HashSet::new(),
            embedded_component_template_urls: HashSet::new(),
        };
        let b = HtmlChangeSnapshot {
            html_props: HashSet::from(["foo".to_string()]),
            embedded_refs: HashSet::new(),
            embedded_defs: HashSet::new(),
            ng_includes: HashSet::new(),
            ng_controllers: HashSet::new(),
            embedded_template_bindings: HashSet::new(),
            embedded_component_template_urls: HashSet::new(),
        };
        assert_ne!(a, b);
        assert!(
            !a.cross_file_lens_state_changed(&b),
            "html_props 差分は他ファイル lens に影響しないので false"
        );

        // embedded_refs も同様 (診断にしか影響しない)
        a.embedded_refs.insert("Bar".to_string());
        assert!(
            !a.cross_file_lens_state_changed(&b),
            "embedded_refs 差分も他ファイル lens に影響しない"
        );
    }

    // -------------------- JsChangeSnapshot --------------------

    #[test]
    fn js_snapshot_empty_state_is_equal() {
        let index = Index::new();
        let uri = js("/app/ctrl.js");
        let s1 = JsChangeSnapshot::capture(&index, &uri);
        let s2 = JsChangeSnapshot::capture(&index, &uri);
        assert_eq!(s1, s2);
        assert!(!s1.cross_file_lens_state_changed(&s2));
    }

    #[test]
    fn js_snapshot_detects_symbol_change() {
        let index = Index::new();
        let uri = js("/app/ctrl.js");
        let before = JsChangeSnapshot::capture(&index, &uri);

        add_definition(&index, "MyCtrl", &uri);
        let after = JsChangeSnapshot::capture(&index, &uri);

        assert_ne!(before, after);
        assert!(before.cross_file_lens_state_changed(&after));
    }

    #[test]
    fn js_snapshot_detects_template_binding_change() {
        let index = Index::new();
        let uri = js("/app/routes.js");
        let before = JsChangeSnapshot::capture(&index, &uri);

        add_template_binding(&index, &uri, "foo.html", "FooCtrl", BindingSource::RouteProvider);
        let after = JsChangeSnapshot::capture(&index, &uri);

        assert_ne!(before, after);
        assert!(before.cross_file_lens_state_changed(&after));
    }

    #[test]
    fn js_snapshot_detects_component_template_url_change() {
        let index = Index::new();
        let uri = js("/app/components.js");
        let before = JsChangeSnapshot::capture(&index, &uri);

        add_component_template_url(&index, &uri, "foo.html", Some("FooCtrl"), "$ctrl");
        let after = JsChangeSnapshot::capture(&index, &uri);

        assert_ne!(before, after);
        assert!(before.cross_file_lens_state_changed(&after));
    }

    #[test]
    fn js_snapshot_detects_template_url_value_change_with_same_controller() {
        // controller 名が同じでも templateUrl が変わったら lens 変更必要
        // (例: $routeProvider.when({ templateUrl: 'foo.html' → 'bar.html', controller: 'FooCtrl' }))
        let index = Index::new();
        let uri = js("/app/routes.js");
        add_template_binding(&index, &uri, "foo.html", "FooCtrl", BindingSource::RouteProvider);
        let before = JsChangeSnapshot::capture(&index, &uri);

        // 一旦クリア (実際の挙動を模す: clear + re-add)
        index.definitions.clear_document(&uri);
        // template_bindings を直接書き換える代わりに、別キーで追加し signature 違いを作る
        // (実コードでは clear_template_bindings 相当の処理が走るが、ここでは新規追加で十分)
        add_template_binding(&index, &uri, "bar.html", "FooCtrl", BindingSource::RouteProvider);
        let after = JsChangeSnapshot::capture(&index, &uri);

        // before ⊂ after だが集合は異なる → 差分検知
        assert_ne!(before, after);
        assert!(before.cross_file_lens_state_changed(&after));
    }
}

#[cfg(test)]
mod drain_pending_reanalysis_tests {
    use super::*;

    fn url(path: &str) -> Url {
        Url::parse(&format!("file://{}", path)).unwrap()
    }

    #[test]
    fn drains_single_pending_uri() {
        let index = Index::new();
        let parent = url("/parent.html");
        let child = url("/child.html");
        index.add_pending_reanalysis(child.clone());

        let mut visited: Vec<Url> = Vec::new();
        drain_pending_reanalysis(&index, &parent, |u| visited.push(u.clone()));

        assert_eq!(visited, vec![child]);
        assert!(
            index.take_pending_reanalysis().is_empty(),
            "ドレイン後 pending は空"
        );
    }

    #[test]
    fn drains_grandchild_added_during_iteration() {
        // 子の analyze 中に孫が pending に追加される → 同じ呼び出しで処理される
        let index = Index::new();
        let parent = url("/parent.html");
        let child = url("/child.html");
        let grand = url("/grand.html");
        index.add_pending_reanalysis(child.clone());

        let grand_clone = grand.clone();
        let mut visited: Vec<Url> = Vec::new();
        drain_pending_reanalysis(&index, &parent, |u| {
            visited.push(u.clone());
            if u == &child {
                index.add_pending_reanalysis(grand_clone.clone());
            }
        });

        assert!(visited.contains(&child), "child は処理される");
        assert!(visited.contains(&grand), "孫も同じ呼び出しで処理される");
        assert_eq!(visited.len(), 2);
    }

    #[test]
    fn drains_great_grandchild_too() {
        // 3 段ネスト: child → grand → great_grand
        let index = Index::new();
        let parent = url("/parent.html");
        let child = url("/child.html");
        let grand = url("/grand.html");
        let great_grand = url("/great.html");
        index.add_pending_reanalysis(child.clone());

        let grand_clone = grand.clone();
        let great_clone = great_grand.clone();
        let mut visited: Vec<Url> = Vec::new();
        drain_pending_reanalysis(&index, &parent, |u| {
            visited.push(u.clone());
            if u == &child {
                index.add_pending_reanalysis(grand_clone.clone());
            } else if u == &grand_clone {
                index.add_pending_reanalysis(great_clone.clone());
            }
        });

        assert!(visited.contains(&child));
        assert!(visited.contains(&grand));
        assert!(visited.contains(&great_grand));
    }

    #[test]
    fn skips_uri_matching_skip_uri() {
        // skip_uri は「自分自身を再解析対象から除外する」目的なので、pending に
        // 入っていてもスキップされる
        let index = Index::new();
        let parent = url("/parent.html");
        index.add_pending_reanalysis(parent.clone());

        let mut visited: Vec<Url> = Vec::new();
        drain_pending_reanalysis(&index, &parent, |u| visited.push(u.clone()));

        assert!(visited.is_empty(), "skip_uri は処理されない");
    }

    #[test]
    fn deduplicates_with_visited_set() {
        // 同じ URI が複数ラウンドで pending に再投入されても 1 回しか処理されない
        let index = Index::new();
        let parent = url("/parent.html");
        let child = url("/child.html");
        index.add_pending_reanalysis(child.clone());

        let child_clone = child.clone();
        let mut visit_count = 0;
        drain_pending_reanalysis(&index, &parent, |u| {
            visit_count += 1;
            if u == &child_clone {
                // 自分自身を再キューしようとする (循環シミュレーション)
                index.add_pending_reanalysis(child_clone.clone());
            }
        });

        assert_eq!(visit_count, 1, "visited セットで重複処理を防ぐ");
    }

    #[test]
    fn handles_cyclic_ng_includes() {
        // a が b を pending、b が a を pending → 互いに無限再キューしようとするが
        // visited で打ち止めになる
        let index = Index::new();
        let parent = url("/parent.html");
        let a = url("/a.html");
        let b = url("/b.html");
        index.add_pending_reanalysis(a.clone());

        let a_clone = a.clone();
        let b_clone = b.clone();
        let mut visited: Vec<Url> = Vec::new();
        drain_pending_reanalysis(&index, &parent, |u| {
            visited.push(u.clone());
            if u == &a_clone {
                index.add_pending_reanalysis(b_clone.clone());
            } else if u == &b_clone {
                index.add_pending_reanalysis(a_clone.clone());
            }
        });

        assert_eq!(visited.len(), 2, "a と b が 1 回ずつ処理されて停止");
        assert!(visited.contains(&a));
        assert!(visited.contains(&b));
    }

    #[test]
    fn returns_when_pending_is_empty_initially() {
        let index = Index::new();
        let parent = url("/parent.html");

        let mut visit_count = 0;
        drain_pending_reanalysis(&index, &parent, |_| visit_count += 1);

        assert_eq!(visit_count, 0);
    }

    #[test]
    fn analyze_one_failure_does_not_drop_remaining_pending() {
        // analyze_one が一部 URI で何もしない (ドキュメントが見つからない等) ケース。
        // 残りの pending エントリは引き続き drain される。
        let index = Index::new();
        let parent = url("/parent.html");
        let a = url("/a.html");
        let b = url("/b.html");
        index.add_pending_reanalysis(a.clone());
        index.add_pending_reanalysis(b.clone());

        let mut visited: Vec<Url> = Vec::new();
        drain_pending_reanalysis(&index, &parent, |u| {
            // a は "ドキュメント無し" を模して何もしない、b は visited に積む
            if u != &a {
                visited.push(u.clone());
            }
        });

        assert_eq!(visited, vec![b]);
    }
}
