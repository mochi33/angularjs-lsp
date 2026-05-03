//! AngularJS 1.x の組み込みサービス allowlist
//!
//! `$scope` / `$http` などコアモジュール由来のサービスや、ngRoute / ngResource /
//! ngCookies / ngAnimate / ui-router といった同梱されることの多い拡張モジュール
//! 由来のサービス・プロバイダ名を静的に列挙する。
//!
//! このリストはユーザー定義サービス (`.service('Foo', ...)` 等) や
//! ワークスペース内で見つからなかったときに「組み込みかどうか」を判定するため
//! に使う。`is_builtin_service` で完全一致判定する。
//!
//! 第三者ライブラリ等で追加された名前は ajsconfig.json で allowlist 拡張する
//! 想定だが、現状は静的リストのみ。

/// AngularJS 組み込みサービス・プロバイダの完全一致 allowlist
const BUILTIN_SERVICES: &[&str] = &[
    // --- core services ---
    "$scope",
    "$rootScope",
    "$rootElement",
    "$injector",
    "$provide",
    "$compile",
    "$compileProvider",
    "$controller",
    "$controllerProvider",
    "$parse",
    "$interpolate",
    "$interpolateProvider",
    "$filter",
    "$filterProvider",
    "$http",
    "$httpProvider",
    "$httpBackend",
    "$httpParamSerializer",
    "$httpParamSerializerJQLike",
    "$location",
    "$locationProvider",
    "$window",
    "$document",
    "$timeout",
    "$interval",
    "$q",
    "$log",
    "$logProvider",
    "$exceptionHandler",
    "$exceptionHandlerProvider",
    "$cacheFactory",
    "$templateCache",
    "$templateRequest",
    "$sce",
    "$sceProvider",
    "$sceDelegate",
    "$sceDelegateProvider",
    "$animate",
    "$animateProvider",
    "$animateCss",
    "$anchorScroll",
    "$anchorScrollProvider",
    "$xhrFactory",
    "$jsonpCallbacks",
    "$browser",
    // --- ngRoute ---
    "$route",
    "$routeProvider",
    "$routeParams",
    // --- ngResource ---
    "$resource",
    "$resourceProvider",
    // --- ngCookies ---
    "$cookies",
    "$cookieStore",
    "$cookiesProvider",
    // --- ngTouch / ngSanitize ---
    "$swipe",
    "$sanitize",
    "$sanitizeProvider",
    // --- ui-router (1.x) ---
    "$state",
    "$stateProvider",
    "$stateParams",
    "$urlRouter",
    "$urlRouterProvider",
    "$urlMatcherFactory",
    "$urlMatcherFactoryProvider",
    "$transitions",
    "$uiRouter",
    "$uiRouterProvider",
    "$uiViewScroll",
    "$uiViewScrollProvider",
];

/// `name` が AngularJS 組み込みサービス名と完全一致するかどうか
pub fn is_builtin_service(name: &str) -> bool {
    BUILTIN_SERVICES.contains(&name)
}

/// `name` に「もしかして」候補となる組み込みサービス名を返す。
/// Levenshtein 距離が `max_distance` 以下のもののみ対象。
///
/// 戻り値は最も距離が近いものを 1 件だけ返す。
pub fn suggest_builtin_service(name: &str, max_distance: usize) -> Option<&'static str> {
    let mut best: Option<(&'static str, usize)> = None;
    for candidate in BUILTIN_SERVICES {
        let d = levenshtein(name, candidate);
        if d == 0 {
            // 完全一致 (allowlist 内) は補正候補ではない
            return None;
        }
        if d <= max_distance && best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((*candidate, d));
        }
    }
    best.map(|(s, _)| s)
}

/// Levenshtein 距離を計算する (バイト単位 / ASCII 前提)。
///
/// AngularJS のサービス名は ASCII (英数 + `$`) の範囲なので、UTF-8 を意識する
/// 必要はないがセーフティとして文字単位 (`chars()`) で扱う。空文字も許容。
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let n = a_chars.len();
    let m = b_chars.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }

    // 1 行ぶんの DP テーブルを使い回す
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            // 削除 / 挿入 / 置換 のうち最小
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_same_string_is_zero() {
        assert_eq!(levenshtein("$timeout", "$timeout"), 0);
    }

    #[test]
    fn levenshtein_empty_left() {
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn levenshtein_empty_right() {
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn levenshtein_single_substitution() {
        // $timeout vs $timoout = 1 substitution
        assert_eq!(levenshtein("$timeout", "$timoout"), 1);
    }

    #[test]
    fn levenshtein_transposition_counts_as_two() {
        // 古典的な Levenshtein では transposition は 2 カウント
        assert_eq!(levenshtein("$tiemout", "$timeout"), 2);
    }

    #[test]
    fn levenshtein_insertion() {
        assert_eq!(levenshtein("$http", "$https"), 1);
    }

    #[test]
    fn levenshtein_deletion() {
        assert_eq!(levenshtein("$scoope", "$scope"), 1);
    }

    #[test]
    fn levenshtein_completely_different() {
        // 完全に異なる文字列でも長さ差より大きくはならない
        let d = levenshtein("$foo", "$timeout");
        assert!((5..=8).contains(&d), "got {}", d);
    }

    #[test]
    fn is_builtin_recognizes_core_services() {
        assert!(is_builtin_service("$scope"));
        assert!(is_builtin_service("$http"));
        assert!(is_builtin_service("$timeout"));
        assert!(is_builtin_service("$rootScope"));
    }

    #[test]
    fn is_builtin_recognizes_ui_router() {
        assert!(is_builtin_service("$state"));
        assert!(is_builtin_service("$stateProvider"));
        assert!(is_builtin_service("$stateParams"));
    }

    #[test]
    fn is_builtin_rejects_user_services() {
        assert!(!is_builtin_service("UserService"));
        assert!(!is_builtin_service("$tiemout"));
        assert!(!is_builtin_service(""));
    }

    #[test]
    fn suggest_returns_close_match() {
        // $tiemout (typo) → $timeout (距離2)
        assert_eq!(
            suggest_builtin_service("$tiemout", 2),
            Some("$timeout")
        );
    }

    #[test]
    fn suggest_returns_none_for_unknown_unrelated() {
        // 完全に別物には候補を出さない
        assert_eq!(
            suggest_builtin_service("MyCustomService", 2),
            None
        );
    }

    #[test]
    fn suggest_does_not_match_exact_name() {
        // 既に組み込みと一致しているなら「もしかして」は不要
        assert_eq!(suggest_builtin_service("$timeout", 2), None);
    }

    #[test]
    fn suggest_picks_closest_when_multiple_candidates() {
        // $sccope (距離1で $scope に一致) — $sce (距離3) より近い
        assert_eq!(suggest_builtin_service("$sccope", 2), Some("$scope"));
    }
}
