use std::fs;
use std::path::Path;

use serde::Deserialize;

/// ajsconfig.json の設定
#[derive(Debug, Clone, Deserialize)]
pub struct AjsConfig {
    #[serde(default)]
    pub interpolate: InterpolateConfig,
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
}
