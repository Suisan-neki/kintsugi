mod case;
mod environment;
use mugiurushi::review;
use std::{collections::BTreeMap, io, path::PathBuf};

const HELP: &str = "mugiurushi
  app DATA_DIR [--port 8765] [--execution-mode bubblewrap]
  ai init|test [--data-dir data/app]
  api CASE_DIR --token-file NEW_FILE [--port 8765] [--repository-root DIR --execution-mode bubblewrap]
  case init CASE_DIR [--file SPEC_JSON | --id CASE_ID --title TEXT] [--actor TEXT] [--reason TEXT]
  case show CASE_DIR
  case apply CASE_DIR CHANGE_JSON
  case import CASE_DIR RUN_DIR --condition CHECK_ID --actor TEXT --reason TEXT [--use-run-context true|false]
  case execute CASE_DIR RECIPE_JSON --repository-root DIR --mode local|bubblewrap --actor TEXT --reason TEXT
  case approve CASE_DIR REQUEST_JSON
  case revoke CASE_DIR
  case report CASE_DIR --output NEW_FILE [--format json|html]
  case backup CASE_DIR --output NEW_JSON
  case restore NEW_CASE_DIR BACKUP_JSON
  case compare CASE_DIR --before EVIDENCE_ID --after EVIDENCE_ID --condition CHECK_ID
  repo prepare NEW_DIR RECIPE_JSON
  environment inspect|run|show|compare ...

引数なしで現在のアプリを起動します。Web実行にはbubblewrapと端末での一回限りの承認が必要です。localは開発ビルドの端末実行専用です。";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
fn print_json(value: &impl serde::Serialize) -> io::Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(io::Error::other)?
    );
    Ok(())
}
fn required(options: &BTreeMap<String, String>, name: &str) -> io::Result<String> {
    options
        .get(name)
        .cloned()
        .ok_or_else(|| invalid(format!("{name}が必要です")))
}
fn port(options: &BTreeMap<String, String>) -> io::Result<u16> {
    options.get("--port").map_or(Ok(8765), |value| {
        value.parse().map_err(|_| invalid("ポートは0〜65535です"))
    })
}
fn options(args: &[String], allowed: &[&str]) -> io::Result<BTreeMap<String, String>> {
    let mut parsed = BTreeMap::new();
    let (pairs, remainder) = args.as_chunks::<2>();
    for pair in pairs {
        if !allowed.contains(&pair[0].as_str()) {
            return Err(invalid(format!("不明なオプション: {}", pair[0])));
        }
        if parsed.insert(pair[0].clone(), pair[1].clone()).is_some() {
            return Err(invalid(format!("重複した指定: {}", pair[0])));
        }
    }
    if !remainder.is_empty() {
        return Err(invalid("オプションには値が必要です"));
    }
    Ok(parsed)
}
fn output(options: &BTreeMap<String, String>) -> PathBuf {
    options
        .get("--output")
        .map_or_else(|| PathBuf::from("runs"), PathBuf::from)
}
pub fn run(args: &[String]) -> io::Result<()> {
    let Some(command) = args.first() else {
        return review::serve_app(
            PathBuf::from("data/app"),
            8765,
            mugiurushi::execution::ExecutionMode::Bubblewrap,
        );
    };
    match command.as_str() {
        "help" | "--help" | "-h" if args.len() == 1 => println!("{HELP}"),
        "ai" if args.len() >= 2 && matches!(args[1].as_str(), "init" | "test") => {
            let opts = options(&args[2..], &["--data-dir"])?;
            let data = opts
                .get("--data-dir")
                .map_or_else(|| PathBuf::from("data/app"), PathBuf::from);
            let path = data.join("ai.json");
            if args[1] == "init" {
                mugiurushi::ai::initialize_connection(&path)?;
                println!(
                    "運営者用の設定ファイルを作成しました: {}\nモデル名とAPIキーをこのファイルへ記入し、ai testで接続を確認してください。",
                    path.display()
                );
            } else {
                let config = mugiurushi::ai::Config::for_app(&data)?.ok_or_else(|| {
                    invalid("AI未設定です。ai initで運営者用の設定ファイルを作成してください")
                })?;
                let result = config.test_connection()?;
                println!("{}", result["message"].as_str().unwrap_or_default());
            }
        }
        "case" => case::run(&args[1..])?,
        "environment" => environment::run(&args[1..])?,
        "api" if args.len() >= 2 => {
            let opts = options(
                &args[2..],
                &[
                    "--port",
                    "--token-file",
                    "--repository-root",
                    "--execution-mode",
                ],
            )?;
            let port = port(&opts)?;
            let token_file = opts
                .get("--token-file")
                .ok_or_else(|| invalid("新規の--token-fileが必要です"))?;
            let execution = match (opts.get("--repository-root"), opts.get("--execution-mode")) {
                (None, None) => None,
                (Some(root), Some(mode)) => Some((PathBuf::from(root), mode.parse()?)),
                _ => return Err(invalid("実行にはリポジトリと方式の両方が必要です")),
            };
            review::serve_api(
                PathBuf::from(&args[1]),
                port,
                PathBuf::from(token_file),
                execution,
            )?;
        }
        "repo" if args.len() == 4 && args[1] == "prepare" => {
            let recipe = mugiurushi::recording::read_json(&PathBuf::from(&args[3]))?;
            mugiurushi::execution::prepare_repository(&PathBuf::from(&args[2]), &recipe)?;
            println!(
                "固定コミットを取得しました: {}。checkoutと対象コードの実行は行っていません。",
                args[2]
            );
        }
        "app" if args.len() >= 2 => {
            let opts = options(&args[2..], &["--port", "--execution-mode"])?;
            let port = port(&opts)?;
            let mode = opts
                .get("--execution-mode")
                .map_or(Ok(mugiurushi::execution::ExecutionMode::Bubblewrap), |m| {
                    m.parse()
                })?;
            review::serve_app(PathBuf::from(&args[1]), port, mode)?;
        }
        _ => return Err(invalid(format!("引数を確認してください\n{HELP}"))),
    }
    Ok(())
}
