//! SQLiteのトランザクションで版を公開し、変更しない項目の本文を共用する。
//! 通常の読出しでは最新版の索引だけを使い、旧JSON形式は初回に取り込む。
use super::{MAX_DOCUMENT, Workspace, invalid, validate};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io, path::Path, time::Duration};

fn sql(error: rusqlite::Error) -> io::Error {
    match &error {
        rusqlite::Error::SqliteFailure(code, _)
            if matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "別の更新が進行中です。読み直して再試行してください",
            )
        }
        _ => io::Error::other(error),
    }
}

fn sync_directory(directory: &Path) -> io::Result<()> {
    fs::File::open(directory)?.sync_all()
}

fn open(directory: &Path) -> io::Result<Connection> {
    if !directory.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "事例の保存先がありません",
        ));
    }
    let connection = Connection::open(directory.join("workspace.sqlite3")).map_err(sql)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(sql)?;
    let schema: i64 = connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(sql)?;
    if schema > 1 {
        return Err(invalid("未対応の保存形式です"));
    }
    if schema == 0 {
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(sql)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(sql)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(sql)?;
        connection
            .execute_batch(
                "
        CREATE TABLE IF NOT EXISTS revisions (revision INTEGER PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS fields (name TEXT PRIMARY KEY, kind INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS objects (sha TEXT PRIMARY KEY, body BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS entries (
            field TEXT NOT NULL REFERENCES fields(name), position TEXT NOT NULL,
            first_revision INTEGER NOT NULL, until_revision INTEGER,
            sha TEXT NOT NULL REFERENCES objects(sha),
            PRIMARY KEY(field, position, first_revision));
        CREATE UNIQUE INDEX IF NOT EXISTS current_entries ON entries(field, position)
            WHERE until_revision IS NULL;
        CREATE INDEX IF NOT EXISTS historical_entries ON entries(first_revision, until_revision);
    ",
            )
            .map_err(sql)?;
        connection
            .pragma_update(None, "user_version", 1)
            .map_err(sql)?;
        sync_directory(directory)?;
    }
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(sql)?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(sql)?;
    Ok(connection)
}

pub(super) struct Write {
    connection: Connection,
}
impl Drop for Write {
    fn drop(&mut self) {
        let _ = self.connection.execute_batch("ROLLBACK");
    }
}
impl Write {
    pub(super) fn open(directory: &Path) -> io::Result<Self> {
        let connection = open(directory)?;
        connection.execute_batch("BEGIN IMMEDIATE").map_err(sql)?;
        let writer = Self { connection };
        if writer.latest()?.is_none() && directory.join("revisions").is_dir() {
            writer.import_legacy(directory)?;
        }
        Ok(writer)
    }
    fn latest(&self) -> io::Result<Option<u64>> {
        let latest: Option<i64> = self
            .connection
            .query_row("SELECT MAX(revision) FROM revisions", [], |r| r.get(0))
            .map_err(sql)?;
        latest
            .map(|n| u64::try_from(n).map_err(io::Error::other))
            .transpose()
    }
    pub(super) fn load(&self) -> io::Result<Workspace> {
        let revision = self
            .latest()?
            .ok_or_else(|| invalid("事例の保存済みの版がありません"))?;
        read_revision(&self.connection, revision)
    }
    fn append(&self, workspace: &Workspace) -> io::Result<()> {
        validate(workspace)?;
        let revision = i64::try_from(workspace.revision).map_err(io::Error::other)?;
        let value = serde_json::to_value(workspace).map_err(io::Error::other)?;
        if serde_json::to_vec(&value).map_err(io::Error::other)?.len() as u64 > MAX_DOCUMENT {
            return Err(invalid(
                "事例が64MiBを超えています。別の事例に分けてください",
            ));
        }
        let expected = self.latest()?.unwrap_or(0) + 1;
        if workspace.revision != expected {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "別の更新があります。最新版を読み直してください",
            ));
        }
        let object = value
            .as_object()
            .ok_or_else(|| invalid("事例の形式が不正です"))?;
        let mut next = BTreeMap::new();
        for (name, value) in object {
            let kind: i64 = match value {
                Value::Array(_) => 1,
                Value::Object(_) => 2,
                _ => 0,
            };
            self.connection
                .execute(
                    "INSERT OR IGNORE INTO fields VALUES (?1,?2)",
                    params![name, kind],
                )
                .map_err(sql)?;
            let previous_kind: i64 = self
                .connection
                .query_row("SELECT kind FROM fields WHERE name=?1", [name], |r| {
                    r.get(0)
                })
                .map_err(sql)?;
            if kind != previous_kind {
                return Err(invalid("事例の項目の形式が変わっています"));
            }
            let items: Vec<(String, &Value)> = match value {
                Value::Array(items) => items
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (format!("{i:020}"), v))
                    .collect(),
                Value::Object(items) => items.iter().map(|(k, v)| (k.clone(), v)).collect(),
                _ => vec![(String::new(), value)],
            };
            for (position, item) in items {
                let body = serde_json::to_vec(item).map_err(io::Error::other)?;
                let sha = format!("{:x}", Sha256::digest(&body));
                self.connection
                    .execute(
                        "INSERT OR IGNORE INTO objects VALUES (?1, ?2)",
                        params![sha, body],
                    )
                    .map_err(sql)?;
                next.insert((name.clone(), position), sha);
            }
        }
        let mut statement = self
            .connection
            .prepare("SELECT field,position,sha FROM entries WHERE until_revision IS NULL")
            .map_err(sql)?;
        let current: BTreeMap<(String, String), String> = statement
            .query_map([], |r| Ok(((r.get(0)?, r.get(1)?), r.get(2)?)))
            .map_err(sql)?
            .collect::<Result<_, _>>()
            .map_err(sql)?;
        for ((field, position), sha) in &current {
            if next.get(&(field.clone(), position.clone())) != Some(sha) {
                self.connection.execute("UPDATE entries SET until_revision=?1 WHERE field=?2 AND position=?3 AND until_revision IS NULL",params![revision,field,position]).map_err(sql)?;
            }
        }
        for ((field, position), sha) in next {
            if current.get(&(field.clone(), position.clone())) != Some(&sha) {
                self.connection
                    .execute(
                        "INSERT INTO entries VALUES (?1,?2,?3,NULL,?4)",
                        params![field, position, revision, sha],
                    )
                    .map_err(sql)?;
            }
        }
        self.connection
            .execute("INSERT INTO revisions VALUES (?1)", [revision])
            .map_err(sql)?;
        Ok(())
    }
    pub(super) fn save(self, workspace: &Workspace) -> io::Result<()> {
        self.append(workspace)?;
        self.connection.execute_batch("COMMIT").map_err(sql)
    }
    fn import_legacy(&self, directory: &Path) -> io::Result<()> {
        let mut paths = BTreeMap::new();
        for entry in fs::read_dir(directory.join("revisions"))? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.len() == 25
                && name.ends_with(".json")
                && name[..20].bytes().all(|b| b.is_ascii_digit())
            {
                paths.insert(
                    name[..20].parse::<u64>().map_err(io::Error::other)?,
                    entry.path(),
                );
            }
        }
        for (revision, path) in paths {
            if fs::metadata(&path)?.len() > 4 * 1024 * 1024 {
                return Err(invalid("旧事例が4MiBを超えています"));
            }
            let workspace: Workspace = crate::recording::read_json(&path)?;
            if workspace.revision != revision {
                return Err(invalid("事例の版とファイル名が一致しません"));
            }
            self.append(&workspace)?;
        }
        Ok(())
    }
}

fn read_revision(connection: &Connection, revision: u64) -> io::Result<Workspace> {
    let number = i64::try_from(revision).map_err(io::Error::other)?;
    let exists: Option<i64> = connection
        .query_row(
            "SELECT revision FROM revisions WHERE revision=?1",
            [number],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql)?;
    if exists.is_none() {
        return Err(invalid("事例の版がありません"));
    }
    let mut object = Map::new();
    let mut fields = connection
        .prepare("SELECT name,kind FROM fields ORDER BY name")
        .map_err(sql)?;
    for row in fields
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(sql)?
    {
        let (name, kind) = row.map_err(sql)?;
        object.insert(
            name,
            match kind {
                1 => Value::Array(vec![]),
                2 => Value::Object(Map::new()),
                0 => Value::Null,
                _ => return Err(invalid("事例の項目の形式が不正です")),
            },
        );
    }
    let latest: i64 = connection
        .query_row("SELECT MAX(revision) FROM revisions", [], |r| r.get(0))
        .map_err(sql)?;
    let query = if latest == number {
        "SELECT field,position,objects.body FROM entries JOIN objects USING(sha) WHERE until_revision IS NULL AND first_revision<=?1 ORDER BY field,position"
    } else {
        "SELECT field,position,objects.body FROM entries JOIN objects USING(sha) WHERE first_revision<=?1 AND (until_revision IS NULL OR until_revision>?1) ORDER BY field,position"
    };
    let mut statement = connection.prepare(query).map_err(sql)?;
    let mut size = 0_u64;
    for row in statement
        .query_map([number], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(sql)?
    {
        let (field, position, body) = row.map_err(sql)?;
        size += body.len() as u64;
        if size > MAX_DOCUMENT {
            return Err(invalid("事例が64MiBを超えています"));
        }
        let item = serde_json::from_slice(&body).map_err(io::Error::other)?;
        match object
            .get_mut(&field)
            .ok_or_else(|| invalid("事例の項目がありません"))?
        {
            Value::Array(items) => {
                if position != format!("{:020}", items.len()) {
                    return Err(invalid("事例の項目の順序が不正です"));
                }
                items.push(item);
            }
            Value::Object(items) => {
                items.insert(position, item);
            }
            scalar => {
                if !position.is_empty() {
                    return Err(invalid("事例の項目の位置が不正です"));
                }
                *scalar = item;
            }
        }
    }
    let workspace: Workspace =
        serde_json::from_value(Value::Object(object)).map_err(io::Error::other)?;
    if workspace.revision != revision {
        return Err(invalid("事例の版が一致しません"));
    }
    validate(&workspace)?;
    Ok(workspace)
}

pub(super) fn load(directory: &Path, revision: Option<u64>) -> io::Result<Workspace> {
    if directory.join("restore.pending").exists() {
        return Err(invalid(
            "復元が完了していません。復元元と新しい保存先を確認してください",
        ));
    }
    let connection = open(directory)?;
    let latest: Option<i64> = connection
        .query_row("SELECT MAX(revision) FROM revisions", [], |r| r.get(0))
        .map_err(sql)?;
    match latest {
        Some(latest) => read_revision(
            &connection,
            revision.unwrap_or(u64::try_from(latest).map_err(io::Error::other)?),
        ),
        None => {
            drop(connection);
            let writer = Write::open(directory)?;
            let latest = writer
                .latest()?
                .ok_or_else(|| invalid("事例の保存済みの版がありません"))?;
            let workspace = read_revision(&writer.connection, revision.unwrap_or(latest))?;
            writer.connection.execute_batch("COMMIT").map_err(sql)?;
            Ok(workspace)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{self, Audit, Change, ChangeRequest};
    struct Case(std::path::PathBuf);
    impl Case {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("mugiurushi-sqlite-{}-{name}", std::process::id()));
            Self(path)
        }
        fn initialize(&self) {
            workspace::initialize(
                &self.0,
                workspace::empty_workspace("test".into(), "合成の保存試験".into()),
                "合成",
                "初期化",
            )
            .unwrap();
        }
    }
    impl Drop for Case {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn unchanged_large_values_are_stored_once_and_aborted_updates_roll_back() {
        let case = Case::new("dedup");
        case.initialize();
        for revision in 1..=12 {
            let action = if revision == 1 {
                Change::SetContext {
                    key: "large".into(),
                    value: "x".repeat(128 * 1024),
                }
            } else {
                Change::SetContext {
                    key: "small".into(),
                    value: revision.to_string(),
                }
            };
            workspace::change(
                &case.0,
                ChangeRequest {
                    expected_revision: revision,
                    actor: "合成".into(),
                    reason: "保存の確認".into(),
                    action,
                },
            )
            .unwrap();
        }
        let connection = open(&case.0).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM objects WHERE length(body)>100000",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        drop(connection);
        {
            let writer = Write::open(&case.0).unwrap();
            let mut workspace = writer.load().unwrap();
            workspace.revision += 1;
            writer.append(&workspace).unwrap();
            // COMMIT前の失敗と同様に、接続を閉じる。
        }
        assert_eq!(workspace::load(&case.0).unwrap().revision, 13);
        fs::write(case.0.join("write.lock"), b"stale legacy lock").unwrap();
        workspace::change(
            &case.0,
            ChangeRequest {
                expected_revision: 13,
                actor: "合成".into(),
                reason: "旧ロックが残っている場合".into(),
                action: Change::SetContext {
                    key: "small".into(),
                    value: "after rollback".into(),
                },
            },
        )
        .unwrap();
    }
    #[test]
    fn legacy_history_is_imported_atomically_and_later_reads_use_the_database() {
        let case = Case::new("legacy");
        fs::create_dir_all(case.0.join("revisions")).unwrap();
        let mut workspace = workspace::empty_workspace("test".into(), "旧形式の合成事例".into());
        for revision in 1..=2 {
            workspace.revision = revision;
            workspace.history.push(Audit {
                revision,
                unix_ms: revision,
                actor: "合成".into(),
                reason: "旧形式".into(),
                operation: "initialize".into(),
            });
            crate::recording::write_json_new(
                &case
                    .0
                    .join("revisions")
                    .join(format!("{revision:020}.json")),
                &workspace,
            )
            .unwrap();
        }
        assert_eq!(workspace::load(&case.0).unwrap().revision, 2);
        fs::rename(case.0.join("revisions"), case.0.join("legacy-backup")).unwrap();
        assert_eq!(workspace::load(&case.0).unwrap().revision, 2);
        assert_eq!(crate::backup::snapshot(&case.0).unwrap().revisions.len(), 2);
    }
}
