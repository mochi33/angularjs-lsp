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

    /// 指定シンボル名の参照を借用イテレートする (Vec 全件 clone を回避)
    ///
    /// 注意: 内部で DashMap shard の read lock を保持するため、`f` 内で同じ
    /// `DefinitionStore` を変更するメソッド (add_reference / clear_document など)
    /// を呼ばないこと。デッドロックする可能性がある。
    pub fn for_each_reference<F: FnMut(&SymbolReference)>(&self, name: &str, mut f: F) {
        if let Some(refs) = self.references.get(name) {
            for r in refs.value().iter() {
                f(r);
            }
        }
    }

    /// 指定シンボル名の参照のうち述語にマッチするものがあるか (短絡評価)
    ///
    /// 注意: `for_each_reference` と同じくデッドロック注意。
    pub fn any_reference<F: FnMut(&SymbolReference) -> bool>(&self, name: &str, mut f: F) -> bool {
        if let Some(refs) = self.references.get(name) {
            for r in refs.value().iter() {
                if f(r) {
                    return true;
                }
            }
        }
        false
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
    ///
    /// `document_symbols` URI 逆引きを使い、当該 URI のシンボル名候補だけ
    /// `definitions` / `references` から取り出して走査する。旧実装は workspace
    /// 全シンボル/参照を走査して O(N) だったが、本実装は O(該当 URI のシンボル
    /// 数 + 参照数) に絞られる。
    pub fn find_symbol_at_position(&self, uri: &Url, line: u32, col: u32) -> Option<String> {
        let Some(names) = self.document_symbols.get(uri) else {
            return None;
        };

        let mut best_match: Option<(String, u32)> = None;

        for name in names.value() {
            if let Some(entry) = self.definitions.get(name) {
                for symbol in entry.value() {
                    if &symbol.uri == uri && symbol.name_span.contains(line, col) {
                        let size = symbol.name_span.range_size();
                        if best_match.is_none() || size < best_match.as_ref().unwrap().1 {
                            best_match = Some((symbol.name.clone(), size));
                        }
                    }
                }
            }

            if let Some(entry) = self.references.get(name) {
                for reference in entry.value() {
                    if &reference.uri == uri && reference.span.contains(line, col) {
                        let size = reference.span.range_size();
                        if best_match.is_none() || size < best_match.as_ref().unwrap().1 {
                            best_match = Some((reference.name.clone(), size));
                        }
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
    ///
    /// `document_symbols` URI 逆引きを使い、当該 URI に紐づくシンボル名だけ
    /// 走査する。旧実装は workspace 全 references を走査して O(全シンボル数 ×
    /// 平均参照数) だったが、本実装は O(該当 URI のシンボル数 × 同名参照数)
    /// に絞られる。`get_definition_names_for_uri` と対称な実装。
    pub fn get_reference_names_for_uri(&self, uri: &Url) -> HashSet<String> {
        let Some(names) = self.document_symbols.get(uri) else {
            return HashSet::new();
        };
        let mut result = HashSet::new();
        for name in names.value() {
            if let Some(entry) = self.references.get(name) {
                if entry.value().iter().any(|r| &r.uri == uri) {
                    result.insert(name.clone());
                }
            }
        }
        result
    }

    /// 指定 URI に定義があるシンボル名集合を取得
    /// (HTML 埋め込みスクリプトの定義を URI 単位で逆引きするのに使う。
    ///  semantic_tokens_refresh の発火判定に利用)
    pub fn get_definition_names_for_uri(&self, uri: &Url) -> HashSet<String> {
        let Some(names) = self.document_symbols.get(uri) else {
            return HashSet::new();
        };
        let mut result = HashSet::new();
        for name in names.value() {
            if let Some(entry) = self.definitions.get(name) {
                if entry.value().iter().any(|s| &s.uri == uri) {
                    result.insert(name.clone());
                }
            }
        }
        result
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

    #[test]
    fn get_definition_names_for_uri_returns_only_definitions_in_uri() {
        let store = DefinitionStore::new();
        let uri_a = Url::parse("file:///a.html").unwrap();
        let uri_b = Url::parse("file:///b.js").unwrap();

        // uri_a に 2 件、uri_b に 1 件の定義
        store.add_definition(make_definition("Ctrl.$scope.foo", &uri_a));
        store.add_definition(make_definition("Ctrl.$scope.bar", &uri_a));
        store.add_definition(make_definition("Ctrl.$scope.baz", &uri_b));

        let names = store.get_definition_names_for_uri(&uri_a);
        let mut names_vec: Vec<String> = names.into_iter().collect();
        names_vec.sort();
        assert_eq!(
            names_vec,
            vec!["Ctrl.$scope.bar".to_string(), "Ctrl.$scope.foo".to_string()],
            "uri_a に定義されたシンボルだけ返るべき"
        );
    }

    #[test]
    fn get_definition_names_for_uri_excludes_reference_only() {
        // document_symbols は定義+参照の和集合だが、本ヘルパーは
        // 「実際に definitions エントリでこの URI を持つ」名前だけ返す
        let store = DefinitionStore::new();
        let uri = make_uri();

        // 定義は無いが参照だけ登録 (例: ユーザがタイポ中の参照名)
        store.add_reference(make_reference("Ctrl.$scope.unknown", &uri));
        // 通常の定義 + 参照
        store.add_definition(make_definition("Ctrl.$scope.foo", &uri));
        store.add_reference(make_reference("Ctrl.$scope.foo", &uri));

        let names = store.get_definition_names_for_uri(&uri);
        assert!(names.contains("Ctrl.$scope.foo"));
        assert!(
            !names.contains("Ctrl.$scope.unknown"),
            "参照のみのシンボルは除外すべき"
        );
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn get_definition_names_for_uri_returns_empty_for_unknown() {
        let store = DefinitionStore::new();
        let uri = make_uri();
        assert!(store.get_definition_names_for_uri(&uri).is_empty());
    }

    #[test]
    fn get_definition_names_for_uri_drops_after_clear_document() {
        // clear_document 後は空になることを確認 (HTML 編集サイクルを模す)
        let store = DefinitionStore::new();
        let uri = make_uri();

        store.add_definition(make_definition("Ctrl.$scope.foo", &uri));
        assert_eq!(store.get_definition_names_for_uri(&uri).len(), 1);

        store.clear_document(&uri);
        assert!(store.get_definition_names_for_uri(&uri).is_empty());
    }

    #[test]
    fn for_each_reference_visits_all_refs() {
        let store = DefinitionStore::new();
        let uri_a = Url::parse("file:///a.js").unwrap();
        let uri_b = Url::parse("file:///b.js").unwrap();

        store.add_reference(make_reference("Ctrl.$scope.x", &uri_a));
        store.add_reference(SymbolReference {
            name: "Ctrl.$scope.x".to_string(),
            uri: uri_b.clone(),
            span: Span::new(1, 0, 1, 1),
        });

        let mut visited: Vec<Url> = Vec::new();
        store.for_each_reference("Ctrl.$scope.x", |r| {
            visited.push(r.uri.clone());
        });
        visited.sort();
        let mut expected = vec![uri_a, uri_b];
        expected.sort();
        assert_eq!(visited, expected);
    }

    #[test]
    fn for_each_reference_does_nothing_for_unknown_name() {
        let store = DefinitionStore::new();
        let mut count = 0;
        store.for_each_reference("does.not.exist", |_| count += 1);
        assert_eq!(count, 0);
    }

    #[test]
    fn any_reference_returns_true_when_predicate_matches() {
        let store = DefinitionStore::new();
        let uri_a = Url::parse("file:///a.js").unwrap();
        let uri_b = Url::parse("file:///b.js").unwrap();

        store.add_reference(make_reference("Ctrl.$scope.x", &uri_a));
        store.add_reference(SymbolReference {
            name: "Ctrl.$scope.x".to_string(),
            uri: uri_b.clone(),
            span: Span::new(1, 0, 1, 1),
        });

        assert!(store.any_reference("Ctrl.$scope.x", |r| r.uri == uri_b));
    }

    #[test]
    fn any_reference_returns_false_when_no_match() {
        let store = DefinitionStore::new();
        let uri_a = Url::parse("file:///a.js").unwrap();
        let uri_other = Url::parse("file:///other.js").unwrap();

        store.add_reference(make_reference("Ctrl.$scope.x", &uri_a));
        assert!(!store.any_reference("Ctrl.$scope.x", |r| r.uri == uri_other));
    }

    #[test]
    fn any_reference_short_circuits_on_first_match() {
        // 述語が複数件にマッチし得る状況で、最初の一致で停止していること
        let store = DefinitionStore::new();
        let uri = make_uri();

        for line in 0..5u32 {
            store.add_reference(SymbolReference {
                name: "Ctrl.$scope.x".to_string(),
                uri: uri.clone(),
                span: Span::new(line, 0, line, 1),
            });
        }

        let mut visits = 0;
        let result = store.any_reference("Ctrl.$scope.x", |_| {
            visits += 1;
            true
        });
        assert!(result);
        assert_eq!(visits, 1, "述語が true を返した時点で停止すべき");
    }

    #[test]
    fn any_reference_returns_false_for_unknown_name() {
        let store = DefinitionStore::new();
        assert!(!store.any_reference("does.not.exist", |_| true));
    }

    fn make_definition_at(name: &str, uri: &Url, span: Span) -> Symbol {
        SymbolBuilder::new(name.to_string(), SymbolKind::ScopeProperty, uri.clone())
            .definition_span(span)
            .name_span(span)
            .build()
    }

    fn make_reference_at(name: &str, uri: &Url, span: Span) -> SymbolReference {
        SymbolReference {
            name: name.to_string(),
            uri: uri.clone(),
            span,
        }
    }

    #[test]
    fn find_symbol_at_position_returns_none_for_unknown_uri() {
        let store = DefinitionStore::new();
        let uri = Url::parse("file:///never-touched.js").unwrap();
        assert_eq!(store.find_symbol_at_position(&uri, 0, 0), None);
    }

    #[test]
    fn find_symbol_at_position_finds_definition() {
        let store = DefinitionStore::new();
        let uri = make_uri();

        // line 5, col 0..6 に "MyCtrl" の定義を入れる
        let span = Span::new(5, 0, 5, 6);
        store.add_definition(make_definition_at("MyCtrl", &uri, span));

        assert_eq!(
            store.find_symbol_at_position(&uri, 5, 3),
            Some("MyCtrl".to_string())
        );
    }

    #[test]
    fn find_symbol_at_position_finds_reference() {
        let store = DefinitionStore::new();
        let uri = make_uri();

        // 定義はないがリファレンスだけある
        let span = Span::new(10, 4, 10, 10);
        store.add_reference(make_reference_at("RefOnly", &uri, span));

        assert_eq!(
            store.find_symbol_at_position(&uri, 10, 7),
            Some("RefOnly".to_string())
        );
    }

    #[test]
    fn find_symbol_at_position_prefers_smallest_range() {
        // 大きな範囲と小さな範囲が同じ位置で重なる場合、小さな方を返す
        let store = DefinitionStore::new();
        let uri = make_uri();

        // 範囲: line 0, col 0..20 (20文字)
        let big_span = Span::new(0, 0, 0, 20);
        store.add_definition(make_definition_at("OuterSymbol", &uri, big_span));

        // 範囲: line 0, col 5..10 (5文字) ← 小さい
        let small_span = Span::new(0, 5, 0, 10);
        store.add_definition(make_definition_at("Inner", &uri, small_span));

        // 両方含む位置 (col=7) → 小さい方の Inner を返す
        assert_eq!(
            store.find_symbol_at_position(&uri, 0, 7),
            Some("Inner".to_string())
        );
    }

    #[test]
    fn find_symbol_at_position_ignores_other_uris() {
        // 別 URI に同じ名前のシンボルがあっても、対象 URI でしか定義/参照を返さない
        let store = DefinitionStore::new();
        let uri_a = Url::parse("file:///a.js").unwrap();
        let uri_b = Url::parse("file:///b.js").unwrap();

        let span = Span::new(0, 0, 0, 6);
        // Same name "Shared" defined in both, position-overlapping
        store.add_definition(make_definition_at("Shared", &uri_a, span));
        store.add_definition(make_definition_at("Shared", &uri_b, span));

        // uri_a で問い合わせると uri_a 側の定義しか拾わない (シンボル名は同じだが OK)
        assert_eq!(
            store.find_symbol_at_position(&uri_a, 0, 3),
            Some("Shared".to_string())
        );
        // uri_b で問い合わせても uri_b 側の定義を拾う
        assert_eq!(
            store.find_symbol_at_position(&uri_b, 0, 3),
            Some("Shared".to_string())
        );
    }

    #[test]
    fn find_symbol_at_position_returns_none_for_unmatched_position() {
        let store = DefinitionStore::new();
        let uri = make_uri();

        let span = Span::new(0, 0, 0, 6);
        store.add_definition(make_definition_at("MyCtrl", &uri, span));

        // span 範囲外
        assert_eq!(store.find_symbol_at_position(&uri, 5, 0), None);
    }

    #[test]
    fn get_reference_names_for_uri_collects_only_target_uri() {
        let store = DefinitionStore::new();
        let uri_a = Url::parse("file:///a.html").unwrap();
        let uri_b = Url::parse("file:///b.html").unwrap();

        // uri_a が "RefA1" / "RefA2" を参照、uri_b が "RefB1" / "RefA1" (同名) を参照
        store.add_reference(make_reference("RefA1", &uri_a));
        store.add_reference(make_reference("RefA2", &uri_a));
        store.add_reference(make_reference("RefB1", &uri_b));
        store.add_reference(make_reference("RefA1", &uri_b)); // 別 URI に同名

        let names_a = store.get_reference_names_for_uri(&uri_a);
        assert_eq!(names_a.len(), 2);
        assert!(names_a.contains("RefA1"));
        assert!(names_a.contains("RefA2"));

        let names_b = store.get_reference_names_for_uri(&uri_b);
        assert_eq!(names_b.len(), 2);
        assert!(names_b.contains("RefA1")); // 同名でも uri_b 側にも参照あるので拾う
        assert!(names_b.contains("RefB1"));
    }

    #[test]
    fn get_reference_names_for_uri_includes_names_with_only_references() {
        // 定義のないシンボル名 (リファレンスのみ) もちゃんと拾う
        let store = DefinitionStore::new();
        let uri = make_uri();

        store.add_reference(make_reference("OnlyRef", &uri));
        // 定義は登録しない

        let names = store.get_reference_names_for_uri(&uri);
        assert_eq!(names.len(), 1);
        assert!(names.contains("OnlyRef"));
    }

    #[test]
    fn get_reference_names_for_uri_excludes_definition_only_names() {
        // 定義しか持たないシンボル名 (この URI からの参照は無い) は除外
        let store = DefinitionStore::new();
        let uri = make_uri();

        store.add_definition(make_definition("DefOnly", &uri));
        // リファレンスは追加しない

        let names = store.get_reference_names_for_uri(&uri);
        assert!(names.is_empty(), "定義しかない URI はリファレンス無し");
    }

    #[test]
    fn get_reference_names_for_uri_returns_empty_for_unknown_uri() {
        let store = DefinitionStore::new();
        let uri = Url::parse("file:///never-touched.js").unwrap();
        assert!(store.get_reference_names_for_uri(&uri).is_empty());
    }

    #[test]
    fn get_reference_names_for_uri_drops_after_clear_document() {
        let store = DefinitionStore::new();
        let uri = make_uri();

        store.add_reference(make_reference("Foo", &uri));
        assert_eq!(store.get_reference_names_for_uri(&uri).len(), 1);

        store.clear_document(&uri);
        assert!(store.get_reference_names_for_uri(&uri).is_empty());
    }
}
