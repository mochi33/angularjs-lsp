mod codelens;
mod completion;
mod document_symbol;
mod hover;
mod references;
mod rename;

pub use codelens::CodeLensHandler;
pub use completion::CompletionHandler;
pub use document_symbol::DocumentSymbolHandler;
pub use hover::HoverHandler;
pub use references::ReferencesHandler;
pub use rename::RenameHandler;
