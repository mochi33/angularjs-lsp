use tower_lsp::lsp_types::Url;

/// ファイルがHTMLかどうか判定
pub fn is_html_file(uri: &Url) -> bool {
    let path = uri.path().to_lowercase();
    path.ends_with(".html") || path.ends_with(".htm")
}

/// ファイルがJSかどうか判定
pub fn is_js_file(uri: &Url) -> bool {
    uri.path().ends_with(".js")
}

/// camelCaseをkebab-caseに変換
/// 例: "myDirective" -> "my-directive"
pub fn camel_to_kebab(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('-');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else {
            result.push(c);
        }
    }
    result
}

/// kebab-caseをcamelCaseに変換
/// 例: "my-directive" -> "myDirective"
pub fn kebab_to_camel(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = false;
    for c in s.chars() {
        if c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_uppercase().next().unwrap());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// テンプレートパスを正規化（クエリパラメータを除去、`../`を除去）
pub fn normalize_template_path(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    let mut normalized = path;
    while normalized.starts_with("../") {
        normalized = &normalized[3..];
    }
    while normalized.starts_with("./") {
        normalized = &normalized[2..];
    }
    if normalized.starts_with('/') {
        normalized = &normalized[1..];
    }
    normalized.to_string()
}

/// 親URIを起点として相対パスを解決し、ファイル名を取得
pub fn resolve_relative_path(parent_uri: &Url, template_path: &str) -> String {
    let template_path = template_path.split('?').next().unwrap_or(template_path);
    let parent_path = parent_uri.path();
    let parent_dir = if let Some(last_slash) = parent_path.rfind('/') {
        &parent_path[..last_slash]
    } else {
        ""
    };

    let resolved = if template_path.starts_with('/') {
        template_path.to_string()
    } else {
        let mut parts: Vec<&str> = parent_dir.split('/').filter(|s| !s.is_empty()).collect();
        for segment in template_path.split('/') {
            match segment {
                ".." => {
                    parts.pop();
                }
                "." | "" => {}
                _ => parts.push(segment),
            }
        }
        format!("/{}", parts.join("/"))
    };

    resolved
        .rsplit('/')
        .next()
        .unwrap_or(&resolved)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camel_to_kebab() {
        assert_eq!(camel_to_kebab("myDirective"), "my-directive");
        assert_eq!(camel_to_kebab("ngRepeat"), "ng-repeat");
        assert_eq!(camel_to_kebab("simple"), "simple");
        assert_eq!(camel_to_kebab("ABC"), "a-b-c");
    }

    #[test]
    fn test_kebab_to_camel() {
        assert_eq!(kebab_to_camel("my-directive"), "myDirective");
        assert_eq!(kebab_to_camel("ng-repeat"), "ngRepeat");
        assert_eq!(kebab_to_camel("simple"), "simple");
    }

    #[test]
    fn test_normalize_template_path() {
        assert_eq!(
            normalize_template_path("../foo/bar/baz.html"),
            "foo/bar/baz.html"
        );
        assert_eq!(
            normalize_template_path("/foo/bar/baz.html"),
            "foo/bar/baz.html"
        );
        assert_eq!(
            normalize_template_path("./foo/bar.html"),
            "foo/bar.html"
        );
        assert_eq!(
            normalize_template_path("foo/bar.html?v=1"),
            "foo/bar.html"
        );
    }
}
