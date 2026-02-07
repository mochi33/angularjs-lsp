use tower_lsp::lsp_types::Url;
use tree_sitter::Node;

use super::context::AnalyzerContext;
use super::AngularJsAnalyzer;
use crate::model::{Span, SymbolReference};

/// Utility: check if name is a common JavaScript keyword
pub(super) fn is_common_keyword(name: &str) -> bool {
    matches!(
        name,
        "function"
            | "var"
            | "let"
            | "const"
            | "if"
            | "else"
            | "for"
            | "while"
            | "return"
            | "true"
            | "false"
            | "null"
            | "undefined"
            | "this"
            | "new"
            | "typeof"
            | "instanceof"
            | "in"
            | "of"
    )
}

impl AngularJsAnalyzer {
    /// Analyze method calls and register as references
    ///
    /// Pattern: UserService.getAll(), AuthService.login(credentials)
    pub(super) fn analyze_method_call(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        ctx: &AnalyzerContext,
    ) {
        if let Some(callee) = node.child_by_field_name("function") {
            if callee.kind() == "member_expression" {
                if let Some(object) = callee.child_by_field_name("object") {
                    if let Some(property) = callee.child_by_field_name("property") {
                        let obj_name = self.node_text(object, source);
                        let method_name = self.node_text(property, source);

                        if obj_name.starts_with('$')
                            || obj_name == "this"
                            || obj_name == "console"
                        {
                            return;
                        }

                        let current_line = node.start_position().row as u32;
                        if !ctx.is_injected_at(&obj_name, current_line) {
                            return;
                        }

                        let full_name = format!("{}.{}", obj_name, method_name);

                        if self.index.definitions.has_definition(&full_name) {
                            let start = property.start_position();
                            let end = property.end_position();

                            let reference = SymbolReference {
                                name: full_name,
                                uri: uri.clone(),
                                span: Span::new(
                                    self.offset_line(start.row as u32),
                                    start.column as u32,
                                    self.offset_line(end.row as u32),
                                    end.column as u32,
                                ),
                            };

                            self.index.definitions.add_reference(reference);
                        }
                    }
                }
            }
        }
    }

    /// Analyze member access (non-call) and register as references
    ///
    /// Pattern: var fn = UserService.getAll; callback(AuthService.login);
    pub(super) fn analyze_member_access(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        ctx: &AnalyzerContext,
    ) {
        if let Some(object) = node.child_by_field_name("object") {
            if let Some(property) = node.child_by_field_name("property") {
                let obj_name = self.node_text(object, source);
                let prop_name = self.node_text(property, source);

                if obj_name.starts_with('$') || obj_name == "this" || obj_name == "console" {
                    return;
                }

                let current_line = node.start_position().row as u32;
                if !ctx.is_injected_at(&obj_name, current_line) {
                    return;
                }

                let full_name = format!("{}.{}", obj_name, prop_name);

                if self.index.definitions.has_definition(&full_name) {
                    let start = property.start_position();
                    let end = property.end_position();

                    let reference = SymbolReference {
                        name: full_name,
                        uri: uri.clone(),
                        span: Span::new(
                            self.offset_line(start.row as u32),
                            start.column as u32,
                            self.offset_line(end.row as u32),
                            end.column as u32,
                        ),
                    };

                    self.index.definitions.add_reference(reference);
                }
            }
        }
    }

    /// Analyze identifiers and register as references to known definitions
    pub(super) fn analyze_identifier(
        &self,
        node: Node,
        source: &str,
        uri: &Url,
        ctx: &AnalyzerContext,
    ) {
        let name = self.node_text(node, source);

        if name.len() < 2 || is_common_keyword(&name) {
            return;
        }

        if self.index.definitions.has_definition(&name) {
            let current_line = node.start_position().row as u32;
            if !ctx.is_injected_at(&name, current_line) {
                return;
            }

            let start = node.start_position();
            let end = node.end_position();

            let reference = SymbolReference {
                name,
                uri: uri.clone(),
                span: Span::new(
                    self.offset_line(start.row as u32),
                    start.column as u32,
                    self.offset_line(end.row as u32),
                    end.column as u32,
                ),
            };

            self.index.definitions.add_reference(reference);
        }
    }
}
