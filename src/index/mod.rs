pub mod component_store;
pub mod controller_store;
pub mod definition_store;
pub mod export_store;
pub mod html_store;
mod query;
pub mod template_store;

pub use component_store::ComponentStore;
pub use controller_store::ControllerStore;
pub use definition_store::DefinitionStore;
pub use export_store::ExportStore;
pub use html_store::HtmlStore;
pub use template_store::TemplateStore;

use tower_lsp::lsp_types::Url;

/// Index ファサード — 6つの専門ストアを束ねる
pub struct Index {
    pub definitions: DefinitionStore,
    pub controllers: ControllerStore,
    pub templates: TemplateStore,
    pub html: HtmlStore,
    pub exports: ExportStore,
    pub components: ComponentStore,
}

impl Index {
    pub fn new() -> Self {
        Self {
            definitions: DefinitionStore::new(),
            controllers: ControllerStore::new(),
            templates: TemplateStore::new(),
            html: HtmlStore::new(),
            exports: ExportStore::new(),
            components: ComponentStore::new(),
        }
    }

    /// 指定URIの全データをクリア
    pub fn clear_document(&self, uri: &Url) {
        self.definitions.clear_document(uri);
        self.controllers.clear_document(uri);
        self.templates.clear_document(uri);
        self.html.clear_document(uri);
        self.exports.clear_document(uri);
        self.components.clear_document(uri);
    }

    /// 全てのインデックスデータをクリア
    pub fn clear_all(&self) {
        self.definitions.clear_all();
        self.controllers.clear_all();
        self.templates.clear_all();
        self.html.clear_all();
        self.exports.clear_all();
        self.components.clear_all();
    }

    /// HTML参照情報のみをクリア（Pass 3で収集する情報）
    pub fn clear_html_references(&self, uri: &Url) {
        self.html.clear_html_references(uri);
    }

    /// 再解析が必要なURIを取得してキューをクリア
    pub fn take_pending_reanalysis(&self) -> Vec<Url> {
        self.templates.take_pending_reanalysis()
    }

    pub fn remove_from_pending_reanalysis(&self, uri: &Url) {
        self.templates.remove_from_pending_reanalysis(uri);
    }

    pub fn mark_html_analyzed(&self, uri: &Url) {
        self.templates.mark_html_analyzed(uri);
    }
}

impl Default for Index {
    fn default() -> Self {
        Self::new()
    }
}
