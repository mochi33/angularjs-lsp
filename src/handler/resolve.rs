use tower_lsp::lsp_types::Url;

use crate::index::Index;
use crate::model::{
    HtmlDirectiveReference, HtmlFormBinding, HtmlLocalVariable,
};

/// HTML内のカーソル位置で解決されたシンボルの種類
pub enum HtmlSymbolAtPosition {
    /// カスタムディレクティブ参照（<my-directive> や属性として使用）
    Directive(HtmlDirectiveReference),
    /// ローカル変数の定義位置（ng-repeat, ng-init の定義行にカーソルがある）
    LocalVariableDefinition(HtmlLocalVariable),
    /// ローカル変数の参照位置（参照箇所にカーソルがある）
    LocalVariableReference {
        variable_name: String,
        definition: HtmlLocalVariable,
    },
    /// フォームバインディングの定義位置（<form name="x"> の name にカーソルがある）
    FormBindingDefinition(HtmlFormBinding),
    /// フォームバインディングの参照位置（スコープ参照としてフォーム名が参照されている）
    FormBindingReference(HtmlFormBinding),
    /// スコープシンボル（$scope.xxx または controller as 構文のプロパティ）
    ScopeSymbol {
        symbol_name: String,
        controller: String,
        is_controller_as: bool,
    },
    /// $rootScopeシンボル
    RootScopeSymbol {
        symbol_name: String,
    },
    /// 継承されたローカル変数（ng-include経由で親テンプレートから継承）
    InheritedLocalVariable(HtmlLocalVariable),
}

/// HTML内のカーソル位置からシンボル情報を解決する
///
/// 複数のハンドラー（hover, references, rename, definition）で共通して使用される
/// 位置からのシンボル解決ロジックを一元化する。
///
/// 解決の優先順位:
/// 1. カスタムディレクティブ参照
/// 2. ローカル変数定義位置
/// 3. ローカル変数参照位置
/// 4. フォームバインディング定義位置
/// 5. スコープ参照 -> フォームバインディング参照、継承ローカル変数、エイリアス解決、$scope、$rootScope
pub fn resolve_html_position(
    index: &Index,
    uri: &Url,
    line: u32,
    col: u32,
) -> Option<HtmlSymbolAtPosition> {
    // 1. カスタムディレクティブ参照をチェック
    if let Some(directive_ref) = index.html.find_html_directive_reference_at(uri, line, col) {
        return Some(HtmlSymbolAtPosition::Directive(directive_ref));
    }

    // 2. ローカル変数定義位置をチェック
    if let Some(local_var_def) = index.html.find_html_local_variable_definition_at(uri, line, col)
    {
        return Some(HtmlSymbolAtPosition::LocalVariableDefinition(local_var_def));
    }

    // 3. ローカル変数参照をチェック
    if let Some(local_var_ref) = index.html.find_html_local_variable_at(uri, line, col) {
        if let Some(var_def) =
            index.find_local_variable_definition(uri, &local_var_ref.variable_name, line)
        {
            return Some(HtmlSymbolAtPosition::LocalVariableReference {
                variable_name: local_var_ref.variable_name.clone(),
                definition: var_def,
            });
        }
    }

    // 4. フォームバインディング定義位置をチェック
    if let Some(form_binding) = index.html.find_html_form_binding_at(uri, line, col) {
        return Some(HtmlSymbolAtPosition::FormBindingDefinition(form_binding));
    }

    // 5. スコープ参照を取得して解析
    let html_ref = index.html.find_html_scope_reference_at(uri, line, col)?;

    // 5a. フォームバインディング参照かどうかをチェック
    let base_name = html_ref
        .property_path
        .split('.')
        .next()
        .unwrap_or(&html_ref.property_path);
    if let Some(form_binding) = index.find_form_binding_definition(uri, base_name, line) {
        return Some(HtmlSymbolAtPosition::FormBindingReference(form_binding));
    }

    // 5b. 継承されたローカル変数かどうかをチェック
    if let Some(var_def) = index.find_local_variable_definition(uri, base_name, line) {
        return Some(HtmlSymbolAtPosition::InheritedLocalVariable(var_def));
    }

    // 5c. alias.property 形式をチェック（controller as alias 構文）
    let (resolved_controller, property_path) = if html_ref.property_path.contains('.') {
        let parts: Vec<&str> = html_ref.property_path.splitn(2, '.').collect();
        if parts.len() == 2 {
            let alias = parts[0];
            let prop = parts[1];
            if let Some(controller) = index.resolve_controller_by_alias(uri, line, alias) {
                (Some(controller), prop.to_string())
            } else {
                (None, html_ref.property_path.clone())
            }
        } else {
            (None, html_ref.property_path.clone())
        }
    } else {
        (None, html_ref.property_path.clone())
    };

    // コントローラーを解決
    let is_controller_as = resolved_controller.is_some();
    let controllers = if let Some(controller) = resolved_controller {
        vec![controller]
    } else {
        index.resolve_controllers_for_html(uri, line)
    };

    // 各コントローラーを順番に試して、定義が見つかったものを返す
    for controller_name in &controllers {
        let symbol_name = format!("{}.$scope.{}", controller_name, property_path);
        if index.definitions.has_definition(&symbol_name) {
            return Some(HtmlSymbolAtPosition::ScopeSymbol {
                symbol_name,
                controller: controller_name.clone(),
                is_controller_as,
            });
        }
    }

    // controller as 構文の場合、ControllerName.method 形式も検索
    if is_controller_as {
        for controller_name in &controllers {
            let symbol_name = format!("{}.{}", controller_name, property_path);
            if index.definitions.has_definition(&symbol_name) {
                return Some(HtmlSymbolAtPosition::ScopeSymbol {
                    symbol_name,
                    controller: controller_name.clone(),
                    is_controller_as,
                });
            }
        }
    }

    // $rootScope からのグローバル参照を検索
    if let Some(root_scope_symbol) = index
        .definitions
        .find_root_scope_symbol_name_by_property(&property_path)
    {
        return Some(HtmlSymbolAtPosition::RootScopeSymbol {
            symbol_name: root_scope_symbol,
        });
    }

    None
}
