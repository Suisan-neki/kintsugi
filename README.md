# 麦漆（mugiurushi）

伝統的な手工芸品のように、手入れを重ねて使い続けられるソフトウェアへ。

麦漆は、ソフトウェアを使う人と作る人が理解・検証・改修に関わり続けられるエコシステムを提案する研究プロジェクトです。現在はそのための基盤を開発しています。

## なぜ麦漆なのか

麦漆は金継ぎで接着に使う材料です。使う人と作る人、疑問と検証をつなぐ基盤でありたいという考えを名前に重ねています。

生成AIによってソフトウェアを作りやすくなる一方で、仕組みや挙動を理解しながら利用・運用することには難しさがあります。使う人が感じた不便や不安を具体的に伝えられず、作る人も何を確かめればよいか分からないことがあります。

金継ぎに重ねるのは、直した経緯を残しながら手入れを続けるという考え方です。誰が何に困ったのか。なぜ変更し、どこまで確かめたのか。その記録を次の検証や改修に引き継ぎながら使い続けることを目指します。

## 理解・検証と、利用者による改善の確認

技術者は、構成図やシステムへの質問を使って仕組みを調べ、実行中の挙動と検証結果を確認します。利用者は、困っている操作を伝え、変更前後のWebアプリを試して意見を返します。利用者が構成図やコードを読めることを必須にしません。

相談、対象の版、実行条件、検証結果、試用後の感想を対応づけて残し、次の改修と再検証に使うことを目指します。適用対象はOpenEMR・OHIF Viewer・MONAIの3件を維持し、Web試用はOpenEMRから接続します。小さな自作アプリによる確認だけで適用を完了したとは扱いません。

現在の基盤では、公開GitHubのコードの読取から次の操作を行えます。

1. 技術者／非技術者を選びます。説明の詳しさを切り替えます。
2. 公開GitHubのURLを入力し、読み取るコミットを固定します。
3. そのシステムの構成図を見て、要素や接続の説明と根拠を調べます。
4. 気になる部分についてAIと相談し、疑問・期待・根拠・未確認事項を記録します。
5. 人が期待する動作を編集し、`/workbench`の検証設定へ進みます。
6. 試行の結果、実際の観測、未確認事項を確認し、改修後の再試験や比較へ進みます。構成図と会話へ戻ることもできます。

図を生成するときはREADMEとコードの抜粋を設定したAIへ送ります。URLを読んだだけでは対象コードを実行しません。AIの説明、対象プログラムによる報告、基盤が直接読み出した結果を区別します。終了コード0だけで業務条件の成立を判定しません。

構成図はGitDiagramの方式を参考に生成し、同じモデルから立場別の説明を表示します。文章の生成には固定版yomiyasuの規則を組み込み、画面にはデジタル庁デザインシステムを使います。Difyを設定した場合はChatflow、未設定なら既存のAI接続で対話します。

現在の実行方式はNode.js/Pythonの固定版、固定した依存先、複数プロセス、固定コンテナとブラウザの宣言的操作に対応します。任意リポジトリの自動準備・自動計測、実参加者の理解評価、医療機関での有効性は未確認です。

## Webアプリの変更前後を試す

`/user-trial`で、開発者が相談・操作・条件とA/Bの固定版、試用URL、検証の根拠IDを登録します。利用者は構成図を経由せず、この画面から試用を始められます。A→BまたはB→Aの順に対象を開き、実際に行った操作と感想を保存します。

試用環境は開発者が別途起動します。登録したURLや版は開発者の申告であり、そのURLで申告どおりのコードが動いていることを麦漆が証明する機能ではありません。利用者の感想は自動試験の通過や採用判断を変更しません。実利用者による評価は未実施です。

詳しい操作と記録の範囲は[Web試用の手順](docs/web-user-trial.md)を参照してください。

## 起動

Rust 1.99.0を使います。開発者・運営者が最初にAI接続を設定します。

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
cargo run -- app data/app --port 8765 --execution-mode bubblewrap
cargo run -- --help
```

Web経由の実行はLinuxの`bubblewrap`を必須とします。隔離を起動できない場合は実行を完了せず、localへ切り替えません。ブラウザからの実行要求には端末での一回限りの承認も必要です。

画面に表示された「実行要求を保存する」からJSONを保存します。端末でコードの固定版・起動内容・引数・入力・読み出し・依存先を確認してから承認し、同じ画面で再試行してください。

```bash
cargo run -- case approve data/app/case /path/to/execution-request.json
# 使用前の承認を取り消す場合
cargo run -- case revoke data/app/case
```

承認は事例の保存先・版・実行レシピ・記入者・理由と一致する一回の要求だけに使えます。条件を変えた場合は承認し直します。承認ファイルはAPIやバックアップから読み出せません。実行受付後に失敗した場合も承認は再利用できません。

`local`は開発ビルドの端末実行だけに残します。Web経由では拒否し、本番ビルドでは共通の実行入口で拒否します。現在のbubblewrap方式ではコンテナと待受プロセスを扱えないため、これらの試行は開発ビルドのCLIでのみ行えます。macOS等では構成図・対話・記録を利用できますが、Webからの対象コード実行はできません。

Cargoパッケージ名とバイナリ名は`mugiurushi`です。旧名称はexecution-evidence-lab、kintsugiです。

証拠不足の判定はUnknownとして残し、画面・API・確認資料で検証未通過と示します。確認事項が空の場合や未確認・未達の条件がある場合は、「採用する」という判断の記録をサーバー側で拒否します。記録した全条件の通過も、実運用の許可や医療安全性の保証を意味しません。

## 構成

| 場所 | 役割 |
| --- | --- |
| `src/main.rs` | CLIの呼び出しと終了時のエラー表示 |
| `src/cli/` | アプリ・AI設定・ケース管理・固定環境のCLI |
| `src/review.rs` | 現在の画面と認証付きAPI |
| `src/reader/`・`src/understanding.rs` | 固定版の読取・静的解析・読取記録 |
| `src/system_diagram/` | 構成図、部分ごとの会話、期待と結果への対応づけ |
| `src/execution.rs`・`src/trial.rs` | 固定版の実行、故障条件、観測と判定 |
| `src/recording.rs`・`src/run_record.rs` | 出来事・試行情報・完了記録の保存 |
| `src/user_trial.rs`・`ui/web-trial.*` | Web試用の計画、操作順、版に対応した利用者の感想 |
| `src/workspace.rs`・`src/comparison.rs` | 条件、根拠、人の判断、履歴と試行比較 |
| `src/backup.rs`・`src/review_report.rs` | 全版のバックアップ・復元と確認資料 |
| `ui/app.*`・`ui/repository-diagram.js` | 構成図と対話の画面 |
| `ui/workbench.*` | コード確認、検証設定、実行、結果と比較 |

ケース操作とAPI起動は`--help`を参照してください。`case init`は条件や回答を含まない空のケースを作ります。過去の試作の廃止と保存済みデータの扱いは[整理と確認範囲](docs/current-workflow-cleanup.md)に記録しています。

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

- [構成図とAI接続](docs/repository-architecture.md)
- [対話と検証への接続](docs/diagram-dialogue.md)
- [図の読みやすさとyomiyasu](docs/readable-diagram-and-writing.md)
- [固定版実行と制約](docs/pinned-execution.md)
- [医療OSSの固定環境](docs/pinned-environments.md)
- [今回の整理と確認範囲](docs/current-workflow-cleanup.md)

`docs/`には過去の試作・研究経緯も残しています。廃止したコマンドや画面を含む過去の記述は、現在の操作手順ではありません。
