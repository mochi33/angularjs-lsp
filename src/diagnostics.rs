use std::sync::Arc;

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag, Position, Range, Url};
use tracing::debug;

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
        let scope_defs = self.index.get_scope_definitions_for_js(uri);
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

            // 参照があるかチェック
            let is_referenced = self.index.is_scope_variable_referenced(&symbol.name);
            debug!(
                "check_unused_scope_variables: symbol='{}', property='{}', is_referenced={}",
                symbol.name, property_name, is_referenced
            );

            if is_referenced {
                continue;
            }

            // 未使用の場合は警告を追加
            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position {
                        line: symbol.name_start_line,
                        character: symbol.name_start_col,
                    },
                    end: Position {
                        line: symbol.name_end_line,
                        character: symbol.name_end_col,
                    },
                },
                severity: Some(severity),
                code: None,
                code_description: None,
                source: Some("angularjs-lsp".to_string()),
                message: format!(
                    "'{}' is defined but never used in HTML templates",
                    property_name
                ),
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
                unused_scope_variables: true,
            },
        );
        assert_eq!(handler.parse_severity(), DiagnosticSeverity::ERROR);

        let handler = DiagnosticsHandler::new(
            Arc::clone(&index),
            DiagnosticsConfig {
                enabled: true,
                severity: "warning".to_string(),
                unused_scope_variables: true,
            },
        );
        assert_eq!(handler.parse_severity(), DiagnosticSeverity::WARNING);

        let handler = DiagnosticsHandler::new(
            Arc::clone(&index),
            DiagnosticsConfig {
                enabled: true,
                severity: "hint".to_string(),
                unused_scope_variables: true,
            },
        );
        assert_eq!(handler.parse_severity(), DiagnosticSeverity::HINT);

        let handler = DiagnosticsHandler::new(
            Arc::clone(&index),
            DiagnosticsConfig {
                enabled: true,
                severity: "information".to_string(),
                unused_scope_variables: true,
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
                unused_scope_variables: true,
            },
        );

        let uri = Url::parse("file:///test.html").unwrap();
        let diagnostics = handler.diagnose_html(&uri);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_disabled_unused_scope_variables() {
        let index = Arc::new(SymbolIndex::new());
        let handler = DiagnosticsHandler::new(
            Arc::clone(&index),
            DiagnosticsConfig {
                enabled: true,
                severity: "warning".to_string(),
                unused_scope_variables: false,
            },
        );

        let uri = Url::parse("file:///test.js").unwrap();
        let diagnostics = handler.diagnose_js(&uri);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn test_unused_scope_variable_detection() {
        use crate::analyzer::AngularJsAnalyzer;
        use crate::analyzer::HtmlAngularJsAnalyzer;

        let index = Arc::new(SymbolIndex::new());
        let js_analyzer = Arc::new(AngularJsAnalyzer::new(Arc::clone(&index)));
        let html_analyzer = HtmlAngularJsAnalyzer::new(Arc::clone(&index), Arc::clone(&js_analyzer));

        // JSファイルでスコープ変数を定義
        let js_uri = Url::parse("file:///app.js").unwrap();
        let js_source = r#"
angular.module('app')
.controller('TestCtrl', ['$scope', function($scope) {
    $scope.usedVar = 'used';
    $scope.unusedVar = 'unused';
}]);
"#;
        js_analyzer.analyze_document(&js_uri, js_source);

        // HTMLで一部の変数のみ参照
        let html_uri = Url::parse("file:///index.html").unwrap();
        let html = r#"
<div ng-controller="TestCtrl">
    <span>{{ usedVar }}</span>
</div>
"#;
        html_analyzer.analyze_document(&html_uri, html);

        // is_scope_variable_referenced の動作確認
        assert!(
            index.is_scope_variable_referenced("TestCtrl.$scope.usedVar"),
            "usedVar should be referenced in HTML"
        );
        assert!(
            !index.is_scope_variable_referenced("TestCtrl.$scope.unusedVar"),
            "unusedVar should NOT be referenced"
        );

        // 診断実行
        let handler = DiagnosticsHandler::new(
            Arc::clone(&index),
            DiagnosticsConfig {
                enabled: true,
                severity: "warning".to_string(),
                unused_scope_variables: true,
            },
        );

        let diagnostics = handler.diagnose_js(&js_uri);

        // unusedVar のみ警告されるべき
        assert_eq!(diagnostics.len(), 1, "Should have exactly 1 warning for unusedVar");
        assert!(
            diagnostics[0].message.contains("unusedVar"),
            "Warning should mention unusedVar"
        );
    }

    #[test]
    fn test_unused_scope_variable_controller_as_syntax() {
        use crate::analyzer::AngularJsAnalyzer;
        use crate::analyzer::HtmlAngularJsAnalyzer;

        let index = Arc::new(SymbolIndex::new());
        let js_analyzer = Arc::new(AngularJsAnalyzer::new(Arc::clone(&index)));
        let html_analyzer = HtmlAngularJsAnalyzer::new(Arc::clone(&index), Arc::clone(&js_analyzer));

        // JSファイルで controller as 構文用の this.xxx を定義
        let js_uri = Url::parse("file:///app.js").unwrap();
        let js_source = r#"
angular.module('app')
.controller('TestCtrl', ['$scope', function($scope) {
    this.usedMethod = function() {};
    this.unusedMethod = function() {};
}]);
"#;
        js_analyzer.analyze_document(&js_uri, js_source);

        // HTMLで controller as 構文で一部のみ参照
        let html_uri = Url::parse("file:///index.html").unwrap();
        let html = r#"
<div ng-controller="TestCtrl as vm">
    <button ng-click="vm.usedMethod()">Click</button>
</div>
"#;
        html_analyzer.analyze_document(&html_uri, html);

        // is_scope_variable_referenced の動作確認
        eprintln!("Testing controller as syntax references:");
        eprintln!("  usedMethod referenced: {}", index.is_scope_variable_referenced("TestCtrl.usedMethod"));
        eprintln!("  unusedMethod referenced: {}", index.is_scope_variable_referenced("TestCtrl.unusedMethod"));

        // controller as 構文の場合、HTML参照はalias経由で解決される
        let used_refs = index.get_all_references("TestCtrl.usedMethod");
        eprintln!("  usedMethod all_refs: {:?}", used_refs.iter().map(|r| (r.start_line, r.start_col)).collect::<Vec<_>>());

        assert!(
            index.is_scope_variable_referenced("TestCtrl.usedMethod"),
            "usedMethod should be referenced via controller as syntax"
        );
    }

    #[test]
    fn test_unused_scope_variable_with_template_binding() {
        use crate::analyzer::AngularJsAnalyzer;
        use crate::analyzer::HtmlAngularJsAnalyzer;

        let index = Arc::new(SymbolIndex::new());
        let js_analyzer = Arc::new(AngularJsAnalyzer::new(Arc::clone(&index)));
        let html_analyzer = HtmlAngularJsAnalyzer::new(Arc::clone(&index), Arc::clone(&js_analyzer));

        // JSファイルで $routeProvider を使用
        let js_uri = Url::parse("file:///app.js").unwrap();
        let js_source = r#"
angular.module('app')
.config(['$routeProvider', function($routeProvider) {
    $routeProvider.when('/users', {
        templateUrl: 'users.html',
        controller: 'UserCtrl'
    });
}])
.controller('UserCtrl', ['$scope', function($scope) {
    $scope.users = [];
    $scope.unusedVar = 'test';
}]);
"#;
        js_analyzer.analyze_document(&js_uri, js_source);

        // HTMLファイル（ng-controller なし、$routeProvider 経由で読み込まれる）
        let html_uri = Url::parse("file:///users.html").unwrap();
        let html = r#"
<ul>
    <li ng-repeat="user in users">{{ user.name }}</li>
</ul>
"#;
        html_analyzer.analyze_document(&html_uri, html);

        // template binding が正しく解決されているか確認
        let controllers = index.resolve_controllers_for_html(&html_uri, 2);
        eprintln!("Controllers for users.html: {:?}", controllers);

        // users への参照があるか確認
        let users_refs = index.get_all_references("UserCtrl.$scope.users");
        eprintln!("users refs: {:?}", users_refs.iter().map(|r| (r.start_line, r.start_col)).collect::<Vec<_>>());

        assert!(
            index.is_scope_variable_referenced("UserCtrl.$scope.users"),
            "users should be referenced in template binding"
        );
    }
}
