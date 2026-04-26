use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::Index;
use crate::model::{HtmlScopeReference, SymbolKind};
use crate::util::is_html_file;

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
    index: Arc<Index>,
}

impl SemanticTokensHandler {
    pub fn new(index: Arc<Index>) -> Self {
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

    /// Compute semantic tokens for an HTML file
    pub fn semantic_tokens_full(&self, uri: &Url) -> Option<SemanticTokens> {
        // Only process HTML files
        if !is_html_file(uri) {
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
        let refs = self.index.html.get_html_scope_references(uri);

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
                    self.index
                        .resolve_controller_by_alias(uri, scope_ref.start_line, alias)
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
            self.index
                .resolve_controllers_for_html(uri, scope_ref.start_line)
        };

        // Try to find definition
        for controller_name in &controllers {
            let symbol_name = format!("{}.$scope.{}", controller_name, property_path);
            if let Some(def) = self
                .index
                .definitions
                .get_definitions(&symbol_name)
                .first()
            {
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
                if let Some(def) = self
                    .index
                    .definitions
                    .get_definitions(&symbol_name)
                    .first()
                {
                    return match def.kind {
                        SymbolKind::ScopeMethod | SymbolKind::Method => {
                            (TOKEN_TYPE_METHOD, 0)
                        }
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
        let vars = self.index.html.get_all_local_variables(uri);

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
        let refs = self
            .index
            .html
            .get_all_local_variable_references_for_uri(uri);

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
        let bindings = self.index.html.get_all_form_bindings(uri);

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
    fn collect_directive_reference_tokens(
        &self,
        uri: &Url,
        tokens: &mut Vec<RawSemanticToken>,
    ) {
        let refs = self.index.html.get_all_directive_references_for_uri(uri);

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
    ///
    /// LSP semantic tokens spec の制約:
    /// - tokens は (line, start_col) 昇順でソート済みであること
    /// - tokens は重複/オーバーラップしてはならない
    /// - length > 0 でなければならない
    ///
    /// この LSP は複数のソース (scope refs / local vars / form bindings /
    /// directive refs) からトークンを集めるため、同一スパンの重複や
    /// 隣接トークンのオーバーラップが発生し得る。クライアント (VS Code 等) は
    /// オーバーラップ等の不正データを検出すると **ファイル全体の semantic
    /// tokens を破棄して何も表示しない** ため、ここで防御的に弾く。
    fn encode_tokens(mut raw_tokens: Vec<RawSemanticToken>) -> Vec<SemanticToken> {
        // 1. length=0 の不正トークンを除外
        raw_tokens.retain(|t| t.length > 0);

        // 2. (line, start_col, -length) でソート
        //    同一 (line, col) の場合は length が大きいものを先に置く
        //    (後段の overlap dedup で長い方を残せるように)
        raw_tokens.sort_by(|a, b| {
            a.line
                .cmp(&b.line)
                .then(a.start_col.cmp(&b.start_col))
                .then(b.length.cmp(&a.length))
        });

        let mut encoded = Vec::new();
        let mut prev_line = 0u32;
        let mut prev_col = 0u32;
        let mut prev_end_col = 0u32; // 直前トークンの右端 (overlap 検出用)
        let mut first = true;

        for token in raw_tokens {
            // 3. 直前トークンと完全に同じスパンならスキップ (重複)
            if !first
                && token.line == prev_line
                && token.start_col == prev_col
                && token.start_col + token.length == prev_end_col
            {
                continue;
            }

            // 4. 直前トークンと overlap (同一行で前トークンの右端より前で開始)
            //    する場合はスキップ。これがあると VS Code が全 tokens を
            //    破棄してハイライトが消える。
            if !first && token.line == prev_line && token.start_col < prev_end_col {
                continue;
            }

            let delta_line = token.line.saturating_sub(prev_line);
            let delta_start = if delta_line == 0 {
                token.start_col.saturating_sub(prev_col)
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
            prev_end_col = token.start_col + token.length;
            first = false;
        }

        encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(line: u32, start_col: u32, length: u32, token_type: u32) -> RawSemanticToken {
        RawSemanticToken {
            line,
            start_col,
            length,
            token_type,
            token_modifiers: 0,
        }
    }

    #[test]
    fn encode_tokens_skips_zero_length() {
        let tokens = vec![raw(0, 0, 0, 0), raw(0, 5, 3, 0)];
        let encoded = SemanticTokensHandler::encode_tokens(tokens);
        assert_eq!(encoded.len(), 1, "length=0 のトークンは除外されるべき");
        assert_eq!(encoded[0].delta_start, 5);
        assert_eq!(encoded[0].length, 3);
    }

    #[test]
    fn encode_tokens_skips_exact_duplicate_span() {
        // 同一スパンが2回現れた場合、片方だけが残る
        let tokens = vec![raw(2, 5, 4, 0), raw(2, 5, 4, 1)];
        let encoded = SemanticTokensHandler::encode_tokens(tokens);
        assert_eq!(encoded.len(), 1, "完全に同じスパンの重複は1つに集約されるべき");
    }

    #[test]
    fn encode_tokens_skips_overlapping_tokens() {
        // [5,10) と [7,12) が overlap → 後者をスキップ
        // (overlap があるとクライアントが全トークン破棄するので必ず除外)
        let tokens = vec![raw(2, 5, 5, 0), raw(2, 7, 5, 1)];
        let encoded = SemanticTokensHandler::encode_tokens(tokens);
        assert_eq!(
            encoded.len(),
            1,
            "オーバーラップトークンの後者はスキップされるべき"
        );
        assert_eq!(encoded[0].delta_start, 5);
        assert_eq!(encoded[0].length, 5);
    }

    #[test]
    fn encode_tokens_keeps_adjacent_non_overlapping() {
        // [5,8) と [8,12) は隣接だが overlap しない → 両方残す
        let tokens = vec![raw(2, 5, 3, 0), raw(2, 8, 4, 1)];
        let encoded = SemanticTokensHandler::encode_tokens(tokens);
        assert_eq!(encoded.len(), 2, "隣接トークンは両方残るべき");
        assert_eq!(encoded[0].delta_start, 5);
        assert_eq!(encoded[1].delta_line, 0);
        assert_eq!(encoded[1].delta_start, 3);
    }

    #[test]
    fn encode_tokens_correct_delta_encoding() {
        // 複数行にまたがる正常ケース
        let tokens = vec![raw(1, 2, 3, 0), raw(1, 10, 4, 1), raw(3, 5, 2, 2)];
        let encoded = SemanticTokensHandler::encode_tokens(tokens);
        assert_eq!(encoded.len(), 3);
        // 1行目, col=2, len=3
        assert_eq!(encoded[0].delta_line, 1);
        assert_eq!(encoded[0].delta_start, 2);
        assert_eq!(encoded[0].length, 3);
        // 同じ行 (delta_line=0), col=10 なので delta_start=8
        assert_eq!(encoded[1].delta_line, 0);
        assert_eq!(encoded[1].delta_start, 8);
        // 行が変わったので delta_start は絶対値
        assert_eq!(encoded[2].delta_line, 2);
        assert_eq!(encoded[2].delta_start, 5);
    }

    #[test]
    fn encode_tokens_unsorted_input_is_normalized() {
        // 入力が順不同でも正しくソート→エンコードされる
        let tokens = vec![raw(3, 5, 2, 0), raw(1, 10, 4, 1), raw(1, 2, 3, 2)];
        let encoded = SemanticTokensHandler::encode_tokens(tokens);
        assert_eq!(encoded.len(), 3);
        // 最初が (1,2,3) であること
        assert_eq!(encoded[0].delta_line, 1);
        assert_eq!(encoded[0].delta_start, 2);
        assert_eq!(encoded[0].length, 3);
    }
}
