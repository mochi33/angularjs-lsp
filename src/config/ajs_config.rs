use std::fs;
use std::path::Path;

use serde::Deserialize;

use super::path_matcher::PathMatcher;

/// ajsconfig.json の設定
///
/// 注意: 旧来は `interpolate.startSymbol` / `interpolate.endSymbol` を持って
/// いたが、現在は AngularJS ソースの `$interpolateProvider.startSymbol(...)` /
/// `.endSymbol(...)` から動的に解決するため当該フィールドは廃止した。
/// 古い `ajsconfig.json` に `interpolate` フィールドが残っていても、`serde` の
/// 標準動作で未知フィールドとして黙って無視される。
#[derive(Debug, Clone, Deserialize)]
pub struct AjsConfig {
    /// 解析対象のglobパターン（空の場合は全ファイル対象）
    #[serde(default)]
    pub include: Vec<String>,
    /// 除外対象のglobパターン
    #[serde(default = "default_exclude")]
    pub exclude: Vec<String>,
    /// キャッシュ機能を有効にする（デフォルト: false）
    #[serde(default)]
    pub cache: bool,
    /// 診断（警告表示）設定
    #[serde(default)]
    pub diagnostics: DiagnosticsConfig,
}

/// 診断（警告表示）設定
#[derive(Debug, Clone, Deserialize)]
pub struct DiagnosticsConfig {
    /// 診断機能を有効にする（デフォルト: true）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 診断の重要度: "error", "warning", "hint", "information"（デフォルト: "warning"）
    #[serde(default = "default_severity")]
    pub severity: String,
    /// 未使用スコープ変数の警告を有効にする（デフォルト: true）
    #[serde(default = "default_true")]
    pub unused_scope_variables: bool,
}

fn default_true() -> bool {
    true
}

fn default_severity() -> String {
    "warning".to_string()
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            severity: default_severity(),
            unused_scope_variables: default_true(),
        }
    }
}

fn default_exclude() -> Vec<String> {
    vec![
        "**/node_modules".to_string(),
        "**/node_modules/**".to_string(),
        "**/dist".to_string(),
        "**/dist/**".to_string(),
        "**/build".to_string(),
        "**/build/**".to_string(),
        "**/.*".to_string(),
        "**/.*/**".to_string(),
    ]
}

impl Default for AjsConfig {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: default_exclude(),
            cache: false,
            diagnostics: DiagnosticsConfig::default(),
        }
    }
}

impl AjsConfig {
    /// 指定ディレクトリからajsconfig.jsonを読み込む
    pub fn load_from_dir(dir: &Path) -> Self {
        let config_path = dir.join("ajsconfig.json");
        Self::load_from_path(&config_path)
    }

    /// 指定パスからajsconfig.jsonを読み込む
    pub fn load_from_path(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }

        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(config) => config,
                Err(e) => {
                    tracing::warn!("Failed to parse ajsconfig.json: {}", e);
                    Self::default()
                }
            },
            Err(e) => {
                tracing::warn!("Failed to read ajsconfig.json: {}", e);
                Self::default()
            }
        }
    }

    /// PathMatcherを作成
    pub fn create_path_matcher(&self) -> Result<PathMatcher, String> {
        PathMatcher::new(&self.include, &self.exclude)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AjsConfig::default();
        assert!(config.include.is_empty());
        assert!(!config.cache);
    }

    #[test]
    fn test_legacy_interpolate_field_is_ignored() {
        // 旧フォーマットの ajsconfig.json (interpolate フィールドあり) を読み込んでも
        // serde は未知フィールドを無視するのでパース成功するべき。
        // interpolate 解決は AngularJS ソース由来に一本化されているのでこの値は使われない。
        let json = r#"{
            "interpolate": {
                "startSymbol": "[[",
                "endSymbol": "]]"
            },
            "cache": true
        }"#;
        let config: AjsConfig = serde_json::from_str(json).unwrap();
        assert!(config.cache, "interpolate フィールドがあっても他フィールドは正しく読み込まれる");
    }

    #[test]
    fn test_empty_config() {
        let json = r#"{}"#;
        let config: AjsConfig = serde_json::from_str(json).unwrap();
        assert!(config.include.is_empty());
    }

    #[test]
    fn test_diagnostics_default() {
        let config = DiagnosticsConfig::default();
        assert!(config.enabled);
        assert_eq!(config.severity, "warning");
        assert!(config.unused_scope_variables);
    }
}
