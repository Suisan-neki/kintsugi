# 汎用バックエンドv1の使い方

2026-10-14の方針により、画面は操作確認に必要な範囲に留め、バックエンドを優先する。#133で扱う。現在のv1は同じ端末で使う基盤。任意の公開Webサービスへそのまま配備する仕様ではない。

## 起動と接続

Rust 1.99.0でビルドする。公開GitHubの読取・静的解析はRustで行い、Node.jsを必要としない。汎用の報告観測と事例管理・比較・バックアップもRustで処理する。対象がNode.js製のときの実行と、任意設定のAI説明は別の経路。

```bash
cargo build --locked
cargo run -- case init cases/general --template general --id general-001 --title "通知の確認" --actor 作成者 --reason 新しい事例
cargo run -- api cases/general --port 8765 --token-file /tmp/eel-api-token
```

トークンファイルは起動ごとに新規のパスを指定する。既存ファイルは置き換えない。作成時の権限は所有者の読書きだけで、トークンを標準出力へ表示しない。停止後は不要なトークンファイルを削除する。

```bash
curl -H "X-Review-Token: $(cat /tmp/eel-api-token)" http://127.0.0.1:8765/api/v1/case
```

APIを呼ぶクライアントは`X-Review-Token`を指定する。POSTは`Content-Type: application/json`も必要。Originがある場合は起動したlocalhostと一致する必要がある。CLIなどOriginのないクライアントはトークンで操作する。ブラウザの別OriginへのCORS許可は提供しない。同一Originへフロントエンドを配置するか、フロントエンド用サーバーからAPIへ接続する。トークンを外部へ配布しない。

旧`/api/case`・`/api/change`などと`/api/v1/repository`は廃止し、HTTP 410を返す。APIは`/api/v1/`を使う。固定版を実行する場合はAPI起動にも`--repository-root DIR --execution-mode local|bubblewrap`を指定する。指定しないAPIサーバーは実行を有効にしない。現在の対応OSはLinux・macOS。Unix固有の処理を使うためWindowsではビルドできない。

## APIの契約

| メソッド・パス | 入力 | 出力・処理 |
| --- | --- | --- |
| GET `/api/v1/capabilities` | なし。トークン不要 | API版、機能、上限、解析の限界 |
| GET `/api/v1/case` | 操作トークン | 最新のWorkspaceView。現在と観測時の判定、適用範囲、未回答、次の確認 |
| POST `/api/v1/change` | ChangeRequest | 条件・出所・質問・依存先・判断の更新、報告観測の取り込み。更新後のWorkspaceView |
| POST `/api/v1/compare` | expected_revision、check_id、before_id、after_id | 同じ確認事項の異なる二試行の判定と構成の差・不足 |
| GET `/api/v1/backup` | 操作トークン | 開始時に読んだ版までの全履歴。取得中の新しい更新は含めない |
| GET `/api/v1/execution` | 操作トークン | 起動時に許可した固定版実行の対象・方式・現在のジョブ |
| POST `/api/v1/execute` | expected_revision、actor、reason、recipe | 指定したリポジトリの固定版実行を受付。HTTP 202 |
| POST `/api/v1/understand` | UnderstandRequest | 公開GitHubの読取、サーバーが保存した読取記録の参照・解析。コード本文は受け付けない |

POST入力は1MiBまで。更新は`expected_revision`を照合し、別の更新があればHTTP 409を返す。不正な形式や対応しない操作は400、トークン・Origin・権限の不一致は403、未定義のパス、未保存の読取記録・構成図は404。HTTP受付にはaxumを使い、長い読取・AI処理は4件まで同時に処理する。超過時は503。失敗した更新を成功した事例版として保存しない。

`ChangeRequest`は次の形を使う。利用者や入力元が期待する動作を記入する。初期条件や回答を自動では生成しない。

```json
{
  "expected_revision": 1,
  "actor": "記入者",
  "reason": "条件を追加する理由",
  "action": {
    "op": "add_check",
    "check": {
      "id": "delivered",
      "version": 1,
      "text": "通知を読み出した結果が入力と一致する",
      "origin": "この条件を決めた発言・資料",
      "source": null,
      "observation": null,
      "rule": "manual_only",
      "dependencies": [],
      "questions": []
    }
  }
}
```

actionのその他の型は`src/workspace.rs`の`Change`を参照する。HTTPでは、サーバー上の任意ディレクトリを読む`import`操作は受け付けない。ファイルのバックアップ・復元は下のCLIを使い、HTTPで復元先を指定する操作は提供しない。

## 言語に依存しない報告観測

`configure_reported_observation`で`criterion`の観測名`assertion`と対象`subject`を条件へ対応づける。条件版が変わり、依存先`report-CHECK_ID`を追加する。固定版Node.js/Python実行の`configure_observation`とは別の規則`reported_assertion`になる。対応する条件の識別子は121文字以内で指定する。

```json
{
  "op": "configure_reported_observation",
  "check_id": "delivered",
  "criterion": {"assertion": "readback_matches", "subject": "notification-1"}
}
```

取り込みは`import_reported`を使う。conditionの識別子・版・文・出所と、trace.sourceを現在の条件と照合する。code sourceがない条件では両方nullにする。ある場合は同じ固定コード参照を渡す。sourceは報告者が対応づけた参照先で、基盤がそのコードを実行した証明ではない。

```json
{
  "op": "import_reported",
  "condition": {
    "id": "delivered", "version": 2,
    "text": "通知を読み出した結果が入力と一致する",
    "origin": "この条件を決めた発言・資料", "kind": "custom"
  },
  "trace": {
    "schema_version": 1,
    "correlation": {"run_id": "trial-1", "record_id": "notification-1", "attempt_id": "trial-1-attempt"},
    "producer": "報告するアプリ・観測器", "origin": "観測方法と報告の出所",
    "completed": true, "source": null,
    "context": {"input": {"message": "合成の通知"}, "runtime": "使用した実行環境"},
    "observations": [{
      "sequence": 1,
      "correlation": {"run_id": "trial-1", "record_id": "notification-1", "attempt_id": "trial-1-attempt"},
      "assertion": "readback_matches", "subject": "notification-1",
      "outcome": "pass", "detail": "実際に確認した方法と結果", "source": null
    }]
  }
}
```

報告は1MiB・2000観測まで。順序番号は1から連続し、全観測の試行・データ・試行内の識別子がtraceと一致する必要がある。同じ試行を同じ条件へ二度取り込まない。producer・origin・source・contextを構成として保存し、その値が変わった場合は古い根拠の適用範囲を確認する。宣言した他の依存先も取り込み時の値を保存する。

| 報告 | 観測時の判定 |
| --- | --- |
| 選んだ観測名・対象のpassだけ | 条件を満たす（報告された範囲） |
| 選んだ観測名・対象のfailだけ | 条件を満たさない |
| 同じ対象にpassとfail | 判定できない |
| 観測なし・completedがfalse | 判定できない |
| 順序・識別子・形式が不正 | 取り込みを拒否する |

終了コード0や以前の成功を、不足する報告の代わりに使わない。未回答の利用条件、参照コード・条件版・依存先の変更がある場合は、観測時の成功とは別に現在の判定を再確認待ちにする。

reportedのmanifestは取り込みを処理した基盤の情報と取り込み時刻を含む。外部アプリの実行環境・入力・計測条件はtrace.contextへ記録する。基盤側のOS等を外部アプリの環境の証明として読まない。署名・本人確認・独立検証は提供しない。

## コード読取と解析

ReaderRequestは次の三形式を使う。

```json
{"operation":"read","url":"https://github.com/OWNER/REPO","reference":"main"}
```

```json
{"operation":"analyze","snapshot":{},"path":"src/main.rs","line":1}
```

```json
{"operation":"file","snapshot":{},"path":"src/storage.rs"}
```

analyze・fileのsnapshotには以前返したsnapshotの実値を渡す。空のオブジェクトは有効な記録ではない。応答にschema_version、snapshot、model、guide、callers、limitsがある。関数を指定すると、入力・分岐・待機・戻りの記述と呼び出し候補、呼び出し元を返す。コードの本文・宣言・作者の説明を同じ版に固定する。対象リポジトリのプログラムは実行しない。

初回は最大4並列で24ファイル、各128KiBまで。読取と解析は同じ30秒の期限を使い、解析の反復で期限を確認する。Rustの処理は子プロセスの強制終了ではないため、期限を超えた直後の停止は保証しない。宣言の解析は中括弧の入れ子128段までとし、超えた入力は結果を返さずエラーにする。出力はread・analyze・fileが8MiB、その他が16MiBまで。HTTPSの接続先はapi.github.comとraw.githubusercontent.comだけで、リダイレクトには従わない。サーバーが設定したHTTP(S)プロキシとNO_PROXYを使い、証明書はOSの信頼設定で検証する。Linuxで独自CAを使う場合はSSL_CERT_FILE・SSL_CERT_DIRも利用できる。NODE_EXTRA_CA_CERTSはRustの読取には使わない。HTTPからこれらの値を上書きする操作はない。未読・取得失敗・途中で切れた一覧を、リポジトリ全体の解析として扱わない。AIによる業務上の意味の説明と完全な束縛解析は未実装。保存したsnapshotの真正性は保証しない。詳しい構文の範囲は[理解の仕様](repository-understanding.md)。

## 比較とバックアップ・復元

Rustの比較はHTTPとCLIで同じ関数を使う。JSONの整数をJavaScriptの数値へ変換せずに比較する。参照コード、条件と観測規則、実行／報告の構成、入力、環境、宣言した依存先を比較し、不足する項目をunknownにする。結果の差だけで改修の因果効果を特定しない。

```bash
cargo run -- case compare CASE_DIR --before evidence-1 --after evidence-2 --condition delivered
cargo run -- case backup CASE_DIR --output NEW_BACKUP.json
cargo run -- case restore NEW_CASE_DIR NEW_BACKUP.json
```

バックアップは全版の事例、取り込まれた観測、質問・回答、判断、判断当時の根拠、更新履歴を保存する。形式・連続した版・事例ID・以前の根拠と判断の保持・判断当時の条件版・試行識別子・質問と参照版との対応を検査して復元する。旧版の判定器が残した説明・判断を現在の規則で再計算して書き換えない。最大1000版・32MiB。既存のディレクトリへ復元しない。途中で失敗した復元先はrestore.pendingを残し、正常な事例として開かない。

元のrunsの作業領域、Gitのコピー、答え合わせ用のtruth、出力全文は含めない。取り込み時に保存した観測・出力の抜粋は含む。再試験には対象のGit固定版と実行環境を別途用意する。バックアップの内部整合性を確認しても、その報告の真正性や外部状態を証明したことにはならない。

## 再現と読む順番

```bash
cargo build --locked
npm run test:backend
```

`tools/backend-demo.mjs`は合成のPython接続例を別プロセスで動かし、報告の取り込み、判断、再試験、比較、バックアップ、復元をAPIとCLIで確認する。`examples/general-observations/producer.py`を他言語の接続例へ置き換える場合も、同じJSON形式で報告する。

読む順番は`src/reported.rs` → `src/workspace.rs`の報告取り込み → `src/comparison.rs` → `src/backup.rs` → `src/review.rs` → `tools/backend-demo.mjs`。旧VitalSensing専用アダプターは廃止し、現在の固定版実行・報告観測を使う。現在の実行状態はサーバー単位の一件で、サーバー停止・再起動後のジョブ再開やキャンセルは未対応。複数人の公開サービス、任意アプリの自動ビルド・自動計測、実参加者の理解評価、実CASE-001・AWS・iOSは別の残件。


## Rustによる読取・解析（#144）

`src/reader.rs`が要求を受け、`src/reader/github.rs`で固定版を取得する。`analysis.rs`と`comprehension.rs`が宣言・参照・呼び出し候補を抽出し、`overview.rs`・`inquiry.rs`・`plan.rs`が概要・質問への箇所集め・試験計画を返す。読取・解析のためのスクリプトや一時ディレクトリを作らず、対象のコードも実行しない。

入力済みのsnapshotの解析は通信せず、トークンファイルも読まない。追加の本文が必要な操作だけがHTTPクライアントを作る。EEL_GITHUB_TOKEN_FILEの値はサーバーだけが読み、api.github.comへの要求ヘッダーにだけ付ける。raw.githubusercontent.comへは付けない。通信エラーにはURL・ヘッダー・応答本文を含めない。

候補の優先順位と24ファイルの上限を保つ。候補内の同順位はロケール依存の並びからパスのバイト順へ変更し、並列取得した本文も候補順で記録する。同じ優先順位に大小文字・記号が混在する場合、初回に選ぶファイルが従来と異なることがある。必要な本文は追加で読める。

旧`explore`画面とJavaScript側の解析は2026年10月24日に廃止した。現在の画面はRustの読取・解析APIを使う。


ブラウザ試行の画面写真は、認証付きの `GET /api/v1/browser-screenshot/{evidence_id}/{step_id}` で取得する。成功時は`image/png`、対応するファイルがない場合は404。任意のファイルパスは指定できない。事例内の実行フォルダーにある宣言済みの写真だけを扱い、バックアップのJSONには写真を含めない。


## 確認モードと確認資料（2026-10-18、#159）

事例の読取は形式1・2に対応する。初めてモードを記録すると形式2へ移行する。モード未選択の旧形式では汎用の候補を表示し、選択済みの履歴は補わない。全版バックアップは移行前後を保存する。形式2の事例には対応したバイナリを使う。

| 操作 | 内容 |
| --- | --- |
| GET /api/v1/profiles | 確認候補とカタログ版を返す。業務要件や判定結果ではない |
| POST /api/v1/change、select_profile | actionにdomainをgeneralまたはmedicalで指定する。選択履歴を追記する |
| POST /api/v1/change、add_profile_check | template_idとselection_versionを指定する。現在の候補を未確認の条件として追加する |
| GET /api/v1/report | 一つの事例版の確認資料をJSONで返す |
| GET /api/v1/report-html | 読み出した事例版の単独HTMLを返す |
| POST /api/v1/report-html | expected_revisionをプレビューの事例版と照合してHTMLを返す。更新されていれば409 |

すべて既存のv1 APIのトークンを使う。確認資料を認証なしの旧/api/report経路からは返さない。変更操作には従来どおりexpected_revision・actor・reasonが必要。医療モードを選んでも権限や実行方式は変わらない。

詳細と出力に含めない情報は[確認モードと参照アプリ](modes-and-queue.md)を参照する。説明の詳しさの変更はブラウザだけで行い、APIの判定結果は変更しない。


## 図の配置との照合（#161）

GET /api/v1/architectureのlayoutsに、保存した構成モデルのmodel_id・model_versionと表示に使えるlayout_model_idを返す。図に描く構成と環境が一致しないモデルには対応を返さない。コードのコミットとモデルの版、根拠の参照はmodelsとlinksで保持する。配置の一致だけでは観測済みとしない。

### 読取記録の参照

`read`または`open`にGitHub URLとreferenceを渡すと、サーバーが取得したコミットと本文を保存する。`analyze`・`overview`・`architecture`・`inquire`・`function`・`file`・`plan`はrepositoryと40桁のcommitで保存済みの記録を参照する。snapshotやfilesを入力すると400を返す。ライブラリのReaderRequestによるオフライン解析はHTTPには公開しない。

```json
{"operation":"analyze","repository":"owner/repository","commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","path":"src/main.rs","line":1}
```

画面版のトークンはHTMLにも含まれ、同じ端末内の別プロセス・利用者を認証するものではない。API専用モードはHTMLを配信せず、所有者だけが読めるトークンファイルを使う。

### 長い処理

読取・AI・環境準備のPOSTに`Prefer: respond-async`を指定すると202と`kind: request`のジョブを返す。`GET /api/v1/jobs/{id}`で待機中・実行中・完了と結果のHTTPステータスを取得する。画面はこの経路を使う。ジョブは最大12件を受け付け、結果を含む履歴は最大16件をメモリーに保持する。再起動で失われる。

`DELETE /api/v1/jobs/{id}`は待機中の処理を取り消す。実行開始後は結果の待受を中止するが、送信済みのAI処理や保存の完了までは取り消せない。同期クライアントはPreferなしで従来どおり結果を受け取れる。

### 事例の保存形式

事例は`workspace.sqlite3`へ保存する。WAL・FULL同期・トランザクションを使い、変更しない項目の本文をSHA-256で共用する。最新版は索引で参照し、版ディレクトリの全走査は行わない。書込みロックはSQLiteが管理し、プロセス終了後の手動ロック削除を不要にする。

旧`revisions/*.json`は最初の読込みで全版を一つのトランザクションに取り込む。元ファイルは残すが、移行後はSQLiteを正とし、旧バイナリで同じ保存先を更新しない。従来の全版JSONバックアップと復元の形式は維持する。通常保存の上限は64MiB、互換バックアップは1000版・32MiBの上限がある。大きい保存先を複製する場合はサーバーを停止して事例ディレクトリ全体を複製する。
