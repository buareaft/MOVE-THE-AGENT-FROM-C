use crate::{scanner, Job, Page, Snapshot};
use anyhow::{bail, ensure, Result};
use rusqlite::{params, Connection};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

const MARKER: &str = "agent-space-fixtures-v1\nNO REAL AGENT DATA\n";

#[derive(Clone)]
pub struct Lab {
    pub(crate) root: PathBuf,
    pub(crate) local: bool,
}

pub(crate) fn reparse(path: &Path) -> Result<bool> {
    let m = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        Ok(m.file_attributes() & 0x400 != 0)
    }
    #[cfg(not(windows))]
    {
        Ok(m.file_type().is_symlink())
    }
}

/// Reject every existing link in an absolute path, including its ancestors.
pub(crate) fn reject_links(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for c in path.components() {
        ensure!(
            !matches!(c, Component::ParentDir),
            "Parent traversal is forbidden"
        );
        current.push(c);
        if !matches!(c, Component::Normal(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(_) => ensure!(
                !reparse(&current)?,
                "Reparse point is outside the allowed boundary: {}",
                current.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

impl Lab {
    pub fn open() -> Result<Self> {
        // Runtime-relative scope also works when the portable folder is copied to a VM.
        let executable = std::env::current_exe()?;
        let mut found = None;
        for dir in executable.parent().unwrap().ancestors().take(4) {
            let marker = dir.join(".agent-space-workspace");
            reject_links(&marker)?;
            if marker.is_file() {
                ensure!(
                    fs::read_to_string(&marker)?.trim() == "agent-space-fixture-workspace-v1",
                    "Invalid workspace marker"
                );
                found = Some(dir.canonicalize()?);
                break;
            }
        }
        let project = found.ok_or_else(|| {
            anyhow::anyhow!("请将程序与 .agent-space-workspace 标记文件放在同一个便携目录中")
        })?;
        let mut lab = Self::at(project.join(".lab/agent-space"))?;
        lab.local = !std::env::args().any(|arg| arg == "--fixtures");
        Ok(lab)
    }

    // No public API accepts an arbitrary root. Tests use only temporary directories.
    pub(crate) fn at(root: PathBuf) -> Result<Self> {
        reject_links(&root)?;
        if root.exists() {
            ensure!(
                root.join(".fixture-marker").is_file(),
                "Unmarked directory; refusing initialization"
            );
            reject_links(&root.join(".fixture-marker"))?;
            ensure!(
                fs::read_to_string(root.join(".fixture-marker"))? == MARKER,
                "Invalid fixture marker"
            );
        } else {
            fs::create_dir_all(&root)?;
            fs::write(root.join(".fixture-marker"), MARKER)?;
        }
        let lab = Self {
            root: root.canonicalize()?,
            local: false,
        };
        lab.db()?;
        Ok(lab)
    }

    pub fn root(&self) -> String {
        self.root.display().to_string()
    }

    pub fn webview_dir(&self) -> Result<PathBuf> {
        let p = self.path("webview")?;
        fs::create_dir_all(&p)?;
        Ok(p)
    }

    pub(crate) fn lock(&self) -> Result<std::fs::File> {
        let f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.path("operation.lock")?)?;
        fs2::FileExt::try_lock_exclusive(&f)
            .map_err(|_| anyhow::anyhow!("测试区有任务正在运行，请稍后重试"))?;
        Ok(f)
    }

    pub(crate) fn path(&self, rel: &str) -> Result<PathBuf> {
        ensure!(!rel.is_empty(), "Empty path");
        let r = Path::new(rel);
        ensure!(
            r.components().all(|c| matches!(c, Component::Normal(_))),
            "Only relative normal components allowed"
        );
        let p = self.root.join(r);
        reject_links(&p)?;
        Ok(p)
    }

    pub(crate) fn db(&self) -> Result<Connection> {
        let path = self.path("index.sqlite")?;
        self.path("index.sqlite-wal")?;
        self.path("index.sqlite-shm")?;
        self.path("index.sqlite-journal")?;
        let mut db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS snapshots (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS files (path TEXT PRIMARY KEY, identity TEXT, bytes INTEGER, allocated INTEGER, category TEXT, owner TEXT, agent TEXT);
            CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, json TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS relocations (id TEXT PRIMARY KEY, json TEXT NOT NULL);")?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS entities (kind TEXT,id TEXT,name TEXT,agent TEXT,path TEXT,bytes INTEGER,category TEXT,sessions INTEGER,updated TEXT,json TEXT,PRIMARY KEY(kind,id)); CREATE INDEX IF NOT EXISTS entities_size ON entities(kind,bytes DESC,id);")?;
        fn has_volume(db: &Connection) -> Result<bool> {
            let mut stmt = db.prepare("PRAGMA table_info(files)")?;
            let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
            for name in names {
                if name? == "volume" {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        if !has_volume(&db)? {
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if !has_volume(&tx)? {
                tx.execute(
                    "ALTER TABLE files ADD COLUMN volume TEXT NOT NULL DEFAULT ''",
                    [],
                )?;
            }
            tx.commit()?;
        }
        Ok(db)
    }

    pub(crate) fn save_job(&self, job: &Job) -> Result<()> {
        self.db()?.execute(
            "INSERT OR REPLACE INTO jobs VALUES (?1,?2)",
            params![job.id, serde_json::to_string(job)?],
        )?;
        Ok(())
    }

    pub fn jobs(&self) -> Result<Vec<Job>> {
        let db = self.db()?;
        let mut stmt = db.prepare("SELECT json FROM jobs ORDER BY rowid DESC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }

    pub fn seed(&self) -> Result<()> {
        if self.path("source")?.exists() {
            return Ok(());
        }
        let files = [
            ("source/agents/codex/cache/runtime.bin", 3_145_728),
            ("source/agents/codex/shared/plugins.bin", 1_048_576),
            ("source/agents/claude/cache/browser.bin", 2_097_152),
            ("source/agents/claude/shared/history.db.demo", 524_288),
            ("source/projects/atlas/src/application.ts.demo", 131_072),
            (
                "source/projects/atlas/node_modules/dependency.bin",
                4_194_304,
            ),
            ("source/projects/atlas/dist/build.bin", 1_048_576),
            ("source/projects/orbit/src/index.rs.demo", 65_536),
            ("source/projects/orbit/target/build.bin", 2_097_152),
            ("source/projects/notes/content.md.demo", 262_144),
            ("source/agents/codex/attachments/c1/screenshot.bin", 786_432),
            ("source/agents/claude/attachments/h1/design.bin", 393_216),
            ("source/unclassified/unknown.bin", 65_536),
        ];
        // Staging avoids accepting partially seeded fixtures after interruption.
        let staging = self.path("seed-staging")?;
        ensure!(
            !staging.exists(),
            "Seed interrupted: inspect seed-staging before retrying"
        );
        fs::create_dir(&staging)?;
        for (name, size) in files {
            let p = staging.join(name.strip_prefix("source/").unwrap());
            fs::create_dir_all(p.parent().unwrap())?;
            fs::write(p, vec![b'a'; size])?;
        }
        let sessions = [
            ("codex", "c1", "修复 Atlas 登录状态", "atlas"),
            ("codex", "c2", "整理 API 错误处理", "atlas"),
            ("codex", "c3", "优化 Orbit 索引性能", "orbit"),
            ("claude", "h1", "设计 Atlas 设置页面", "atlas"),
            ("claude", "h2", "更新个人知识库", "notes"),
        ];
        for (agent, id, title, project) in sessions {
            let p = staging.join(format!("agents/{agent}/sessions/{id}.jsonl"));
            fs::create_dir_all(p.parent().unwrap())?;
            let row = serde_json::json!({"type":"agent_space_demo_session", "schemaVersion":1, "id":id,"title":title,"project":project,"updated":"2026-11-06 10:30"});
            fs::write(p, format!("{row}\n"))?;
        }
        fs::rename(staging, self.path("source")?)?;
        Ok(())
    }

    pub fn scan(
        &self,
        cancelled: &std::sync::atomic::AtomicBool,
        progress: impl Fn(usize),
    ) -> Result<Snapshot> {
        let _lock = self.lock()?;
        if self.local_enabled() {
            return crate::live_scan::scan(self, cancelled, progress);
        }
        self.seed()?;
        scanner::scan(self, cancelled, progress)
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let db = self.db()?;
        let json: Option<String> = db
            .query_row("SELECT json FROM snapshots WHERE id=1", [], |r| r.get(0))
            .optional()?;
        let mut s = match json {
            Some(j) => {
                let cached: Snapshot = serde_json::from_str(&j)?;
                if cached.mode
                    != if self.local_enabled() {
                        "native-local"
                    } else {
                        "native-lab"
                    }
                {
                    self.initial_snapshot()?
                } else {
                    cached
                }
            }
            None => self.initial_snapshot()?,
        };
        s.jobs = self.jobs()?;
        Ok(s)
    }

    fn initial_snapshot(&self) -> Result<Snapshot> {
        if !self.local_enabled() {
            return self.scan(&std::sync::atomic::AtomicBool::new(false), |_| {});
        }
        let s = Snapshot {
            mode: "native-local".into(),
            root: self.root(),
            scanned_at: "0".into(),
            total_bytes: 0,
            allocated_bytes: 0,
            file_count: 0,
            exclusive_bytes: 0,
            shared_bytes: 0,
            project_bytes: 0,
            unknown_bytes: 0,
            backup_bytes: 0,
            agents: vec![],
            projects: vec![],
            sessions: vec![],
            jobs: vec![],
            warnings: vec![],
            volumes: vec![],
        };
        self.store_snapshot(&s, &[])?;
        Ok(s)
    }

    pub fn list(&self, kind: &str, search: &str, offset: usize, limit: usize) -> Result<Page> {
        self.list_sorted(kind, search, offset, limit, "bytes", true)
    }

    pub fn list_sorted(
        &self,
        kind: &str,
        search: &str,
        offset: usize,
        limit: usize,
        sort: &str,
        descending: bool,
    ) -> Result<Page> {
        ensure!(
            matches!(kind, "agents" | "projects" | "sessions"),
            "Unknown entity kind"
        );
        ensure!(
            matches!(
                sort,
                "name" | "agent" | "bytes" | "category" | "sessions" | "updated"
            ),
            "Unknown sort column"
        );
        let snapshot = self.snapshot()?;
        let db = self.db()?;
        let indexed: usize = db.query_row("SELECT count(*) FROM entities", [], |r| r.get(0))?;
        if indexed == 0 {
            Self::write_entities(&db, &snapshot)?;
        }
        let clause = "kind=?1 AND instr(lower(name||' '||agent||' '||path),lower(?2))>0";
        let total = db.query_row(
            &format!("SELECT count(*) FROM entities WHERE {clause}"),
            params![kind, search],
            |r| r.get(0),
        )?;
        let order = if descending { "DESC" } else { "ASC" };
        let mut stmt=db.prepare(&format!("SELECT json FROM entities WHERE {clause} ORDER BY {sort} {order},id ASC LIMIT ?3 OFFSET ?4"))?;
        let rows = stmt.query_map(
            params![kind, search, limit.clamp(1, 100) as i64, offset as i64],
            |r| r.get::<_, String>(0),
        )?;
        let items = rows
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect::<Result<Vec<_>>>()?;
        Ok(Page { items, total })
    }

    fn write_entities(db: &Connection, snapshot: &Snapshot) -> Result<()> {
        db.execute("DELETE FROM entities", [])?;
        let mut stmt =
            db.prepare_cached("INSERT INTO entities VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")?;
        for (kind, entities) in [
            ("agents", &snapshot.agents),
            ("projects", &snapshot.projects),
            ("sessions", &snapshot.sessions),
        ] {
            for e in entities {
                stmt.execute(params![
                    kind,
                    e.id,
                    e.name,
                    e.agent,
                    e.path,
                    e.bytes,
                    e.category,
                    e.sessions,
                    e.updated,
                    serde_json::to_string(e)?
                ])?;
            }
        }
        Ok(())
    }
    pub(crate) fn store_snapshot(
        &self,
        snapshot: &Snapshot,
        files: &[crate::models::FileEntry],
    ) -> Result<()> {
        let mut db = self.db()?;
        let tx = db.transaction()?;
        tx.execute("DELETE FROM files", [])?;
        {
            let mut stmt =
                tx.prepare_cached("INSERT INTO files VALUES (?1,?2,?3,?4,?5,?6,?7,?8)")?;
            for f in files {
                stmt.execute(params![
                    f.path,
                    f.identity,
                    f.bytes,
                    f.allocated,
                    f.category,
                    f.owner,
                    f.agent,
                    f.volume
                ])?;
            }
        }
        Self::write_entities(&tx, snapshot)?;
        tx.execute(
            "INSERT OR REPLACE INTO snapshots VALUES (1,?1)",
            [serde_json::to_string(snapshot)?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn export_report(&self, format: &str) -> Result<String> {
        let snapshot = self.snapshot()?;
        let _lock = self.lock()?;
        let root = self.path("exports")?;
        fs::create_dir_all(&root)?;
        ensure!(matches!(format, "json" | "csv"), "仅支持 JSON 或 CSV 报告");
        let path = root.join(format!(
            "agent-space-{}-{}.{}",
            crate::models::timestamp(),
            uuid::Uuid::new_v4(),
            format
        ));
        let bytes = if format == "json" {
            serde_json::to_vec_pretty(
                &serde_json::json!({"version":1,"mode":snapshot.mode,"scannedAt":snapshot.scanned_at,"totalBytes":snapshot.total_bytes,"allocatedBytes":snapshot.allocated_bytes,"fileCount":snapshot.file_count,"exclusiveBytes":snapshot.exclusive_bytes,"sharedBytes":snapshot.shared_bytes,"associatedBytes":snapshot.project_bytes,"unknownBytes":snapshot.unknown_bytes,"retainedBackupBytes":snapshot.backup_bytes,"volumes":snapshot.volumes,"warnings":snapshot.warnings,"agents":snapshot.agents,"projects":snapshot.projects,"sessions":snapshot.sessions,"relocations":self.move_jobs()?}),
            )?
        } else {
            fn quote(s: &str) -> String {
                let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
                    format!("'{s}")
                } else {
                    s.to_string()
                };
                format!("\"{}\"", s.replace('"', "\"\""))
            }
            let mut text = String::from(
                "\u{feff}kind,name,agent,path,bytes,allocatedBytes,category,sessions\r\n",
            );
            for (kind, items) in [
                ("agent", snapshot.agents),
                ("project", snapshot.projects),
                ("session", snapshot.sessions),
            ] {
                for e in items {
                    text.push_str(&format!(
                        "{},{},{},{},{},{},{},{}\r\n",
                        kind,
                        quote(&e.name),
                        quote(&e.agent),
                        quote(&e.path),
                        e.bytes,
                        e.allocated,
                        quote(&e.category),
                        e.sessions
                    ));
                }
            }
            text.into_bytes()
        };
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        use std::io::Write;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(path.display().to_string())
    }
    pub fn validated_export_path(&self, path: &str) -> Result<PathBuf> {
        let p = PathBuf::from(path);
        ensure!(
            p.parent() == Some(self.path("exports")?.as_path()),
            "只能查看本程序导出的报告"
        );
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow::anyhow!("报告路径无效"))?;
        ensure!(
            name.starts_with("agent-space-") && (name.ends_with(".json") || name.ends_with(".csv")),
            "报告格式无效"
        );
        let p = self.path(&format!("exports/{name}"))?;
        ensure!(p.is_file(), "报告不存在");
        Ok(p)
    }

    /// Resolve only indexed objects, never a caller-supplied filesystem path.
    pub fn validated_entity_path(&self, kind: &str, id: &str) -> Result<PathBuf> {
        let snapshot = self.snapshot()?;
        let live = snapshot.mode == "native-local";
        ensure!(!live || self.local_enabled(), "当前模式不允许本机路径");
        let entities = match kind {
            "agents" => snapshot.agents,
            "projects" => snapshot.projects,
            "sessions" => snapshot.sessions,
            _ => bail!("Unknown entity kind"),
        };
        let entity = entities
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow::anyhow!("Unknown entity"))?;
        let indexed = PathBuf::from(&entity.path);
        let p = if !live && !indexed.is_absolute() {
            self.path(&entity.path)?
        } else {
            indexed
        };
        if live {
            crate::local::safe_local(&p)?;
        } else {
            ensure!(
                p.is_absolute() && p.starts_with(&self.root),
                "路径不在生成样本区内"
            );
            reject_links(&p)?;
        }
        ensure!(p.exists(), "索引路径已不存在，请重新扫描");
        Ok(p.canonicalize()?)
    }

    pub fn breakdown(&self, kind: &str, id: &str) -> Result<Vec<crate::Breakdown>> {
        self.breakdown_on_volume(kind, id, "")
    }

    pub fn breakdown_on_volume(
        &self,
        kind: &str,
        id: &str,
        volume: &str,
    ) -> Result<Vec<crate::Breakdown>> {
        let snapshot = self.snapshot()?;
        ensure!(
            volume.is_empty() || snapshot.volumes.iter().any(|v| v.volume == volume),
            "未知磁盘"
        );
        let collection = match kind {
            "agents" => snapshot.agents,
            "projects" => snapshot.projects,
            "sessions" => snapshot.sessions,
            _ => bail!("Unknown entity kind"),
        };
        let entity = collection
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow::anyhow!("Unknown entity"))?;
        let db = self.db()?;
        let mut stmt=db.prepare("SELECT path,bytes,category FROM files WHERE ((?1='agents' AND agent=?2 AND category!='associated') OR (?1='projects' AND category='associated' AND owner=?2) OR (?1='sessions' AND category='exclusive' AND owner=?2 AND agent=?3)) AND (?4='' OR volume=?4)")?;
        let rows = stmt.query_map(params![kind, id, entity.agent, volume], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut groups = std::collections::BTreeMap::<String, (u64, usize)>::new();
        for row in rows {
            let (path, bytes, category) = row?;
            let path = path.replace('\\', "/").to_lowercase();
            let label = if category == "unknown" {
                "未归属文件"
            } else if path.contains("/node_modules/") {
                "依赖文件"
            } else if path.contains("/target/") || path.contains("/dist/") {
                "构建产物"
            } else if path.contains("/attachments/") {
                "独占附件"
            } else if category == "exclusive" || path.contains("/sessions/") {
                "会话记录"
            } else if path.contains("/cache/") {
                "共享缓存"
            } else if path.contains("/shared/") {
                "共享组件"
            } else if category == "shared" {
                "共享运行数据"
            } else {
                "项目内容"
            };
            let e = groups.entry(label.into()).or_default();
            e.0 += bytes;
            e.1 += 1;
        }
        Ok(groups
            .into_iter()
            .map(|(label, (bytes, files))| crate::Breakdown {
                label,
                bytes,
                files,
            })
            .collect())
    }
}

use rusqlite::OptionalExtension;

#[cfg(test)]
pub(crate) fn test_dir() -> Result<tempfile::TempDir> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?
        .join(".lab/agent-space/test-runs");
    reject_links(&root)?;
    fs::create_dir_all(&root)?;
    Ok(tempfile::tempdir_in(root)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn attribution_is_a_partition_and_shared_projects_are_not_repeated() -> Result<()> {
        let t = test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        let s = lab.scan(&AtomicBool::new(false), |_| {})?;
        assert_eq!(
            s.total_bytes,
            s.exclusive_bytes + s.shared_bytes + s.project_bytes + s.unknown_bytes
        );
        assert_eq!(s.sessions.len(), 5);
        assert_eq!(
            s.volumes.iter().map(|v| v.bytes).sum::<u64>(),
            s.total_bytes
        );
        assert_eq!(
            s.volumes.iter().map(|v| v.allocated_bytes).sum::<u64>(),
            s.allocated_bytes
        );
        assert_eq!(
            s.volumes.iter().map(|v| v.files).sum::<usize>(),
            s.file_count
        );
        assert_eq!(
            s.projects
                .iter()
                .find(|p| p.id == "atlas")
                .unwrap()
                .sessions,
            3
        );
        assert_eq!(lab.list("sessions", "Atlas", 0, 1)?.total, 2);
        assert_eq!(lab.list("sessions", "", 0, 1)?.items.len(), 1);
        Ok(())
    }

    #[test]
    fn traversal_unmarked_roots_and_unknown_agents_are_rejected() -> Result<()> {
        let t = test_dir()?;
        assert!(Lab::at(t.path().to_path_buf()).is_err());
        let lab = Lab::at(t.path().join("lab"))?;
        assert!(lab.path("../outside").is_err());
        assert!(lab.path("C:\\Users\\example\\.codex").is_err());
        assert!(lab.plan("../../outside").is_err());
        Ok(())
    }

    #[test]
    fn cancellation_preserves_last_complete_snapshot() -> Result<()> {
        let t = test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        let old = lab.scan(&AtomicBool::new(false), |_| {})?;
        assert!(lab.scan(&AtomicBool::new(true), |_| {}).is_err());
        assert_eq!(lab.snapshot()?.total_bytes, old.total_bytes);
        Ok(())
    }

    #[test]
    fn unknown_session_schema_downgrades_without_claiming_attachments() -> Result<()> {
        let t = test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        lab.seed()?;
        fs::write(
            lab.path("source/agents/codex/sessions/c1.jsonl")?,
            "{\"schemaVersion\":999}",
        )?;
        let s = lab.scan(&AtomicBool::new(false), |_| {})?;
        assert_eq!(s.sessions.len(), 4);
        assert!(s.unknown_bytes >= 786432);
        assert_eq!(s.warnings.len(), 1);
        Ok(())
    }

    #[test]
    fn hardlinks_do_not_inflate_global_totals() -> Result<()> {
        let t = test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        let before = lab.scan(&AtomicBool::new(false), |_| {})?;
        fs::hard_link(
            lab.path("source/unclassified/unknown.bin")?,
            lab.path("source/unclassified/alias.bin")?,
        )?;
        let after = lab.scan(&AtomicBool::new(false), |_| {})?;
        assert_eq!(before.total_bytes, after.total_bytes);
        assert_eq!(before.file_count, after.file_count);
        let breakdown = lab.breakdown("projects", "atlas")?;
        assert_eq!(
            breakdown.iter().map(|b| b.bytes).sum::<u64>(),
            after
                .projects
                .iter()
                .find(|p| p.id == "atlas")
                .unwrap()
                .bytes
        );
        Ok(())
    }
    #[test]
    fn global_sort_precedes_pagination_and_search_is_literal() -> Result<()> {
        let t = test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        let snapshot = lab.scan(&AtomicBool::new(false), |_| {})?;
        let first = lab.list_sorted("sessions", "", 0, 1, "name", false)?;
        let second = lab.list_sorted("sessions", "", 1, 1, "name", false)?;
        assert!(first.items[0].name < second.items[0].name);
        assert_eq!(first.total, snapshot.sessions.len());
        assert_eq!(
            lab.list_sorted("sessions", "%", 0, 8, "bytes", true)?.total,
            0
        );
        assert!(lab
            .list_sorted("sessions", "", 0, 8, "bytes;DELETE FROM files", true)
            .is_err());
        Ok(())
    }
    #[test]
    fn reveal_accepts_only_indexed_paths_inside_the_current_scope() -> Result<()> {
        let t = test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        let mut snapshot = lab.scan(&AtomicBool::new(false), |_| {})?;
        let id = snapshot.sessions[0].id.clone();
        let path = lab.validated_entity_path("sessions", &id)?;
        assert!(path.starts_with(&lab.root));
        assert!(lab.validated_entity_path("sessions", "missing").is_err());
        assert!(lab.validated_entity_path("invalid", &id).is_err());
        // A stale or forged index cannot make the host inspect an outside path.
        snapshot.sessions[0].path = t.path().join("outside").display().to_string();
        lab.store_snapshot(&snapshot, &[])?;
        assert!(lab.validated_entity_path("sessions", &id).is_err());
        Ok(())
    }

    #[test]
    fn exports_include_partitions_and_escape_spreadsheet_formulas() -> Result<()> {
        let t = test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        lab.seed()?;
        let p = lab.path("source/agents/codex/sessions/c1.jsonl")?;
        let mut v: serde_json::Value = serde_json::from_str(fs::read_to_string(&p)?.trim())?;
        v["title"] = "=HYPERLINK(\"example\")".into();
        fs::write(p, format!("{v}\n"))?;
        let snapshot = lab.scan(&AtomicBool::new(false), |_| {})?;
        let json = lab.export_report("json")?;
        let report: serde_json::Value = serde_json::from_slice(&fs::read(json)?)?;
        assert_eq!(report["totalBytes"], snapshot.total_bytes);
        assert_eq!(report["sessions"].as_array().unwrap().len(), 5);
        let csv = fs::read_to_string(lab.export_report("csv")?)?;
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains("'=HYPERLINK(\"\"example\"\")"));
        Ok(())
    }
}
