use std::sync::Arc;

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag, Position, Range, Url};
use tracing::debug;

use crate::config::DiagnosticsConfig;
use crate::index::Index;
use crate::model::SymbolKind;

/// 診断ハンドラー
pub struct DiagnosticsHandler {
    index: Arc<Index>,
    config: DiagnosticsConfig,
}

impl DiagnosticsHandler {
    pub fn new(index: Arc<Index>, config: DiagnosticsConfig) -> Self {
        Self { index, config }
    }

    /// 重要度文字列をDiagnosticSeverityに変換
    fn parse_severity(&self) -> DiagnosticSeverity {
        Self::severity_from_str(&self.config.severity)
    }

    /// 任意の重要度文字列を `DiagnosticSeverity` に変換
    fn severity_from_str(s: &str) -> DiagnosticSeverity {
        match s.to_lowercase().as_str() {
            "error" => DiagnosticSeverity::ERROR,
            "warning" => DiagnosticSeverity::WARNING,
            "hint" => DiagnosticSeverity::HINT,
            "information" | "info" => DiagnosticSeverity::INFORMATION,
            _ => DiagnosticSeverity::WARNING,
        }
    }

    /// `bindings_mismatch` 専用の severity (未指定なら全体 severity を継承)
    fn bindings_mismatch_severity(&self) -> DiagnosticSeverity {
        match self.config.bindings_mismatch_severity.as_deref() {
            Some(s) => Self::severity_from_str(s),
            None => self.parse_severity(),
        }
    }

    /// HTMLファイルの診断を実行
    pub fn diagnose_html(&self, uri: &Url) -> Vec<Diagnostic> {
        if !self.config.enabled {
            return Vec::new();
        }

        let mut diagnostics = Vec::new();

        // スコープ参照のチェック
        diagnostics.extend(self.check_scope_references(uri));

        // ローカル変数参照のチェック
        diagnostics.extend(self.check_local_variable_references(uri));

        // component bindings と HTML 属性の対応漏れチェック (#64)
        if self.config.component_bindings_mismatch {
            diagnostics.extend(self.check_component_bindings_mismatch_html(uri));
        }

        diagnostics
    }

    /// JSファイルの診断を実行
    pub fn diagnose_js(&self, uri: &Url) -> Vec<Diagnostic> {
        if !self.config.enabled {
            return Vec::new();
        }

        let mut diagnostics = Vec::new();

        // 未使用スコープ変数のチェック
        if self.config.unused_scope_variables {
            diagnostics.extend(self.check_unused_scope_variables(uri));
        }

        diagnostics
    }

    /// 未使用スコープ変数をチェックし警告生成
    /// DiagnosticTag::UNNECESSARY を付与（グレーアウト表示）
    fn check_unused_scope_variables(&self, uri: &Url) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let severity = self.parse_severity();

        // 指定JSファイルの全スコープ変数定義を取得
        let scope_defs = self.index.definitions.get_scope_definitions_for_js(uri);
        debug!(
            "check_unused_scope_variables: uri={}, scope_defs_count={}",
            uri,
            scope_defs.len()
        );

        for symbol in scope_defs {
            // シンボル名からプロパティ名を抽出
            // 形式: "ControllerName.$scope.propertyName" または "ControllerName.propertyName"
            let property_name = if let Some(idx) = symbol.name.find(".$scope.") {
                &symbol.name[idx + 8..] // ".$scope." の長さ = 8
            } else if let Some(idx) = symbol.name.rfind('.') {
                &symbol.name[idx + 1..]
            } else {
                continue;
            };

            // HTML内での参照があるかチェック
            let is_referenced_in_html =
                self.index.is_scope_variable_referenced(&symbol.name);

            // 他のJSファイル（他のコントローラー）からの参照があるかチェック
            // any_reference で短絡評価し、Vec 全件 clone を回避
            let is_referenced_in_other_js = self
                .index
                .definitions
                .any_reference(&symbol.name, |r| r.uri != *uri);

            debug!(
                "check_unused_scope_variables: symbol='{}', property='{}', html_ref={}, other_js_ref={}",
                symbol.name, property_name, is_referenced_in_html, is_referenced_in_other_js
            );

            // HTMLか他のJSで参照されていればスキップ
            if is_referenced_in_html || is_referenced_in_other_js {
                continue;
            }

            // 同一ファイル内での参照があるかチェック
            let is_referenced_in_same_js = self
                .index
                .definitions
                .any_reference(&symbol.name, |r| r.uri == *uri);

            // 警告メッセージを分岐: 完全に未参照か、同一ファイル内でのみ参照されているか
            let message = if is_referenced_in_same_js {
                format!(
                    "'{}' is defined but not used in HTML templates or other controllers",
                    property_name
                )
            } else {
                format!(
                    "'{}' is defined but never referenced",
                    property_name
                )
            };

            // 未使用の場合は警告を追加
            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position {
                        line: symbol.name_span.start_line,
                        character: symbol.name_span.start_col,
                    },
                    end: Position {
                        line: symbol.name_span.end_line,
                        character: symbol.name_span.end_col,
                    },
                },
                severity: Some(severity),
                code: None,
                code_description: None,
                source: Some("angularjs-lsp".to_string()),
                message,
                related_information: None,
                tags: Some(vec![DiagnosticTag::UNNECESSARY]),
                data: None,
            });
        }

        diagnostics
    }

    /// スコープ参照（vm.xxx, $scope.xxx）のチェック
    fn check_scope_references(&self, uri: &Url) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let severity = self.parse_severity();

        // 全スコープ参照を取得
        let references = self.index.html.get_html_scope_references(uri);

        for reference in references {
            // 動的式（配列アクセス）はスキップ
            if reference.property_path.contains('[') {
                continue;
            }

            // 文字列リテラル（'xxx' や "xxx"）はスキップ
            if reference.property_path.starts_with('\'')
                || reference.property_path.starts_with('"')
            {
                continue;
            }

            // $で始まるシンボル（$index, $first, $scope等）はスキップ
            if reference.property_path.starts_with('$') {
                continue;
            }

            // property_pathを解析
            // 形式: "alias.property" または "property"
            let (alias, property) = if reference.property_path.contains('.') {
                let parts: Vec<&str> =
                    reference.property_path.splitn(2, '.').collect();
                if parts.len() == 2 {
                    (Some(parts[0]), parts[1])
                } else {
                    (None, reference.property_path.as_str())
                }
            } else {
                (None, reference.property_path.as_str())
            };

            // ローカル変数として定義されているかチェック
            let var_name = alias.unwrap_or(property);
            if self
                .index
                .find_local_variable_definition(uri, var_name, reference.start_line)
                .is_some()
            {
                continue;
            }

            // フォームバインディングとして定義されているかチェック
            if self
                .index
                .find_form_binding_definition(uri, var_name, reference.start_line)
                .is_some()
            {
                continue;
            }

            // aliasがある場合はコントローラーを解決
            if let Some(alias_name) = alias {
                if let Some(controller_name) = self.index.resolve_controller_by_alias(
                    uri,
                    reference.start_line,
                    alias_name,
                ) {
                    // コントローラー自体がJS側で定義されているかチェック
                    // JSファイルがまだ解析されていない場合は警告を出さない
                    if !self.index.definitions.has_definition(&controller_name) {
                        continue;
                    }

                    // コントローラーの$scopeまたはthisにプロパティが定義されているかチェック
                    let scope_symbol =
                        format!("{}.$scope.{}", controller_name, property);
                    let this_symbol =
                        format!("{}.{}", controller_name, property);
                    if self.index.definitions.has_definition(&scope_symbol)
                        || self.index.definitions.has_definition(&this_symbol)
                    {
                        continue;
                    }

                    // $rootScopeも確認
                    if !self
                        .index
                        .definitions
                        .find_root_scope_definitions_by_property(property)
                        .is_empty()
                    {
                        continue;
                    }

                    // ng-model="vm.foo" のような暗黙的 \$scope 書き込みでも
                    // 定義済みとして扱う (controller 側で `$scope.foo = ...` を
                    // 書かなくても AngularJS が \$scope に property を作るため)
                    if self.index.has_ng_model_implicit_def(uri, &controller_name, property) {
                        continue;
                    }

                    // 定義が見つからない場合は警告
                    diagnostics.push(Diagnostic {
                        range: Range {
                            start: Position {
                                line: reference.start_line,
                                character: reference.start_col,
                            },
                            end: Position {
                                line: reference.end_line,
                                character: reference.end_col,
                            },
                        },
                        severity: Some(severity),
                        code: None,
                        code_description: None,
                        source: Some("angularjs-lsp".to_string()),
                        message: format!(
                            "Property '{}' is not defined in controller '{}'",
                            property, controller_name
                        ),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
                // aliasが解決できない場合（コントローラーが見つからない）は警告を出さない
            } else {
                // aliasがない場合（直接プロパティアクセス）

                // まず、propertyがコントローラーエイリアスとして定義されているかチェック
                if self
                    .index
                    .resolve_controller_by_alias(uri, reference.start_line, property)
                    .is_some()
                {
                    continue;
                }

                // すべてのコントローラーを取得して$scopeプロパティをチェック
                let controllers = self
                    .index
                    .resolve_controllers_for_html(uri, reference.start_line);

                let mut found = false;
                let mut any_controller_defined = false;

                // いずれかのコントローラーで定義されているか確認
                for controller_name in &controllers {
                    if !self.index.definitions.has_definition(controller_name) {
                        continue;
                    }
                    any_controller_defined = true;

                    let scope_symbol =
                        format!("{}.$scope.{}", controller_name, property);
                    let this_symbol =
                        format!("{}.{}", controller_name, property);
                    if self.index.definitions.has_definition(&scope_symbol)
                        || self.index.definitions.has_definition(&this_symbol)
                    {
                        found = true;
                        break;
                    }
                }

                // $rootScopeも確認
                if !found
                    && !self
                        .index
                        .definitions
                        .find_root_scope_definitions_by_property(property)
                        .is_empty()
                {
                    found = true;
                }

                // ng-model 経由の暗黙的 \$scope 書き込みもチェック
                // (アクティブな controller のいずれかに ng-model のターゲットが
                //  あれば定義済みとみなす)
                if !found {
                    for ctrl in &controllers {
                        if self.index.has_ng_model_implicit_def(uri, ctrl, property) {
                            found = true;
                            break;
                        }
                    }
                }

                // コントローラーのJS定義が存在する場合のみ警告
                if !found && any_controller_defined {
                    diagnostics.push(Diagnostic {
                        range: Range {
                            start: Position {
                                line: reference.start_line,
                                character: reference.start_col,
                            },
                            end: Position {
                                line: reference.end_line,
                                character: reference.end_col,
                            },
                        },
                        severity: Some(severity),
                        code: None,
                        code_description: None,
                        source: Some("angularjs-lsp".to_string()),
                        message: format!(
                            "Property '{}' is not defined in scope",
                            property
                        ),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
            }
        }

        diagnostics
    }

    /// component bindings と HTML 属性の対応漏れをチェック (#64)
    ///
    /// 2 方向で照合する:
    /// 1. **HTML 側 → JS**: `<user-card foo="...">` の各属性を kebab→camel 変換し、
    ///    対応するコンポーネントの bindings に存在しないなら警告 (typo / 不要属性)
    /// 2. **JS 側 → HTML**: 必須 bindings (`<` / `=` / `@` で `?` プレフィックス
    ///    無し) が HTML で指定されていないなら警告。
    ///    `&` (callback) は `require_callback_bindings` 設定で制御。
    ///
    /// 除外:
    /// - 標準 HTML 属性 (`class`, `id`, `style`, ...) - `STANDARD_HTML_ATTRIBUTES`
    /// - AngularJS ビルトインディレクティブ (`ng-*`, `data-ng-*`)
    /// - DI / 互換のためによく付く `data-*` / `aria-*`
    pub fn check_component_bindings_mismatch_html(&self, uri: &Url) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let severity = self.bindings_mismatch_severity();

        let usages = self.index.html.get_component_usages_for_uri(uri);
        for usage in usages {
            // コンポーネント定義が無いものは対象外 (custom directive 等)
            if !self
                .index
                .definitions
                .has_definition_of_kind(&usage.component_name, SymbolKind::Component)
            {
                continue;
            }

            let bindings = self
                .index
                .definitions
                .get_component_bindings(&usage.component_name);
            // bindings が登録されていない場合 (空の bindings) は対応漏れの判定不能
            if bindings.is_empty() {
                continue;
            }

            // bindings 名 (camelCase, prefix を取り除いたもの) を集める
            let prefix = format!("{}.", usage.component_name);
            let mut bindings_meta: Vec<BindingMeta> = Vec::new();
            for b in &bindings {
                let Some(local) = b.name.strip_prefix(&prefix) else {
                    continue;
                };
                let (kind, optional) = parse_binding_type(b.docs.as_deref());
                bindings_meta.push(BindingMeta {
                    name: local.to_string(),
                    kind,
                    optional,
                });
            }
            if bindings_meta.is_empty() {
                continue;
            }

            // ----- 1. HTML 側 → JS: 不正な属性 (typo / 不要) を検出 -----
            for attr in &usage.attributes {
                if should_skip_attribute(&attr.name) {
                    continue;
                }
                // bindings に同名のものがあるかチェック (camelCase で照合)
                let matches_binding = bindings_meta
                    .iter()
                    .any(|b| b.name == attr.camel_name);
                if matches_binding {
                    continue;
                }
                // この属性が他のディレクティブとして定義されているなら警告しない
                // (例: `<user-card my-other-directive>` で my-other-directive が
                //  別途 .directive() として登録されている場合)
                if self
                    .index
                    .definitions
                    .has_definition_of_kind(&attr.camel_name, SymbolKind::Directive)
                {
                    continue;
                }

                diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position {
                            line: attr.start_line,
                            character: attr.start_col,
                        },
                        end: Position {
                            line: attr.end_line,
                            character: attr.end_col,
                        },
                    },
                    severity: Some(severity),
                    code: None,
                    code_description: None,
                    source: Some("angularjs-lsp".to_string()),
                    message: format!(
                        "Attribute '{}' does not match any binding of component '{}'",
                        attr.name, usage.component_name
                    ),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }

            // ----- 2. JS 側 → HTML: 必須 bindings が漏れていないかチェック -----
            //
            // 既に HTML に書かれている属性 (camelCase) の集合を作る
            let provided: std::collections::HashSet<String> = usage
                .attributes
                .iter()
                .map(|a| a.camel_name.clone())
                .collect();

            // 漏れた binding 名を集めて 1 件にまとめる (#79 review #4: 同じ要素に
            // 対する N 件の重複警告を避ける)
            let mut missing: Vec<&str> = Vec::new();
            for b in &bindings_meta {
                if b.optional {
                    continue;
                }
                if matches!(b.kind, BindingKind::Callback)
                    && !self.config.require_callback_bindings
                {
                    continue;
                }
                if matches!(b.kind, BindingKind::Unknown) {
                    // bindings 値の型を判定できないものはスキップ (false positive 防止)
                    continue;
                }
                if provided.contains(&b.name) {
                    continue;
                }
                missing.push(&b.name);
            }

            if !missing.is_empty() {
                let message = if missing.len() == 1 {
                    format!(
                        "Missing required binding '{}' on component '{}'",
                        missing[0], usage.component_name
                    )
                } else {
                    let list = missing
                        .iter()
                        .map(|m| format!("'{}'", m))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "Missing required bindings on component '{}': {}",
                        usage.component_name, list
                    )
                };
                // 要素名トークンの位置に「漏れ」警告を 1 件だけ出す
                diagnostics.push(Diagnostic {
                    range: Range {
                        start: Position {
                            line: usage.element_start_line,
                            character: usage.element_start_col,
                        },
                        end: Position {
                            line: usage.element_end_line,
                            character: usage.element_end_col,
                        },
                    },
                    severity: Some(severity),
                    code: None,
                    code_description: None,
                    source: Some("angularjs-lsp".to_string()),
                    message,
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }
        }

        diagnostics
    }

    /// ローカル変数参照のチェック
    fn check_local_variable_references(&self, uri: &Url) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let severity = self.parse_severity();

        // 全ローカル変数参照を取得
        let references = self
            .index
            .html
            .get_all_local_variable_references_for_uri(uri);

        for reference in references {
            // $で始まるシンボル（$index, $first等）はスキップ
            if reference.variable_name.starts_with('$') {
                continue;
            }

            // 定義があるかチェック
            if self
                .index
                .find_local_variable_definition(
                    uri,
                    &reference.variable_name,
                    reference.start_line,
                )
                .is_some()
            {
                continue;
            }

            // 定義が見つからない場合は警告
            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position {
                        line: reference.start_line,
                        character: reference.start_col,
                    },
                    end: Position {
                        line: reference.end_line,
                        character: reference.end_col,
                    },
                },
                severity: Some(severity),
                code: None,
                code_description: None,
                source: Some("angularjs-lsp".to_string()),
                message: format!(
                    "Local variable '{}' is not defined in scope",
                    reference.variable_name
                ),
                related_information: None,
                tags: None,
                data: None,
            });
        }

        diagnostics
    }
}

/// component binding の種類 (`<` / `=` / `@` / `&` / 不明)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BindingKind {
    /// `<` (one-way)
    OneWay,
    /// `=` (two-way)
    TwoWay,
    /// `@` (string)
    String,
    /// `&` (callback)
    Callback,
    /// 値が文字列でなく解析不能だった場合
    Unknown,
}

/// 単一バインディングの解析結果
#[derive(Debug, Clone)]
struct BindingMeta {
    name: String,
    kind: BindingKind,
    optional: bool,
}

/// `extract_component_bindings` が `Symbol.docs` に書き込む文字列
/// (`"Component binding: <"` / `"Component binding: ?<"` 等) を
/// `(BindingKind, optional)` にパースする。
///
/// 接頭辞は `crate::model::COMPONENT_BINDING_DOCS_PREFIX` で定数化されており、
/// 書き手 (analyzer) と読み手 (この関数) が同じ定数を共有している。
///
/// docs が `None` または期待形式でない場合は `Unknown` / `false` を返す
/// (false positive を出さないための保守的フォールバック)。
fn parse_binding_type(docs: Option<&str>) -> (BindingKind, bool) {
    let Some(value) = docs.and_then(|s| s.strip_prefix(crate::model::COMPONENT_BINDING_DOCS_PREFIX))
    else {
        return (BindingKind::Unknown, false);
    };
    let value = value.trim();
    // `?` プレフィックス (optional) を剥がす
    let (optional, rest) = match value.strip_prefix('?') {
        Some(r) => (true, r),
        None => (false, value),
    };
    // 先頭の symbol だけ見る (`&onSelected` のような alias は無視)
    let kind = match rest.chars().next() {
        Some('<') => BindingKind::OneWay,
        Some('=') => BindingKind::TwoWay,
        Some('@') => BindingKind::String,
        Some('&') => BindingKind::Callback,
        _ => BindingKind::Unknown,
    };
    (kind, optional)
}

/// この属性は component bindings 診断の対象外か
///
/// - 標準 HTML 属性 (`class`, `id`, `style` 等) は除外
/// - AngularJS ビルトイン (`ng-*`, `data-ng-*`) は除外
/// - `aria-*`, `data-*` も除外 (バインディング名と衝突しない)
///
/// 標準 HTML 属性の集合は `analyzer::html::directives::is_standard_html_attribute`
/// に集約済み (#79 review: 重複定義を避けるためここでは独自リストを持たない)。
fn should_skip_attribute(attr_name: &str) -> bool {
    let lower = attr_name.to_ascii_lowercase();

    // ng-*, data-ng-* はビルトイン
    if crate::analyzer::html::directives::is_ng_directive(&lower) {
        return true;
    }

    // aria-* / data-* (data-ng-* 以外でも) は標準パターン
    if lower.starts_with("aria-") || lower.starts_with("data-") {
        return true;
    }

    // 標準 HTML 属性 (analyzer 側と同じ集合を共有)
    crate::analyzer::html::directives::is_standard_html_attribute(&lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_binding_type_oneway() {
        assert_eq!(
            parse_binding_type(Some("Component binding: <")),
            (BindingKind::OneWay, false)
        );
    }

    #[test]
    fn parse_binding_type_optional_oneway() {
        assert_eq!(
            parse_binding_type(Some("Component binding: ?<")),
            (BindingKind::OneWay, true)
        );
    }

    #[test]
    fn parse_binding_type_callback_with_alias() {
        // `&onSelected` のように alias 指定がついていても先頭シンボルだけ拾う
        assert_eq!(
            parse_binding_type(Some("Component binding: &onSelected")),
            (BindingKind::Callback, false)
        );
    }

    #[test]
    fn parse_binding_type_optional_callback_with_alias() {
        assert_eq!(
            parse_binding_type(Some("Component binding: ?&onSelected")),
            (BindingKind::Callback, true)
        );
    }

    #[test]
    fn parse_binding_type_string_and_twoway() {
        assert_eq!(
            parse_binding_type(Some("Component binding: @")),
            (BindingKind::String, false)
        );
        assert_eq!(
            parse_binding_type(Some("Component binding: =")),
            (BindingKind::TwoWay, false)
        );
        assert_eq!(
            parse_binding_type(Some("Component binding: ?=")),
            (BindingKind::TwoWay, true)
        );
    }

    #[test]
    fn parse_binding_type_none_or_unknown() {
        assert_eq!(parse_binding_type(None), (BindingKind::Unknown, false));
        assert_eq!(
            parse_binding_type(Some("not a binding doc")),
            (BindingKind::Unknown, false)
        );
        // 空 / 未認識記号
        assert_eq!(
            parse_binding_type(Some("Component binding: ")),
            (BindingKind::Unknown, false)
        );
        assert_eq!(
            parse_binding_type(Some("Component binding: ~")),
            (BindingKind::Unknown, false)
        );
    }

    #[test]
    fn should_skip_attribute_standard() {
        assert!(should_skip_attribute("class"));
        assert!(should_skip_attribute("id"));
        assert!(should_skip_attribute("style"));
        assert!(should_skip_attribute("aria-label"));
        assert!(should_skip_attribute("data-foo"));
        assert!(should_skip_attribute("ng-if"));
    }

    #[test]
    fn should_skip_attribute_not_standard() {
        assert!(!should_skip_attribute("user"));
        assert!(!should_skip_attribute("on-select"));
        assert!(!should_skip_attribute("my-binding"));
    }
}
