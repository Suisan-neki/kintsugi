# 開発・実行の手引き

この文書は開発者向けの起動手順、実行上の制約、コード構成と確認方法をまとめたものです。プロジェクトの背景と概要は[README](../README.md)を参照してください。

## 起動

対応OSはLinux・macOSです。Unix固有のプロセス・ファイル操作を使うため、Windowsではビルドできません。Rust 1.99.0を使います。開発者・運営者が最初にAI接続を設定します。

```bash
git clone https://github.com/Suisan-neki/mugiurushi.git
cd mugiurushi
cargo run -- ai init
# 作成された data/app/ai.json をエディタで開き、api_key と model を記入する
cargo run -- ai test
cargo run
```

表示されたlocalhostのURLを開きます。既定の保存先は`data/app`、ポートは8765です。設定ファイルはGitの管理対象外で、既存の設定は上書きしません。利用者向け画面にはキーの入力欄を設けません。

```bash
cargo run -- app data/app --port 8765 --execution-mode local
cargo run -- --help
```

`local`で起動するNode.js・Pythonとブラウザ操作器はホスト上で動き、OSによる隔離を行いません。コンテナの対象は、既定で外部へ出られない内部ネットワークで動かし、権限を絞ります（下記）。ホスト側の操作器まで隔離する方式ではありません。Dockerの操作権限は強い権限を持つため、専用の試験環境で使います。信頼できる対象を使い、実行内容を確認してから試験を開始します。Linuxの`bubblewrap`を指定した場合、隔離を起動できなくてもlocalへ切り替えません。

コンテナの設定の`egress`は、既定の`blocked`で試験用ネットワークを`--internal`にし、対象から外部へ通信できなくします。ホストの127.0.0.1と、対象どうしの接続は、基盤が対象ごとに起動する中継コンテナ（固定イメージのnginx）を通ります。`egress: "allowed"`を明示した場合だけ外部へ通信でき、実行記録に`allowed`と残ります。対象のコンテナは`--cap-drop ALL`に最小限の権限だけ戻し、`no-new-privileges`とプロセス数の上限を付けます。中継は接続ごとに接続元・接続先の対象、成否、バイト数を記録し、実行記録の`containers.traffic`へ集計します。`failed`は中継の状態が200以外だった接続で、クライアントの切断や中継停止も含み、アプリの処理の失敗件数ではありません。閉じなかった接続は`unfinished`の件数だけで、成否とバイト数は分かりません。通信の中身（SQLや本文）、中継を通らない通信、`egress: "allowed"`の試行は記録しません。試験案の`recovery`（`ready_timeout_ms`）を指定すると、DBの停止・切断・読み取り専用の故障を解除し、各コンテナの準備と、故障中に行ったDB行・HTTPの読み出しを復旧後にもう一度行って、実行記録の`trial.recovery`へ残します。故障中に読めなかった行は「復旧後に読めた」と記録し、「行がなかった」とは扱いません。採用の判定には使いません。故障中に失われた操作の再実行と、実運用の復旧手順の妥当性は確認しません。この遮断は対象のコンテナからの通信が対象で、基盤・ブラウザ操作器・中継コンテナのホスト側には及びません。

Cargoパッケージ名とバイナリ名は`mugiurushi`です。旧名称はexecution-evidence-lab、kintsugiです。

証拠不足の判定はUnknownとして残し、画面・API・確認資料で検証未通過と示します。確認事項が空の場合や未確認・未達の条件がある場合は、「採用する」という判断の記録をサーバー側で拒否します。記録した全条件の通過も、実運用の許可や医療安全性の保証を意味しません。

## 構成

| 場所 | 役割 |
| --- | --- |
| `src/main.rs` | CLIの呼び出しと終了時のエラー表示 |
| `src/cli/` | アプリ・AI設定・ケース管理・固定環境のCLI |
| `src/review.rs` | 現在の画面と操作トークン付きAPI |
| `src/reader/`・`src/understanding.rs` | 固定版の読取・静的解析・読取記録 |
| `src/system_diagram/` | 構成図、部分ごとの会話、期待と結果への対応づけ |
| `src/execution.rs`・`src/trial.rs` | 固定版の実行、故障条件、観測と判定 |
| `src/recording.rs`・`src/run_record.rs` | 出来事・試行情報・完了記録の保存 |
| `src/user_trial.rs`・`ui/web-trial.*` | 過去の試作のWeb試用記録（保存済みデータを継続して扱う） |
| `src/workspace.rs`・`src/comparison.rs` | 条件、根拠、人の判断、履歴と試行比較 |
| `src/backup.rs`・`src/review_report.rs` | 全版のバックアップ・復元と確認資料 |
| `ui/app.*`・`ui/repository-diagram.js` | 構成の調査と検証課題の画面 |
| `ui/workbench.*` | コード確認、検証設定、実行、結果と比較 |

ケース操作とAPI起動は`--help`を参照してください。`case init`は条件や回答を含まない空のケースを作ります。過去の試作の廃止と保存済みデータの扱いは[整理と確認範囲](current-workflow-cleanup.md)に記録しています。

## 確認

```bash
cargo fmt --check
cargo check --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
python3 tools/check-yomiyasu.py
npm ci --ignore-scripts
npx playwright install --with-deps chromium
npm run test:ui
npm run test:reader-native
npm run test:app
```

`test:ui`は構成図と対話を実Rustサーバー・合成AIで操作します。`test:app`は公開リポジトリsjl/tを使って、検証・故障・改修・再試験・比較を操作します。合成AIでの成功は実モデルの品質や利用者の理解を証明しません。

ブラウザを使わずに対話のAPIを確認する場合は、`EEL_DIALOGUE_API_ONLY=1 npm run test:dialogue`を使います。現在の実行経路については`tools/queue-demo.mjs`と`tools/pinned-demo.mjs`でも検証できます。ここでの番号表示は実際にコードを起動する参照アプリであり、廃止した名前・年齢デモとは別です。

## 操作と研究の記録

- [現在の対象・目的・評価方針](developer-verification.md)

- [構成図とAI接続](repository-architecture.md)
- [対話と検証への接続](diagram-dialogue.md)
- [対話の追加読取とブラウザ試験](dialogue-browser-handoff.md)
- [図の読みやすさとyomiyasu](readable-diagram-and-writing.md)
- [固定版実行と制約](pinned-execution.md)
- [医療OSSの固定環境](pinned-environments.md)
- [今回の整理と確認範囲](current-workflow-cleanup.md)

既存の`/user-trial`と保存済みの試用記録は引き続き読めますが、技術的な検証の主導線からは外しています。

`docs/`には過去の試作・研究経緯も残しています。廃止したコマンドや画面を含む過去の記述は、現在の操作手順ではありません。

## 検証の実行

PRではRustの検査と、合成データ・固定フィクスチャによる試験を実行します。公開GitHub、Docker、外部OSSに依存する試験は手動・週次実行のワークフローに分けています。OpenEMR・OHIF・MONAIの試験は検証例であり、PRごとの必須条件にはしません。

簡易解析は正規表現で宣言や参照を抽出します。テンプレート文字列、複数行の式、マクロ、動的な参照などの意味を正確に解析するものではありません。抽出した行の存在、解析の推定、AIの説明、実行で得た観測を区別します。

画面版の操作トークンはHTMLにも含まれ、別サイトからの操作を防ぐために使います。同じ端末の別プロセスや利用者を認証する仕組みではありません。API専用モードはトークン付きHTMLを配信せず、所有者だけが読めるトークンファイルで操作します。

開発ビルドはUIをチェックアウト内のファイルから配信するため、JS・CSSの変更にRustの再ビルドは不要です。配布時は`cargo build --release --locked`を使い、UIを実行ファイルへ埋め込みます。試行の`source_fingerprint`は起動した実行ファイルのSHA-256です。ソースの署名ではありません。ソースのコミットも記録する場合は、ビルド時に`MUGIURUSHI_SOURCE_REVISION`へ40桁のSHAを設定します。未設定なら`unrecorded`と記録します。

HTTP受付、処理ジョブ、UI資材、事例の永続化はそれぞれ`src/review/`・`src/workspace/storage.rs`へ分けています。UIのHTTP操作と画面部品も別モジュールにしています。`npm run lint`はUI全体、`npm run typecheck`は共通HTTPモジュールを検査します。従来の画面コード全体の型移行は完了していません。
