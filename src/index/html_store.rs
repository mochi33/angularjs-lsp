use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

use crate::model::{
    HtmlDirectiveReference, HtmlFormBinding, HtmlLocalVariable, HtmlLocalVariableReference,
    HtmlScopeReference,
};

/// HTMLスコープ参照・ローカル変数・フォーム・ディレクティブの管理ストア
pub struct HtmlStore {
    /// HTML内のスコープ参照（URI -> Vec<HtmlScopeReference>）
    html_scope_references: DashMap<Url, Vec<HtmlScopeReference>>,
    /// HTML内のローカル変数定義（URI -> Vec<HtmlLocalVariable>）
    html_local_variables: DashMap<Url, Vec<HtmlLocalVariable>>,
    /// HTML内のローカル変数参照（変数名 -> Vec<HtmlLocalVariableReference>）
    html_local_variable_references: DashMap<String, Vec<HtmlLocalVariableReference>>,
    /// HTML内のフォームバインディング（URI -> Vec<HtmlFormBinding>）
    html_form_bindings: DashMap<Url, Vec<HtmlFormBinding>>,
    /// HTML内のカスタムディレクティブ参照（URI -> Vec<HtmlDirectiveReference>）
    html_directive_references: DashMap<Url, Vec<HtmlDirectiveReference>>,
}

impl HtmlStore {
    pub fn new() -> Self {
        Self {
            html_scope_references: DashMap::new(),
            html_local_variables: DashMap::new(),
            html_local_variable_references: DashMap::new(),
            html_form_bindings: DashMap::new(),
            html_directive_references: DashMap::new(),
        }
    }

    // ========== スコープ参照 ==========

    pub fn add_html_scope_reference(&self, reference: HtmlScopeReference) {
        let uri = reference.uri.clone();
        self.html_scope_references
            .entry(uri)
            .or_default()
            .push(reference);
    }

    pub fn find_html_scope_reference_at(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
    ) -> Option<HtmlScopeReference> {
        if let Some(refs) = self.html_scope_references.get(uri) {
            for r in refs.iter() {
                if r.span().contains(line, col) {
                    return Some(r.clone());
                }
            }
        }
        None
    }

    pub fn get_html_scope_references(&self, uri: &Url) -> Vec<HtmlScopeReference> {
        self.html_scope_references
            .get(uri)
            .map(|refs| refs.value().clone())
            .unwrap_or_default()
    }

    /// 全HTMLスコープ参照をイテレート
    pub fn iter_all_html_scope_references(
        &self,
    ) -> dashmap::iter::Iter<'_, Url, Vec<HtmlScopeReference>> {
        self.html_scope_references.iter()
    }

    /// テスト用: 指定URIの全HTMLスコープ参照を取得
    #[cfg(test)]
    pub fn html_scope_references_for_test(
        &self,
        uri: &Url,
    ) -> Option<Vec<HtmlScopeReference>> {
        self.html_scope_references
            .get(uri)
            .map(|v| v.value().clone())
    }

    /// 全HTMLスコープ参照を取得（キャッシュ用）
    pub fn get_all_html_scope_references_for_cache(&self) -> Vec<HtmlScopeReference> {
        self.html_scope_references
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    // ========== ローカル変数 ==========

    pub fn add_html_local_variable(&self, variable: HtmlLocalVariable) {
        let uri = variable.uri.clone();
        let mut entry = self.html_local_variables.entry(uri).or_default();
        let is_duplicate = entry.iter().any(|v| {
            v.name == variable.name
                && v.name_start_line == variable.name_start_line
                && v.name_start_col == variable.name_start_col
        });
        if !is_duplicate {
            entry.push(variable);
        }
    }

    pub fn add_html_local_variable_reference(&self, reference: HtmlLocalVariableReference) {
        let var_name = reference.variable_name.clone();
        let mut entry = self.html_local_variable_references.entry(var_name).or_default();
        let is_duplicate = entry.iter().any(|r| {
            r.uri == reference.uri
                && r.start_line == reference.start_line
                && r.start_col == reference.start_col
        });
        if !is_duplicate {
            entry.push(reference);
        }
    }

    /// 指定位置で有効なローカル変数を取得
    pub fn get_local_variables_at(&self, uri: &Url, line: u32) -> Vec<HtmlLocalVariable> {
        self.html_local_variables
            .get(uri)
            .map(|vars| {
                vars.iter()
                    .filter(|v| v.is_in_scope(line))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// ローカル変数の定義を取得（最も内側のスコープを優先）
    pub fn find_local_variable_definition(
        &self,
        uri: &Url,
        variable_name: &str,
        line: u32,
    ) -> Option<HtmlLocalVariable> {
        self.html_local_variables.get(uri).and_then(|vars| {
            vars.iter()
                .filter(|v| v.name == variable_name && v.is_in_scope(line))
                .max_by_key(|v| v.scope_start_line)
                .cloned()
        })
    }

    /// ローカル変数の全参照を取得（スコープ内のみ）
    pub fn get_local_variable_references(
        &self,
        uri: &Url,
        variable_name: &str,
        scope_start_line: u32,
        scope_end_line: u32,
    ) -> Vec<HtmlLocalVariableReference> {
        self.html_local_variable_references
            .get(variable_name)
            .map(|refs| {
                refs.iter()
                    .filter(|r| {
                        &r.uri == uri
                            && r.start_line >= scope_start_line
                            && r.start_line <= scope_end_line
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 指定位置のローカル変数参照を検索
    pub fn find_html_local_variable_at(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
    ) -> Option<HtmlLocalVariableReference> {
        for entry in self.html_local_variable_references.iter() {
            for r in entry.value() {
                if &r.uri == uri && r.span().contains(line, col) {
                    return Some(r.clone());
                }
            }
        }
        None
    }

    /// 指定位置のローカル変数定義を検索（定義位置にカーソルがある場合）
    pub fn find_html_local_variable_definition_at(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
    ) -> Option<HtmlLocalVariable> {
        self.html_local_variables.get(uri).and_then(|vars| {
            vars.iter()
                .filter(|v| v.name_span().contains(line, col))
                .cloned()
                .next()
        })
    }

    /// 指定URIの全てのローカル変数定義を取得
    pub fn get_all_local_variables(&self, uri: &Url) -> Vec<HtmlLocalVariable> {
        self.html_local_variables
            .get(uri)
            .map(|vars| vars.value().clone())
            .unwrap_or_default()
    }

    /// 指定URIの全てのローカル変数参照を取得
    pub fn get_all_local_variable_references_for_uri(
        &self,
        uri: &Url,
    ) -> Vec<HtmlLocalVariableReference> {
        let mut result = Vec::new();
        for entry in self.html_local_variable_references.iter() {
            for reference in entry.value() {
                if &reference.uri == uri {
                    result.push(reference.clone());
                }
            }
        }
        result
    }

    /// ローカル変数参照DashMapへの直接アクセス（テンプレート継承チェック用）
    pub fn html_local_variable_references_raw(
        &self,
    ) -> &DashMap<String, Vec<HtmlLocalVariableReference>> {
        &self.html_local_variable_references
    }

    /// 全HTMLローカル変数を取得（キャッシュ用）
    pub fn get_all_html_local_variables_for_cache(&self) -> Vec<HtmlLocalVariable> {
        self.html_local_variables
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    /// 全HTMLローカル変数参照を取得（キャッシュ用）
    pub fn get_all_html_local_variable_references_for_cache(
        &self,
    ) -> Vec<HtmlLocalVariableReference> {
        self.html_local_variable_references
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    // ========== フォームバインディング ==========

    pub fn add_html_form_binding(&self, binding: HtmlFormBinding) {
        let uri = binding.uri.clone();
        let mut entry = self.html_form_bindings.entry(uri).or_default();
        let is_duplicate = entry.iter().any(|b| {
            b.name == binding.name
                && b.name_start_line == binding.name_start_line
                && b.name_start_col == binding.name_start_col
        });
        if !is_duplicate {
            entry.push(binding);
        }
    }

    /// 指定位置で有効なフォームバインディングを取得
    pub fn get_form_bindings_at(&self, uri: &Url, line: u32) -> Vec<HtmlFormBinding> {
        self.html_form_bindings
            .get(uri)
            .map(|bindings| {
                bindings
                    .iter()
                    .filter(|b| b.is_in_scope(line))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// フォームバインディングの定義を取得（最も内側のスコープを優先）
    pub fn find_form_binding_definition(
        &self,
        uri: &Url,
        form_name: &str,
        line: u32,
    ) -> Option<HtmlFormBinding> {
        self.html_form_bindings.get(uri).and_then(|bindings| {
            bindings
                .iter()
                .filter(|b| b.name == form_name && b.is_in_scope(line))
                .max_by_key(|b| b.scope_start_line)
                .cloned()
        })
    }

    /// 指定位置のフォームバインディング定義を検索
    pub fn find_html_form_binding_at(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
    ) -> Option<HtmlFormBinding> {
        self.html_form_bindings.get(uri).and_then(|bindings| {
            bindings
                .iter()
                .filter(|b| b.name_span().contains(line, col))
                .cloned()
                .next()
        })
    }

    /// 指定URIの全てのフォームバインディングを取得
    pub fn get_all_form_bindings(&self, uri: &Url) -> Vec<HtmlFormBinding> {
        self.html_form_bindings
            .get(uri)
            .map(|bindings| bindings.value().clone())
            .unwrap_or_default()
    }

    /// 全HTMLフォームバインディングを取得（キャッシュ用）
    pub fn get_all_html_form_bindings_for_cache(&self) -> Vec<HtmlFormBinding> {
        self.html_form_bindings
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    // ========== ディレクティブ参照 ==========

    pub fn add_html_directive_reference(&self, reference: HtmlDirectiveReference) {
        let uri = reference.uri.clone();
        let mut entry = self.html_directive_references.entry(uri).or_default();
        let is_duplicate = entry.iter().any(|r| {
            r.directive_name == reference.directive_name
                && r.start_line == reference.start_line
                && r.start_col == reference.start_col
        });
        if !is_duplicate {
            entry.push(reference);
        }
    }

    /// 指定位置のディレクティブ参照を検索
    pub fn find_html_directive_reference_at(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
    ) -> Option<HtmlDirectiveReference> {
        self.html_directive_references.get(uri).and_then(|refs| {
            refs.iter()
                .filter(|r| r.span().contains(line, col))
                .cloned()
                .next()
        })
    }

    /// ディレクティブ名に対応する全HTML参照を取得
    pub fn get_html_directive_references(
        &self,
        directive_name: &str,
    ) -> Vec<HtmlDirectiveReference> {
        let mut references = Vec::new();
        for entry in self.html_directive_references.iter() {
            for r in entry.value() {
                if r.directive_name == directive_name {
                    references.push(r.clone());
                }
            }
        }
        references
    }

    /// 指定URIの全ディレクティブ参照を取得
    pub fn get_all_directive_references_for_uri(
        &self,
        uri: &Url,
    ) -> Vec<HtmlDirectiveReference> {
        self.html_directive_references
            .get(uri)
            .map(|refs| refs.clone())
            .unwrap_or_default()
    }

    /// 全HTMLディレクティブ参照を取得（キャッシュ用）
    pub fn get_all_html_directive_references_for_cache(&self) -> Vec<HtmlDirectiveReference> {
        self.html_directive_references
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    // ========== クリア ==========

    /// HTML参照情報のみをクリア（Pass 3で収集する情報）
    pub fn clear_html_references(&self, uri: &Url) {
        self.html_scope_references.remove(uri);
        self.html_local_variables.remove(uri);
        for mut entry in self.html_local_variable_references.iter_mut() {
            entry.value_mut().retain(|r| &r.uri != uri);
        }
        self.html_directive_references.remove(uri);
    }

    pub fn clear_document(&self, uri: &Url) {
        self.html_scope_references.remove(uri);
        self.html_local_variables.remove(uri);
        for mut entry in self.html_local_variable_references.iter_mut() {
            entry.value_mut().retain(|r| &r.uri != uri);
        }
        self.html_form_bindings.remove(uri);
        self.html_directive_references.remove(uri);
    }

    pub fn clear_all(&self) {
        self.html_scope_references.clear();
        self.html_local_variables.clear();
        self.html_local_variable_references.clear();
        self.html_form_bindings.clear();
        self.html_directive_references.clear();
    }
}

impl Default for HtmlStore {
    fn default() -> Self {
        Self::new()
    }
}
