use std::collections::HashSet;
use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::Index;
use crate::model::SymbolKind;
use crate::util::camel_to_kebab;

/// HTML補完候補のラベル重複を避けつつ追加するヘルパー
fn push_unique(items: &mut Vec<CompletionItem>, seen: &mut HashSet<String>, item: CompletionItem) {
    if seen.insert(item.label.clone()) {
        items.push(item);
    }
}

pub struct CompletionHandler {
    index: Arc<Index>,
}

impl CompletionHandler {
    pub fn new(index: Arc<Index>) -> Self {
        Self { index }
    }

    /// サービスプレフィックスに基づいて補完候補を返す
    /// service_prefix: "ServiceName" の場合、"ServiceName.xxx" のメソッドのみ返す
    /// service_prefix: "$scope" の場合、current_controller の $scope プロパティを返す
    /// injected_services: 現在のコントローラーでDIされているサービス（優先表示）
    pub fn complete_with_context(
        &self,
        service_prefix: Option<&str>,
        current_controller: Option<&str>,
        injected_services: &[String],
    ) -> Option<CompletionResponse> {
        let definitions = self.index.definitions.get_all_definitions();

        let items: Vec<CompletionItem> = if let Some(prefix) = service_prefix {
            if prefix == "$rootScope" {
                // $rootScope. の場合、全モジュールの $rootScope プロパティを返す
                let mut seen_props: HashSet<String> = HashSet::new();
                let mut items: Vec<CompletionItem> = Vec::new();

                for symbol in definitions.iter().filter(|s| {
                    s.kind == SymbolKind::RootScopeProperty
                        || s.kind == SymbolKind::RootScopeMethod
                }) {
                    // "ModuleName.$rootScope.propertyName" から "propertyName" を抽出
                    if let Some(idx) = symbol.name.find(".$rootScope.") {
                        let module_name = &symbol.name[..idx];
                        let prop_name =
                            symbol.name[idx + ".$rootScope.".len()..].to_string();

                        // 重複チェック
                        if seen_props.contains(&prop_name) {
                            continue;
                        }
                        seen_props.insert(prop_name.clone());

                        let (item_kind, type_str) =
                            if symbol.kind == SymbolKind::RootScopeMethod {
                                (CompletionItemKind::FUNCTION, "method")
                            } else {
                                (CompletionItemKind::PROPERTY, "property")
                            };

                        items.push(CompletionItem {
                            label: prop_name,
                            kind: Some(item_kind),
                            detail: Some(format!(
                                "{} ($rootScope {})",
                                module_name, type_str
                            )),
                            documentation: symbol.docs.clone().map(|docs| {
                                Documentation::MarkupContent(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: docs,
                                })
                            }),
                            ..Default::default()
                        });
                    }
                }

                items
            } else if prefix == "$scope" {
                // $scope. の場合、現在のコントローラーの $scope プロパティのみを返す
                let mut seen_props: HashSet<String> = HashSet::new();
                let mut items: Vec<CompletionItem> = Vec::new();

                // 定義からプロパティを収集
                for symbol in definitions.iter().filter(|s| {
                    s.kind == SymbolKind::ScopeProperty || s.kind == SymbolKind::ScopeMethod
                }) {
                    // "ControllerName.$scope.propertyName" から "propertyName" を抽出
                    let parts: Vec<&str> = symbol.name.split(".$scope.").collect();
                    if parts.len() == 2 {
                        let controller_name = parts[0];
                        let prop_name = parts[1].to_string();

                        // 現在のコントローラーが指定されている場合、それ以外はスキップ
                        if let Some(current) = current_controller {
                            if controller_name != current {
                                continue;
                            }
                        }

                        // 重複チェック
                        if seen_props.contains(&prop_name) {
                            continue;
                        }
                        seen_props.insert(prop_name.clone());

                        // ScopeMethod の場合は FUNCTION、それ以外は PROPERTY
                        let (item_kind, type_str) =
                            if symbol.kind == SymbolKind::ScopeMethod {
                                (CompletionItemKind::FUNCTION, "function")
                            } else {
                                (CompletionItemKind::PROPERTY, "property")
                            };

                        items.push(CompletionItem {
                            label: prop_name,
                            kind: Some(item_kind),
                            detail: Some(format!(
                                "{} (scope {})",
                                controller_name, type_str
                            )),
                            documentation: symbol.docs.clone().map(|docs| {
                                Documentation::MarkupContent(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: docs,
                                })
                            }),
                            ..Default::default()
                        });
                    }
                }

                // 参照のみ（定義がない）のプロパティも収集
                // ただし、既存の定義のプレフィックスはスキップ（入力中の不完全な識別子を除外）
                for ref_name in self.index.definitions.get_reference_only_names() {
                    if ref_name.contains(".$scope.") {
                        let parts: Vec<&str> = ref_name.split(".$scope.").collect();
                        if parts.len() == 2 {
                            let controller_name = parts[0];
                            let prop_name = parts[1].to_string();

                            // 現在のコントローラーが指定されている場合、それ以外はスキップ
                            if let Some(current) = current_controller {
                                if controller_name != current {
                                    continue;
                                }
                            }

                            // 重複チェック（定義と重複する場合はスキップ）
                            if seen_props.contains(&prop_name) {
                                continue;
                            }

                            // 既存の「定義」のプレフィックスかどうかチェック
                            let is_prefix_of_definition =
                                seen_props.iter().any(|existing| {
                                    existing.starts_with(&prop_name)
                                        && existing != &prop_name
                                });
                            if is_prefix_of_definition {
                                continue;
                            }

                            seen_props.insert(prop_name.clone());

                            items.push(CompletionItem {
                                label: prop_name,
                                kind: Some(CompletionItemKind::PROPERTY),
                                detail: Some(format!(
                                    "{} (scope property, reference only)",
                                    controller_name
                                )),
                                ..Default::default()
                            });
                        }
                    }
                }

                items
            } else {
                // サービス名が指定された場合、そのサービスのメソッドのみを返す
                let method_prefix = format!("{}.", prefix);
                definitions
                    .into_iter()
                    .filter(|s| s.name.starts_with(&method_prefix))
                    .map(|symbol| {
                        // "ServiceName.methodName" から "methodName" だけを抽出
                        let method_name = symbol
                            .name
                            .strip_prefix(&method_prefix)
                            .unwrap_or(&symbol.name)
                            .to_string();

                        CompletionItem {
                            label: method_name,
                            kind: Some(CompletionItemKind::METHOD),
                            detail: Some(format!(
                                "{} ({})",
                                prefix,
                                symbol.kind.as_str()
                            )),
                            documentation: symbol.docs.map(|docs| {
                                Documentation::MarkupContent(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: docs,
                                })
                            }),
                            ..Default::default()
                        }
                    })
                    .collect()
            }
        } else {
            // 通常の補完: 全シンボルを返す（メソッドと$scopeプロパティ/メソッドは除外）
            // 現在のコントローラー自身も除外する
            // DIされているサービスは優先表示（sort_textで制御）
            let injected_set: HashSet<&str> =
                injected_services.iter().map(|s| s.as_str()).collect();

            definitions
                .into_iter()
                .filter(|s| {
                    s.kind != SymbolKind::Method
                        && s.kind != SymbolKind::ScopeProperty
                        && s.kind != SymbolKind::ScopeMethod
                        && s.kind != SymbolKind::Controller
                })
                .map(|symbol| {
                    let kind = self.symbol_kind_to_completion_kind(symbol.kind);
                    let is_injected = injected_set.contains(symbol.name.as_str());
                    let detail = if is_injected {
                        format!("{} (injected)", symbol.kind.as_str())
                    } else {
                        symbol.kind.as_str().to_string()
                    };
                    // DIされているサービスは "0_" プレフィックス、それ以外は "1_" で並べ替え
                    let sort_text = if is_injected {
                        format!("0_{}", symbol.name)
                    } else {
                        format!("1_{}", symbol.name)
                    };

                    CompletionItem {
                        label: symbol.name.clone(),
                        kind: Some(kind),
                        detail: Some(detail),
                        sort_text: Some(sort_text),
                        documentation: symbol.docs.map(|docs| {
                            Documentation::MarkupContent(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: docs,
                            })
                        }),
                        ..Default::default()
                    }
                })
                .collect()
        };

        Some(CompletionResponse::Array(items))
    }

    /// HTMLテンプレート内のAngular式コンテキスト（{{ ... }} や ng-* 属性値内）の補完候補を返す
    ///
    /// 含まれる候補:
    /// - スコープ内のコントローラーの $scope プロパティ/メソッド
    /// - コントローラー本体の this.X メソッド/プロパティ（component の `$ctrl.x` 等）
    /// - HTMLローカル変数（ng-repeat / ng-init 等）と継承された変数
    /// - フォームバインディングと継承されたフォームバインディング
    /// - ng-controller の "as" エイリアス
    /// - component template の controllerAs エイリアス（デフォルト $ctrl）
    pub fn complete_in_html_angular_context(
        &self,
        uri: &Url,
        line: u32,
    ) -> Vec<CompletionItem> {
        let controllers = self.index.resolve_controllers_for_html(uri, line);
        let mut items: Vec<CompletionItem> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        // $scope プロパティ + コントローラー本体の this.X
        if controllers.is_empty() {
            if let Some(CompletionResponse::Array(scope_items)) =
                self.complete_with_context(Some("$scope"), None, &[])
            {
                for item in scope_items {
                    push_unique(&mut items, &mut seen, item);
                }
            }
        } else {
            for controller_name in &controllers {
                if let Some(CompletionResponse::Array(scope_items)) =
                    self.complete_with_context(Some("$scope"), Some(controller_name), &[])
                {
                    for item in scope_items {
                        push_unique(&mut items, &mut seen, item);
                    }
                }
                // controller の this.X メソッド/プロパティ
                // (component の `$ctrl.x` や ng-controller の "as" エイリアス越しのアクセス)
                if let Some(CompletionResponse::Array(method_items)) =
                    self.complete_with_context(Some(controller_name), None, &[])
                {
                    for item in method_items {
                        push_unique(&mut items, &mut seen, item);
                    }
                }
            }
        }

        // ローカル変数
        for var in self.index.html.get_local_variables_at(uri, line) {
            push_unique(
                &mut items,
                &mut seen,
                CompletionItem {
                    label: var.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some(format!("local variable ({})", var.source.as_str())),
                    ..Default::default()
                },
            );
        }

        // 継承されたローカル変数
        for var in self
            .index
            .templates
            .get_inherited_local_variables_for_template(uri)
        {
            push_unique(
                &mut items,
                &mut seen,
                CompletionItem {
                    label: var.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some(format!("inherited variable ({})", var.source.as_str())),
                    ..Default::default()
                },
            );
        }

        // フォームバインディング
        for binding in self.index.html.get_form_bindings_at(uri, line) {
            push_unique(
                &mut items,
                &mut seen,
                CompletionItem {
                    label: binding.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some("form binding ($scope)".to_string()),
                    ..Default::default()
                },
            );
        }

        // 継承されたフォームバインディング
        for binding in self
            .index
            .templates
            .get_inherited_form_bindings_for_template(uri)
        {
            push_unique(
                &mut items,
                &mut seen,
                CompletionItem {
                    label: binding.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some("inherited form binding ($scope)".to_string()),
                    ..Default::default()
                },
            );
        }

        // ng-controller の "as" エイリアス
        for (alias, controller_name) in self.index.controllers.get_html_alias_mappings(uri, line) {
            push_unique(
                &mut items,
                &mut seen,
                CompletionItem {
                    label: alias,
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some(format!("controller alias ({})", controller_name)),
                    ..Default::default()
                },
            );
        }

        // component template の controllerAs エイリアス（デフォルト $ctrl）
        if let Some(binding) = self.index.components.get_component_binding_for_template(uri) {
            let controller_label = binding
                .controller_name
                .clone()
                .unwrap_or_else(|| "component".to_string());
            push_unique(
                &mut items,
                &mut seen,
                CompletionItem {
                    label: binding.controller_as,
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some(format!("component alias ({})", controller_label)),
                    ..Default::default()
                },
            );
        }

        items
    }

    /// HTMLでのディレクティブ補完を返す
    /// prefix: 入力中のプレフィックス（kebab-case）
    /// is_tag_name: タグ名位置かどうか（要素として補完）
    pub fn complete_directives(
        &self,
        prefix: &str,
        is_tag_name: bool,
    ) -> Option<CompletionResponse> {
        let definitions = self.index.definitions.get_all_definitions();

        // ディレクティブとコンポーネントをフィルタ
        let directives: Vec<_> = definitions
            .into_iter()
            .filter(|s| s.kind == SymbolKind::Directive || s.kind == SymbolKind::Component)
            .collect();

        if directives.is_empty() {
            return None;
        }

        let items: Vec<CompletionItem> = directives
            .into_iter()
            .filter_map(|symbol| {
                // camelCase を kebab-case に変換
                let kebab_name = camel_to_kebab(&symbol.name);

                // プレフィックスでフィルタ
                if !prefix.is_empty() && !kebab_name.starts_with(prefix) {
                    return None;
                }

                let detail = if symbol.kind == SymbolKind::Component {
                    if is_tag_name {
                        "component (element)".to_string()
                    } else {
                        "component (attribute)".to_string()
                    }
                } else if is_tag_name {
                    "directive (element)".to_string()
                } else {
                    "directive (attribute)".to_string()
                };

                Some(CompletionItem {
                    label: kebab_name,
                    kind: Some(CompletionItemKind::CLASS),
                    detail: Some(detail),
                    documentation: symbol.docs.map(|docs| {
                        Documentation::MarkupContent(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: docs,
                        })
                    }),
                    ..Default::default()
                })
            })
            .collect();

        if items.is_empty() {
            None
        } else {
            Some(CompletionResponse::Array(items))
        }
    }

    fn symbol_kind_to_completion_kind(&self, kind: SymbolKind) -> CompletionItemKind {
        match kind {
            SymbolKind::Module => CompletionItemKind::MODULE,
            SymbolKind::Controller => CompletionItemKind::CLASS,
            SymbolKind::Service => CompletionItemKind::CLASS,
            SymbolKind::Factory => CompletionItemKind::CLASS,
            SymbolKind::Directive => CompletionItemKind::CLASS,
            SymbolKind::Component => CompletionItemKind::CLASS,
            SymbolKind::Provider => CompletionItemKind::CLASS,
            SymbolKind::Filter => CompletionItemKind::FUNCTION,
            SymbolKind::Constant => CompletionItemKind::CONSTANT,
            SymbolKind::Value => CompletionItemKind::VALUE,
            SymbolKind::Method => CompletionItemKind::METHOD,
            SymbolKind::ScopeProperty => CompletionItemKind::PROPERTY,
            SymbolKind::ScopeMethod => CompletionItemKind::FUNCTION,
            SymbolKind::RootScopeProperty => CompletionItemKind::PROPERTY,
            SymbolKind::RootScopeMethod => CompletionItemKind::FUNCTION,
            SymbolKind::FormBinding => CompletionItemKind::VARIABLE,
            SymbolKind::ExportedComponent => CompletionItemKind::CLASS,
            SymbolKind::ComponentBinding => CompletionItemKind::PROPERTY,
        }
    }
}
