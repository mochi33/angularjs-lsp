mod angularjs;
mod html_angularjs;
mod html_parser;
mod parser;

pub use angularjs::AngularJsAnalyzer;
pub use html_angularjs::{EmbeddedScript, HtmlAngularJsAnalyzer};
pub use html_parser::HtmlParser;
pub use parser::JsParser;
