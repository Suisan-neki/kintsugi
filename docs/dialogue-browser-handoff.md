# 対話の追加読取とブラウザ試験

## 質問に応じたコードの読取

構成図の対話を送ると、既存の質問用読取を実行する。質問に含まれる名前と一致するパスを優先し、固定コミットから最大8ファイルを補う。PHP、Twig、SQLは本文を読めるが、既存のRust・Python・JavaScript用の構文解析へ対応したとは扱わない。

以前に保存した読取記録でも、ファイル一覧から追加候補を選べる。`SOAP`を含む質問では、`interface/forms/soap/`の保存・表示・テンプレートなどが候補になる。名前を含まない曖昧な質問で、必ず目的の処理へたどれる保証はない。

質問の根拠を構成図の根拠より先に、合計48KiBまで行番号付きでAIへ渡す。本文を読んだことと、その全行をAIへ渡したことは別である。AIの主張は今回渡した行と照合し、範囲外の参照を含む返答は保存しない。参照行の存在は説明の正しさや実行時の挙動の証明ではない。

対話の各往復に、追加で読んだファイル、取得に失敗したファイル、未読候補、文脈の指紋を保存する。画面でも追加読取と取得失敗を表示する。既存の会話形式は引き続き読める。

Difyが以前の`system_context`を使い続けないよう、渡す図とコードが変わった場合はリモートの会話を新しくする。ローカルの対話ID、直近6往復、前の整理案、同じ対話に属する検証結果は引き継ぐ。図とコードが同じ場合は同じリモート会話を使う。

## 画面から選ぶSOAP試験

`/workbench`でOpenEMRの確認項目を選ぶと、同梱のブラウザ試験を選べる。`GET /api/v1/browser-recipes`は既存と同じ操作トークンの検査を使い、選択候補を返す。取得・実行はしない。

- 通常時のSOAP保存・再表示・DB照合。
- SOAP入力後、保存直前にDBを停止してから保存を試す条件。

どちらも`examples/openemr-browser/soap-recipe.json`から作る。DB停止では入力までを準備へ移し、保存操作は試験中に残す。先に保存した患者・受診理由の条件は外し、DB停止後のSOAPの条件を観測する。

確認項目とレシピのコミットが異なる場合は両方を表示し、人が「レシピの版に切り替えて設定する」を押して参照先を更新する。元の相談の出所と過去の根拠は残る。レシピと異なる版で、そのレシピを流用して実行しない。

起動・入力、試す状況、判定条件、実行前の確認を順に表示する。確認画面まで進んでも実行しない。「試験を始める」で既存の準備・実行APIを呼び、確認項目のIDと観測条件を対応づける。準備・操作・HTTP応答・DBの観測先・入力・判定条件のIDを保持し、過去の結果から戻る場合もブラウザ設定を復元する。

## 実行環境と確認範囲

Docker、固定版のPlaywrightとChromium、サーバーの`EEL_BROWSER_SECRETS_FILE`で指定する試験専用の認証情報が必要。認証値は[既存のOpenEMR試験手順](../examples/openemr-browser/README.md)と同じ。画面に実アカウントの入力欄は追加しない。

```sh
npm run test:browser-handoff
npm run test:browser-handoff -- --live
EEL_DIALOGUE_API_ONLY=1 npm run test:dialogue
EEL_DIALOGUE_API_ONLY=1 EEL_DIALOGUE_NATIVE=1 npm run test:dialogue
```

通常の画面試験は合成の読取記録を使い、実行要求を送る直前で止める。`--live`は固定コミットのComposeを読み、画面から実OpenEMRを起動してSOAPの保存・再表示・DB照合を行う。参照したGitコミットと実行イメージのコードが同一であること、実利用者の理解・改善効果は、この接続確認では証明しない。

読む順番は`src/reader/inquiry.rs`、`src/system_diagram/dialogue.rs`、`src/browser_recipes.rs`、`ui/workbench.js`、`tools/browser-handoff-smoke.mjs`。
