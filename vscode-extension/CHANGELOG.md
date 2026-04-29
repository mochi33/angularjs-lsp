# Change Log

## [0.3.1] - 2026-04-30

### Fixes
- LSP 起動時に既に開かれていた HTML/JS ファイルが、新規ファイルを開くまで解析されない問題を修正 (PR #39)
  - `initialized()` の workspace scan 完了後に `republish_open_files_after_init` を実行し、(a) 開いている全ファイルを buffer 内容で再解析、(b) 全 open file の診断を再発行 (HTML + JS)、(c) `semantic_tokens_refresh` / `code_lens_refresh` を発火
  - 旧来は scan_workspace が disk 内容で開いているファイルを上書きしてしまうレースと、HTML 診断 / refresh signal の取りこぼしが重なり、新規ファイル open まで解析が走らないように見える症状があった

## [0.3.0] - 2026-04-30

### Breaking changes
- `ajsconfig.json` の `interpolate.startSymbol` / `interpolate.endSymbol` 設定を撤去。interpolate 記号は AngularJS ソース中の `$interpolateProvider.startSymbol(...)` / `.endSymbol(...)` 呼び出しから自動検出されるようになった。配列 DI rename / 暗黙 DI / チェイン呼び出しすべて対応。古い ajsconfig.json に `interpolate` フィールドが残っていてもパースエラーにはならず黙って無視される

### Features
- `$interpolateProvider` カスタム記号を JS ソースから自動検出 (PR #34)
- `InterpolateStore` を cache に持続化、cache hit 起動でも custom 記号を維持

### Fixes
- `$routeProvider` / `$stateProvider` の receiver 検証を追加し、無関係な `obj.when()` / `obj.state()` を route binding として誤検知しないように (PR #33)
- HTML 補完で `$scope.X` (Method) と `X` (Function) が重複して出る問題を修正 (PR #26)
- `pending_reanalysis` の取りこぼし & 孫漏れを drain ループ + visited セットで解消 (PR #31)

### Performance
- 全 LSP リクエストハンドラを `spawn_blocking` で隔離し、tokio worker 占有を解消 (PR #35)
- `find_symbol_at_position` を URI 逆引きで O(該当URI) 化 — hover/refs/rename/定義ジャンプを高速化 (PR #36)
- `get_reference_names_for_uri` を URI 逆引きで O(該当URI) 化 — HTML edit 時の snapshot 計算を高速化 (PR #37)
- 定義取得を `document_symbols` URI 逆引きで O(該当ファイル) 化 (PR #25)
- HTML 変更時の JS 再診断を依存関係に基づいて絞り込み (PR #27)
- シンボル参照アクセスを borrowing API に置換し Vec clone を回避 (PR #28)
- 不要な `semantic_tokens_refresh` / `code_lens_refresh` をスキップ — cross-file dep snapshot で gate (PR #29 / #30)
- tsserver の `did_change` を debounce + 必要時 flush (PR #32)

## [0.1.0] - Initial Release

### Added
- Language Server Protocol client for angularjs-lsp
- Completion support for AngularJS components
- Hover information
- Go to Definition
- Find References
- Rename support
- Document Symbols
- Code Lens for controller-template bindings
- Custom command: angularjs.openLocation
- Configurable server path
- ajsconfig.json support for custom interpolation symbols
