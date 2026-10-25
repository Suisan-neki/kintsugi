# ブラウザ操作の試験

`runtime: "browser"` のRecipeで、試験用コンテナの画面をChromiumから操作する。既存のAPI・CLIのRecipe読込を使える。`npm ci --ignore-scripts` と `npx playwright install --with-deps chromium` がサーバー側に必要。Playwrightはpackage-lockで固定し、`browser.expected_version` と実際のブラウザの版が一致しなければ操作を始めない。版は `node_modules/playwright-core/browsers.json` に記載される。

ChromiumはPlaywrightに固定された通常版をheadlessモードで起動し、OSのサンドボックスを有効にする。起動できない環境では試験を未完了として記録し、`--no-sandbox`へ切り替えない。UbuntuでAppArmorがuser namespaceを制限する場合、CIは導入済みChromeのsandbox helperを使うか、固定したChromiumの実行ファイルの正確なパスだけをAppArmorの例外にする。OS全体のuser namespace制限は解除しない。ローカル環境でも[Chromium公式の手順](https://chromium.googlesource.com/chromium/src/+/main/docs/security/apparmor-userns-restrictions.md)に従ってサンドボックスを用意する。Nodeの制御処理自体はホストで動き、ブラウザのサンドボックスとは別の範囲になる。

流れは、コンテナの準備 → `browser.preparation`（ログインなど）→ 故障の注入 → `browser.actions`（保存など）→ DBの直接読取 → コンテナ停止・掃除。各操作はラベル・役割、またはCSSで対象を指定する。iframeは`target.frames`のCSSセレクターで親から順に選ぶ。任意のJavaScriptを設定から実行する機能はない。

```json
{
  "service": "web",
  "tls": false,
  "start_path": "/",
  "expected_version": "151.0.7922.34",
  "preparation": [],
  "actions": [
    {"id": "message", "action": "read_text", "target": {"locator": {"by": "role", "role": "heading", "name": "Welcome to nginx!"}}}
  ],
  "responses": [],
  "database": []
}
```

これはRecipeの`browser`フィールド。完全な例は`tests/containers.rs`の`browser_observes_nginx_and_preserves_screenshots`にある。`fill`の値は`{"source":"literal","value":"{{test_value}}"}`、または`{"source":"secret","key":"password"}`。秘密の値をRecipeに書かず、サーバーの`EEL_BROWSER_SECRETS_FILE`で渡すJSONから読む。例：`{"password":"試験専用の値"}`。ファイルはサーバー管理者だけが読める権限で置く。ページに投入する認証情報は架空の試験用に限る。

| 判定方法 | `target` | 観測の範囲 |
| --- | --- | --- |
| `browser_text` | `read_text`の手順ID | 完了した試験操作で読んだ表示。保存の証明とは別 |
| `browser_response` | `responses`のID | 試験操作中の要求に対するHTTP応答。`status=200`、`body_match=true`または`body_match=false`を照合 |
| `database_rows` | `database`のID | MariaDBから試験値に一致する行を直接読んだ結果 |

`responses`にはID、同じ試験用オリジン内の`path`、`method`、任意の`contains`を指定する。本文は保存せず、指定文字列が含まれるかだけを記録する。Cookie、ヘッダー、クエリーは記録しない。準備時の応答は判定に使わない。200件の記録上限に達した場合は、記録できた応答だけで成功としない。同じ観測先で条件を満たす応答と満たさない応答が混じった場合は未観測扱いとする。本文が取れなかったことを「含まれない」の成功にはしない。

`database`にはID、DB役の`service`、`database`、`table`、`match_column`、`columns`を指定する。識別子は英数字と下線だけで、検索値は基盤が作る試行ごとの架空の値。任意のSQLは受け取らない。正常終了した読取で0行なら、その値に一致する行がないことを記録できる。DB停止・認証失敗・出力超過の場合は不在とは判定しない。MariaDB以外のDB、複数テーブルの結合、実際の業務上の閲覧権限は未対応。

記録は`execution.json`内の`containers.browser`と`containers.database_reads`、画面写真は試行の`browser/`へ置く。途中失敗も完了した手順と停止した手順を残す。認証入力直後の画面写真を省き、その後も入力欄をマスクする。エラーや文字列では秘密の値を伏せる。ただし、対象アプリが別の場所へ秘密を反映する画面まで自動で隠す保証はない。結果画面から開く方法は後述する。

手順が未完了となった場合は、その直後の画面写真を一度だけ、最大1秒で取得する。操作の待機時間を使い切っていても撮影を試みるが、操作の再試行はしない。写真の取得に失敗した場合は理由を手順の記録へ残す。秘密入力の手順では、失敗後も写真を省く。停止後の写真には撮影時点を説明し、写真の有無で操作の完了・保存・条件の判定を変えない。

ブラウザの通信先は、その試行のコンテナの公開ポートに限定する。Service WorkerとWebSocketは使わない。コンテナ自身の外部通信を隔離する機能ではない。ページやコンテナは試験用の信頼できる対象に限る。

ブラウザ実行では、Recipeの`source`に指定したファイルだけを固定コミットから複製して、構成を参照した根拠にする。実際に起動するコンテナのコードとそのGitコミットが一致することは、この参照だけでは証明しない。イメージの固定は別のダイジェストとして残す。

#140のうち、操作・三種類の観測・途中失敗の記録を扱う最初の基盤。OpenEMRの患者登録・診療記録、OHIFの一覧・画像表示を実行確認したという意味ではない。

## 結果画面で観測と画面写真を開く

事例の「ブラウザの観測と画面写真」を開くと、準備・試験操作の完了状況、表示の文字列、HTTP応答、DBの直接読取を別の欄で確認できる。各手順の「画面写真を開く」で、撮影時の写真をダイアログに表示する。写真がなくても、元の判定と観測記録を書き換えない。

写真の読取には`GET /api/v1/browser-screenshot/{evidence_id}/{step_id}`と`X-Review-Token`が必要。基盤は事例に登録した試行と手順からファイルを決め、任意のパスは受け付けない。事例内の`runs/{run_id}/browser/`にある宣言済みのPNGだけを読み、symlink・PNGヘッダーがないファイル・4MiB超の画像を拒否する。

現在のバックアップは観測記録のJSONを保存し、`runs/`を含めない。事例の外から取り込んだ試行やJSONだけを復元した事例では、画面写真の元ファイルを本APIでは開けない。その場合は画面に不足を表示する。写真を含めた持ち出し形式は別の実装範囲。
