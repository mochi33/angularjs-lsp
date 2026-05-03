pub mod component_store;
pub mod controller_store;
pub mod definition_store;
pub mod di_usage_store;
pub mod export_store;
pub mod html_resolve;
pub mod html_store;
pub mod interpolate_store;
mod query;
pub mod template_store;

pub use html_resolve::HtmlResolution;

pub use component_store::ComponentStore;
pub use controller_store::ControllerStore;
pub use definition_store::DefinitionStore;
pub use di_usage_store::{DiUsage, DiUsageStore};
pub use export_store::ExportStore;
pub use html_store::HtmlStore;
pub use interpolate_store::InterpolateStore;
pub use template_store::TemplateStore;

use std::sync::atomic::{AtomicBool, Ordering};
use tower_lsp::lsp_types::Url;

/// Index ファサード — 8つの専門ストアを束ねる
pub struct Index {
    pub definitions: DefinitionStore,
    pub controllers: ControllerStore,
    pub templates: TemplateStore,
    pub html: HtmlStore,
    pub exports: ExportStore,
    pub components: ComponentStore,
    pub interpolate: InterpolateStore,
    pub di_usages: DiUsageStore,
    /// workspace の初回スキャンが完了したか。
    ///
    /// 「未知サービス警告 (#63)」のように workspace 全シンボルが揃わないと false
    /// positive を出してしまう診断は、このフラグが true になるまで黙る。
    workspace_scanned: AtomicBool,
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
            interpolate: InterpolateStore::new(),
            di_usages: DiUsageStore::new(),
            workspace_scanned: AtomicBool::new(false),
        }
    }

    /// workspace スキャン完了をマークする (false positive 抑制用)
    pub fn mark_workspace_scanned(&self) {
        self.workspace_scanned.store(true, Ordering::Release);
    }

    /// workspace スキャン完了済みかどうか
    pub fn is_workspace_scanned(&self) -> bool {
        self.workspace_scanned.load(Ordering::Acquire)
    }

    /// 指定URIの全データをクリア
    pub fn clear_document(&self, uri: &Url) {
        self.definitions.clear_document(uri);
        self.controllers.clear_document(uri);
        self.templates.clear_document(uri);
        self.html.clear_document(uri);
        self.exports.clear_document(uri);
        self.components.clear_document(uri);
        self.interpolate.clear_document(uri);
        self.di_usages.clear_document(uri);
    }

    /// 全てのインデックスデータをクリア
    pub fn clear_all(&self) {
        self.definitions.clear_all();
        self.controllers.clear_all();
        self.templates.clear_all();
        self.html.clear_all();
        self.exports.clear_all();
        self.components.clear_all();
        self.interpolate.clear_all();
        self.di_usages.clear_all();
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

    /// 再解析キューに URI を追加 (drain_pending_reanalysis のテスト用に主に利用)
    pub fn add_pending_reanalysis(&self, uri: Url) {
        self.templates.add_pending_reanalysis(uri);
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
