use std::fs;
use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;

/// ajsconfig.json の設定
#[derive(Debug, Clone, Deserialize)]
pub struct AjsConfig {
    #[serde(default)]
    pub interpolate: InterpolateConfig,
    /// 解析対象のglobパターン（空の場合は全ファイル対象）
    #[serde(default)]
    pub include: Vec<String>,
    /// 除外対象のglobパターン
    #[serde(default = "default_exclude")]
    pub exclude: Vec<String>,
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

/// interpolate記号の設定
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterpolateConfig {
    #[serde(default = "default_start_symbol")]
    pub start_symbol: String,
    #[serde(default = "default_end_symbol")]
    pub end_symbol: String,
}

fn default_start_symbol() -> String {
    "{{".to_string()
}

fn default_end_symbol() -> String {
    "}}".to_string()
}

impl Default for InterpolateConfig {
    fn default() -> Self {
        Self {
            start_symbol: default_start_symbol(),
            end_symbol: default_end_symbol(),
        }
    }
}

impl Default for AjsConfig {
    fn default() -> Self {
        Self {
            interpolate: InterpolateConfig::default(),
            include: Vec::new(),
            exclude: default_exclude(),
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

/// パスマッチング用の構造体
#[derive(Debug, Clone)]
pub struct PathMatcher {
    include: Option<GlobSet>,
    exclude: GlobSet,
}

impl PathMatcher {
    /// include/excludeパターンからPathMatcherを作成
    pub fn new(include: &[String], exclude: &[String]) -> Result<Self, String> {
        let include_set = if include.is_empty() {
            None
        } else {
            let mut builder = GlobSetBuilder::new();
            for pattern in include {
                let glob = Glob::new(pattern)
                    .map_err(|e| format!("Invalid include pattern '{}': {}", pattern, e))?;
                builder.add(glob);
            }
            Some(
                builder
                    .build()
                    .map_err(|e| format!("Failed to build include set: {}", e))?,
            )
        };

        let mut exclude_builder = GlobSetBuilder::new();
        for pattern in exclude {
            let glob = Glob::new(pattern)
                .map_err(|e| format!("Invalid exclude pattern '{}': {}", pattern, e))?;
            exclude_builder.add(glob);
        }
        let exclude_set = exclude_builder
            .build()
            .map_err(|e| format!("Failed to build exclude set: {}", e))?;

        Ok(Self {
            include: include_set,
            exclude: exclude_set,
        })
    }

    /// ファイルが解析対象かどうかを判定
    pub fn should_include(&self, relative_path: &Path) -> bool {
        // excludeにマッチしたら除外
        if self.exclude.is_match(relative_path) {
            return false;
        }

        // includeが指定されていればマッチするかチェック
        match &self.include {
            Some(include_set) => include_set.is_match(relative_path),
            None => true,
        }
    }

    /// ディレクトリを走査すべきかどうかを判定（excludeのみチェック）
    pub fn should_traverse_dir(&self, relative_path: &Path) -> bool {
        !self.exclude.is_match(relative_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AjsConfig::default();
        assert_eq!(config.interpolate.start_symbol, "{{");
        assert_eq!(config.interpolate.end_symbol, "}}");
    }

    #[test]
    fn test_parse_config() {
        let json = r#"{
            "interpolate": {
                "startSymbol": "[[",
                "endSymbol": "]]"
            }
        }"#;

        let config: AjsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.interpolate.start_symbol, "[[");
        assert_eq!(config.interpolate.end_symbol, "]]");
    }

    #[test]
    fn test_partial_config() {
        let json = r#"{
            "interpolate": {
                "startSymbol": "[["
            }
        }"#;

        let config: AjsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.interpolate.start_symbol, "[[");
        assert_eq!(config.interpolate.end_symbol, "}}");
    }

    #[test]
    fn test_empty_config() {
        let json = r#"{}"#;

        let config: AjsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.interpolate.start_symbol, "{{");
        assert_eq!(config.interpolate.end_symbol, "}}");
    }

    #[test]
    fn test_include_exclude_config() {
        let json = r#"{
            "include": ["src/**/*.js", "app/**/*.js"],
            "exclude": ["**/test/**", "**/node_modules/**"]
        }"#;

        let config: AjsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.include.len(), 2);
        assert_eq!(config.include[0], "src/**/*.js");
        assert_eq!(config.exclude.len(), 2);
    }

    #[test]
    fn test_default_exclude() {
        let json = r#"{}"#;
        let config: AjsConfig = serde_json::from_str(json).unwrap();

        // デフォルトのexcludeパターンがあるはず
        assert!(!config.exclude.is_empty());
        assert!(config.exclude.iter().any(|p| p.contains("node_modules")));
    }

    #[test]
    fn test_empty_include_means_all() {
        let json = r#"{}"#;
        let config: AjsConfig = serde_json::from_str(json).unwrap();

        let matcher = config.create_path_matcher().unwrap();

        // includeが空なら、excludeにマッチしないパスは全て含まれる
        assert!(matcher.should_include(Path::new("src/app.js")));
        assert!(matcher.should_include(Path::new("lib/utils.js")));
    }

    #[test]
    fn test_path_matcher_include() {
        let json = r#"{
            "include": ["src/**/*.js"],
            "exclude": []
        }"#;
        let config: AjsConfig = serde_json::from_str(json).unwrap();
        let matcher = config.create_path_matcher().unwrap();

        assert!(matcher.should_include(Path::new("src/app.js")));
        assert!(matcher.should_include(Path::new("src/utils/helper.js")));
        assert!(!matcher.should_include(Path::new("lib/other.js")));
    }

    #[test]
    fn test_path_matcher_exclude() {
        let json = r#"{
            "include": [],
            "exclude": ["**/test/**", "**/spec/**"]
        }"#;
        let config: AjsConfig = serde_json::from_str(json).unwrap();
        let matcher = config.create_path_matcher().unwrap();

        assert!(matcher.should_include(Path::new("src/app.js")));
        assert!(!matcher.should_include(Path::new("src/test/app.spec.js")));
        assert!(!matcher.should_include(Path::new("spec/unit/test.js")));
    }

    #[test]
    fn test_invalid_pattern_error() {
        let matcher = PathMatcher::new(&["[invalid".to_string()], &[]);
        assert!(matcher.is_err());
    }

    #[test]
    fn test_include_exclude_interaction() {
        // includeにマッチ かつ excludeにマッチしない場合のみ含まれる
        let json = r#"{
            "include": ["src/**/*.js"],
            "exclude": ["**/test/**"]
        }"#;
        let config: AjsConfig = serde_json::from_str(json).unwrap();
        let matcher = config.create_path_matcher().unwrap();

        assert!(matcher.should_include(Path::new("src/app.js")));
        assert!(!matcher.should_include(Path::new("src/test/app.js"))); // excluded
        assert!(!matcher.should_include(Path::new("lib/app.js"))); // not in include
    }

    #[test]
    fn test_should_traverse_dir() {
        // ディレクトリ走査はexcludeのみチェック
        let json = r#"{
            "include": ["static/**/*.js"],
            "exclude": ["**/node_modules", "**/node_modules/**"]
        }"#;
        let config: AjsConfig = serde_json::from_str(json).unwrap();
        let matcher = config.create_path_matcher().unwrap();

        // ディレクトリはincludeパターンに関係なく走査可能
        assert!(matcher.should_traverse_dir(Path::new("static")));
        assert!(matcher.should_traverse_dir(Path::new("static/subdir")));
        assert!(matcher.should_traverse_dir(Path::new("other")));

        // excludeにマッチするディレクトリは走査しない
        assert!(!matcher.should_traverse_dir(Path::new("node_modules")));
        assert!(!matcher.should_traverse_dir(Path::new("static/node_modules")));

        // ファイルはincludeパターンでフィルタ
        assert!(matcher.should_include(Path::new("static/app.js")));
        assert!(matcher.should_include(Path::new("static/subdir/app.js")));
        assert!(!matcher.should_include(Path::new("other/app.js"))); // not in include
    }

    #[test]
    fn test_multiple_include_patterns() {
        // 複数のincludeパターンが正しく動作するかテスト
        let json = r#"{
            "include": [
                "static/**/*.js",
                "static/**/*.html",
                "templates/**/*.js",
                "templates/**/*.html"
            ],
            "exclude": ["**/node_modules", "**/node_modules/**"]
        }"#;
        let config: AjsConfig = serde_json::from_str(json).unwrap();
        let matcher = config.create_path_matcher().unwrap();

        // staticフォルダのファイル
        assert!(matcher.should_include(Path::new("static/app.js")));
        assert!(matcher.should_include(Path::new("static/views/index.html")));
        assert!(matcher.should_include(Path::new("static/subdir/deep/file.js")));

        // templatesフォルダのファイル
        assert!(matcher.should_include(Path::new("templates/form.html")));
        assert!(matcher.should_include(Path::new("templates/form.js")));
        assert!(matcher.should_include(Path::new("templates/subdir/form.html")));
        assert!(matcher.should_include(Path::new("templates/subdir/form.js")));

        // 他のフォルダはマッチしない
        assert!(!matcher.should_include(Path::new("other/file.js")));
        assert!(!matcher.should_include(Path::new("other/file.html")));
    }
}
