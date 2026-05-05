use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

use crate::model::DiArityIssue;

/// アナライザーが収集した診断補助情報を保持するストア。
///
/// 解析処理の中でしか取れない情報 (AST 由来の DI arity 不一致など) を
/// `DiagnosticsHandler` から読み出せるよう中継する。
pub struct DiagnosticsStore {
    /// URI ごとの DI arity 不一致リスト
    di_arity_issues: DashMap<Url, Vec<DiArityIssue>>,
}

impl DiagnosticsStore {
    pub fn new() -> Self {
        Self {
            di_arity_issues: DashMap::new(),
        }
    }

    /// DI arity 不一致を登録する
    pub fn add_di_arity_issue(&self, issue: DiArityIssue) {
        self.di_arity_issues
            .entry(issue.uri.clone())
            .or_default()
            .push(issue);
    }

    /// 指定 URI の DI arity 不一致リストを取得する
    pub fn get_di_arity_issues(&self, uri: &Url) -> Vec<DiArityIssue> {
        self.di_arity_issues
            .get(uri)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    /// 指定 URI の情報をクリアする
    pub fn clear_document(&self, uri: &Url) {
        self.di_arity_issues.remove(uri);
    }

    /// 全データをクリアする
    pub fn clear_all(&self) {
        self.di_arity_issues.clear();
    }
}

impl Default for DiagnosticsStore {
    fn default() -> Self {
        Self::new()
    }
}
