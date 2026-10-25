//! ダイジェスト付きの公式イメージで、試験専用コンテナの起動・HTTP操作・故障・掃除を確かめる。
//! Dockerのエンジンがない環境では省略し、成功とは書かない。
mod support;
use mugiurushi::{container::*, execution::*, model::*, recording::*, trial::*, workspace::*};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::execute;

const NGINX: &str =
    "nginx:1.27.4-alpine@sha256:4ff102c5d78d254a6f0da062b3cf39eaf07f01eec0927fd21e219d0af8bc0591";

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}

fn fixture() -> (std::path::PathBuf, std::path::PathBuf, String) {
    let directory = new_run_directory(&std::env::temp_dir().join("evidence-containers"))
        .expect("dir")
        .0;
    let root = directory.join("repo");
    fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.name", "Synthetic test"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/web.git",
        ],
    );
    fs::write(
        root.join("README.md"),
        "架空の対象。コンテナは公式イメージをダイジェストで固定する。\n",
    )
    .unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "synthetic"]);
    let commit = git(&root, &["rev-parse", "HEAD"]);
    (directory, root, commit)
}

fn nginx(name: &str, role: ServiceRole) -> Service {
    Service {
        name: name.into(),
        image: NGINX.into(),
        role,
        expose: 80,
        env: Default::default(),
        volumes: vec![],
        web_build: None,
        command: vec![],
        ready: ReadyCheck::Http {
            path: "/".into(),
            tls: false,
        },
        ready_timeout_ms: 20000,
    }
}

fn recipe(
    commit: &str,
    services: Vec<Service>,
    fault: TrialFault,
    checks: Vec<TrialCheck>,
) -> Recipe {
    let http_service = services
        .iter()
        .find(|s| s.role == ServiceRole::App)
        .expect("app")
        .name
        .clone();
    Recipe {
        schema_version: 1,
        check_id: "web".into(),
        source: CodeReference {
            repository: "example/web".into(),
            commit: commit.into(),
            path: "README.md".into(),
            line: 1,
            end_line: 1,
        },
        runtime: Runtime::Http,
        entrypoint: "/".into(),
        args: vec![],
        input: serde_json::json!({}),
        criterion: Criterion {
            assertion: "eel.trial".into(),
            subject: "web".into(),
        },
        timeout_ms: 30000,
        trial: Some(TrialSpec {
            schema_version: 1,
            fault,
            storage: None,
            readback: None,
            checks,
        }),
        launch: Default::default(),
        python: None,
        server: None,
        containers: Some(ContainerSpec { services }),
        browser: None,
        http: Some(HttpOp {
            service: http_service,
            method: "GET".into(),
            path: "/".into(),
            body_file: None,
            content_type: None,
        }),
    }
}

fn check(
    id: &str,
    scope: &str,
    method: CheckMethod,
    expect: Expectation,
    text: &str,
) -> TrialCheck {
    TrialCheck {
        id: id.into(),
        scope: scope.into(),
        method,
        expect,
        text: if text.is_empty() {
            None
        } else {
            Some(text.into())
        },
        target: None,
    }
}

fn docker_available() -> bool {
    match engine() {
        Ok(_) => true,
        Err(error) => {
            eprintln!("Dockerがないためコンテナ試験を省略: {error}");
            false
        }
    }
}

#[test]
fn tag_without_digest_is_rejected() {
    let mut spec = ContainerSpec {
        services: vec![nginx("web", ServiceRole::App)],
    };
    spec.services[0].image = "nginx:1.27.4-alpine".into();
    assert!(validate_spec(&spec).is_err());
}

#[test]
#[ignore = "Dockerと固定イメージを使うため、手動・週次の統合試験で実行"]
fn nginx_http_is_satisfied() {
    if !docker_available() {
        return;
    }
    let (directory, root, commit) = fixture();
    let result = execute(
        &root,
        &directory.join("runs"),
        recipe(
            &commit,
            vec![nginx("web", ServiceRole::App)],
            TrialFault::Normal,
            vec![
                check(
                    "status",
                    "受付",
                    CheckMethod::ExitCode,
                    Expectation::Zero,
                    "",
                ),
                check(
                    "welcome",
                    "応答",
                    CheckMethod::Stdout,
                    Expectation::Contains,
                    "Welcome to nginx",
                ),
                check(
                    "log",
                    "ログ",
                    CheckMethod::ContainerLog,
                    Expectation::Contains,
                    "Configuration complete",
                ),
            ],
        ),
        Condition {
            id: "web".into(),
            version: 1,
            text: "試験用のnginxが応答する".into(),
            origin: "試験".into(),
            kind: ConditionKind::Custom,
        },
        None,
    )
    .unwrap();
    assert_eq!(
        result.assessment.verdict,
        Verdict::Satisfied,
        "{}",
        result.assessment.reason
    );
    let trace: ExecutionTrace =
        serde_json::from_str(&fs::read_to_string(result.directory.join("execution.json")).unwrap())
            .unwrap();
    let phase = trace.containers.expect("containers");
    assert!(phase.leftovers.is_empty(), "{:?}", phase.leftovers);
    assert!(phase.services[0].ready);
    assert!(phase.services[0].digest.contains("sha256:"));
    assert!(!phase.services[0].running_after_stop);
}

#[test]
#[ignore = "Dockerと固定イメージを使うため、手動・週次の統合試験で実行"]
fn stopping_the_database_container_is_effective() {
    if !docker_available() {
        return;
    }
    let (directory, root, commit) = fixture();
    let result = execute(
        &root,
        &directory.join("runs"),
        recipe(
            &commit,
            vec![
                nginx("web", ServiceRole::App),
                nginx("db", ServiceRole::Database),
            ],
            TrialFault::DatabaseStopped,
            vec![
                check(
                    "welcome",
                    "応答",
                    CheckMethod::Stdout,
                    Expectation::Contains,
                    "Welcome to nginx",
                ),
                check(
                    "log",
                    "ログ",
                    CheckMethod::ContainerLog,
                    Expectation::Contains,
                    "Configuration complete",
                ),
            ],
        ),
        Condition {
            id: "web".into(),
            version: 1,
            text: "データベースを止めても、止めたことを記録する".into(),
            origin: "試験".into(),
            kind: ConditionKind::Custom,
        },
        None,
    )
    .unwrap();
    let trace: ExecutionTrace =
        serde_json::from_str(&fs::read_to_string(result.directory.join("execution.json")).unwrap())
            .unwrap();
    let trial = trace.trial.expect("trial");
    assert!(trial.fault_effective, "{}", trial.fault_applied);
    let db = trial
        .containers
        .as_ref()
        .unwrap()
        .services
        .iter()
        .find(|s| s.name == "db")
        .unwrap();
    assert!(!db.running_after_stop);
    assert!(trial.containers.as_ref().unwrap().leftovers.is_empty());
}

#[test]
#[ignore = "Dockerと固定イメージを使うため、手動・週次の統合試験で実行"]
fn disconnecting_the_database_is_effective() {
    if !docker_available() {
        return;
    }
    let (directory, root, commit) = fixture();
    let result = execute(
        &root,
        &directory.join("runs"),
        recipe(
            &commit,
            vec![
                nginx("web", ServiceRole::App),
                nginx("db", ServiceRole::Database),
            ],
            TrialFault::DatabaseUnreachable,
            vec![check(
                "welcome",
                "応答",
                CheckMethod::Stdout,
                Expectation::Contains,
                "Welcome to nginx",
            )],
        ),
        Condition {
            id: "web".into(),
            version: 1,
            text: "データベースとの通信を切ったことを記録する".into(),
            origin: "試験".into(),
            kind: ConditionKind::Custom,
        },
        None,
    )
    .unwrap();
    let trace: ExecutionTrace =
        serde_json::from_str(&fs::read_to_string(result.directory.join("execution.json")).unwrap())
            .unwrap();
    assert!(trace.trial.as_ref().unwrap().fault_effective);
}

const ORTHANC: &str = "orthancteam/orthanc:26.9.1@sha256:d2705f2c56547e55ce9bc2250fd22d05d326493b3f3d9fa191ecb7db8ca6b5a2";
const OPENEMR: &str = "openemr/openemr:latest@sha256:f0bfbc17fae13503ab64bbd271f74d80dd4a0786f5a8d0683841aeef4e159700";
const MARIADB: &str =
    "mariadb:12.3.3@sha256:2bdff1534a7e569fecaf4b10b66ad0d410806e382d9db29ba305448addda97c4";

fn sample_dicom(root: &Path) -> bool {
    let src = PathBuf::from(format!(
        "{}/.cache/pynetdicom/pynetdicom/tests/dicom_files/CTImageStorage.dcm",
        std::env::var("HOME").unwrap_or_default()
    ));
    if !src.is_file() {
        eprintln!("試験用DICOMがないため Orthanc の登録試験を省略");
        return false;
    }
    fs::copy(src, root.join("sample.dcm")).unwrap();
    Command::new("git")
        .args(["-C", root.to_str().unwrap(), "add", "sample.dcm"])
        .status()
        .ok();
    git(root, &["commit", "-qm", "sample"]);
    true
}

#[test]
#[ignore = "Orthancのイメージ取得が必要"]
fn orthanc_stores_the_sample_dicom() {
    if !docker_available() {
        return;
    }
    let (directory, root, _) = fixture();
    if !sample_dicom(&root) {
        return;
    }
    let commit = git(&root, &["rev-parse", "HEAD"]);
    let mut env = std::collections::BTreeMap::new();
    env.insert("DICOM_WEB_PLUGIN_ENABLED".into(), "true".into());
    env.insert("ORTHANC__REMOTE_ACCESS_ALLOWED".into(), "true".into());
    env.insert("ORTHANC__AUTHENTICATION_ENABLED".into(), "false".into());
    let mut recipe = recipe(
        &commit,
        vec![Service {
            name: "orthanc".into(),
            image: ORTHANC.into(),
            role: ServiceRole::App,
            expose: 8042,
            env,
            volumes: vec![],
            web_build: None,
            command: vec![],
            ready: ReadyCheck::Http {
                path: "/system".into(),
                tls: false,
            },
            ready_timeout_ms: 60000,
        }],
        TrialFault::Normal,
        vec![
            check(
                "status",
                "受付",
                CheckMethod::ExitCode,
                Expectation::Zero,
                "",
            ),
            check(
                "stored",
                "書き込み",
                CheckMethod::Stdout,
                Expectation::Contains,
                "ParentStudy",
            ),
        ],
    );
    recipe.http = Some(HttpOp {
        service: "orthanc".into(),
        method: "POST".into(),
        path: "/instances".into(),
        body_file: Some("sample.dcm".into()),
        content_type: Some("application/dicom".into()),
    });
    recipe.entrypoint = "/instances".into();
    recipe.timeout_ms = 60000;
    let result = execute(
        &root,
        &directory.join("runs"),
        recipe,
        Condition {
            id: "web".into(),
            version: 1,
            text: "試験用のDICOMをOrthancへ登録できる".into(),
            origin: "試験".into(),
            kind: ConditionKind::Custom,
        },
        None,
    )
    .unwrap();
    assert_eq!(
        result.assessment.verdict,
        Verdict::Satisfied,
        "{}",
        result.assessment.reason
    );
}

#[test]
#[ignore = "OpenEMRとMariaDBのイメージ取得と数分の起動が必要"]
fn openemr_becomes_ready_with_pinned_images() {
    if !docker_available() {
        return;
    }
    let (directory, root, commit) = fixture();
    let mut db_env = std::collections::BTreeMap::new();
    db_env.insert("MYSQL_ROOT_PASSWORD".into(), "eel-root-trial".into());
    db_env.insert("MARIADB_AUTO_UPGRADE".into(), "1".into());
    let mut app_env = std::collections::BTreeMap::new();
    app_env.insert("MYSQL_HOST".into(), "mysql".into());
    app_env.insert("MYSQL_ROOT_PASS".into(), "eel-root-trial".into());
    app_env.insert("MYSQL_USER".into(), "openemr".into());
    app_env.insert("MYSQL_PASS".into(), "eel-openemr-trial".into());
    app_env.insert("OE_USER".into(), "eel-operator".into());
    app_env.insert("OE_PASS".into(), "eel-pass-trial".into());
    let mut recipe = recipe(
        &commit,
        vec![
            Service {
                name: "mysql".into(),
                image: MARIADB.into(),
                role: ServiceRole::Database,
                expose: 3306,
                env: db_env,
                volumes: vec![VolumeMount {
                    name: "db".into(),
                    path: "/var/lib/mysql".into(),
                }],
                web_build: None,
                command: vec!["mariadbd".into(), "--character-set-server=utf8mb4".into()],
                ready: ReadyCheck::Exec {
                    args: vec![
                        "/usr/local/bin/healthcheck.sh".into(),
                        "--su-mysql".into(),
                        "--connect".into(),
                        "--innodb_initialized".into(),
                    ],
                },
                ready_timeout_ms: 120000,
            },
            Service {
                name: "openemr".into(),
                image: OPENEMR.into(),
                role: ServiceRole::App,
                expose: 443,
                env: app_env,
                volumes: vec![],
                web_build: None,
                command: vec![],
                ready: ReadyCheck::Http {
                    path: "/meta/health/readyz".into(),
                    tls: true,
                },
                ready_timeout_ms: 240000,
            },
        ],
        TrialFault::Normal,
        vec![
            check(
                "status",
                "受付",
                CheckMethod::ExitCode,
                Expectation::Zero,
                "",
            ),
            check(
                "ready",
                "応答",
                CheckMethod::Stdout,
                Expectation::Contains,
                "HTTP 200",
            ),
            check(
                "db",
                "保存先",
                CheckMethod::DatabaseQuery,
                Expectation::Contains,
                "SELECT 1",
            ),
        ],
    );
    recipe.http = Some(HttpOp {
        service: "openemr".into(),
        method: "GET".into(),
        path: "/meta/health/readyz".into(),
        body_file: None,
        content_type: None,
    });
    recipe.timeout_ms = 300000;
    let result = execute(
        &root,
        &directory.join("runs"),
        recipe,
        Condition {
            id: "web".into(),
            version: 1,
            text: "固定したイメージでOpenEMRが準備できる".into(),
            origin: "試験".into(),
            kind: ConditionKind::Custom,
        },
        None,
    )
    .unwrap();
    assert_eq!(
        result.assessment.verdict,
        Verdict::Satisfied,
        "{}",
        result.assessment.reason
    );
}

#[test]
#[ignore = "CIのPlaywright導入後にDockerとChromiumで実行"]
fn browser_observes_nginx_and_preserves_screenshots() {
    assert!(engine().is_ok(), "この明示試験にはDockerが必要");
    let (directory, root, commit) = fixture();
    let browsers: serde_json::Value = serde_json::from_str(
        &fs::read_to_string("node_modules/playwright-core/browsers.json").unwrap(),
    )
    .unwrap();
    let version = browsers["browsers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "chromium")
        .unwrap()["browserVersion"]
        .as_str()
        .unwrap();
    let mut trial_check = check(
        "display",
        "表示",
        CheckMethod::BrowserText,
        Expectation::Contains,
        "Welcome to nginx",
    );
    trial_check.target = Some("welcome".into());
    let mut recipe = recipe(
        &commit,
        vec![nginx("web", ServiceRole::App)],
        TrialFault::Normal,
        vec![trial_check],
    );
    recipe.runtime = Runtime::Browser;
    recipe.http = None;
    recipe.browser = Some(serde_json::from_value(serde_json::json!({"service":"web","tls":false,"start_path":"/","expected_version":version,"actions":[{"id":"welcome","action":"read_text","target":{"locator":{"by":"role","role":"heading","name":"Welcome to nginx!"}}}]})).unwrap());
    let result = execute(
        &root,
        &directory.join("runs"),
        recipe,
        Condition {
            id: "web".into(),
            version: 1,
            text: "画面の見出しを読む".into(),
            origin: "合成の接続試験".into(),
            kind: ConditionKind::Custom,
        },
        None,
    )
    .unwrap();
    let trace: ExecutionTrace = read_json(&result.directory.join("execution.json")).unwrap();
    let report = trace.containers.as_ref().unwrap().browser.as_ref().unwrap();
    assert!(report.completed, "{report:?}");
    assert_eq!(
        trace.trial.as_ref().unwrap().results[0].observed,
        Observed::Pass
    );
    assert!(
        result
            .directory
            .join(report.steps[0].screenshot.as_ref().unwrap())
            .is_file()
    );
    assert!(trace.containers.as_ref().unwrap().leftovers.is_empty());
    validate_trace(&trace, &result.manifest).unwrap();
}

#[test]
#[ignore = "OpenEMRの画面とMariaDBをCIで操作する。Playwrightと試験用の認証ファイルが必要"]
fn openemr_browser_registers_patient_and_reads_back_visit() {
    assert!(engine().is_ok(), "この明示試験にはDockerが必要");
    assert!(
        std::env::var_os("EEL_BROWSER_SECRETS_FILE").is_some(),
        "試験用の認証ファイルが必要"
    );
    let original: Recipe = read_json(Path::new("examples/openemr-browser/recipe.json")).unwrap();
    let root = fetch_commit(
        Path::new("target/browser-sources"),
        &original.source.repository,
        &original.source.commit,
    )
    .unwrap();
    for fault in [TrialFault::Normal, TrialFault::DatabaseStopped] {
        let mut recipe = original.clone();
        recipe.trial.as_mut().unwrap().fault = fault;
        if fault == TrialFault::DatabaseStopped {
            // 患者登録と受診理由の入力までは済ませ、保存ボタンを押す直前にDBを止める。
            let browser = recipe.browser.as_mut().unwrap();
            let save = browser
                .actions
                .iter()
                .position(|s| s.id == "save-visit")
                .unwrap();
            browser.preparation.extend(browser.actions.drain(..save));
            recipe
                .trial
                .as_mut()
                .unwrap()
                .checks
                .retain(|c| c.id.starts_with("visit-"));
        }
        let label = if fault == TrialFault::Normal {
            "normal"
        } else {
            "database-stopped"
        };
        let result = execute(
            &root,
            &Path::new("evaluations/ci-openemr").join(label),
            recipe,
            Condition {
                id: "openemr".into(),
                version: 1,
                text: "架空の患者と受診理由を保存し再表示できる".into(),
                origin: "実OSSの試験用構成".into(),
                kind: ConditionKind::Custom,
            },
            None,
        )
        .unwrap();
        let trace: ExecutionTrace = read_json(&result.directory.join("execution.json")).unwrap();
        // 実際の正常・DB停止の記録を、同じ固定版の構成へ対応づける。
        let case = Path::new("evaluations/ci-openemr").join(label).join("case");
        let mut w = mugiurushi::workspace::empty_workspace(
            "openemr".into(),
            "OpenEMRの構成と実試験".into(),
        );
        w.architecture_models.push(
            serde_json::from_str(include_str!(
                "../examples/openemr-browser/architecture.json"
            ))
            .unwrap(),
        );
        w.checks.push(mugiurushi::workspace::Check {
            id: "openemr".into(),
            version: 1,
            text: result.manifest.condition.text.clone(),
            origin: result.manifest.condition.origin.clone(),
            source: Some(original.source.clone()),
            observation: Some(original.criterion.clone()),
            rule: mugiurushi::workspace::Rule::ObservedAssertion,
            dependencies: vec!["execution-openemr".into()],
            questions: vec![],
        });
        mugiurushi::workspace::initialize(&case, w, "CI", "実試験と構成の対応づけ").unwrap();
        let mut w = mugiurushi::workspace::change(
            &case,
            mugiurushi::workspace::ChangeRequest {
                expected_revision: 1,
                actor: "CI".into(),
                reason: "実際の試行を取り込む".into(),
                action: mugiurushi::workspace::Change::Import {
                    check_id: "openemr".into(),
                    run_directory: result.directory.to_string_lossy().into_owned(),
                    use_run_context: true,
                },
            },
        )
        .unwrap()
        .workspace;
        for check in &trace.recipe.trial.as_ref().unwrap().checks {
            let object = match check.method {
                CheckMethod::BrowserText => "browser",
                CheckMethod::BrowserResponse => "https",
                CheckMethod::DatabaseRows => "db",
                _ => unreachable!(),
            };
            w = mugiurushi::workspace::change(
                &case,
                mugiurushi::workspace::ChangeRequest {
                    expected_revision: w.revision,
                    actor: "CI".into(),
                    reason: "観測条件を構成要素へ対応づける".into(),
                    action: mugiurushi::workspace::Change::AddArchitectureLink {
                        link: mugiurushi::architecture::Link {
                            id: check.id.clone(),
                            model_id: "openemr-trial".into(),
                            model_version: 1,
                            object_id: object.into(),
                            target: mugiurushi::architecture::Target::Evidence {
                                evidence_id: w.evidence[0].id.clone(),
                                observation_id: check.id.clone(),
                            },
                        },
                    },
                },
            )
            .unwrap()
            .workspace;
        }
        let links = mugiurushi::architecture::links(&w);
        assert_eq!(
            links.len(),
            trace.recipe.trial.as_ref().unwrap().checks.len()
        );
        assert!(links.iter().all(|l| l.applicable && l.current_model));
        if fault == TrialFault::DatabaseStopped {
            assert!(
                links
                    .iter()
                    .filter(|l| l.link.object_id == "db")
                    .all(|l| l.observed == Some(Observed::NotObserved))
            );
        }
        mugiurushi::backup::validate(&mugiurushi::backup::snapshot(&case).unwrap()).unwrap();
        let phase = trace.containers.as_ref().unwrap();
        let browser = phase.browser.as_ref().expect("ブラウザ記録");
        println!("{}",serde_json::to_string(&serde_json::json!({"condition":label,"directory":result.directory,"browser":browser,"database_reads":phase.database_reads,"results":trace.trial.as_ref().unwrap().results,"containers":phase.services.iter().map(|s|(&s.name,s.ready,&s.detail)).collect::<Vec<_>>()})).unwrap());
        assert!(browser.prepared, "{browser:?}");
        assert!(phase.leftovers.is_empty(), "試験用の資源が残った");
        if fault == TrialFault::Normal {
            assert!(browser.completed, "{browser:?}");
            assert!(
                browser.responses.iter().any(|r| r.watch == "visit-reload"
                    && r.request_step.as_deref() == Some("refresh-visit")
                    && r.status == 200),
                "タブ更新によるサーバーからの再読込を確認できない: {:?}",
                browser.responses
            );
            assert!(
                trace
                    .trial
                    .as_ref()
                    .unwrap()
                    .results
                    .iter()
                    .all(|r| r.observed == Observed::Pass),
                "{:?}",
                trace.trial.as_ref().unwrap().results
            );
            assert_eq!(result.assessment.verdict, Verdict::Satisfied);
        } else {
            assert!(trace.trial.as_ref().unwrap().fault_effective);
            assert!(!browser.completed);
            assert!(phase.database_reads.iter().all(|r| !r.performed));
            assert!(
                trace
                    .trial
                    .as_ref()
                    .unwrap()
                    .results
                    .iter()
                    .filter(|r| r.id.ends_with("-row"))
                    .all(|r| r.observed == Observed::NotObserved)
            );
            // HTTPの失敗を観測した場合は条件を満たさない。DBの読取不能は別に未観測として残す。
            assert_ne!(result.assessment.verdict, Verdict::Satisfied);
        }
    }
}
