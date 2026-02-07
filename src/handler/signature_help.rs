use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::index::Index;
use crate::model::Symbol;
use crate::util::is_html_file;

pub struct SignatureHelpHandler {
    index: Arc<Index>,
}

impl SignatureHelpHandler {
    pub fn new(index: Arc<Index>) -> Self {
        Self { index }
    }

    /// signatureHelpリクエストを処理する
    pub fn signature_help(
        &self,
        uri: &Url,
        line: u32,
        col: u32,
        source: &str,
    ) -> Option<SignatureHelp> {
        // 1. カーソル位置から関数呼び出しコンテキストを取得
        let call_context = self.find_call_context(source, line, col)?;

        // 2. シンボル定義を取得
        let symbol =
            self.find_symbol_definition(uri, line, &call_context.function_name)?;

        // 3. SignatureHelpを構築
        self.build_signature_help(
            &symbol.name,
            symbol.parameters.as_ref(),
            symbol.docs.as_ref(),
            call_context.active_parameter,
        )
    }

    /// カーソル位置から関数呼び出しのコンテキストを取得
    fn find_call_context(&self, source: &str, line: u32, col: u32) -> Option<CallContext> {
        let lines: Vec<&str> = source.lines().collect();
        if (line as usize) >= lines.len() {
            return None;
        }

        let current_line = lines[line as usize];
        let col = (col as usize).min(current_line.len());

        // カーソル位置から逆方向に開き括弧を探す
        let before_cursor = &current_line[..col];

        // 開き括弧を探し、対応する関数名を取得
        let mut paren_depth = 0;
        let mut paren_pos = None;

        for (i, c) in before_cursor.char_indices().rev() {
            match c {
                ')' => paren_depth += 1,
                '(' => {
                    if paren_depth == 0 {
                        paren_pos = Some(i);
                        break;
                    }
                    paren_depth -= 1;
                }
                _ => {}
            }
        }

        let paren_pos = paren_pos?;

        // 開き括弧の前の識別子を取得（関数名またはメソッド名）
        let before_paren = &before_cursor[..paren_pos];
        let function_name = self.extract_function_name(before_paren)?;

        // アクティブなパラメータを計算（カンマの数をカウント）
        let inside_parens = &before_cursor[paren_pos + 1..];
        let active_parameter = self.count_commas(inside_parens);

        Some(CallContext {
            function_name,
            active_parameter,
        })
    }

    /// 関数名またはメソッド名を抽出
    /// 例: "ServiceName.methodName" -> "ServiceName.methodName"
    /// 例: "methodName" -> "methodName"
    fn extract_function_name(&self, text: &str) -> Option<String> {
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            return None;
        }

        // 識別子とドットを逆方向に収集
        let mut name_chars = Vec::new();
        let mut found_identifier = false;

        for c in trimmed.chars().rev() {
            if c.is_alphanumeric() || c == '_' || c == '$' {
                name_chars.push(c);
                found_identifier = true;
            } else if c == '.' && found_identifier {
                name_chars.push(c);
                found_identifier = false; // ドットの後は新しい識別子を期待
            } else if found_identifier {
                break;
            }
        }

        if name_chars.is_empty() {
            return None;
        }

        let name: String = name_chars.into_iter().rev().collect();
        Some(name)
    }

    /// カンマの数をカウント（ネストされた括弧内のカンマは除外）
    fn count_commas(&self, text: &str) -> u32 {
        let mut count: u32 = 0;
        let mut paren_depth: i32 = 0;
        let mut bracket_depth: i32 = 0;
        let mut brace_depth: i32 = 0;

        for c in text.chars() {
            match c {
                '(' => paren_depth += 1,
                ')' => paren_depth = paren_depth.saturating_sub(1),
                '[' => bracket_depth += 1,
                ']' => bracket_depth = bracket_depth.saturating_sub(1),
                '{' => brace_depth += 1,
                '}' => brace_depth = brace_depth.saturating_sub(1),
                ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                    count += 1;
                }
                _ => {}
            }
        }

        count
    }

    /// シンボル定義を検索
    fn find_symbol_definition(
        &self,
        uri: &Url,
        line: u32,
        function_name: &str,
    ) -> Option<Symbol> {
        // 1. まず完全な名前で検索（ServiceName.methodName）
        let definitions = self.index.definitions.get_definitions(function_name);
        if let Some(def) = definitions.first() {
            return Some(def.clone());
        }

        // 2. $scope.methodName 形式の場合、ControllerName.$scope.methodName に変換
        if function_name.starts_with("$scope.") {
            if let Some(controller_name) =
                self.index.controllers.get_controller_at(uri, line)
            {
                let method_name = &function_name[7..]; // "$scope." の後の部分
                let full_name =
                    format!("{}.$scope.{}", controller_name, method_name);
                let defs = self.index.definitions.get_definitions(&full_name);
                if let Some(def) = defs.first() {
                    return Some(def.clone());
                }
            }
        }

        // 3. HTMLファイルの場合
        if is_html_file(uri) {
            // 3a. alias.method 形式をコントローラー名に解決 (例: ctrl.doSomething)
            let alias_mappings =
                self.index.controllers.get_html_alias_mappings(uri, line);
            for (alias, controller_name) in &alias_mappings {
                if function_name.starts_with(&format!("{}.", alias)) {
                    let method_part = &function_name[alias.len() + 1..];
                    let full_name =
                        format!("{}.$scope.{}", controller_name, method_part);
                    let defs = self.index.definitions.get_definitions(&full_name);
                    if let Some(def) = defs.first() {
                        return Some(def.clone());
                    }
                }
            }

            // 3b. 直接メソッド呼び出し (例: doSomething) - エイリアスなしの場合
            if !function_name.contains('.') {
                // テンプレートバインディングまたはng-controllerからコントローラーを解決
                if let Some(controller_name) =
                    self.index.resolve_controller_for_html(uri, line)
                {
                    let full_name =
                        format!("{}.$scope.{}", controller_name, function_name);
                    let defs = self.index.definitions.get_definitions(&full_name);
                    if let Some(def) = defs.first() {
                        return Some(def.clone());
                    }
                }
            }
        }

        None
    }

    /// SignatureHelpレスポンスを構築
    fn build_signature_help(
        &self,
        name: &str,
        parameters: Option<&Vec<String>>,
        docs: Option<&String>,
        active_parameter: u32,
    ) -> Option<SignatureHelp> {
        let params = parameters?;

        if params.is_empty() {
            return None;
        }

        // シグネチャラベルを構築
        let param_labels: Vec<String> = params.iter().map(|p| p.clone()).collect();
        let label = format!("{}({})", name, param_labels.join(", "));

        // パラメータ情報を構築
        let parameter_info: Vec<ParameterInformation> = params
            .iter()
            .map(|p| ParameterInformation {
                label: ParameterLabel::Simple(p.clone()),
                documentation: None,
            })
            .collect();

        let documentation = docs.map(|d| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: d.clone(),
            })
        });

        let signature = SignatureInformation {
            label,
            documentation,
            parameters: Some(parameter_info),
            active_parameter: Some(active_parameter),
        };

        Some(SignatureHelp {
            signatures: vec![signature],
            active_signature: Some(0),
            active_parameter: Some(active_parameter),
        })
    }
}

/// 関数呼び出しのコンテキスト情報
struct CallContext {
    /// 呼び出されている関数/メソッド名
    function_name: String,
    /// アクティブなパラメータのインデックス（0始まり）
    active_parameter: u32,
}
