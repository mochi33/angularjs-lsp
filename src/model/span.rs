use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::{Position, Range};

/// 位置情報の統一型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl Span {
    pub fn new(start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> Self {
        Self {
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }

    /// 指定位置がスパン内に含まれるかチェック
    pub fn contains(&self, line: u32, col: u32) -> bool {
        if line < self.start_line || line > self.end_line {
            return false;
        }
        if line == self.start_line && col < self.start_col {
            return false;
        }
        if line == self.end_line && col > self.end_col {
            return false;
        }
        true
    }

    /// 指定行がスパンの行範囲内に含まれるかチェック
    pub fn contains_line(&self, line: u32) -> bool {
        line >= self.start_line && line <= self.end_line
    }

    /// LSP Range に変換
    pub fn to_lsp_range(&self) -> Range {
        Range {
            start: Position {
                line: self.start_line,
                character: self.start_col,
            },
            end: Position {
                line: self.end_line,
                character: self.end_col,
            },
        }
    }

    /// 範囲のサイズを計算（行数 * 10000 + 列数で近似）
    pub fn range_size(&self) -> u32 {
        let line_diff = self.end_line - self.start_line;
        let col_diff = if line_diff == 0 {
            self.end_col - self.start_col
        } else {
            self.end_col + (10000 - self.start_col)
        };
        line_diff * 10000 + col_diff
    }
}

impl Default for Span {
    fn default() -> Self {
        Self {
            start_line: 0,
            start_col: 0,
            end_line: 0,
            end_col: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains() {
        let span = Span::new(5, 10, 5, 20);
        assert!(span.contains(5, 10));
        assert!(span.contains(5, 15));
        assert!(span.contains(5, 20));
        assert!(!span.contains(5, 9));
        assert!(!span.contains(5, 21));
        assert!(!span.contains(4, 15));
        assert!(!span.contains(6, 15));
    }

    #[test]
    fn test_contains_multiline() {
        let span = Span::new(5, 10, 8, 20);
        assert!(span.contains(5, 10));
        assert!(span.contains(6, 0));
        assert!(span.contains(7, 50));
        assert!(span.contains(8, 20));
        assert!(!span.contains(5, 9));
        assert!(!span.contains(8, 21));
    }

    #[test]
    fn test_to_lsp_range() {
        let span = Span::new(5, 10, 8, 20);
        let range = span.to_lsp_range();
        assert_eq!(range.start.line, 5);
        assert_eq!(range.start.character, 10);
        assert_eq!(range.end.line, 8);
        assert_eq!(range.end.character, 20);
    }
}
