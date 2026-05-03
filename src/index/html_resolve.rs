//! HTML 上のカーソル位置を「何の参照か」に解決するロジック。
//!
//! `definition` / `hover` / `references` の3ハンドラが共通で使う優先順位チェインを
//! 1箇所に集約する (issue #49)。各ハンドラは [`HtmlResolution`] の variant ごとに
//! `match` するだけで済むため:
//!
//! - チェインを変える (例: 新しい属性種別を追加) ときに3ハンドラへ自動的に反映される
//! - コンパイラが variant 実装漏れを警告してくれる
//! - 各ハンドラは「resolution → response」のマッピングに専念できる

use tower_lsp::lsp_types::{Position, Url};

use super::Index;
use crate::model::{
    HtmlDirectiveReference, HtmlFormBinding, HtmlLocalVariable, HtmlUiSrefReference,
};

/// HTML 上のカーソル位置に対応する解決結果。
///
/// 解決優先順位 (高い順):
/// 1. `UiSref`               — `ui-sref="state"` の state 名
/// 2. `Directive`            — カスタムディレクティブ / コンポーネント参照
/// 3. `LocalVarDef`          — `ng-init` / `ng-repeat` ローカル変数の定義位置
/// 4. `LocalVarRef`          — ローカル変数の参照 (定義済み)
/// 5. `FormBindingDef`       — `<form name="x">` の name 属性値
/// 6. `InheritedFormBinding` — 親テンプレートで定義されたフォーム名への参照
/// 7. `InheritedLocalVar`    — 親テンプレートで定義されたローカル変数への参照
/// 8. `Scope`                — `$scope` プロパティ参照 (controller as alias 含む)
///
/// `Scope` の後段処理 (`$scope.X` → `controller.X` (alias) → `$rootScope.X` →
/// ng-model 暗黙的 → 失敗) は各ハンドラ側で実装する。これらの fallback chain は
/// ハンドラごとに作る出力 (Location / Hover / Vec<Location>) が異なるため、
/// 共通化はせず variant のレベルだけ揃える。
#[derive(Debug, Clone)]
pub enum HtmlResolution {
    UiSref(HtmlUiSrefReference),
    Directive(HtmlDirectiveReference),
    LocalVarDef(HtmlLocalVariable),
    /// 参照位置から解決した「変数定義」を保持する。後段処理は `LocalVarDef` と同じ
    /// (=定義位置にジャンプ / hover で var の情報を表示) のため、共通の payload。
    LocalVarRef(HtmlLocalVariable),
    FormBindingDef(HtmlFormBinding),
    InheritedFormBinding(HtmlFormBinding),
    InheritedLocalVar(HtmlLocalVariable),
    Scope {
        /// 候補となる controller 名 (alias 解決済みなら 1 件、未解決なら継承チェイン全部)
        controllers: Vec<String>,
        /// `alias.foo` のうち `foo` 部分 (alias 未解決なら元の文字列全体)
        property_path: String,
        /// alias 解決が成功したかどうか。`controllers` が単一要素であることを意味し、
        /// `controller as` 構文経由の `this.X` メソッドルックアップを許可するシグナル。
        is_alias: bool,
    },
}

impl Index {
    /// HTML 上のカーソル位置に対応する [`HtmlResolution`] を返す。
    ///
    /// `source` は `find_html_scope_reference_at` でも `find_html_*_at` でも当たらない
    /// 場合の最終フォールバック (継承された var / form binding を識別子だけで引く) で
    /// のみ使う。`None` の場合はそのフォールバック自体がスキップされるが、それ以外の
    /// 解決には影響しない。
    pub fn resolve_html_position(
        &self,
        uri: &Url,
        position: Position,
        source: Option<&str>,
    ) -> Option<HtmlResolution> {
        // 0. ui-router の `ui-sref="state"` を最優先
        // (state 名は専用空間なので `controller` 等として解決すると誤動作する)
        if let Some(ui_sref) = self
            .html
            .find_ui_sref_reference_at(uri, position.line, position.character)
        {
            return Some(HtmlResolution::UiSref(ui_sref));
        }

        // 0b. カスタムディレクティブ / コンポーネント参照
        if let Some(directive_ref) = self
            .html
            .find_html_directive_reference_at(uri, position.line, position.character)
        {
            return Some(HtmlResolution::Directive(directive_ref));
        }

        // 1. ローカル変数の「定義位置」にカーソルがあるか
        if let Some(var_def) = self
            .html
            .find_html_local_variable_definition_at(uri, position.line, position.character)
        {
            return Some(HtmlResolution::LocalVarDef(var_def));
        }

        // 2. ローカル変数の「参照位置」にカーソルがあるか → 定義を引く
        if let Some(var_ref) = self
            .html
            .find_html_local_variable_at(uri, position.line, position.character)
        {
            if let Some(var_def) =
                self.find_local_variable_definition(uri, &var_ref.variable_name, position.line)
            {
                return Some(HtmlResolution::LocalVarRef(var_def));
            }
        }

        // 3. フォームバインディングの「定義位置」(`<form name="x">` の name) にカーソル
        if let Some(form_binding) = self
            .html
            .find_html_form_binding_at(uri, position.line, position.character)
        {
            return Some(HtmlResolution::FormBindingDef(form_binding));
        }

        // 4. $scope 参照 (式内の識別子 / 補間内の識別子)
        if let Some(html_ref) = self
            .html
            .find_html_scope_reference_at(uri, position.line, position.character)
        {
            let base_name = html_ref
                .property_path
                .split('.')
                .next()
                .unwrap_or(&html_ref.property_path);

            // 4a. base_name が継承されたフォーム名と一致 → InheritedFormBinding
            if let Some(form_binding) =
                self.find_form_binding_definition(uri, base_name, position.line)
            {
                return Some(HtmlResolution::InheritedFormBinding(form_binding));
            }

            // 4b. base_name が継承されたローカル変数名と一致 → InheritedLocalVar
            if let Some(var_def) =
                self.find_local_variable_definition(uri, base_name, position.line)
            {
                return Some(HtmlResolution::InheritedLocalVar(var_def));
            }

            // 4c. alias.property 形式なら controller 解決を試みる
            //     (alias 解決成功 → controllers=[ctrl], property_path=prop, is_alias=true)
            //     (alias 解決失敗 / 単純識別子 → controllers=継承チェイン全部, is_alias=false)
            let (controllers, property_path, is_alias) = self.resolve_scope_target(
                uri,
                position.line,
                &html_ref.property_path,
            );

            return Some(HtmlResolution::Scope {
                controllers,
                property_path,
                is_alias,
            });
        }

        // 5. 最終フォールバック: scope ref が登録されていない位置でも、識別子を切り出して
        //    継承された form binding / local var を引く (子テンプレートが親より先に
        //    解析された場合に発生する)
        if let Some(src) = source {
            if let Some(identifier) = extract_identifier_at_position(src, position) {
                let base_name = identifier.split('.').next().unwrap_or(&identifier);
                if let Some(form_binding) =
                    self.find_form_binding_definition(uri, base_name, position.line)
                {
                    return Some(HtmlResolution::InheritedFormBinding(form_binding));
                }
                if let Some(var_def) =
                    self.find_local_variable_definition(uri, &identifier, position.line)
                {
                    return Some(HtmlResolution::InheritedLocalVar(var_def));
                }
            }
        }

        None
    }

    /// `property_path` を controller 解決ルールに従って `(controllers, prop, is_alias)`
    /// に分解する。
    fn resolve_scope_target(
        &self,
        uri: &Url,
        line: u32,
        property_path: &str,
    ) -> (Vec<String>, String, bool) {
        if let Some((alias, prop)) = property_path.split_once('.') {
            if let Some(controller) = self.resolve_controller_by_alias(uri, line, alias) {
                return (vec![controller], prop.to_string(), true);
            }
        }
        (
            self.resolve_controllers_for_html(uri, line),
            property_path.to_string(),
            false,
        )
    }
}

/// ソースの行/列位置から識別子文字列を切り出す。
///
/// 識別子文字 = `is_alphanumeric() || '_' || '$'`。境界外なら `None`。
/// `find_html_scope_reference_at` でヒットしない場合の最終フォールバックで使う。
fn extract_identifier_at_position(source: &str, position: Position) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    let line = lines.get(position.line as usize)?;
    let col = position.character as usize;

    if col >= line.len() {
        return None;
    }

    let chars: Vec<char> = line.chars().collect();

    let mut start = col;
    while start > 0 {
        let c = chars[start - 1];
        if !c.is_alphanumeric() && c != '_' && c != '$' {
            break;
        }
        start -= 1;
    }

    let mut end = col;
    while end < chars.len() {
        let c = chars[end];
        if !c.is_alphanumeric() && c != '_' && c != '$' {
            break;
        }
        end += 1;
    }

    if start == end {
        return None;
    }

    let identifier: String = chars[start..end].iter().collect();
    if identifier.is_empty() {
        None
    } else {
        Some(identifier)
    }
}
