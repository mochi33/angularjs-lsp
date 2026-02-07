use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};

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
        if self.exclude.is_match(relative_path) {
            return false;
        }
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
    fn test_empty_include_means_all() {
        let matcher = PathMatcher::new(&[], &[]).unwrap();
        assert!(matcher.should_include(Path::new("src/app.js")));
        assert!(matcher.should_include(Path::new("lib/utils.js")));
    }

    #[test]
    fn test_include_filter() {
        let matcher = PathMatcher::new(
            &["src/**/*.js".to_string()],
            &[],
        )
        .unwrap();
        assert!(matcher.should_include(Path::new("src/app.js")));
        assert!(!matcher.should_include(Path::new("lib/other.js")));
    }

    #[test]
    fn test_exclude_filter() {
        let matcher = PathMatcher::new(
            &[],
            &["**/test/**".to_string()],
        )
        .unwrap();
        assert!(matcher.should_include(Path::new("src/app.js")));
        assert!(!matcher.should_include(Path::new("src/test/app.spec.js")));
    }

    #[test]
    fn test_should_traverse_dir() {
        let matcher = PathMatcher::new(
            &[],
            &["**/node_modules".to_string(), "**/node_modules/**".to_string()],
        )
        .unwrap();
        assert!(matcher.should_traverse_dir(Path::new("src")));
        assert!(!matcher.should_traverse_dir(Path::new("node_modules")));
    }
}
