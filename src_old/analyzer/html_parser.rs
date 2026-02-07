use tree_sitter::{Parser, Tree};

pub struct HtmlParser {
    parser: Parser,
}

impl HtmlParser {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_html::LANGUAGE.into())
            .expect("Failed to load HTML grammar");

        Self { parser }
    }

    pub fn parse(&mut self, source: &str) -> Option<Tree> {
        self.parser.parse(source, None)
    }
}

impl Default for HtmlParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_django_template() {
        let mut parser = HtmlParser::new();

        let source = r#"<!DOCTYPE html>
{% load static %}
{% load custom_tag %}
<html lang="{{ LANGUAGE_CODE|default:'ja' }}" ng-app="WfApp">
<head>
  <title ng-bind="tab_title"></title>
</head>
<body ng-controller="MainCtrl">
  {{ message }}
</body>
</html>"#;

        let result = parser.parse(source);
        assert!(result.is_some(), "Parser should return Some even with Django templates");

        if let Some(tree) = result {
            println!("Root node kind: {}", tree.root_node().kind());
            println!("Has error: {}", tree.root_node().has_error());
        }
    }

    #[test]
    fn test_parse_django_extends_template() {
        let mut parser = HtmlParser::new();

        // ユーザーの実際のファイル
        let source = r#"{% extends "wf/base.html" %}

{% block extrahead %}
    {% include "wf/analytics_tags/thx_partial.html" %}
{% endblock %}

{% block content %}
    <div ng-controller="ThxController" translate>
        自動で画面が切り替わらない場合は<a href="/">ここ</a>をクリック。
    </div>
{% endblock %}"#;

        let result = parser.parse(source);
        println!("Parse result: {:?}", result.is_some());

        if let Some(tree) = result {
            println!("Root node kind: {}", tree.root_node().kind());
            println!("Has error: {}", tree.root_node().has_error());

            // ツリー構造を出力
            fn print_tree(node: tree_sitter::Node, source: &str, indent: usize) {
                let indent_str = "  ".repeat(indent);
                let text = if node.child_count() == 0 {
                    format!(" = {:?}", &source[node.byte_range()])
                } else {
                    String::new()
                };
                println!("{}{} [{}-{}] (error: {}){}",
                    indent_str,
                    node.kind(),
                    node.start_position().row,
                    node.end_position().row,
                    node.has_error(),
                    text
                );
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    print_tree(child, source, indent + 1);
                }
            }
            print_tree(tree.root_node(), source, 0);
        }
    }

    #[test]
    fn test_parse_simple_footer() {
        let mut parser = HtmlParser::new();

        // 解析されるファイル
        let source = r#"<footer>&copy; 2016 DONUTS Co. Ltd.</footer>"#;

        let result = parser.parse(source);
        println!("Parse result: {:?}", result.is_some());

        if let Some(tree) = result {
            println!("Root node kind: {}", tree.root_node().kind());
            println!("Has error: {}", tree.root_node().has_error());
        }
    }
}
