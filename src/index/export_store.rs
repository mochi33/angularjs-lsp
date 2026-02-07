use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

use crate::model::{ExportInfo, ExportedComponentObject};

/// ES6エクスポート・インポートの管理ストア
pub struct ExportStore {
    /// ES6 export default 情報（ファイルパス -> ExportInfo）
    exports: DashMap<String, ExportInfo>,
    /// ES6 export default { name: 'xxx', ... } オブジェクトパターン
    exported_component_objects: DashMap<String, ExportedComponentObject>,
    /// ES6 import文のマッピング（URI -> (識別子名 -> インポート元パス)）
    imports: DashMap<Url, DashMap<String, String>>,
}

impl ExportStore {
    pub fn new() -> Self {
        Self {
            exports: DashMap::new(),
            exported_component_objects: DashMap::new(),
            imports: DashMap::new(),
        }
    }

    pub fn add_export(&self, export_info: ExportInfo) {
        let path = export_info.uri.path().to_string();
        self.exports.insert(path, export_info);
    }

    pub fn get_export(&self, uri: &Url) -> Option<ExportInfo> {
        self.exports.get(uri.path()).map(|e| e.value().clone())
    }

    pub fn get_all_exports(&self) -> Vec<ExportInfo> {
        self.exports
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    pub fn add_exported_component_object(&self, obj: ExportedComponentObject) {
        let path = obj.uri.path().to_string();
        self.exported_component_objects.insert(path, obj);
    }

    pub fn get_exported_component_object(
        &self,
        uri: &Url,
    ) -> Option<ExportedComponentObject> {
        self.exported_component_objects
            .get(uri.path())
            .map(|e| e.value().clone())
    }

    pub fn get_all_exported_component_objects(&self) -> Vec<ExportedComponentObject> {
        self.exported_component_objects
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    pub fn add_import(&self, uri: &Url, identifier: String, import_path: String) {
        self.imports
            .entry(uri.clone())
            .or_default()
            .insert(identifier, import_path);
    }

    pub fn get_import_path(&self, uri: &Url, identifier: &str) -> Option<String> {
        self.imports
            .get(uri)
            .and_then(|map| map.get(identifier).map(|p| p.value().clone()))
    }

    /// インポート識別子名からコンポーネント名を取得
    pub fn get_exported_component_name(&self, identifier: &str) -> Option<String> {
        for import_entry in self.imports.iter() {
            if let Some(import_path) = import_entry.value().get(identifier) {
                for obj_entry in self.exported_component_objects.iter() {
                    let obj_path = obj_entry.key();
                    let normalized_import = import_path.value().trim_end_matches(".js");
                    let normalized_obj = obj_path.trim_end_matches(".js");
                    if normalized_obj.ends_with(normalized_import) {
                        return Some(obj_entry.value().name.clone());
                    }
                }
            }
        }
        None
    }

    pub fn clear_document(&self, uri: &Url) {
        self.exports.remove(uri.path());
        self.exported_component_objects.remove(uri.path());
        self.imports.remove(uri);
    }

    pub fn clear_all(&self) {
        self.exports.clear();
        self.exported_component_objects.clear();
        self.imports.clear();
    }
}

impl Default for ExportStore {
    fn default() -> Self {
        Self::new()
    }
}
