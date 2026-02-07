use std::sync::Arc;

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag, Position, Range, Url};
use tracing::debug;

use crate::config::DiagnosticsConfig;
use crate::index::Index;

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
        match self.config.severity.to_lowercase().as_str() {
            "error" => DiagnosticSeverity::ERROR,
            "warning" => DiagnosticSeverity::WARNING,
            "hint" => DiagnosticSeverity::HINT,
            "information" | "info" => DiagnosticSeverity::INFORMATION,
            _ => DiagnosticSeverity::WARNING,
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
            let js_refs = self.index.definitions.get_references(&symbol.name);
            let is_referenced_in_other_js = js_refs.iter().any(|r| r.uri != *uri);

            debug!(
                "check_unused_scope_variables: symbol='{}', property='{}', html_ref={}, other_js_ref={}",
                symbol.name, property_name, is_referenced_in_html, is_referenced_in_other_js
            );

            // HTMLか他のJSで参照されていればスキップ
            if is_referenced_in_html || is_referenced_in_other_js {
                continue;
            }

            // 同一ファイル内での参照があるかチェック
            let is_referenced_in_same_js = js_refs.iter().any(|r| r.uri == *uri);

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
