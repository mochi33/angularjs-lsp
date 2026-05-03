//! DI 配列の string literal を URI ごとに保持するストア
//!
//! `extract_dependencies` はユーザ定義サービスのみ `SymbolReference` として
//! 登録し、`$` 始まりの組み込みサービスはスキップする。一方、未登録サービス
//! 警告 (issue #63) では `$tiemout` のような `$` 付き typo も検出したいので、
//! DI 配列で書かれた string literal を生のまま保存しておく必要がある。
//!
//! 本ストアは「未知サービス警告」用に閉じたデータを持つ：
//! - URI 単位で全 DI usage を保持
//! - `clear_document` で当該 URI 分だけ消す
//! - 順序保持・重複は許容 (位置で識別)

use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

use crate::model::Span;

/// DI 配列内に出現した 1 個の string literal
#[derive(Debug, Clone)]
pub struct DiUsage {
    /// 文字列リテラルの中身 (例: `$scope`, `UserService`, `$tiemout`)
    pub name: String,
    /// 文字列リテラル全体の位置 (クォート込み)
    pub span: Span,
}

/// DI 配列内の string literal を URI 単位で蓄積するストア
pub struct DiUsageStore {
    usages: DashMap<Url, Vec<DiUsage>>,
}

impl DiUsageStore {
    pub fn new() -> Self {
        Self {
            usages: DashMap::new(),
        }
    }

    /// `uri` に DI usage を 1 件追加する
    pub fn add(&self, uri: &Url, usage: DiUsage) {
        self.usages
            .entry(uri.clone())
            .or_default()
            .push(usage);
    }

    /// 指定 URI の DI usage を全件取得する (clone)
    pub fn get_for_uri(&self, uri: &Url) -> Vec<DiUsage> {
        self.usages
            .get(uri)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    pub fn clear_document(&self, uri: &Url) {
        self.usages.remove(uri);
    }

    pub fn clear_all(&self) {
        self.usages.clear();
    }
}

impl Default for DiUsageStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(line: u32) -> Span {
        Span::new(line, 0, line, 5)
    }

    #[test]
    fn add_and_get_for_uri_returns_usages_in_order() {
        let store = DiUsageStore::new();
        let uri = Url::parse("file:///a.js").unwrap();
        store.add(
            &uri,
            DiUsage {
                name: "$scope".into(),
                span: span(1),
            },
        );
        store.add(
            &uri,
            DiUsage {
                name: "$tiemout".into(),
                span: span(2),
            },
        );

        let usages = store.get_for_uri(&uri);
        assert_eq!(usages.len(), 2);
        assert_eq!(usages[0].name, "$scope");
        assert_eq!(usages[1].name, "$tiemout");
    }

    #[test]
    fn get_for_uri_returns_empty_when_unknown() {
        let store = DiUsageStore::new();
        let uri = Url::parse("file:///unknown.js").unwrap();
        assert!(store.get_for_uri(&uri).is_empty());
    }

    #[test]
    fn clear_document_drops_only_target_uri() {
        let store = DiUsageStore::new();
        let uri_a = Url::parse("file:///a.js").unwrap();
        let uri_b = Url::parse("file:///b.js").unwrap();
        store.add(
            &uri_a,
            DiUsage {
                name: "X".into(),
                span: span(1),
            },
        );
        store.add(
            &uri_b,
            DiUsage {
                name: "Y".into(),
                span: span(1),
            },
        );

        store.clear_document(&uri_a);
        assert!(store.get_for_uri(&uri_a).is_empty());
        assert_eq!(store.get_for_uri(&uri_b).len(), 1);
    }

    #[test]
    fn clear_all_drops_all_usages() {
        let store = DiUsageStore::new();
        let uri_a = Url::parse("file:///a.js").unwrap();
        store.add(
            &uri_a,
            DiUsage {
                name: "X".into(),
                span: span(1),
            },
        );
        store.clear_all();
        assert!(store.get_for_uri(&uri_a).is_empty());
    }
}
