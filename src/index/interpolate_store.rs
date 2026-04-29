use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

/// `js_detected` から取り出した 1 エントリ。`(URI, (start_symbol, end_symbol))`。
type DetectedEntry = (Url, (Option<String>, Option<String>));

/// AngularJS の interpolate 記号 (`{{` / `}}` または `$interpolateProvider` で
/// カスタマイズされた値) を解決するストア。
///
/// 解決順:
/// 1. JS ソース中で検出された `$interpolateProvider.startSymbol(...)` /
///    `$interpolateProvider.endSymbol(...)` の値 (URI ごとに保持)
/// 2. AngularJS デフォルトの `{{` / `}}`
///
/// 旧来は `ajsconfig.json` の `interpolate.startSymbol/endSymbol` を
/// フォールバックとしていたが、現在は AngularJS 構文からの解決に一本化している。
pub struct InterpolateStore {
    /// JS から検出された symbols (URI → (start, end))。
    /// 各 URI は `$interpolateProvider.startSymbol(...)` か
    /// `$interpolateProvider.endSymbol(...)` のどちらか/両方を持ち得る。
    js_detected: DashMap<Url, (Option<String>, Option<String>)>,
}

impl InterpolateStore {
    pub fn new() -> Self {
        Self {
            js_detected: DashMap::new(),
        }
    }

    /// 指定 URI で `$interpolateProvider.startSymbol(...)` を検出した
    pub fn set_start_symbol(&self, uri: Url, symbol: String) {
        let mut entry = self.js_detected.entry(uri).or_insert((None, None));
        entry.0 = Some(symbol);
    }

    /// 指定 URI で `$interpolateProvider.endSymbol(...)` を検出した
    pub fn set_end_symbol(&self, uri: Url, symbol: String) {
        let mut entry = self.js_detected.entry(uri).or_insert((None, None));
        entry.1 = Some(symbol);
    }

    /// 指定 URI の検出値を削除 (clear_document 時)
    pub fn clear_document(&self, uri: &Url) {
        self.js_detected.remove(uri);
    }

    /// 全エントリをクリア
    pub fn clear_all(&self) {
        self.js_detected.clear();
    }

    /// キャッシュ書き出し用に全 JS 検出エントリを取り出す。
    ///
    /// 返り値は `(URI, start_symbol, end_symbol)` の Vec。順序は不定。
    /// `config_fallback` は `ajsconfig.json` から起動時に再構築されるので
    /// キャッシュ対象には含めない。
    pub fn iter_js_detected_for_cache(&self) -> Vec<(Url, Option<String>, Option<String>)> {
        self.js_detected
            .iter()
            .map(|e| {
                let (s, end) = e.value().clone();
                (e.key().clone(), s, end)
            })
            .collect()
    }

    /// キャッシュ復元用にエントリを丸ごと書き戻す。
    /// 既存エントリがあれば上書きする (start/end どちらか片方だけ既存値があるケースは
    /// 通常の `set_*_symbol` 経路で発生するが、復元時は丸ごとの上書きで十分)。
    pub fn restore_from_cache(&self, uri: Url, start: Option<String>, end: Option<String>) {
        if start.is_none() && end.is_none() {
            return;
        }
        self.js_detected.insert(uri, (start, end));
    }

    /// 解決された (start_symbol, end_symbol) を返す。
    ///
    /// JS 検出値 → AngularJS デフォルト (`{{` / `}}`) の順で解決する。
    /// 複数の URI が JS 検出値を持つ場合は URI 順 (lexicographic) で最初に
    /// 見つかった非 None 値を採用する (決定的)。
    /// start と end は別々に解決されるので、片方だけ JS 検出されたケースも
    /// 正しく扱える。
    pub fn resolved(&self) -> (String, String) {
        // 決定的にするため URI でソートして走査
        let mut entries: Vec<DetectedEntry> = self
            .js_detected
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        let mut start: Option<String> = None;
        let mut end: Option<String> = None;
        for (_, (s, e)) in entries {
            if start.is_none() && s.is_some() {
                start = s;
            }
            if end.is_none() && e.is_some() {
                end = e;
            }
            if start.is_some() && end.is_some() {
                break;
            }
        }

        (
            start.unwrap_or_else(|| "{{".to_string()),
            end.unwrap_or_else(|| "}}".to_string()),
        )
    }
}

impl Default for InterpolateStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(path: &str) -> Url {
        Url::parse(&format!("file://{}", path)).unwrap()
    }

    #[test]
    fn defaults_to_double_curly() {
        let store = InterpolateStore::new();
        assert_eq!(store.resolved(), ("{{".to_string(), "}}".to_string()));
    }

    #[test]
    fn js_detected_overrides_default() {
        let store = InterpolateStore::new();
        let uri = url("/app/config.js");
        store.set_start_symbol(uri.clone(), "{<".to_string());
        store.set_end_symbol(uri, ">}".to_string());
        assert_eq!(store.resolved(), ("{<".to_string(), ">}".to_string()));
    }

    #[test]
    fn partial_js_detected_falls_back_to_default_for_missing_side() {
        // start のみ JS 検出、end はデフォルトの `}}` にフォールバック
        let store = InterpolateStore::new();
        let uri = url("/app/config.js");
        store.set_start_symbol(uri, "{<".to_string());
        // end_symbol は未設定 → default `}}`
        assert_eq!(store.resolved(), ("{<".to_string(), "}}".to_string()));
    }

    #[test]
    fn clear_document_removes_entry() {
        let store = InterpolateStore::new();
        let uri = url("/app/config.js");
        store.set_start_symbol(uri.clone(), "{<".to_string());
        store.set_end_symbol(uri.clone(), ">}".to_string());
        assert_eq!(store.resolved(), ("{<".to_string(), ">}".to_string()));

        store.clear_document(&uri);
        // 検出値が消えたのでデフォルトに戻る
        assert_eq!(store.resolved(), ("{{".to_string(), "}}".to_string()));
    }

    #[test]
    fn multiple_uris_first_in_uri_order_wins() {
        // 同じシンボルを複数 JS が宣言した場合、URI 順で最初に出るものを採用 (決定的)
        let store = InterpolateStore::new();
        // /a.js が後、/b.js が先になるよう URI を選ぶ
        store.set_start_symbol(url("/b.js"), "B<".to_string());
        store.set_start_symbol(url("/a.js"), "A<".to_string());
        // /a.js が URI sort で先 → A< が採用
        assert_eq!(store.resolved().0, "A<".to_string());
    }

    #[test]
    fn split_across_uris_each_contributes() {
        // 1 URI が start のみ、別 URI が end のみ宣言したケース
        let store = InterpolateStore::new();
        store.set_start_symbol(url("/a.js"), "<%".to_string());
        store.set_end_symbol(url("/b.js"), "%>".to_string());
        assert_eq!(store.resolved(), ("<%".to_string(), "%>".to_string()));
    }

    #[test]
    fn iter_js_detected_for_cache_returns_all_entries() {
        let store = InterpolateStore::new();
        store.set_start_symbol(url("/a.js"), "<%".to_string());
        store.set_end_symbol(url("/a.js"), "%>".to_string());
        store.set_start_symbol(url("/b.js"), "[[".to_string());

        let mut entries = store.iter_js_detected_for_cache();
        entries.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, url("/a.js"));
        assert_eq!(entries[0].1, Some("<%".to_string()));
        assert_eq!(entries[0].2, Some("%>".to_string()));
        assert_eq!(entries[1].0, url("/b.js"));
        assert_eq!(entries[1].1, Some("[[".to_string()));
        assert_eq!(entries[1].2, None);
    }

    #[test]
    fn restore_from_cache_round_trip() {
        // 1. 元の store を構築
        let store = InterpolateStore::new();
        store.set_start_symbol(url("/a.js"), "<%".to_string());
        store.set_end_symbol(url("/a.js"), "%>".to_string());
        store.set_start_symbol(url("/b.js"), "[[".to_string());
        let snapshot = store.iter_js_detected_for_cache();

        // 2. 別 store に復元
        let restored = InterpolateStore::new();
        for (uri, s, e) in snapshot {
            restored.restore_from_cache(uri, s, e);
        }

        // 3. resolved() が同じ結果になること
        assert_eq!(store.resolved(), restored.resolved());

        // /a.js の start/end が両方復元されていること
        let entries = restored.iter_js_detected_for_cache();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn restore_from_cache_skips_empty_entries() {
        let store = InterpolateStore::new();
        store.restore_from_cache(url("/a.js"), None, None);
        // 空エントリはスキップされる
        assert!(store.iter_js_detected_for_cache().is_empty());
    }
}
