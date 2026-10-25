# 実装を読む手順

現在の操作は[README](../README.md)、要件と実装案の区別は[requirements.md](requirements.md)を参照する。以下はコードを読むための案内で、学習にかける日数を指定するものではない。

1. HTTP入力から保存までを読む。`src/review.rs`のAPI、`src/workspace.rs`の型と変更、`tests/workspace.rs`の正常・不正入力を対応づける。
2. 根拠と判定を読む。`src/model.rs`、`src/reported.rs`、`src/browser_evidence.rs`と、それぞれのテストで、報告・直接読取・未確認の違いを確認する。
3. 実行と停止を読む。`src/execution.rs`、`src/server.rs`、`src/container.rs`で、固定版、入出力、タイムアウト、試行後の停止を確認する。
4. 読取から説明までを読む。`src/understanding.rs`、`src/reader/`、`src/system_diagram/`で、保存したコード、簡易解析、AIの説明の出所を確認する。

修正する場合は、変更した境界のテストと通常の操作を確認し、未検証の範囲をPRに残す。
