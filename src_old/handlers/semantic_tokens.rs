use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::{HtmlScopeReference, SymbolIndex, SymbolKind};

/// Token types (index in legend)
const TOKEN_TYPE_PROPERTY: u32 = 0;
const TOKEN_TYPE_METHOD: u32 = 1;
const TOKEN_TYPE_VARIABLE: u32 = 2;
const TOKEN_TYPE_MACRO: u32 = 3; // directive

/// Token modifiers (bit flags)
const TOKEN_MOD_READONLY: u32 = 1 << 0;
const TOKEN_MOD_STATIC: u32 = 1 << 1;
const TOKEN_MOD_DECLARATION: u32 = 1 << 2;

/// Raw token with absolute positions (before encoding)
struct RawSemanticToken {
    line: u32,
    start_col: u32,
    length: u32,
    token_type: u32,
    token_modifiers: u32,
}

pub struct SemanticTokensHandler {
    index: Arc<SymbolIndex>,
}

impl SemanticTokensHandler {
    pub fn new(index: Arc<SymbolIndex>) -> Self {
        Self { index }
    }

    /// Build the semantic tokens legend (token types and modifiers)
    pub fn legend() -> SemanticTokensLegend {
        SemanticTokensLegend {
            token_types: vec![
                SemanticTokenType::PROPERTY, // 0: scope property
                SemanticTokenType::METHOD,   // 1: scope method
                SemanticTokenType::VARIABLE, // 2: local variable, form binding
                SemanticTokenType::MACRO,    // 3: directive
            ],
            token_modifiers: vec![
                SemanticTokenModifier::READONLY,    // 0: for form bindings
                SemanticTokenModifier::STATIC,      // 1: for $rootScope
                SemanticTokenModifier::DECLARATION, // 2: for definitions
            ],
        }
    }

    /// Check if file is HTML
    fn is_html_file(uri: &Url) -> bool {
        let path = uri.path().to_lowercase();
        path.ends_with(".html") || path.ends_with(".htm")
    }

    /// Compute semantic tokens for an HTML file
    pub fn semantic_tokens_full(&self, uri: &Url) -> Option<SemanticTokens> {
        // Only process HTML files
        if !Self::is_html_file(uri) {
            return None;
        }

        let raw_tokens = self.collect_html_tokens(uri);
        let encoded = Self::encode_tokens(raw_tokens);

        Some(SemanticTokens {
            result_id: None,
            data: encoded,
        })
    }

    /// Collect all semantic tokens for HTML document
    fn collect_html_tokens(&self, uri: &Url) -> Vec<RawSemanticToken> {
        let mut raw_tokens = Vec::new();

        // 1. Scope references
        self.collect_scope_reference_tokens(uri, &mut raw_tokens);

        // 2. Local variable definitions (ng-repeat, ng-init)
        self.collect_local_variable_definition_tokens(uri, &mut raw_tokens);

        // 3. Local variable references
        self.collect_local_variable_reference_tokens(uri, &mut raw_tokens);

        // 4. Form binding definitions
        self.collect_form_binding_tokens(uri, &mut raw_tokens);

        // 5. Directive references
        self.collect_directive_reference_tokens(uri, &mut raw_tokens);

        raw_tokens
    }

    /// Collect tokens from scope references
    fn collect_scope_reference_tokens(&self, uri: &Url, tokens: &mut Vec<RawSemanticToken>) {
        let refs = self.index.get_html_scope_references(uri);

        for scope_ref in refs {
            let (token_type, token_modifiers) =
                self.determine_scope_token_type(uri, &scope_ref);

            tokens.push(RawSemanticToken {
                line: scope_ref.start_line,
                start_col: scope_ref.start_col,
                length: (scope_ref.end_col - scope_ref.start_col),
                token_type,
                token_modifiers,
            });
        }
    }

    /// Determine token type for a scope reference by resolving its definition
    fn determine_scope_token_type(
        &self,
        uri: &Url,
        scope_ref: &HtmlScopeReference,
    ) -> (u32, u32) {
        // Check if this is an alias.property pattern
        let (resolved_controller, property_path) = if scope_ref.property_path.contains('.') {
            let parts: Vec<&str> = scope_ref.property_path.splitn(2, '.').collect();
            if parts.len() == 2 {
                let alias = parts[0];
                let prop = parts[1];
                if let Some(controller) =
                    self.index.resolve_controller_by_alias(uri, scope_ref.start_line, alias)
                {
                    (Some(controller), prop.to_string())
                } else {
                    (None, scope_ref.property_path.clone())
                }
            } else {
                (None, scope_ref.property_path.clone())
            }
        } else {
            (None, scope_ref.property_path.clone())
        };

        // Resolve controllers
        let controllers = if let Some(ref controller) = resolved_controller {
            vec![controller.clone()]
        } else {
            self.index.resolve_controllers_for_html(uri, scope_ref.start_line)
        };

        // Try to find definition
        for controller_name in &controllers {
            let symbol_name = format!("{}.$scope.{}", controller_name, property_path);
            if let Some(def) = self.index.get_definitions(&symbol_name).first() {
                return match def.kind {
                    SymbolKind::ScopeMethod => (TOKEN_TYPE_METHOD, 0),
                    SymbolKind::ScopeProperty => (TOKEN_TYPE_PROPERTY, 0),
                    SymbolKind::RootScopeMethod => (TOKEN_TYPE_METHOD, TOKEN_MOD_STATIC),
                    SymbolKind::RootScopeProperty => (TOKEN_TYPE_PROPERTY, TOKEN_MOD_STATIC),
                    _ => (TOKEN_TYPE_PROPERTY, 0),
                };
            }
        }

        // Try controller as syntax (this.method pattern)
        if resolved_controller.is_some() {
            for controller_name in &controllers {
                let symbol_name = format!("{}.{}", controller_name, property_path);
                if let Some(def) = self.index.get_definitions(&symbol_name).first() {
                    return match def.kind {
                        SymbolKind::ScopeMethod | SymbolKind::Method => (TOKEN_TYPE_METHOD, 0),
                        SymbolKind::ScopeProperty => (TOKEN_TYPE_PROPERTY, 0),
                        _ => (TOKEN_TYPE_PROPERTY, 0),
                    };
                }
            }
        }

        // Default to property
        (TOKEN_TYPE_PROPERTY, 0)
    }

    /// Collect tokens from local variable definitions
    fn collect_local_variable_definition_tokens(
        &self,
        uri: &Url,
        tokens: &mut Vec<RawSemanticToken>,
    ) {
        let vars = self.index.get_all_local_variables(uri);

        for var in vars {
            tokens.push(RawSemanticToken {
                line: var.name_start_line,
                start_col: var.name_start_col,
                length: (var.name_end_col - var.name_start_col),
                token_type: TOKEN_TYPE_VARIABLE,
                token_modifiers: TOKEN_MOD_DECLARATION,
            });
        }
    }

    /// Collect tokens from local variable references
    fn collect_local_variable_reference_tokens(
        &self,
        uri: &Url,
        tokens: &mut Vec<RawSemanticToken>,
    ) {
        let refs = self.index.get_all_local_variable_references_for_uri(uri);

        for var_ref in refs {
            tokens.push(RawSemanticToken {
                line: var_ref.start_line,
                start_col: var_ref.start_col,
                length: (var_ref.end_col - var_ref.start_col),
                token_type: TOKEN_TYPE_VARIABLE,
                token_modifiers: 0,
            });
        }
    }

    /// Collect tokens from form bindings
    fn collect_form_binding_tokens(&self, uri: &Url, tokens: &mut Vec<RawSemanticToken>) {
        let bindings = self.index.get_all_form_bindings(uri);

        for binding in bindings {
            tokens.push(RawSemanticToken {
                line: binding.name_start_line,
                start_col: binding.name_start_col,
                length: (binding.name_end_col - binding.name_start_col),
                token_type: TOKEN_TYPE_VARIABLE,
                token_modifiers: TOKEN_MOD_READONLY | TOKEN_MOD_DECLARATION,
            });
        }
    }

    /// Collect tokens from directive references
    fn collect_directive_reference_tokens(&self, uri: &Url, tokens: &mut Vec<RawSemanticToken>) {
        let refs = self.index.get_all_directive_references_for_uri(uri);

        for directive_ref in refs {
            tokens.push(RawSemanticToken {
                line: directive_ref.start_line,
                start_col: directive_ref.start_col,
                length: (directive_ref.end_col - directive_ref.start_col),
                token_type: TOKEN_TYPE_MACRO,
                token_modifiers: 0,
            });
        }
    }

    /// Encode raw tokens as delta-encoded SemanticTokens
    fn encode_tokens(mut raw_tokens: Vec<RawSemanticToken>) -> Vec<SemanticToken> {
        // Sort by line, then by column
        raw_tokens.sort_by(|a, b| a.line.cmp(&b.line).then(a.start_col.cmp(&b.start_col)));

        let mut encoded = Vec::new();
        let mut prev_line = 0u32;
        let mut prev_col = 0u32;

        for token in raw_tokens {
            let delta_line = token.line - prev_line;
            let delta_start = if delta_line == 0 {
                token.start_col - prev_col
            } else {
                token.start_col
            };

            encoded.push(SemanticToken {
                delta_line,
                delta_start,
                length: token.length,
                token_type: token.token_type,
                token_modifiers_bitset: token.token_modifiers,
            });

            prev_line = token.line;
            prev_col = token.start_col;
        }

        encoded
    }
}
