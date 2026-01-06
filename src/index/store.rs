use dashmap::DashMap;
use tower_lsp::lsp_types::Url;
use tracing::debug;

use super::symbol::{Symbol, SymbolReference};

pub struct SymbolIndex {
    definitions: DashMap<String, Vec<Symbol>>,
    references: DashMap<String, Vec<SymbolReference>>,
    document_symbols: DashMap<Url, Vec<String>>,
}

impl SymbolIndex {
    pub fn new() -> Self {
        Self {
            definitions: DashMap::new(),
            references: DashMap::new(),
            document_symbols: DashMap::new(),
        }
    }

    pub fn add_definition(&self, symbol: Symbol) {
        let name = symbol.name.clone();
        let uri = symbol.uri.clone();

        let mut entry = self.definitions.entry(name.clone()).or_default();
        // 重複チェック
        let is_duplicate = entry.iter().any(|s| {
            s.uri == symbol.uri
                && s.start_line == symbol.start_line
                && s.start_col == symbol.start_col
        });
        if !is_duplicate {
            entry.push(symbol);
            self.document_symbols.entry(uri).or_default().push(name);
        }
    }

    pub fn add_reference(&self, reference: SymbolReference) {
        let name = reference.name.clone();
        let uri = reference.uri.clone();

        let mut entry = self.references.entry(name.clone()).or_default();
        // 重複チェック
        let is_duplicate = entry.iter().any(|r| {
            r.uri == reference.uri
                && r.start_line == reference.start_line
                && r.start_col == reference.start_col
        });
        if !is_duplicate {
            entry.push(reference);
            self.document_symbols.entry(uri).or_default().push(name);
        }
    }

    pub fn get_definitions(&self, name: &str) -> Vec<Symbol> {
        self.definitions
            .get(name)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    pub fn get_references(&self, name: &str) -> Vec<SymbolReference> {
        self.references
            .get(name)
            .map(|v| v.value().clone())
            .unwrap_or_default()
    }

    pub fn has_definition(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }

    pub fn get_all_definitions(&self) -> Vec<Symbol> {
        self.definitions
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    pub fn clear_document(&self, uri: &Url) {
        if let Some((_, symbols)) = self.document_symbols.remove(uri) {
            for symbol_name in symbols {
                if let Some(mut defs) = self.definitions.get_mut(&symbol_name) {
                    defs.retain(|s| &s.uri != uri);
                }
                if let Some(mut refs) = self.references.get_mut(&symbol_name) {
                    refs.retain(|r| &r.uri != uri);
                }
            }
        }
    }

    pub fn remove_document(&self, uri: &Url) {
        self.clear_document(uri);
    }

    pub fn find_symbol_at_position(&self, uri: &Url, line: u32, col: u32) -> Option<String> {
        let mut best_match: Option<(String, u32)> = None; // (name, range_size)

        // Check definitions - use name position for matching (not definition position)
        for entry in self.definitions.iter() {
            for symbol in entry.value() {
                // シンボル名の位置（name_start_*, name_end_*）を使って検索
                if &symbol.uri == uri && self.is_position_in_range(
                    line, col,
                    symbol.name_start_line, symbol.name_start_col,
                    symbol.name_end_line, symbol.name_end_col,
                ) {
                    debug!(
                        "find_symbol_at_position: definition match '{}' at {}:{}-{}:{}",
                        symbol.name,
                        symbol.name_start_line, symbol.name_start_col,
                        symbol.name_end_line, symbol.name_end_col
                    );
                    let range_size = self.calculate_range_size(
                        symbol.name_start_line, symbol.name_start_col,
                        symbol.name_end_line, symbol.name_end_col,
                    );
                    if best_match.is_none() || range_size < best_match.as_ref().unwrap().1 {
                        best_match = Some((symbol.name.clone(), range_size));
                    }
                }
            }
        }

        // Check references - find the smallest matching range
        for entry in self.references.iter() {
            for reference in entry.value() {
                if &reference.uri == uri && self.is_position_in_range(
                    line, col,
                    reference.start_line, reference.start_col,
                    reference.end_line, reference.end_col,
                ) {
                    debug!(
                        "find_symbol_at_position: reference match '{}' at {}:{}-{}:{}",
                        reference.name,
                        reference.start_line, reference.start_col,
                        reference.end_line, reference.end_col
                    );
                    let range_size = self.calculate_range_size(
                        reference.start_line, reference.start_col,
                        reference.end_line, reference.end_col,
                    );
                    if best_match.is_none() || range_size < best_match.as_ref().unwrap().1 {
                        best_match = Some((reference.name.clone(), range_size));
                    }
                }
            }
        }

        debug!(
            "find_symbol_at_position: result for {}:{} = {:?}",
            line, col, best_match.as_ref().map(|(n, _)| n)
        );
        best_match.map(|(name, _)| name)
    }

    /// 範囲のサイズを計算（行数 * 10000 + 列数で近似）
    fn calculate_range_size(
        &self,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    ) -> u32 {
        let line_diff = end_line - start_line;
        let col_diff = if line_diff == 0 {
            end_col - start_col
        } else {
            end_col + (10000 - start_col) // 近似値
        };
        line_diff * 10000 + col_diff
    }

    /// 位置が範囲内にあるかどうかをチェック
    fn is_position_in_range(
        &self,
        line: u32,
        col: u32,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    ) -> bool {
        if line < start_line || line > end_line {
            return false;
        }
        if line == start_line && col < start_col {
            return false;
        }
        if line == end_line && col > end_col {
            return false;
        }
        true
    }
}

impl Default for SymbolIndex {
    fn default() -> Self {
        Self::new()
    }
}
