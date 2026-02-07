//! Parse variables from ng-repeat/ng-init expressions

use crate::model::HtmlLocalVariableSource;

/// Parsed variable info
#[derive(Clone, Debug)]
pub struct ParsedVariable {
    pub name: String,
    pub source: HtmlLocalVariableSource,
    /// Byte offset within the expression
    pub offset: usize,
    /// Length of variable name
    pub len: usize,
}

/// Parse ng-repeat expression for variables
/// e.g. "item in items" -> [ParsedVariable { name: "item", ... }]
/// e.g. "(key, value) in obj" -> [ParsedVariable { name: "key", ... }, ParsedVariable { name: "value", ... }]
pub fn parse_ng_repeat_expression(expr: &str) -> Vec<ParsedVariable> {
    let mut result = Vec::new();

    let Some(in_idx) = expr.find(" in ") else {
        return result;
    };

    let iter_part = &expr[..in_idx];

    if iter_part.trim().starts_with('(') {
        // (key, value) pattern
        if let Some(open_paren) = iter_part.find('(') {
            if let Some(close_paren) = iter_part.find(')') {
                let inner = &iter_part[open_paren + 1..close_paren];
                let current_offset = open_paren + 1;

                for var in inner.split(',') {
                    let var_trimmed = var.trim();
                    if !var_trimmed.is_empty() {
                        let var_offset_in_inner =
                            var.as_ptr() as usize - inner.as_ptr() as usize;
                        let leading_spaces = var.len() - var.trim_start().len();
                        let offset = current_offset + var_offset_in_inner + leading_spaces;

                        result.push(ParsedVariable {
                            name: var_trimmed.to_string(),
                            source: HtmlLocalVariableSource::NgRepeatKeyValue,
                            offset,
                            len: var_trimmed.len(),
                        });
                    }
                }
            }
        }
    } else {
        // item pattern
        let trimmed = iter_part.trim();
        if !trimmed.is_empty() {
            let leading_spaces = iter_part.len() - iter_part.trim_start().len();
            result.push(ParsedVariable {
                name: trimmed.to_string(),
                source: HtmlLocalVariableSource::NgRepeatIterator,
                offset: leading_spaces,
                len: trimmed.len(),
            });
        }
    }

    result
}

/// Parse ng-init expression for variables
/// e.g. "a = 1" -> [ParsedVariable { name: "a", ... }]
/// e.g. "a = 1; b = 2" -> [ParsedVariable { name: "a", ... }, ParsedVariable { name: "b", ... }]
pub fn parse_ng_init_expression(expr: &str) -> Vec<ParsedVariable> {
    let mut result = Vec::new();
    let mut pos = 0;

    for statement in expr.split(';') {
        if let Some(eq_idx) = statement.find('=') {
            let before_eq = &statement[..eq_idx];
            let after_eq_char = statement.chars().nth(eq_idx + 1);
            // Exclude ==, ===, !=, !==
            if after_eq_char != Some('=') && !before_eq.ends_with('!') {
                let lhs = before_eq.trim();
                if !lhs.is_empty() && is_valid_identifier(lhs) {
                    let leading_spaces = before_eq.len() - before_eq.trim_start().len();
                    let offset = pos + leading_spaces;

                    result.push(ParsedVariable {
                        name: lhs.to_string(),
                        source: HtmlLocalVariableSource::NgInit,
                        offset,
                        len: lhs.len(),
                    });
                }
            }
        }
        pos += statement.len() + 1; // +1 for semicolon
    }

    result
}

/// Check if string is a valid JavaScript identifier
pub fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    if let Some(first) = chars.next() {
        if !first.is_alphabetic() && first != '_' && first != '$' {
            return false;
        }
        chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ng_repeat_simple() {
        let vars = parse_ng_repeat_expression("item in items");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "item");
        assert_eq!(vars[0].offset, 0);
        assert_eq!(vars[0].len, 4);
        assert!(matches!(
            vars[0].source,
            HtmlLocalVariableSource::NgRepeatIterator
        ));
    }

    #[test]
    fn test_parse_ng_repeat_key_value() {
        let vars = parse_ng_repeat_expression("(key, value) in obj");
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].name, "key");
        assert_eq!(vars[1].name, "value");
        assert!(matches!(
            vars[0].source,
            HtmlLocalVariableSource::NgRepeatKeyValue
        ));
        assert!(matches!(
            vars[1].source,
            HtmlLocalVariableSource::NgRepeatKeyValue
        ));
    }

    #[test]
    fn test_parse_ng_init_single() {
        let vars = parse_ng_init_expression("a = 1");
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0].name, "a");
        assert_eq!(vars[0].offset, 0);
        assert_eq!(vars[0].len, 1);
        assert!(matches!(vars[0].source, HtmlLocalVariableSource::NgInit));
    }

    #[test]
    fn test_parse_ng_init_multiple() {
        let vars = parse_ng_init_expression("a = 1; b = 2");
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].name, "a");
        assert_eq!(vars[1].name, "b");
    }

    #[test]
    fn test_parse_ng_init_excludes_comparison() {
        let vars = parse_ng_init_expression("a == 1");
        assert_eq!(vars.len(), 0);

        let vars = parse_ng_init_expression("a != 1");
        assert_eq!(vars.len(), 0);
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_bar"));
        assert!(is_valid_identifier("$scope"));
        assert!(is_valid_identifier("item1"));
        assert!(!is_valid_identifier("1item"));
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("foo.bar"));
    }
}
