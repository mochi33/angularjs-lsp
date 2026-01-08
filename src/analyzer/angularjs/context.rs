use std::collections::HashMap;

/// ローカル変数/関数の定義位置
#[derive(Clone)]
pub(super) struct LocalVarLocation {
    pub(super) start_line: u32,
    pub(super) start_col: u32,
    pub(super) end_line: u32,
    pub(super) end_col: u32,
}

/// コンポーネントのDIスコープ情報
#[derive(Clone, Debug)]
pub(super) struct DiScope {
    /// コンポーネント名
    pub(super) component_name: String,
    /// DIされた依存サービス名のリスト
    pub(super) injected_services: Vec<String>,
    /// 関数本体の開始行
    pub(super) body_start_line: u32,
    /// 関数本体の終了行
    pub(super) body_end_line: u32,
    /// $scope がDIされているかどうか
    pub(super) has_scope: bool,
}

/// 解析コンテキスト
pub(super) struct AnalyzerContext {
    /// 現在有効なDIスコープのスタック
    pub(super) di_scopes: Vec<DiScope>,
    /// $inject パターン用: 関数名 -> DI依存関係
    pub(super) inject_map: HashMap<String, Vec<String>>,
    /// $inject パターン用: 関数名 -> 関数本体の範囲 (start_line, end_line)
    pub(super) function_ranges: HashMap<String, (u32, u32)>,
    /// $inject パターン用: 関数名 -> $scope がDIされているか
    pub(super) inject_has_scope: HashMap<String, bool>,
    /// 既に定義済みの $scope プロパティ名（コントローラー名.プロパティ名 -> true）
    /// 最初の定義のみを登録するために使用
    pub(super) defined_scope_properties: HashMap<String, bool>,
}

impl AnalyzerContext {
    pub(super) fn new() -> Self {
        Self {
            di_scopes: Vec::new(),
            inject_map: HashMap::new(),
            function_ranges: HashMap::new(),
            inject_has_scope: HashMap::new(),
            defined_scope_properties: HashMap::new(),
        }
    }

    /// 指定位置でサービスがDIされているかどうかをチェック
    pub(super) fn is_injected_at(&self, service_name: &str, line: u32) -> bool {
        // 1. di_scopes から現在位置のスコープを探す（内側から外側へ）
        for scope in self.di_scopes.iter().rev() {
            if line >= scope.body_start_line && line <= scope.body_end_line {
                return scope.injected_services.iter().any(|s| s == service_name);
            }
        }

        // 2. $inject パターンのスコープもチェック
        for (func_name, range) in &self.function_ranges {
            if line >= range.0 && line <= range.1 {
                if let Some(deps) = self.inject_map.get(func_name) {
                    return deps.iter().any(|s| s == service_name);
                }
            }
        }

        false
    }

    /// DIスコープを追加
    pub(super) fn push_scope(&mut self, scope: DiScope) {
        self.di_scopes.push(scope);
    }

    /// DIスコープを削除
    #[allow(dead_code)]
    pub(super) fn pop_scope(&mut self) {
        self.di_scopes.pop();
    }

    /// 指定位置で $scope がDIされているかどうかをチェック
    #[allow(dead_code)]
    pub(super) fn has_scope_at(&self, line: u32) -> bool {
        self.get_scope_info_at(line).map(|(_, has_scope)| has_scope).unwrap_or(false)
    }

    /// 指定位置のコントローラー名を取得
    #[allow(dead_code)]
    pub(super) fn get_controller_name_at(&self, line: u32) -> Option<String> {
        self.get_scope_info_at(line).map(|(name, _)| name)
    }

    /// 指定位置のスコープ情報を取得（コントローラー名, has_scope）
    /// has_scope_at と get_controller_name_at を統合し、同じスコープを返すことを保証
    pub(super) fn get_scope_info_at(&self, line: u32) -> Option<(String, bool)> {
        // 1. di_scopes から現在位置のスコープを探す（内側から外側へ）
        for scope in self.di_scopes.iter().rev() {
            if line >= scope.body_start_line && line <= scope.body_end_line {
                return Some((scope.component_name.clone(), scope.has_scope));
            }
        }

        // 2. $inject パターンのスコープもチェック
        for (func_name, range) in &self.function_ranges {
            if line >= range.0 && line <= range.1 {
                if let Some(&has_scope) = self.inject_has_scope.get(func_name) {
                    return Some((func_name.clone(), has_scope));
                }
            }
        }

        None
    }
}
