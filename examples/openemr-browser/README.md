# OpenEMRの患者と受診理由を試験する

`recipe.json`は、架空の患者を登録し、受診理由を保存して再表示するブラウザ試験。OpenEMRの固定コミット `d7ecc612e166c6f9f3adc9f50bc9f8cc68cac701` のproduction composeを参照し、記載されたOpenEMRとMariaDBのイメージをダイジェストで固定する。通常時と、患者登録と受診理由の入力後、保存する直前にDBを止める条件を試す。

```sh
npm ci --ignore-scripts
npx playwright install --with-deps chromium
# 管理者だけが読める試験用ファイルを作る。値はこのRecipe専用の架空のアカウント。
umask 077
printf '%s' '{"username":"eel-operator","password":"eel-pass-trial"}' > /tmp/eel-openemr-secrets.json
EEL_BROWSER_SECRETS_FILE=/tmp/eel-openemr-secrets.json cargo test --locked --test containers openemr_browser -- --ignored --nocapture
```

Dockerのエンジンが必要。試行ごとに新しいDBとコンテナを作り、終了後に消す。既存のOpenEMRや実患者のデータには接続しない。認証情報は公開した架空の値なので、実環境には流用しない。

画面の選択は、固定コミットのログインテンプレート、メニュー、患者登録画面、受診フォームを読んで定義した。初回の製品登録案内では「Ask again later」を選び、登録しない。同じ名前の非表示メニューを避け、開いている患者メニューを選ぶ。再表示する受診記録が折り畳まれていれば`expand`で開く。受診区分は試験用にOffice Visitを選ぶ。架空の識別値は名前の自動大文字化で変わらない大文字の接頭辞を使う。ラベルがない箇所はCSSを使う。`read_text.contains`は指定文字列が表示されるまで待つ条件であり、保存先を検査するものではない。OpenEMRはiframeの中へ画面を読み込むため、親から順にframeを指定する。

| 条件 | 調べる根拠 |
| --- | --- |
| 患者の表示 | 患者パネルに試行ごとの架空の値が表示されたか |
| 患者登録の応答 | 登録操作のPOST応答がHTTP 200か |
| 患者の保存 | `patient_data.lname`が試験値に一致する行をDBから読めるか |
| 受診理由の応答 | 保存操作のPOST応答がHTTP 200か |
| 受診理由の保存 | `form_encounter.reason`が試験値に一致する行をDBから読めるか |
| 受診理由の再表示 | 受診タブを更新しサーバーから読み直して、試験値を表示できるか |

DBを止めた場合は、DBから行を読めないことを「行がない」に置き換えない。HTTP 200のエラーページが返る可能性もあるため、HTTPだけで保存を成功と判定しない。観測記録と写真は`evaluations/ci-openemr/`に残す。

元の`recipe.json`で扱う診療記録は受診理由だけ。SOAPは下の別レシピで確認する。署名、各職種の閲覧権限、長期保存、実環境の構成との一致は別の確認が必要。

## SOAPの4項目を試す

`soap-recipe.json`は上の患者・受診を作った後、Subjective、Objective、Assessment、Planへ別々の試験値を入力する。SOAPの保存POST、受診タブを更新した後の4項目、`form_soap`の4列を別条件で観測する。DBの`pid`を患者の`pid`とも照合する。受診との外部キーの照合や署名は含めない。

ログイン、画面操作、画面写真の取得を含む試験全体の待機上限は300秒。手元の実行では150秒の上限に達し、SOAPの保存とDB照合まで進んだが、再読込後の表示は取得できなかった。待機上限を延ばしても、保存・再表示・DB照合の判定条件は変えない。上限へ達した場合は、取得できなかった観測を未確認として残す。

```sh
EEL_BROWSER_SECRETS_FILE=/tmp/eel-openemr-secrets.json node tools/openemr-soap-smoke.mjs
```

通常時、SOAP入力後・保存直前のDB停止、新しいDBとコンテナでの再試験を同じ事例に残す。DB停止時も保存操作を実行する。保存先の読取不能と、再表示へ進めなかった未観測を区別する。既存DBの修復や同じDBでの復旧を試した意味ではない。結果は`evaluations/ci-openemr-soap/`に残す。

選択の根拠は固定版OpenEMRの`templates/encounter/forms/navbar.html.twig`、`interface/forms/soap/templates/soap_form.twig`、`interface/forms/soap/report.php`、`interface/forms/soap/table.sql`。画面操作はSOAPのフォームを使い、DBへの書込みで代用しない。ローカルにDockerがない場合、実際の保存・再表示と故障試験は専用のGitHub Actionsで検証する。

実行後に`node tools/target-architecture-smoke.mjs openemr-soap`で、同じ事例のSOAP観測をOpenEMRの構成図へ対応づける。構成図の元の根拠は先行した患者・受診理由の試験で、DB・アプリ・ブラウザーの固定した構成を共用する。SOAPの操作の根拠は各試行のレシピで確認する。図・合成の疑問・SOAPの観測を往復し、DB停止時の未観測を正常時の成功で補わない。

## DB停止の後に復旧して読み直す

`tools/openemr-soap-smoke.mjs`は、DB停止の試行に`trial.recovery`を付けた`database-stopped-recovered`も実行する。DBを起動し直し、停止中に読めなかった患者・受診・SOAPの行を読み直す。患者と受診は1行、SOAPは停止中に保存操作が失敗しているため0行であることを確認する。同じDBで復旧した結果の読み直しであり、保存操作の再実行や実運用の復旧手順は含まない。2026年10月26日（日本時間）に[CI run 40213801704](https://github.com/Suisan-neki/mugiurushi/actions/runs/40213801704)でこのケースを実行し、患者・受診各1行、SOAPの各項目と識別用の行は0行、両サービスの準備、後片付け後の残骸0件を確認した。通常時・停止時・新しい環境での再試行と、画面からの試験実行も成功した。試験用の合成認証ファイルを使用し、人による評価や実運用の復旧試験ではない。
