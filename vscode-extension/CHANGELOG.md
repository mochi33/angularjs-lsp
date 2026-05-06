# Change Log

## [0.4.2] - 2026-05-07

### Fixes
- HTML 編集後にセマンティックトークン位置が実際のシンボル位置からずれる問題を修正 (PR #91)
  - cross-file dep 変化がない同一ファイル編集でも `semantic_tokens_refresh` を
    必ず発火するようにした。これがないと VS Code が didChange 直後に取得した
    旧 position をそのまま新しいバッファに適用し続けてハイライトがずれていた
- `alias.property` 形式の `HtmlScopeReference` の span を property 部分のみに
  絞って登録するように変更 (PR #90)。以前は alias + dot + property 全体を
  覆う長い span が登録されていて、overlap dedup により alias 単独 token が
  消され METHOD 色が alias 部分まで広がっていた
- HTML 解析の position を UTF-16 単位に統一 (PR #88)。`<form name="X">` の
  HtmlFormBinding、ng-controller の SymbolReference、ng-repeat / ng-init の
  継承ローカル変数で tree-sitter の byte column を直接保存していた箇所を
  `byte_col_to_utf16_col` 経由に修正

## [0.4.1] - 2026-05-06

### Fixes
- `.component('foo', { controller: SomeIdentifier, ... })` で controller が
  ファイル内の `function SomeIdentifier(...)` / `class SomeIdentifier {...}` を
  identifier 参照していたとき、CodeLens が `(not found)` を表示する問題を修正
  (PR #85)
- 同じ class の問題が `$routeProvider.when` / `$stateProvider.state` /
  ui-router 名前付き views / `$uibModal.open` / `$mdDialog.show` /
  `$mdBottomSheet` / `$mdToast` / `$mdPanel.open` / `ngDialog.open` でも
  起きていたのを `extract_template_binding_from_object` 経路で一括修正
- `.component({ controller: Identifier, controllerAs: 'foo' })` の HTML 側
  `foo.X` で誤った "Property is not defined" 診断と goto-definition 失敗が
  起きていた問題を修正。`vm.X` メソッド登録時の prefix を
  `ComponentTemplateUrl.controller_name` の決定規則と揃えた

## [0.4.0] - 2026-05-06

### Features
- Inlay hints で DI alias と controller as の隠れた束縛を可視化 (PR #78)
- Document highlight: カーソル位置の同名シンボルを同ファイル内でハイライト (PR #77, #81)
- Signature help を実装 (PR #72)
- Rename refactoring を実装 (PR #71)
- DI 配列の要素数と関数の引数数の不一致を診断で警告 (PR #73)

### Performance
- did_open / initialized 後の `semantic_tokens_refresh` を診断発行と並列化 (PR #60)
- Inlay hints の JS Tree をキャッシュして再パースを省略 (PR #78)

### Refactor
- `scope_reference.rs` の UTF-16 walker を `position_in_text` に共通化 (PR #50)
- HTML ハンドラ三兄弟の位置解決ロジックを `HtmlResolution` で統合 (PR #49)
- `scope_reference.rs` から ui-sref / ng-model 処理を別モジュールへ分離 (PR #48)
- receiver 判定を `ReceiverMatch` enum で統合 (PR #46)
- route/state の config 解析を `ConfigKind` enum で統合 (PR #47)
- `span_of` helper を導入し Node→Span 変換を整理

### Reverted
- 未登録 directive / component の使用を診断で警告する機能 (PR #75) を revert (PR #83)
- component bindings と HTML 属性の対応漏れを警告する診断 (PR #79) を revert (PR #82)

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
