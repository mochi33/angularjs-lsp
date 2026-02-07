use tree_sitter::{Parser, Tree};

pub struct JsParser {
    parser: Parser,
}

impl JsParser {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .expect("Failed to load JavaScript grammar");

        Self { parser }
    }

    pub fn parse(&mut self, source: &str) -> Option<Tree> {
        self.parser.parse(source, None)
    }
}

impl Default for JsParser {
    fn default() -> Self {
        Self::new()
    }
}
