use std::collections::HashSet;

use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

use crate::model::{Symbol, SymbolKind, SymbolReference};

/// シンボル定義・参照の管理ストア
pub struct DefinitionStore {
    definitions: DashMap<String, Vec<Symbol>>,
    references: DashMap<String, Vec<SymbolReference>>,
    /// URI → そのドキュメントから add された定義/参照のシンボル名集合。
    /// `get_definitions_for_uri` 等の URI 逆引きを O(該当ドキュメントのシンボル数)
    /// で行うためのインデックス。重複登録を避けるため HashSet で保持する。
    document_symbols: DashMap<Url, HashSet<String>>,
}

impl DefinitionStore {
    pub fn new() -> Self {
        Self {
            definitions: DashMap::new(),
            references: DashMap::new(),
            document_symbols: DashMap::new(),
        }
    }

    pub fn add_definition(&self, symbol: Symbol) {
        let name = symbol.name.clone();
        let uri = symbol.uri.clone();

        let mut entry = self.definitions.entry(name.clone()).or_default();
        let is_duplicate = entry.iter().any(|s| {
            s.uri == symbol.uri
                && s.definition_span.start_line == symbol.definition_span.start_line
                && s.definition_span.start_col == symbol.definition_span.start_col
        });
        if !is_duplicate {
            entry.push(symbol);
            self.document_symbols.entry(uri).or_default().insert(name);
        }
    }

    pub fn add_reference(&self, reference: SymbolReference) {
        let name = reference.name.clone();
        let uri = reference.uri.clone();

        let mut entry = self.references.entry(name.clone()).or_default();
        let is_duplicate = entry.iter().any(|r| {
            r.uri == reference.uri
                && r.span.start_line == reference.span.start_line
                && r.span.start_col == reference.span.start_col
        });
        if !is_duplicate {
            entry.push(reference);
            self.document_symbols.entry(uri).or_default().insert(name);
        }
    }

    pub fn get_definitions(&self, name: &str) -> Vec<Symbol> {
        self.definitions
            .get(name)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    pub fn has_definition(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }

    /// 指定した名前 + Kind の定義が存在するか
    pub fn has_definition_of_kind(&self, name: &str, kind: SymbolKind) -> bool {
        self.definitions
            .get(name)
            .map(|defs| defs.iter().any(|s| s.kind == kind))
            .unwrap_or(false)
    }

    pub fn get_references(&self, name: &str) -> Vec<SymbolReference> {
        self.references
            .get(name)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    pub fn get_all_definitions(&self) -> Vec<Symbol> {
        self.definitions
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    /// 指定した名前がService/Factoryかどうかを判定
    pub fn is_service_or_factory(&self, name: &str) -> bool {
        if let Some(symbols) = self.definitions.get(name) {
            return symbols
                .iter()
                .any(|s| s.kind == SymbolKind::Service || s.kind == SymbolKind::Factory);
        }
        false
    }

    /// 指定JSファイルの全スコープ変数定義を取得
    pub fn get_scope_definitions_for_js(&self, uri: &Url) -> Vec<Symbol> {
        self.collect_definitions_for_uri(uri, |s| {
            s.kind == SymbolKind::ScopeProperty || s.kind == SymbolKind::ScopeMethod
        })
    }

    /// 参照のみ存在するシンボル名を取得（定義がないもの）
    pub fn get_reference_only_names(&self) -> Vec<String> {
        self.references
            .iter()
            .filter(|entry| {
                !entry.value().is_empty()
                    && self
                        .definitions
                        .get(entry.key())
                        .map(|d| d.is_empty())
                        .unwrap_or(true)
            })
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// プロパティパスで$rootScopeシンボルを検索
    pub fn find_root_scope_definitions_by_property(&self, property_path: &str) -> Vec<Symbol> {
        let suffix = format!(".$rootScope.{}", property_path);
        self.definitions
            .iter()
            .filter(|entry| entry.key().ends_with(&suffix))
            .flat_map(|entry| entry.value().clone())
            .filter(|s| {
                s.kind == SymbolKind::RootScopeProperty || s.kind == SymbolKind::RootScopeMethod
            })
            .collect()
    }

    /// プロパティパスで$rootScopeの参照を検索
    pub fn find_root_scope_references_by_property(
        &self,
        property_path: &str,
    ) -> Vec<SymbolReference> {
        let suffix = format!(".$rootScope.{}", property_path);
        self.references
            .iter()
            .filter(|entry| entry.key().ends_with(&suffix))
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    /// プロパティパスに一致する$rootScopeシンボル名を取得
    pub fn find_root_scope_symbol_name_by_property(
        &self,
        property_path: &str,
    ) -> Option<String> {
        let suffix = format!(".$rootScope.{}", property_path);
        self.definitions
            .iter()
            .find(|entry| entry.key().ends_with(&suffix))
            .map(|entry| entry.key().clone())
    }

    /// 位置からシンボルを検索（最も小さい範囲を優先）
    pub fn find_symbol_at_position(&self, uri: &Url, line: u32, col: u32) -> Option<String> {
        let mut best_match: Option<(String, u32)> = None;

        for entry in self.definitions.iter() {
            for symbol in entry.value() {
                if &symbol.uri == uri && symbol.name_span.contains(line, col) {
                    let size = symbol.name_span.range_size();
                    if best_match.is_none() || size < best_match.as_ref().unwrap().1 {
                        best_match = Some((symbol.name.clone(), size));
                    }
                }
            }
        }

        for entry in self.references.iter() {
            for reference in entry.value() {
                if &reference.uri == uri && reference.span.contains(line, col) {
                    let size = reference.span.range_size();
                    if best_match.is_none() || size < best_match.as_ref().unwrap().1 {
                        best_match = Some((reference.name.clone(), size));
                    }
                }
            }
        }

        best_match.map(|(name, _)| name)
    }

    /// 指定URIのドキュメント内定義を取得
    pub fn get_definitions_for_uri(&self, uri: &Url) -> Vec<Symbol> {
        self.collect_definitions_for_uri(uri, |_| true)
    }

    /// `document_symbols` の URI 逆引きを使い、`definitions` を全件走査せずに
    /// 当該ドキュメントの定義だけ取り出す。`predicate` が true の Symbol のみ
    /// 返す。
    fn collect_definitions_for_uri<F>(&self, uri: &Url, predicate: F) -> Vec<Symbol>
    where
        F: Fn(&Symbol) -> bool,
    {
        let Some(names) = self.document_symbols.get(uri) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for name in names.value() {
            if let Some(entry) = self.definitions.get(name) {
                for symbol in entry.value() {
                    if &symbol.uri == uri && predicate(symbol) {
                        result.push(symbol.clone());
                    }
                }
            }
        }
        result
    }

    /// 指定 URI から参照されているシンボル名集合を取得
    /// (HTML 埋め込みスクリプトが書き込んだ参照を URI 単位で逆引きするのに使う)
    pub fn get_reference_names_for_uri(&self, uri: &Url) -> std::collections::HashSet<String> {
        let mut names = std::collections::HashSet::new();
        for entry in self.references.iter() {
            if entry.value().iter().any(|r| &r.uri == uri) {
                names.insert(entry.key().clone());
            }
        }
        names
    }

    pub fn clear_document(&self, uri: &Url) {
        if let Some((_, symbols)) = self.document_symbols.remove(uri) {
            for symbol_name in symbols {
                let defs_empty = if let Some(mut defs) = self.definitions.get_mut(&symbol_name) {
                    defs.retain(|s| &s.uri != uri);
                    defs.is_empty()
                } else {
                    false
                };
                if defs_empty {
                    self.definitions.remove_if(&symbol_name, |_, v| v.is_empty());
                }

                let refs_empty = if let Some(mut refs) = self.references.get_mut(&symbol_name) {
                    refs.retain(|r| &r.uri != uri);
                    refs.is_empty()
                } else {
                    false
                };
                if refs_empty {
                    self.references.remove_if(&symbol_name, |_, v| v.is_empty());
                }
            }
        }
    }

    pub fn clear_all(&self) {
        self.definitions.clear();
        self.references.clear();
        self.document_symbols.clear();
    }
}

impl Default for DefinitionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Span, SymbolBuilder, SymbolKind};

    fn make_uri() -> Url {
        Url::parse("file:///test.js").unwrap()
    }

    fn make_reference(name: &str, uri: &Url) -> SymbolReference {
        SymbolReference {
            name: name.to_string(),
            uri: uri.clone(),
            span: Span::new(0, 0, 0, name.len() as u32),
        }
    }

    fn make_definition(name: &str, uri: &Url) -> Symbol {
        let span = Span::new(0, 0, 0, name.len() as u32);
        SymbolBuilder::new(name.to_string(), SymbolKind::ScopeProperty, uri.clone())
            .definition_span(span)
            .name_span(span)
            .build()
    }

    #[test]
    fn clear_document_removes_empty_reference_keys() {
        let store = DefinitionStore::new();
        let uri = make_uri();

        // 入力途中のプレフィックス参照（mo, moc, moch, mochi）を登録
        for name in ["Ctrl.$scope.mo", "Ctrl.$scope.moc", "Ctrl.$scope.moch", "Ctrl.$scope.mochi"] {
            store.add_reference(make_reference(name, &uri));
        }

        store.clear_document(&uri);

        // 補完候補ソースとなる get_reference_only_names が空であることを確認
        let names = store.get_reference_only_names();
        assert!(
            names.is_empty(),
            "expected no reference-only names after clear, got {:?}",
            names
        );
    }

    #[test]
    fn deleted_reference_does_not_resurface_in_completion() {
        let store = DefinitionStore::new();
        let uri = make_uri();

        // ユーザが $scope.mochi を参照として書いた状態
        store.add_reference(make_reference("Ctrl.$scope.mochi", &uri));
        assert_eq!(store.get_reference_only_names(), vec!["Ctrl.$scope.mochi"]);

        // ユーザが該当行を削除 → 再解析で clear_document が呼ばれる
        store.clear_document(&uri);

        // 再解析後、$scope.mochi はソースに無いので何も add されない
        assert!(store.get_reference_only_names().is_empty());
    }

    #[test]
    fn clear_document_preserves_other_uris() {
        let store = DefinitionStore::new();
        let uri_a = Url::parse("file:///a.js").unwrap();
        let uri_b = Url::parse("file:///b.js").unwrap();

        store.add_reference(make_reference("Ctrl.$scope.shared", &uri_a));
        store.add_reference(make_reference("Ctrl.$scope.shared", &uri_b));

        store.clear_document(&uri_a);

        // uri_b の参照は残っているので、reference-only として現れる
        let names = store.get_reference_only_names();
        assert_eq!(names, vec!["Ctrl.$scope.shared"]);
        assert_eq!(store.get_references("Ctrl.$scope.shared").len(), 1);
    }

    #[test]
    fn clear_document_removes_empty_definition_keys() {
        let store = DefinitionStore::new();
        let uri = make_uri();

        store.add_definition(make_definition("Ctrl.$scope.mochi", &uri));
        store.add_reference(make_reference("Ctrl.$scope.other", &uri));

        store.clear_document(&uri);

        // 定義が空になったキーは contains_key で false を返す必要がある
        // （これがなければ get_reference_only_names で他の reference-only も誤判定する）
        assert!(!store.has_definition("Ctrl.$scope.mochi"));
        assert!(store.get_reference_only_names().is_empty());
    }

    #[test]
    fn get_definitions_for_uri_returns_only_target_uri() {
        let store = DefinitionStore::new();
        let uri_a = Url::parse("file:///a.js").unwrap();
        let uri_b = Url::parse("file:///b.js").unwrap();

        store.add_definition(make_definition("Ctrl.$scope.foo", &uri_a));
        store.add_definition(make_definition("Ctrl.$scope.bar", &uri_a));
        store.add_definition(make_definition("Ctrl.$scope.shared", &uri_a));
        store.add_definition(make_definition("Ctrl.$scope.shared", &uri_b));

        let mut names: Vec<String> = store
            .get_definitions_for_uri(&uri_a)
            .into_iter()
            .map(|s| s.name)
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "Ctrl.$scope.bar".to_string(),
                "Ctrl.$scope.foo".to_string(),
                "Ctrl.$scope.shared".to_string(),
            ]
        );

        let names_b: Vec<String> = store
            .get_definitions_for_uri(&uri_b)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names_b, vec!["Ctrl.$scope.shared".to_string()]);
    }

    #[test]
    fn get_scope_definitions_for_js_filters_by_kind() {
        let store = DefinitionStore::new();
        let uri = make_uri();

        let span = Span::new(0, 0, 0, 4);
        let scope_prop =
            SymbolBuilder::new("Ctrl.$scope.x".to_string(), SymbolKind::ScopeProperty, uri.clone())
                .definition_span(span)
                .name_span(span)
                .build();
        let scope_method = SymbolBuilder::new(
            "Ctrl.$scope.fn".to_string(),
            SymbolKind::ScopeMethod,
            uri.clone(),
        )
        .definition_span(Span::new(1, 0, 1, 2))
        .name_span(Span::new(1, 0, 1, 2))
        .build();
        let controller =
            SymbolBuilder::new("Ctrl".to_string(), SymbolKind::Controller, uri.clone())
                .definition_span(Span::new(2, 0, 2, 4))
                .name_span(Span::new(2, 0, 2, 4))
                .build();

        store.add_definition(scope_prop);
        store.add_definition(scope_method);
        store.add_definition(controller);

        let mut names: Vec<String> = store
            .get_scope_definitions_for_js(&uri)
            .into_iter()
            .map(|s| s.name)
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["Ctrl.$scope.fn".to_string(), "Ctrl.$scope.x".to_string()]
        );
    }

    #[test]
    fn document_symbols_dedupes_repeated_adds() {
        // 同じ name に対して複数の定義/参照が登録されても、document_symbols 側
        // には 1 度だけ含まれるべき (URI 逆引きで同じ name を何度も
        // definitions.get() しないようにするため)
        let store = DefinitionStore::new();
        let uri = make_uri();

        // 定義側: 異なる span で 2 回登録 (どちらも有効な定義として残る)
        let def1 = SymbolBuilder::new(
            "Ctrl.$scope.x".to_string(),
            SymbolKind::ScopeProperty,
            uri.clone(),
        )
        .definition_span(Span::new(0, 0, 0, 1))
        .name_span(Span::new(0, 0, 0, 1))
        .build();
        let def2 = SymbolBuilder::new(
            "Ctrl.$scope.x".to_string(),
            SymbolKind::ScopeProperty,
            uri.clone(),
        )
        .definition_span(Span::new(2, 0, 2, 1))
        .name_span(Span::new(2, 0, 2, 1))
        .build();
        store.add_definition(def1);
        store.add_definition(def2);

        // 同じ name で参照も登録
        store.add_reference(SymbolReference {
            name: "Ctrl.$scope.x".to_string(),
            uri: uri.clone(),
            span: Span::new(3, 0, 3, 1),
        });

        // document_symbols には "Ctrl.$scope.x" が 1 度だけ入っているべき
        let entry = store
            .document_symbols
            .get(&uri)
            .expect("uri entry exists");
        assert_eq!(entry.value().len(), 1);
        assert!(entry.value().contains("Ctrl.$scope.x"));

        // 一方で definitions 側には 2 件残っている
        assert_eq!(store.get_definitions("Ctrl.$scope.x").len(), 2);
    }
}
