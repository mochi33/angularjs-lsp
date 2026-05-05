use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

use crate::model::{
    HtmlComponentUsage, HtmlDirectiveReference, HtmlFormBinding, HtmlLocalVariable,
    HtmlLocalVariableReference, HtmlNgModelTarget, HtmlScopeReference, HtmlUiSrefReference,
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
    /// HTML内の `ng-model="X"` ターゲット (URI -> Vec<HtmlNgModelTarget>)
    /// controller 側で明示的に \$scope に書かれていないプロパティでも、
    /// ng-model がバインドする箇所があれば暗黙的に scope に存在するとみなす
    /// (診断の false positive 抑制用)
    ng_model_targets: DashMap<Url, Vec<HtmlNgModelTarget>>,
    /// HTML 内の ui-router `ui-sref="state"` 参照 (URI -> Vec<HtmlUiSrefReference>)
    /// state 名 → state 定義へのジャンプ・ホバー解決に使う
    ui_sref_references: DashMap<Url, Vec<HtmlUiSrefReference>>,
    /// HTML 内のコンポーネント使用箇所 (URI -> Vec<HtmlComponentUsage>)
    /// `<user-card user="..." on-select="...">` のような要素単位での
    /// 「コンポーネント名 + 全属性リスト」を保持する。
    /// component bindings との対応漏れ診断 (#64) で使う。
    component_usages: DashMap<Url, Vec<HtmlComponentUsage>>,
}

impl HtmlStore {
    pub fn new() -> Self {
        Self {
            html_scope_references: DashMap::new(),
            html_local_variables: DashMap::new(),
            html_local_variable_references: DashMap::new(),
            html_form_bindings: DashMap::new(),
            html_directive_references: DashMap::new(),
            ng_model_targets: DashMap::new(),
            ui_sref_references: DashMap::new(),
            component_usages: DashMap::new(),
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

    // ========== コンポーネント使用箇所 ==========

    /// HTML 上で使われたコンポーネント要素 1 件を登録
    pub fn add_component_usage(&self, usage: HtmlComponentUsage) {
        let uri = usage.uri.clone();
        let mut entry = self.component_usages.entry(uri).or_default();
        // 同位置の重複 (同じ start_line, start_col) は無視
        let is_duplicate = entry.iter().any(|u| {
            u.component_name == usage.component_name
                && u.element_start_line == usage.element_start_line
                && u.element_start_col == usage.element_start_col
        });
        if !is_duplicate {
            entry.push(usage);
        }
    }

    /// 指定 URI のコンポーネント使用箇所を取得
    pub fn get_component_usages_for_uri(&self, uri: &Url) -> Vec<HtmlComponentUsage> {
        self.component_usages
            .get(uri)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    // ========== ng-model ターゲット ==========

    pub fn add_ng_model_target(&self, target: HtmlNgModelTarget) {
        let uri = target.uri.clone();
        self.ng_model_targets
            .entry(uri)
            .or_default()
            .push(target);
    }

    pub fn get_ng_model_targets_for_uri(&self, uri: &Url) -> Vec<HtmlNgModelTarget> {
        self.ng_model_targets
            .get(uri)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// 全 ng-model ターゲットを取得 (キャッシュ用)
    pub fn get_all_ng_model_targets_for_cache(&self) -> Vec<HtmlNgModelTarget> {
        self.ng_model_targets
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    // ========== ui-sref ==========

    pub fn add_ui_sref_reference(&self, reference: HtmlUiSrefReference) {
        let uri = reference.uri.clone();
        self.ui_sref_references
            .entry(uri)
            .or_default()
            .push(reference);
    }

    pub fn find_ui_sref_reference_at(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
    ) -> Option<HtmlUiSrefReference> {
        if let Some(refs) = self.ui_sref_references.get(uri) {
            for r in refs.iter() {
                if r.span().contains(line, col) {
                    return Some(r.clone());
                }
            }
        }
        None
    }

    pub fn get_ui_sref_references_for_uri(&self, uri: &Url) -> Vec<HtmlUiSrefReference> {
        self.ui_sref_references
            .get(uri)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// 指定 state 名にマッチする全 ui-sref 参照を返す
    pub fn get_ui_sref_references_by_state(&self, state_name: &str) -> Vec<HtmlUiSrefReference> {
        let mut result = Vec::new();
        for entry in self.ui_sref_references.iter() {
            for r in entry.value().iter() {
                if r.state_name == state_name {
                    result.push(r.clone());
                }
            }
        }
        result
    }

    /// 全 ui-sref 参照を取得 (キャッシュ用)
    pub fn get_all_ui_sref_references_for_cache(&self) -> Vec<HtmlUiSrefReference> {
        self.ui_sref_references
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
        self.ng_model_targets.remove(uri);
        self.ui_sref_references.remove(uri);
        self.component_usages.remove(uri);
    }

    pub fn clear_document(&self, uri: &Url) {
        self.html_scope_references.remove(uri);
        self.html_local_variables.remove(uri);
        for mut entry in self.html_local_variable_references.iter_mut() {
            entry.value_mut().retain(|r| &r.uri != uri);
        }
        self.html_form_bindings.remove(uri);
        self.html_directive_references.remove(uri);
        self.ng_model_targets.remove(uri);
        self.ui_sref_references.remove(uri);
        self.component_usages.remove(uri);
    }

    pub fn clear_all(&self) {
        self.html_scope_references.clear();
        self.html_local_variables.clear();
        self.html_local_variable_references.clear();
        self.html_form_bindings.clear();
        self.html_directive_references.clear();
        self.ng_model_targets.clear();
        self.ui_sref_references.clear();
        self.component_usages.clear();
    }
}

impl Default for HtmlStore {
    fn default() -> Self {
        Self::new()
    }
}
