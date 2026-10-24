# OpenEMRの変更前後を、手動で試用する環境

既存の医療OSSから始める方針に沿い、既に固定版の自動試験があるOpenEMRを最初の接続対象にする。OHIF・MONAIへの適用目標も維持する。OpenEMRに「保存完了が分からない」という問題があると確認したものではなく、改善内容は実際のコード・操作と相談を踏まえて決める。

`tools/openemr-trial.py`は開発者が端末で使う準備用ツール。ブラウザのAPIから任意のコンテナを起動する機能は追加しない。試用と感想の記録は[利用者の試用と記録](web-user-trial.md)を参照する。

## 環境を準備する

Docker Engine 28以降とDocker Compose v2が必要。Docker公式が説明する[旧版のlocalhost公開ポートへの外部到達](https://docs.docker.com/engine/network/port-publishing/#setting-the-default-bind-address-for-containers)を避けるため、起動時にEngineの版を確認する。既存のレシピが指定する`linux/amd64`のイメージを使う。Dockerが使えなければ停止し、ホストでの直接実行へ切り替えない。

変更前のレシピは`examples/openemr-browser/soap-recipe.json`。参照するソースはコミット`d7ecc612e166c6f9f3adc9f50bc9f8cc68cac701`、OpenEMRとMariaDBはイメージのダイジェストで固定している。

変更後には、レビューしたOpenEMRの変更を含むイメージを用意し、レシピの`source`とOpenEMRサービスの`image`を更新する。A・Bには異なる固定コミットを指定し、変更前を上流、変更後を研究用のフォークで参照する場合も、それぞれのリポジトリを残す。タグだけのイメージ、未固定のコミットは受け付けない。ソース参照とイメージ内のコードの一致は、この準備ツールだけでは証明しない。ビルドの手順と使用したコミット、得られたダイジェストを別途記録する。

```bash
python3 tools/openemr-trial.py prepare data/openemr-trial-001 \
  --after-recipe /path/to/reviewed-after-recipe.json
python3 tools/openemr-trial.py up data/openemr-trial-001
python3 tools/openemr-trial.py status data/openemr-trial-001
```

`prepare`は未作成のディレクトリだけを受け付け、次を保存する。

| ファイル | 内容 |
| --- | --- |
| `compose.json` | A・BのOpenEMR・DB・固定転送プロキシ、ネットワーク、試用専用の認証設定 |
| `credentials.json` | 試用専用のユーザー名とパスワード |
| `trial.json` | 各版のソース参照・イメージ・レシピのハッシュ、起動状態・URL |

ファイルは0600で作成する。認証情報を含むディレクトリを公開・共有しない。認証情報はコマンドの出力やURLへ含めない。

このツールは各レシピからイメージとソース参照を読み、手動試用用の構成を生成する。レシピのブラウザ自動操作・故障条件・任意の環境変数・コマンドは実行しない。レシピと同じ全条件で実行したという意味にはしない。実際の構成は`compose.json`と、そのハッシュで識別する。

A・Bには別々の空のDBと内部ネットワークを作り、OpenEMRとDBはそれぞれの内部ネットワークだけへ接続する。アプリとDBのポートはホストへ公開しない。各環境に固定版nginxのTCP転送プロキシを1つ置き、そのプロキシだけを内部ネットワークと専用のbridgeへ接続する。プロキシの8443番を`127.0.0.1`の空きポートへ公開し、同じ環境のOpenEMRの443番へ転送する。TLSの終端はOpenEMRのままとする。

内部ネットワークだけへ接続したコンテナでは、起動できても公開ポートが割り当てられないという[Docker Engine 29系の報告](https://github.com/moby/moby/discussions/53256)がある。OpenEMRとDBの接続先を内部ネットワークに限定し、ホストへ公開するポートをプロキシに設けた。ランダムなポート割当は`published: "0"`で要求し、`status`では実際に公開されたポートと接続先の応答を確認する。公開ポートがなければ、サービス名・状態・ポート情報だけをエラーへ含める。環境変数や起動コマンドの原文は含めない。

nginxは`1.27.4-alpine`のイメージをダイジェストで固定する。転送先を要求や環境変数から選ばせず、生成した設定の`openemr-a:443`または`openemr-b:443`へ限定する。設定は固定コマンドからコンテナ内の`/tmp`へ作り、ホストのファイルをマウントしない。プロキシはUID101、読み取り専用のルートファイルシステム、`/tmp`のtmpfs、全capabilityの削除、追加特権の取得禁止を指定して起動する。

プロキシ自身にはbridge経由の外部への経路がある。固定転送設定と権限制限を設けたことだけで、あらゆる外部通信や本番への影響を防いだ証明にはならない。既存のDBや認証情報は引き継がず、実際の分離の確認は#59で継続する。

`up`が返っても、OpenEMRの初期設定が終わっていない場合がある。`status`で両方の`ready`が`true`になることを確認する。準備できない場合は、試用を開始せず、起動失敗と記録する。

`prepare`の直後に`status`で調べても、コンテナがまだなければ`prepared`を保ち、その後に`up`を実行できる。削除済みの環境にコンテナがなければ`removed`を保つ。起動を試した後に停止・未起動のサービスがある場合は`not_ready`と記録する。

## 利用者に試してもらう

1. 試用専用のブラウザプロファイルを用意する。`trial.json`のA・BのURLを開き、自己署名証明書の例外はこのローカル試用先に限定する。
2. `credentials.json`の試用専用アカウントでログインする。実際の患者情報・病院の認証情報を入力しない。
3. A・Bに同じ架空の患者・入力内容を用意する。利用者がどの画面で何をするかを説明し、初期データと操作条件を試用計画へ記録する。
4. `/user-trial`で計画を作る。`trial.json`から各版のリポジトリ・コミット・パス・URLを転記し、技術者の検証結果があれば証拠IDも結びつける。試用前にURLの接続先と準備状態を確認する。
5. 利用者がA・Bを操作し、実際に試した順番と操作、保存完了を判断できたか、迷った点や追加の負担を記録する。技術者が計測した所要時間や保存結果と、人の感想を区別する。

Aは`127.0.0.1`、Bは`localhost`を使い、Cookieのホスト名を分ける。ポートが違うだけではCookieを分離できないため。ブラウザやアプリが同じホスト名へ転送する場合は、A・Bを別々のブラウザプロファイルで開く。`localhost`がこの端末のループバックへ解決されることを確認する。自動で開くリンクからはブラウザプロファイルを選べない。

この手順は開発者の端末で一緒に試す使い方を想定する。病院のネットワークへの接続、外部公開、遠隔の協力者へのURL配布は扱わない。利用者の協力が決まるまで、環境の準備と記録機能の開発を進められる。自動試験や開発者の確認を、実利用者による試用と記録しない。

## 検証記録から試用計画の登録要求を作る

`tools/openemr-trial-plan.mjs`は、同じ事例にあるA・Bのブラウザ検証記録から各版のコード参照を取り出し、試用計画の登録要求を作る。各版に既存の検証記録が1件以上必要。正常時と再試験が同じコミットを使う既存のSOAP試験を、そのまま変更前後の比較へ読み替えない。

[入力例](../examples/openemr-browser/trial-plan-input.example.json)をコピーし、技術者の識別名・操作・初期条件・変更理由とPR参照、実際のURLと検証記録IDを記入する。入力例のままでは実行できない。認証情報は記入しない。

```bash
cp examples/openemr-browser/trial-plan-input.example.json data/openemr-trial-001/plan-input.json
# plan-input.jsonを編集してから実行する。
node tools/openemr-trial-plan.mjs \
  --case data/app/case \
  --input data/openemr-trial-001/plan-input.json \
  --environment data/openemr-trial-001/trial.json \
  --output data/openemr-trial-001/add-plan.json
cargo run -- case apply data/app/case data/openemr-trial-001/add-plan.json
```

ツールは存在しない検証記録、1つの版へ混ぜた異なるコミット、同じコミット同士、不正なURLを拒否する。失敗・未観測の検証記録も参照でき、試験結果を成功へ変更しない。`--environment`を指定した場合は、保存された試用環境の記録が`ready`で、版・URL・OpenEMRイメージが検証記録と一致するかも調べる。レシピと試用環境の全条件の一致や、起動中の版、Gitコミットとイメージ内のコードの同一性を確認したことにはならない。

手動で起動した環境を使う場合は`--environment`を省略し、版と環境の対応を確認した方法を条件欄に記入する。画面で直接計画を作る場合と異なり、この生成ツールは両版の既存のブラウザ検証記録を必須にする。

生成ツールは要求ファイルを新規に保存するだけで、対象アプリの起動・事例の更新・APIへの送信は行わない。`case apply`を実行する前に内容を確認する。生成後に事例が更新された場合は登録を拒否するため、最新の事例から別の出力ファイル名で作り直す。事例ファイル自体を編集して検証記録を作らない。

`app DATA_DIR`の画面は`DATA_DIR/case`を使う。上の例では`cargo run -- app data/app`を起動して`/user-trial`を開く。API専用サーバーはこの画面を配信しない。計画を登録したことと、実利用者が操作したことは別々に記録する。

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
node --test tools/openemr-trial-plan-tests.mjs
# 実Dockerで2環境の準備・ログイン画面・後片付けまで確認する
python3 tools/openemr-trial-smoke.py
```

実Dockerの試験は同一の固定版を使う。ログイン画面への接続に加え、`docker inspect`でアプリ・DBの接続先が内部ネットワークだけであること、プロキシだけがbridgeとループバックの公開ポートを持つことを検査する。`evaluations/openemr-manual-trial/boundaries.json`にネットワークと公開ポートの必要な項目だけを残し、認証情報を含むinspect結果の原文は保存しない。`ready.json`に起動時の参照を残し、`trial.json`に削除後の状態を残す。人の試用や、改善の効果を確認する試験ではない。

構成の根拠はDocker公式の[ポート設定](https://docs.docker.com/reference/compose-file/services/#ports)、[内部ネットワーク](https://docs.docker.com/reference/compose-file/networks/#internal)、[Composeの状態出力](https://docs.docker.com/reference/cli/docker/compose/ps/)。OpenEMRの操作の根拠と既存の自動試験は[固定版レシピ](../examples/openemr-browser/README.md)に記載する。

TCP転送の根拠はnginx公式の[stream proxy](https://nginx.org/en/docs/stream/ngx_stream_proxy_module.html)。固定版の[Alpineパッケージ設定](https://github.com/nginx/pkg-oss/blob/1.27.4-1/alpine/Makefile)は`--with-stream`を指定しており、この構成で追加の動的モジュールは読み込まない。
