# OpenEMRの変更前後を、手動で試用する環境

既存の医療OSSから始める方針に沿い、既に固定版の自動試験があるOpenEMRを最初の接続対象にする。OHIF・MONAIへの適用目標も維持する。OpenEMRに「保存完了が分からない」という問題があると確認したものではなく、改善内容は実際のコード・操作と相談を踏まえて決める。

`tools/openemr-trial.py`は開発者が端末で使う準備用ツール。ブラウザのAPIから任意のコンテナを起動する機能は追加しない。試用と感想の記録は[利用者の試用と記録](web-user-trial.md)を参照する。

## 環境を準備する

Docker Engine 28以降とDocker Compose v2が必要。Docker公式が説明する[旧版のlocalhost公開ポートへの外部到達](https://docs.docker.com/engine/network/port-publishing/#setting-the-default-bind-address-for-containers)を避けるため、起動時にEngineの版を確認する。既存のレシピが指定する`linux/amd64`のイメージを使う。Dockerが使えなければ停止し、ホストでの直接実行へ切り替えない。

変更前のレシピは`examples/openemr-browser/soap-recipe.json`。参照するソースはコミット`d7ecc612e166c6f9f3adc9f50bc9f8cc68cac701`、OpenEMRとMariaDBはイメージのダイジェストで固定している。

変更後には、レビューしたOpenEMRの変更を含むイメージを用意し、レシピの`source`とOpenEMRサービスの`image`を更新する。A・Bには同じリポジトリの異なるコミットを指定する。フォークを使う場合は、変更前のコミットも同じフォークで参照し、`--before-recipe`を指定する。タグだけのイメージ、未固定のコミットは受け付けない。ソース参照とイメージ内のコードの一致は、この準備ツールだけでは証明しない。ビルドの手順と使用したコミット、得られたダイジェストを別途記録する。

```bash
python3 tools/openemr-trial.py prepare data/openemr-trial-001 \
  --after-recipe /path/to/reviewed-after-recipe.json
python3 tools/openemr-trial.py up data/openemr-trial-001
python3 tools/openemr-trial.py status data/openemr-trial-001
```

`prepare`は未作成のディレクトリだけを受け付け、次を保存する。

| ファイル | 内容 |
| --- | --- |
| `compose.json` | A・BのOpenEMRとDB、ネットワーク、試用専用の認証設定 |
| `credentials.json` | 試用専用のユーザー名とパスワード |
| `trial.json` | 各版のソース参照・イメージ・レシピのハッシュ、起動状態・URL |

ファイルは0600で作成する。認証情報を含むディレクトリを公開・共有しない。認証情報はコマンドの出力やURLへ含めない。

このツールは各レシピからイメージとソース参照を読み、手動試用用の構成を生成する。レシピのブラウザ自動操作・故障条件・任意の環境変数・コマンドは実行しない。レシピと同じ全条件で実行したという意味にはしない。実際の構成は`compose.json`と、そのハッシュで識別する。

A・Bには別々の空のDBと内部ネットワークを作り、DBのポートはホストへ公開しない。OpenEMRだけを`127.0.0.1`の空きポートへ公開する。ホストのディレクトリをコンテナへマウントせず、既存のDBや認証情報を引き継がない。コンテナによる分離と、この設定の生成だけで、本番への影響が一切ないと証明したことにはならない。実際の分離の確認は#59で継続する。

`up`が返っても、OpenEMRの初期設定が終わっていない場合がある。`status`で両方の`ready`が`true`になることを確認する。準備できない場合は、試用を開始せず、起動失敗と記録する。

## 利用者に試してもらう

1. 試用専用のブラウザプロファイルを用意する。`trial.json`のA・BのURLを開き、自己署名証明書の例外はこのローカル試用先に限定する。
2. `credentials.json`の試用専用アカウントでログインする。実際の患者情報・病院の認証情報を入力しない。
3. A・Bに同じ架空の患者・入力内容を用意する。利用者がどの画面で何をするかを説明し、初期データと操作条件を試用計画へ記録する。
4. `/user-trial`で計画を作る。`trial.json`から各版のリポジトリ・コミット・パス・URLを転記し、技術者の検証結果があれば証拠IDも結びつける。試用前にURLの接続先と準備状態を確認する。
5. 利用者がA・Bを操作し、実際に試した順番と操作、保存完了を判断できたか、迷った点や追加の負担を記録する。技術者が計測した所要時間や保存結果と、人の感想を区別する。

Aは`127.0.0.1`、Bは`localhost`を使い、Cookieのホスト名を分ける。ポートが違うだけではCookieを分離できないため。ブラウザやアプリが同じホスト名へ転送する場合は、A・Bを別々のブラウザプロファイルで開く。`localhost`がこの端末のループバックへ解決されることを確認する。自動で開くリンクからはブラウザプロファイルを選べない。

この手順は開発者の端末で一緒に試す使い方を想定する。病院のネットワークへの接続、外部公開、遠隔の協力者へのURL配布は扱わない。利用者の協力が決まるまで、環境の準備と記録機能の開発を進められる。自動試験や開発者の確認を、実利用者による試用と記録しない。

## 終了と再試用

```bash
python3 tools/openemr-trial.py down data/openemr-trial-001
```

この試用プロジェクトのコンテナ・ネットワーク・DBボリュームを削除し、ローカルの記録ファイルは残す。停止前に必要な検証結果と感想を麦漆へ記録する。DBの内容は削除される。共通の一括削除コマンドは使用しない。

起動の途中で失敗・中断した場合も同じ`down`を実行する。削除に失敗したときはエラーを解決して再実行する。新しい試用では別のディレクトリを準備し、初期データをもう一度そろえる。使用済みの環境で`up`を繰り返して、前の操作の影響がない試用と扱わない。

## 起動手順だけを確認する

変更したイメージが未準備でも、同一版を2環境に起動する接続確認はできる。

```bash
python3 tools/openemr-trial.py prepare data/openemr-setup-check \
  --after-recipe examples/openemr-browser/soap-recipe.json --allow-identical
python3 tools/openemr-trial.py up data/openemr-setup-check
python3 tools/openemr-trial.py down data/openemr-setup-check
```

これは改善案の比較ではない。同一版の比較を改善の試用計画として登録せず、起動手順の確認と記録する。

検証用のコマンドは次のとおり。

```bash
python3 tools/openemr-trial-tests.py
# 実Dockerで2環境の準備・ログイン画面・後片付けまで確認する
python3 tools/openemr-trial-smoke.py
```

後者は同一の固定版を使う。`evaluations/openemr-manual-trial/ready.json`に起動時の参照を残し、`trial.json`に削除後の状態を残す。人の試用や、改善の効果を確認する試験ではない。

構成の根拠はDocker公式の[ポート設定](https://docs.docker.com/reference/compose-file/services/#ports)、[内部ネットワーク](https://docs.docker.com/reference/compose-file/networks/#internal)、[Composeの状態出力](https://docs.docker.com/reference/cli/docker/compose/ps/)。OpenEMRの操作の根拠と既存の自動試験は[固定版レシピ](../examples/openemr-browser/README.md)に記載する。
