use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use tower_lsp::lsp_types::Url;

use crate::cache::FileMetadata;
use crate::config::PathMatcher;

/// Collect files with given extensions from workspace directory
pub fn collect_files(
    dir: &Path,
    root: &Path,
    path_matcher: Option<&PathMatcher>,
    extensions: &[&str],
    files: &mut Vec<(Url, String)>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let relative_path = path.strip_prefix(root).unwrap_or(&path);

            if path.is_dir() {
                if let Some(matcher) = path_matcher {
                    if !matcher.should_traverse_dir(relative_path) {
                        continue;
                    }
                } else {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with('.')
                            || name == "node_modules"
                            || name == "dist"
                            || name == "build"
                        {
                            continue;
                        }
                    }
                }
                collect_files(&path, root, path_matcher, extensions, files);
            } else {
                let ext_match = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| extensions.contains(&e))
                    .unwrap_or(false);

                if ext_match {
                    if let Some(matcher) = path_matcher {
                        if !matcher.should_include(relative_path) {
                            continue;
                        }
                    }
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(uri) = Url::from_file_path(&path) {
                            files.push((uri, content));
                        }
                    }
                }
            }
        }
    }
}

/// Collect file metadata for caching
pub fn collect_file_metadata(
    dir: &Path,
    root: &Path,
    path_matcher: Option<&PathMatcher>,
    metadata: &mut HashMap<PathBuf, FileMetadata>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let relative_path = path.strip_prefix(root).unwrap_or(&path);

            if path.is_dir() {
                if let Some(matcher) = path_matcher {
                    if !matcher.should_traverse_dir(relative_path) {
                        continue;
                    }
                } else {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with('.')
                            || name == "node_modules"
                            || name == "dist"
                            || name == "build"
                        {
                            continue;
                        }
                    }
                }
                collect_file_metadata(&path, root, path_matcher, metadata);
            } else {
                let ext = path.extension().and_then(|e| e.to_str());
                if ext == Some("js") || ext == Some("html") || ext == Some("htm") {
                    if let Some(matcher) = path_matcher {
                        if !matcher.should_include(relative_path) {
                            continue;
                        }
                    }

                    if let Ok(meta) = fs::metadata(&path) {
                        let mtime = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        metadata.insert(
                            path,
                            FileMetadata {
                                mtime,
                                size: meta.len(),
                            },
                        );
                    }
                }
            }
        }
    }
}

/// Find tsconfig.json in workspace
pub fn find_tsconfig_root(root_uri: &Option<Url>) -> Option<Url> {
    let root_uri = root_uri.as_ref()?;
    let root_path = root_uri.to_file_path().ok()?;
    find_tsconfig_in_dir(&root_path)
}

fn find_tsconfig_in_dir(dir: &Path) -> Option<Url> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_file() && path.file_name().map_or(false, |n| n == "tsconfig.json") {
                return Url::from_file_path(dir).ok();
            }

            if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.')
                        || name == "node_modules"
                        || name == "dist"
                        || name == "build"
                    {
                        continue;
                    }
                }
                if let Some(found) = find_tsconfig_in_dir(&path) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Extract service prefix at cursor position ("ServiceName." pattern)
pub fn get_service_prefix_at_cursor(text: &str, line: u32, col: u32) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    if line as usize >= lines.len() {
        return None;
    }

    let line_text = lines[line as usize];
    let col = col as usize;
    if col > line_text.len() {
        return None;
    }

    let before_cursor = &line_text[..col];

    if before_cursor.ends_with('.') {
        let without_dot = &before_cursor[..before_cursor.len() - 1];
        let service_name: String = without_dot
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect::<String>()
            .chars()
            .rev()
            .collect();

        if !service_name.is_empty() {
            return Some(service_name);
        }
    }

    None
}
