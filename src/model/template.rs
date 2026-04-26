use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Url;

/// テンプレートバインディングのソース
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingSource {
    NgController,
    RouteProvider,
    StateProvider,
    UibModal,
    MdDialog,
    MdBottomSheet,
    MdToast,
    MdPanel,
    NgDialog,
}

impl BindingSource {
    /// バインディング元を表す表示用ラベル
    /// (Code Lens のタイトルなど、ユーザーに見せる文字列で使う)
    pub fn label(&self) -> &'static str {
        match self {
            BindingSource::NgController => "ng-controller",
            BindingSource::RouteProvider => "$routeProvider",
            BindingSource::StateProvider => "$stateProvider",
            BindingSource::UibModal => "$uibModal",
            BindingSource::MdDialog => "$mdDialog",
            BindingSource::MdBottomSheet => "$mdBottomSheet",
            BindingSource::MdToast => "$mdToast",
            BindingSource::MdPanel => "$mdPanel",
            BindingSource::NgDialog => "ngDialog",
        }
    }
}

/// HTMLテンプレートとコントローラーのバインディング
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemplateBinding {
    pub template_path: String,
    pub controller_name: String,
    pub source: BindingSource,
    /// バインディング定義のURI（JSファイル）
    pub binding_uri: Url,
    /// バインディング定義の行番号（templateUrlプロパティの位置）
    pub binding_line: u32,
}

/// コンポーネントのtemplateUrl情報（CodeLens用）
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComponentTemplateUrl {
    /// 定義元のURI（JSファイル）
    pub uri: Url,
    /// templateUrlの値（パス）
    pub template_path: String,
    /// templateUrlプロパティの行番号
    pub line: u32,
    /// templateUrlプロパティの列番号
    pub col: u32,
    /// コントローラー名（文字列参照、識別子参照、またはインラインコントローラーの場合はNone）
    pub controller_name: Option<String>,
    /// controllerAsエイリアス（デフォルト: "$ctrl"）
    pub controller_as: String,
}
