use std::collections::HashMap;

use crate::model::Span;

/// ローカル変数/関数の定義位置
#[derive(Clone)]
pub(super) struct LocalVarLocation {
    pub(super) span: Span,
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
    /// $rootScope がDIされているかどうか
    pub(super) has_root_scope: bool,
}

/// ノードから抽出されたDI情報
///
/// 配列記法・関数パラメータ・識別子解決など、あらゆるパターンから
/// 統一的にDI情報を取得した結果をまとめる構造体
pub(super) struct DiInfo {
    /// $以外のDIされた依存サービス名
    pub(super) injected_services: Vec<String>,
    /// $scope がDIされているか
    pub(super) has_scope: bool,
    /// $rootScope がDIされているか
    pub(super) has_root_scope: bool,
}

impl DiInfo {
    pub(super) fn empty() -> Self {
        Self {
            injected_services: Vec::new(),
            has_scope: false,
            has_root_scope: false,
        }
    }

    /// DI情報があるかどうか（サービス、$scope、$rootScope のいずれか）
    pub(super) fn has_any(&self) -> bool {
        !self.injected_services.is_empty() || self.has_scope || self.has_root_scope
    }
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
    /// $inject パターン用: 関数名 -> $rootScope がDIされているか
    pub(super) inject_has_root_scope: HashMap<String, bool>,
    /// 既に定義済みの $scope プロパティ名（コントローラー名.プロパティ名 -> true）
    /// 最初の定義のみを登録するために使用
    pub(super) defined_scope_properties: HashMap<String, bool>,
    /// 既に定義済みの $rootScope プロパティ名（モジュール名.プロパティ名 -> true）
    /// 最初の定義のみを登録するために使用
    pub(super) defined_root_scope_properties: HashMap<String, bool>,
    /// 現在のモジュール名
    pub(super) current_module: Option<String>,
}

impl AnalyzerContext {
    pub(super) fn new() -> Self {
        Self {
            di_scopes: Vec::new(),
            inject_map: HashMap::new(),
            function_ranges: HashMap::new(),
            inject_has_scope: HashMap::new(),
            inject_has_root_scope: HashMap::new(),
            defined_scope_properties: HashMap::new(),
            defined_root_scope_properties: HashMap::new(),
            current_module: None,
        }
    }

    /// 現在のモジュール名を設定
    pub(super) fn set_current_module(&mut self, name: String) {
        self.current_module = Some(name);
    }

    /// 現在のモジュール名を取得
    pub(super) fn get_current_module(&self) -> Option<&String> {
        self.current_module.as_ref()
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

    /// 指定位置の $rootScope 情報を取得（モジュール名, has_root_scope）
    pub(super) fn get_root_scope_info_at(&self, line: u32) -> Option<(String, bool)> {
        // モジュール名が設定されていない場合は None
        let module_name = self.current_module.as_ref()?;

        // 1. di_scopes から現在位置のスコープを探す（内側から外側へ）
        for scope in self.di_scopes.iter().rev() {
            if line >= scope.body_start_line && line <= scope.body_end_line {
                return Some((module_name.clone(), scope.has_root_scope));
            }
        }

        // 2. $inject パターンのスコープもチェック
        for (func_name, range) in &self.function_ranges {
            if line >= range.0 && line <= range.1 {
                if let Some(&has_root_scope) = self.inject_has_root_scope.get(func_name) {
                    return Some((module_name.clone(), has_root_scope));
                }
            }
        }

        None
    }
}
