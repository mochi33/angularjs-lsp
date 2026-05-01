use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Url;

use super::span::Span;

/// HTML内のスコープ参照
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HtmlScopeReference {
    pub property_path: String,
    pub uri: Url,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl HtmlScopeReference {
    pub fn span(&self) -> Span {
        Span::new(self.start_line, self.start_col, self.end_line, self.end_col)
    }
}

/// HTML内で定義されたローカル変数のソース
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HtmlLocalVariableSource {
    /// ng-init="counter = 0" -> "counter"
    NgInit,
    /// ng-repeat="item in items" -> "item"
    NgRepeatIterator,
    /// ng-repeat="(key, value) in obj" -> "key", "value"
    NgRepeatKeyValue,
    /// ng-repeat スコープで暗黙に利用可能な特殊変数
    /// ($index, $first, $last, $middle, $odd, $even)
    NgRepeatSpecial,
}

impl HtmlLocalVariableSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            HtmlLocalVariableSource::NgInit => "ng-init",
            HtmlLocalVariableSource::NgRepeatIterator => "ng-repeat",
            HtmlLocalVariableSource::NgRepeatKeyValue => "ng-repeat",
            HtmlLocalVariableSource::NgRepeatSpecial => "ng-repeat (special)",
        }
    }
}

/// HTML内で定義されたローカル変数（ng-init, ng-repeat由来）
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HtmlLocalVariable {
    pub name: String,
    pub source: HtmlLocalVariableSource,
    pub uri: Url,
    /// スコープの開始行（定義要素の開始）
    pub scope_start_line: u32,
    /// スコープの終了行（定義要素の終了）
    pub scope_end_line: u32,
    /// 変数名の定義位置（正確な位置）
    pub name_start_line: u32,
    pub name_start_col: u32,
    pub name_end_line: u32,
    pub name_end_col: u32,
}

impl HtmlLocalVariable {
    pub fn name_span(&self) -> Span {
        Span::new(
            self.name_start_line,
            self.name_start_col,
            self.name_end_line,
            self.name_end_col,
        )
    }

    pub fn is_in_scope(&self, line: u32) -> bool {
        line >= self.scope_start_line && line <= self.scope_end_line
    }
}

/// HTML内のローカル変数への参照
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HtmlLocalVariableReference {
    pub variable_name: String,
    pub uri: Url,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl HtmlLocalVariableReference {
    pub fn span(&self) -> Span {
        Span::new(self.start_line, self.start_col, self.end_line, self.end_col)
    }
}

/// HTML内の<form name="x">で定義されるフォームバインディング
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HtmlFormBinding {
    pub name: String,
    pub uri: Url,
    /// スコープの開始行（ng-controllerスコープの開始、またはファイル先頭）
    pub scope_start_line: u32,
    /// スコープの終了行（ng-controllerスコープの終了、またはファイル末尾）
    pub scope_end_line: u32,
    /// name属性値の位置（正確な位置）
    pub name_start_line: u32,
    pub name_start_col: u32,
    pub name_end_line: u32,
    pub name_end_col: u32,
}

impl HtmlFormBinding {
    pub fn name_span(&self) -> Span {
        Span::new(
            self.name_start_line,
            self.name_start_col,
            self.name_end_line,
            self.name_end_col,
        )
    }

    pub fn is_in_scope(&self, line: u32) -> bool {
        line >= self.scope_start_line && line <= self.scope_end_line
    }
}

/// ng-include経由で継承されるローカル変数
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InheritedLocalVariable {
    pub name: String,
    pub source: HtmlLocalVariableSource,
    pub uri: Url,
    pub scope_start_line: u32,
    pub scope_end_line: u32,
    pub name_start_line: u32,
    pub name_start_col: u32,
    pub name_end_line: u32,
    pub name_end_col: u32,
}

/// ng-include経由で継承されるフォームバインディング
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InheritedFormBinding {
    pub name: String,
    pub uri: Url,
    pub scope_start_line: u32,
    pub scope_end_line: u32,
    pub name_start_line: u32,
    pub name_start_col: u32,
    pub name_end_line: u32,
    pub name_end_col: u32,
}

/// HTML内でのディレクティブ使用タイプ
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DirectiveUsageType {
    /// <my-directive>...</my-directive>
    Element,
    /// <div my-directive>...</div>
    Attribute,
}

/// `ng-model="X"` のターゲットとなるスコープパス。
///
/// AngularJS では `ng-model` は **書き込み可能なスコープパス** を取り、ディレクティブが
/// 実行時に対象 \$scope のプロパティを生成・更新する。すなわち
/// `<input ng-model="currentPage">` を書けば controller 側で
/// `$scope.currentPage = ...` を明示的に書かなくても \$scope にプロパティが
/// 生まれる。LSP の診断ではこのケースを controller 側の明示的定義と同等に扱う
/// ための **暗黙的定義** (implicit definition) のレコードとして使う。
///
/// 同名の明示的 `$scope.X = ...` 定義が controller にある場合は、そちらが
/// canonical な定義として優先される。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HtmlNgModelTarget {
    /// `ng-model` の値そのもの (例: "currentPage", "vm.currentPage", "user.profile.name")
    pub property_path: String,
    pub uri: Url,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl HtmlNgModelTarget {
    pub fn span(&self) -> Span {
        Span::new(self.start_line, self.start_col, self.end_line, self.end_col)
    }
}

/// HTML内のカスタムディレクティブ参照
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HtmlDirectiveReference {
    /// ディレクティブ名（camelCase形式、正規化済み）
    pub directive_name: String,
    pub uri: Url,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub usage_type: DirectiveUsageType,
}

impl HtmlDirectiveReference {
    pub fn span(&self) -> Span {
        Span::new(self.start_line, self.start_col, self.end_line, self.end_col)
    }
}

/// `ui-sref="home"` / `ui-sref="home.detail({id: 1})"` などで参照される
/// ui-router state 名と、それが属性値として書かれているHTML上の位置範囲を表す。
///
/// `state_name` は `(` の前までを切り出した state 名のみ。
/// `start_*` / `end_*` はその state 名部分の位置 (引数部分は含まない)。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HtmlUiSrefReference {
    pub state_name: String,
    pub uri: Url,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl HtmlUiSrefReference {
    pub fn span(&self) -> Span {
        Span::new(self.start_line, self.start_col, self.end_line, self.end_col)
    }
}
