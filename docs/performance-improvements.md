# ファイル更新時の処理パフォーマンス改善候補

`textDocument/didOpen` / `didChange` / `didSave` を起点とした解析パイプラインにおいて、現状把握できている非効率・冗長・取りこぼしリスクのある箇所をまとめる。

各項目は以下の構成:

- **該当箇所**: 主要なファイルパスと行番号
- **問題**: 何が起きているか
- **改善案**: 具体的な書き換え方針
- **影響度 / 改修難度**

優先度の付け方: 「ファイル更新ごとに毎回走るコスト」「依存深さに比例して悪化するコスト」が高いものを High、効くが影響範囲が限定的なものを Medium、クリーンアップ系を Low とする。

---

## High

### #1 `get_definitions_for_uri` / `get_scope_definitions_for_js` が DashMap 全件走査 ✅ 対応済み

- **該当箇所**: `src/index/definition_store.rs`
  - 呼び出し元: `src/server/mod.rs:232-235`(HTML on_change), `src/server/mod.rs:330-344`(JS on_change before/after), `src/handler/diagnostics.rs:check_unused_scope_variables`, `src/index/query.rs:292`
- **問題 (対応前)**:
  - シンボル名 → `Vec<Symbol>` の `definitions: DashMap<String, Vec<Symbol>>` を**全エントリ走査**し、`Vec<Symbol>` を毎回 clone してから URI で絞り込んでいた
  - 毎キーストロークごと (debounce 後)、JS 編集では before/after の **2 回**呼ばれる
  - シンボル数が増えるほど線形にコストが上がる (編集中ファイルの大きさではなく**ワークスペース全体のシンボル数**に比例)
  - `document_symbols: DashMap<Url, Vec<String>>` という URI 逆引きが既に存在していたのに使っていなかった
- **対応**:
  - 新ヘルパー `collect_definitions_for_uri(uri, predicate)` を導入し、`document_symbols.get(uri)` で候補名リストを取り、各 name について `definitions.get(name)` → URI / `predicate` フィルタ、という **O(該当ドキュメントのシンボル数)** 実装に置き換えた
  - `get_definitions_for_uri` は `predicate = |_| true`、`get_scope_definitions_for_js` は kind フィルタを `predicate` に渡すだけで合流
  - 呼び出し元のシグネチャは変えていないので、既存の利用箇所はそのまま高速化された
  - 単体テスト追加: `get_definitions_for_uri_returns_only_target_uri`, `get_scope_definitions_for_js_filters_by_kind`
  - 同時に #10 も解決 (`document_symbols` を `HashSet<String>` 化)
- **影響度**: 大 (毎更新の常時コスト) / **改修難度**: 中

### #2 `republish_all_js_diagnostics` が「開いている JS 全部」

- **該当箇所**: `src/server/mod.rs:76-91, 277-283`
- **問題**:
  - HTML 更新時、embedded `<script>` の有無で発火条件は絞られているが、発火したら**開いている JS ファイル全部**を再診断する
  - 実際に診断結果が変わり得るのは「変更された HTML が参照する scope シンボルを定義している JS」だけのはず
  - JS 側には `collect_affected_html_uris` 相当の絞り込み関数が無い
- **改善案**:
  - JS 用の影響範囲計算 `collect_affected_js_uris(changed_html_uri)` を追加する
  - 算出根拠: 変更前後の HTML の `$scope` 参照名集合の和 → それらを定義している JS URI を `definitions` 経由で逆引き(または `document_symbols` の逆引き)
- **影響度**: 中 (大量の JS を開いている環境ほど顕著) / **改修難度**: 中

### #3 `collect_affected_html_uris` の参照ルックアップで全件 clone

- **該当箇所**: `src/server/mod.rs:106-135`, 内部呼出 `src/index/definition_store.rs:73`(`get_references`)
- **問題**:
  - `get_references(name)` は `Vec<SymbolReference>::clone()` を返すので、**開いていない HTML のリファレンスも一旦丸ごと clone してから捨てている**
  - JS 編集時に `before ∪ after` の各シンボル名で呼び出されるため、シンボル変化が大きいリネーム等で膨らむ
- **改善案**:
  - `for_each_reference(name, |r| …) -> bool` 形式の borrowing API を `DefinitionStore` に追加し、HTML 該当時点で early return / 早期判定する
  - もしくは `references_by_uri: DashMap<Url, Vec<SymbolReference>>` の逆引きインデックスを追加する
- **影響度**: 中 / **改修難度**: 中

### #4 `take_pending_reanalysis` の単発消費 — 取りこぼし & 1 段までしか追わない

- **該当箇所**: `src/index/template_store.rs:679-681`, `src/server/mod.rs:249-263`
- **問題**:
  1. `take` した直後に `analyze_document` 内で panic / 失敗すると、pending エントリが消失して二度と再解析されない
  2. 子の再解析中に**孫**が新たに `add_pending_reanalysis()` されても、その同じラウンドでは消費されない (ループ化されていない)
  3. 子の再解析パスで `analyze_document()` を呼んでおり、親の Tree を流用せずに**再パース**している (#7 とも関連)
- **改善案**:
  - `while !pending.is_empty()` でドレインするループ構造にする (深さ上限 / 重複検知付き)
  - 取り出し → 失敗 → pending に戻す、もしくは「処理中セット」と「pending セット」を分ける
  - 親パスで作った Tree を流用するパス (`analyze_document_with_tree` の公開化) を子側でも使う
- **影響度**: 中 (`ng-include` 依存が深いプロジェクトで顕著) / **改修難度**: 中

---

## Medium

### #5 `clear_document` が必要以上に消す

- **該当箇所**: `src/analyzer/html/mod.rs:81-96` (`analyze_document_with_tree` 冒頭で `index.clear_document(uri)`)
  - 比較対象: `analyze_document_references_only_with_tree` (`html/mod.rs:110-112`) は `clear_html_references` のみ
- **問題**:
  - `clear_document` は 6 ストア (definitions / controllers / templates / html / exports / components) 全部を消してから Pass 1〜3 を再実行する
  - 実際には HTML の編集で変わるのは大半が Pass 3 (scope 参照) のみ
  - Pass 1 (ng-controller) / 1.5 (ng-include) / 2 (form bindings) の出力が変わらないケースでも全消しになる
- **改善案**:
  - `clear_document` を「ストア別に消すか選べる」API に分割する
  - もしくは Pass 1/1.5/2 の出力をハッシュ等で差分判定し、変化がなければ該当ストアの clear と再投入をスキップ
- **影響度**: 中 / **改修難度**: 中

### #6 `semantic_tokens_refresh` / `code_lens_refresh` を無条件発火

- **該当箇所**: `src/server/mod.rs:286-287, 363-364, 419-420, 436-437`
- **問題**:
  - LSP 仕様上 `semanticTokens/refresh` は**クライアントが開いている全ファイル分** `semanticTokens/full` を再リクエストさせる
  - HTML を 1 ファイル編集するだけで開いている HTML × N に対するリクエストが返ってくる構造
  - JS 編集時の `semantic_tokens_refresh` は、シンボル名集合に変化がなければ HTML 側のトークンも変わらないため不要
  - `code_lens_refresh` は JS 側の意味付けが主なので、HTML 編集時には不要なことが多い
- **改善案**:
  - JS パス: `before_symbols == after_symbols` のときは両 refresh をスキップ
  - HTML パス: embedded script 有無や scope 参照集合に変化があったときだけ `semantic_tokens_refresh`、`code_lens_refresh` は基本省略
- **影響度**: 中 (体感レイテンシ) / **改修難度**: 小

### #7 HTML パース重複 (`analyze_document` vs `analyze_document_and_extract_scripts`)

- **該当箇所**: `src/analyzer/html/mod.rs:61-78`, 利用側 `src/server/mod.rs:261`
- **問題**:
  - 通常パスは `analyze_document_and_extract_scripts` (Tree を 1 回パースして使い回す) で済んでいる
  - だが pending 子テンプレートの再解析 (`server/mod.rs:261`) は `analyze_document` 経由なので、親と独立に**再パース**している
  - `analyze_document_with_tree` は `pub(self)` 相当でクレート外に出ていない (現状 `fn`)
- **改善案**:
  - `analyze_document_with_tree` を呼び出し可能にして、親側で作った Tree (または子用に新規パースした Tree) を使い回せる経路を 1 つに統一
  - `analyze_document` 系の API を整理し、薄い wrapper だけにする
- **影響度**: 小 / **改修難度**: 小

### #8 tsserver への `did_change` が debounce されない

- **該当箇所**: `src/server/mod.rs:369-375`
- **問題**:
  - 解析パイプラインは 200ms debounce しているのに、`ts_proxy.did_change` は `on_change` 末尾で**毎キーストローク**呼ばれている
  - tsserver 側にも内部処理があるとはいえ、IPC とテキスト送信のオーバーヘッドが無駄に増える
  - `version` も常に `0` (`did_save` 経路) または引数 version で渡されている
- **改善案**:
  - debounce 後の `tokio::spawn` ブロック内に `ts_proxy.did_change` を移動する
  - もしくは `debounce_versions` を共有して「最終キー入力からアイドル後にだけ送る」キューを作る
- **影響度**: 小 / **改修難度**: 小

---

## Low

### #9 `clone()` の連鎖 — Symbol / Vec / String

- **該当箇所**: `src/index/definition_store.rs` 各 getter, `src/index/html_store.rs:60-64` ほか
- **問題**:
  - `get_definitions_for_uri`, `get_scope_definitions_for_js`, `get_html_scope_references` などが軒並み `Vec<…>::clone()` を返す
  - ホットパス上にある (`on_change` 内) ためピーク時のヒープ確保量が大きい
- **改善案**:
  - `Symbol` を `Arc<Symbol>` 化し、ストア内も外も共有参照で済ます
  - もしくは `for_each_*` パターンで内部反復して、呼び出し側で必要なものだけ name 等を取り出す
- **影響度**: 小 (体感には出にくいが GC アロケータ圧は減る) / **改修難度**: 小

### #10 `add_definition` / `add_reference` が `document_symbols` に重複 push ✅ 対応済み

- **該当箇所**: `src/index/definition_store.rs`
- **問題 (対応前)**:
  - `definitions` 側には is_duplicate チェックがあるが、`document_symbols` 側には無かった
  - 同じ name が同じ URI で複数回現れると、`document_symbols.get(uri)` の `Vec<String>` に重複が積まれる
  - `document_symbols` を読み取り口として使っていなかったので顕在化していなかったが、#1 で逆引き活用するときに必ず踏むため先回りで解決
- **対応**:
  - `document_symbols` の値型を `Vec<String>` → `HashSet<String>` に変更し、`add_definition` / `add_reference` での `.push` を `.insert` に置き換え
  - `clear_document` の `for symbol_name in symbols` は HashSet の IntoIterator でそのまま動くため改修不要
  - 単体テスト追加: `document_symbols_dedupes_repeated_adds` (同じ name が定義 2 件 + 参照 1 件で add されても `document_symbols` には 1 件のみ)
- **影響度**: 小 (#1 の前提条件) / **改修難度**: 小

---

## 推奨着手順

| 順 | 項目 | 主効果 | 状態 |
|----|------|--------|------|
| 1 | #1 + #10 — `document_symbols` を活用した URI 逆引き | 毎更新の常時コストを大きく削減 | ✅ 対応済み |
| 2 | #6 — refresh をシンボル変化時のみに絞る | 体感レイテンシ改善 | 未対応 |
| 3 | #4 + #7 — pending_reanalysis のループ化と Tree 再利用 | 依存深いテンプレで効く | 未対応 |
| 4 | #2 — JS 単位の影響範囲計算 | 大量 JS を開く環境で効く | 未対応 |
| 5 | #5 — `clear_document` の選択的分割 | 中規模 HTML で効く | 未対応 |
| 6 | #8 — tsserver `did_change` の debounce | IPC トラフィック削減 | 未対応 |
| 7 | #3 / #9 — borrowing API・`Arc` 化 | アロケーション圧削減 | 未対応 |
