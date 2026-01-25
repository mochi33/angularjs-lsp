use std::sync::Arc;

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range, Url};

use crate::config::DiagnosticsConfig;
use crate::index::SymbolIndex;

/// 診断ハンドラー
pub struct DiagnosticsHandler {
    index: Arc<SymbolIndex>,
    config: DiagnosticsConfig,
}

impl DiagnosticsHandler {
    pub fn new(index: Arc<SymbolIndex>, config: DiagnosticsConfig) -> Self {
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

    /// スコープ参照（vm.xxx, $scope.xxx）のチェック
    fn check_scope_references(&self, uri: &Url) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let severity = self.parse_severity();

        // 全スコープ参照を取得
        let references = self.index.get_html_scope_references(uri);

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
                let parts: Vec<&str> = reference.property_path.splitn(2, '.').collect();
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
                    // コントローラーの$scopeまたはthisにプロパティが定義されているかチェック
                    let scope_symbol = format!("{}.$scope.{}", controller_name, property);
                    let this_symbol = format!("{}.{}", controller_name, property);
                    if self.index.has_definition(&scope_symbol)
                        || self.index.has_definition(&this_symbol)
                    {
                        continue;
                    }

                    // $rootScopeも確認
                    if !self
                        .index
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
                } else {
                    // aliasが解決できない場合（コントローラーが見つからない）
                    // ローカル変数やフォームバインディングではない場合のみ警告
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
                            "Variable '{}' is not defined in scope",
                            alias_name
                        ),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
            } else {
                // aliasがない場合（直接プロパティアクセス）
                // すべてのコントローラーを取得して$scopeプロパティをチェック
                let controllers =
                    self.index.resolve_controllers_for_html(uri, reference.start_line);

                let mut found = false;

                // いずれかのコントローラーで定義されているか確認
                for controller_name in &controllers {
                    let scope_symbol = format!("{}.$scope.{}", controller_name, property);
                    if self.index.has_definition(&scope_symbol) {
                        found = true;
                        break;
                    }
                }

                // $rootScopeも確認
                if !found
                    && !self
                        .index
                        .find_root_scope_definitions_by_property(property)
                        .is_empty()
                {
                    found = true;
                }

                if !found && !controllers.is_empty() {
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
                        message: format!("Property '{}' is not defined in scope", property),
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
        let references = self.index.get_all_local_variable_references_for_uri(uri);

        for reference in references {
            // $で始まるシンボル（$index, $first等）はスキップ
            if reference.variable_name.starts_with('$') {
                continue;
            }

            // 定義があるかチェック
            if self
                .index
                .find_local_variable_definition(uri, &reference.variable_name, reference.start_line)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_severity() {
        let index = Arc::new(SymbolIndex::new());

        let handler = DiagnosticsHandler::new(
            Arc::clone(&index),
            DiagnosticsConfig {
                enabled: true,
                severity: "error".to_string(),
            },
        );
        assert_eq!(handler.parse_severity(), DiagnosticSeverity::ERROR);

        let handler = DiagnosticsHandler::new(
            Arc::clone(&index),
            DiagnosticsConfig {
                enabled: true,
                severity: "warning".to_string(),
            },
        );
        assert_eq!(handler.parse_severity(), DiagnosticSeverity::WARNING);

        let handler = DiagnosticsHandler::new(
            Arc::clone(&index),
            DiagnosticsConfig {
                enabled: true,
                severity: "hint".to_string(),
            },
        );
        assert_eq!(handler.parse_severity(), DiagnosticSeverity::HINT);

        let handler = DiagnosticsHandler::new(
            Arc::clone(&index),
            DiagnosticsConfig {
                enabled: true,
                severity: "information".to_string(),
            },
        );
        assert_eq!(handler.parse_severity(), DiagnosticSeverity::INFORMATION);
    }

    #[test]
    fn test_disabled_diagnostics() {
        let index = Arc::new(SymbolIndex::new());
        let handler = DiagnosticsHandler::new(
            Arc::clone(&index),
            DiagnosticsConfig {
                enabled: false,
                severity: "warning".to_string(),
            },
        );

        let uri = Url::parse("file:///test.html").unwrap();
        let diagnostics = handler.diagnose_html(&uri);
        assert!(diagnostics.is_empty());
    }
}
