use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::Url;

use super::span::Span;

/// DI 配列の要素数と関数の引数数の不一致を表す診断情報
///
/// 認識パターン:
/// ```javascript
/// // di_count = 2, param_count = 1 → 警告
/// .controller('Ctrl', ['$scope', '$timeout', function($scope) {}])
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiArityIssue {
    /// この診断を出すドキュメント
    pub uri: Url,
    /// DI 配列の文字列要素の数
    pub di_count: usize,
    /// 関数 (または class constructor) の引数の数
    pub param_count: usize,
    /// 警告の表示位置 (関数本体または class 全体)
    pub span: Span,
}
