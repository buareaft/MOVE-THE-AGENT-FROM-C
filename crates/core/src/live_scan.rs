//! Metadata adapters for local Agents. Message bodies are never analyzed.
use crate::{
    lab::reparse,
    local::safe_local,
    models::{timestamp, FileEntry},
    scanner::{actual_volume, file_info, volume_usage},
    Entity, Lab, Snapshot,
};
use anyhow::{ensure, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
const MAX_FILES: usize = 100_000;
const MAX_LINE: u64 = 2 * 1024 * 1024;
#[derive(Clone, Deserialize)]
struct Meta {
    id: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    timestamp: String,
    #[serde(skip)]
    agent: String,
}
#[derive(Default)]
struct Rules {
    links: Vec<(PathBuf, PathBuf)>,
    excluded: Vec<PathBuf>,
}
fn key(p: &Path) -> String {
    p.to_string_lossy()
        .replace("\\\\?\\", "")
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}
fn warn(w: &mut Vec<String>, s: String) {
    if w.len() < 100 {
        w.push(s);
    }
}
fn collect(
    root: &Path,
    out: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
    cancel: &AtomicBool,
    rules: &Rules,
) -> Result<()> {
    if rules.excluded.iter().any(|p| root.starts_with(p)) {
        return Ok(());
    }
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "扫描已取消；保留上次完整索引"
    );
    if reparse(root)? {
        if let Some((_, dest)) = rules.links.iter().find(|(p, _)| key(p) == key(root)) {
            safe_local(dest)?;
            ensure!(
                root.canonicalize()? == dest.canonicalize()?,
                "登记目录链接的目标发生变化"
            );
        } else {
            warn(warnings, format!("跳过未经登记的链接：{}", root.display()));
            return Ok(());
        }
    }
    let entries = match fs::read_dir(root) {
        Ok(x) => x,
        Err(e) => {
            warn(
                warnings,
                format!("部分目录未计入：{} ({e})", root.display()),
            );
            return Ok(());
        }
    };
    for item in entries {
        ensure!(!cancel.load(Ordering::Relaxed), "扫描已取消");
        let entry = match item {
            Ok(e) => e,
            Err(e) => {
                warn(warnings, format!("目录枚举不完整：{e}"));
                continue;
            }
        };
        let p = entry.path();
        if p.file_name().is_some_and(|n| n == ".sandbox-secrets") {
            warn(
                warnings,
                "敏感内部目录 .sandbox-secrets 已排除，统计不包含此目录".into(),
            );
            continue;
        }
        let meta = match fs::symlink_metadata(&p) {
            Ok(m) => m,
            Err(e) => {
                warn(warnings, format!("属性不可读：{} ({e})", p.display()));
                continue;
            }
        };
        if meta.is_dir() || reparse(&p).unwrap_or(true) {
            collect(&p, out, warnings, cancel, rules)?;
        } else {
            out.push(p);
            ensure!(out.len() <= MAX_FILES, "达到 100000 文件上限；保留上次索引");
        }
    }
    Ok(())
}
fn codex_meta(path: &Path) -> Result<Option<Meta>> {
    let mut first = Vec::new();
    BufReader::new(fs::File::open(path)?)
        .take(MAX_LINE + 1)
        .read_until(b'\n', &mut first)?;
    if first.len() as u64 > MAX_LINE {
        return Ok(None);
    }
    let v: serde_json::Value = match serde_json::from_slice(&first) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    if v["type"] != "session_meta" {
        return Ok(None);
    }
    let mut meta: Meta = match serde_json::from_value(v["payload"].clone()) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    if uuid::Uuid::parse_str(&meta.id).is_err() {
        return Ok(None);
    }
    meta.agent = "codex".into();
    Ok(Some(meta))
}
fn claude_meta(path: &Path) -> Result<Option<Meta>> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Header {
        session_id: Option<String>,
        #[serde(default)]
        cwd: String,
        #[serde(default)]
        timestamp: String,
    }
    // Deserialize only metadata fields. Serde ignores message/content values.
    // Some versions put a file-history record before the first session header.
    let mut reader = BufReader::new(fs::File::open(path)?.take(8 * 1024 * 1024));
    for _ in 0..16 {
        let mut line = Vec::new();
        reader
            .by_ref()
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line)?;
        if line.is_empty() || line.len() as u64 > MAX_LINE {
            break;
        }
        if let Ok(h) = serde_json::from_slice::<Header>(&line) {
            if let Some(id) = h.session_id {
                if uuid::Uuid::parse_str(&id).is_ok() {
                    return Ok(Some(Meta {
                        id,
                        cwd: h.cwd,
                        timestamp: h.timestamp,
                        agent: "claude".into(),
                    }));
                }
            }
        }
    }
    Ok(None)
}
fn codex_names(root: &Path) -> HashMap<String, (String, String)> {
    let path = root.join("session_index.jsonl");
    if crate::lab::reject_links(&path).is_err() {
        return HashMap::new();
    }
    let Ok(file) = fs::File::open(path) else {
        return HashMap::new();
    };
    let mut result = HashMap::new();
    for line in BufReader::new(file.take(16 * 1024 * 1024))
        .lines()
        .map_while(std::result::Result::ok)
    {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            if let (Some(id), Some(name)) = (v["id"].as_str(), v["thread_name"].as_str()) {
                result.insert(
                    id.into(),
                    (
                        name.chars().filter(|c| !c.is_control()).take(160).collect(),
                        v["updated_at"].as_str().unwrap_or("").into(),
                    ),
                );
            }
        }
    }
    result
}
fn project_candidate(path: &Path, roots: &[(String, String, PathBuf)]) -> bool {
    safe_local(path).is_ok()
        && path.is_dir()
        && path.components().count() >= 4
        && !roots.iter().any(|(_, _, r)| path.starts_with(r))
        && [
            ".git",
            "Cargo.toml",
            "package.json",
            "pyproject.toml",
            "go.mod",
            "CMakeLists.txt",
            ".agent-space-project",
        ]
        .iter()
        .any(|name| fs::symlink_metadata(path.join(name)).is_ok())
}
pub(crate) fn scan(lab: &Lab, cancel: &AtomicBool, progress: impl Fn(usize)) -> Result<Snapshot> {
    ensure!(lab.local_enabled(), "当前模式不允许扫描本机目录");
    let profile = PathBuf::from(
        std::env::var_os("USERPROFILE").ok_or_else(|| anyhow::anyhow!("没有用户目录"))?,
    );
    let codex = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| profile.join(".codex"));
    safe_local(&codex)?;
    let mut roots = vec![("codex".into(), "Codex".into(), codex)];
    let deepseek = crate::deepseek::current_home(&profile);
    if deepseek.is_dir() {
        roots.push((
            crate::deepseek::AGENT.into(),
            "DeepSeek Harness".into(),
            deepseek,
        ));
    }
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| profile.join(".claude"));
    if safe_local(&claude).is_ok() && claude.is_dir() {
        roots.push(("claude".into(), "Claude Code".into(), claude));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        let desktop = local.join("OpenAI/Codex");
        if safe_local(&desktop).is_ok() && desktop.is_dir() {
            roots.push(("codex-desktop".into(), "Codex 桌面运行数据".into(), desktop));
        }
        let packages = local.join("Packages");
        if safe_local(&packages).is_ok() {
            if let Ok(entries) = fs::read_dir(packages) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with("OpenAI.Codex_")
                        || name.starts_with("OpenAI.ChatGPT-")
                        || name.starts_with("OpenAI.ChatGPT_")
                    {
                        for sub in ["LocalCache", "LocalState", "TempState"] {
                            let p = entry.path().join(sub);
                            if safe_local(&p).is_ok() && p.is_dir() {
                                roots.push((
                                    "chatgpt".into(),
                                    "ChatGPT/Codex 桌面本地数据".into(),
                                    p,
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    scan_roots(lab, &roots, cancel, progress)
}
fn scan_roots(
    lab: &Lab,
    roots: &[(String, String, PathBuf)],
    cancel: &AtomicBool,
    progress: impl Fn(usize),
) -> Result<Snapshot> {
    let (links, mut excluded, backup_bytes) = lab.move_scan_rules()?;
    excluded.push(lab.root.clone());
    if let Some(parent) = lab.root.parent() {
        if parent.file_name().is_some_and(|n| n == ".lab") {
            excluded.push(parent.to_path_buf());
        }
    }
    let rules = Rules { links, excluded };
    let mut files = Vec::<FileEntry>::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    let mut metas = BTreeMap::<String, (Meta, String)>::new();
    let mut titles = HashMap::new();
    for (agent, _, root) in roots {
        if let Err(error) = safe_local(root) {
            warn(
                &mut warnings,
                format!("跳过无效 Agent 根目录：{} ({error})", root.display()),
            );
            continue;
        }
        if agent == "codex" {
            titles = codex_names(root);
        }
        if !root.is_dir() {
            warn(&mut warnings, format!("目录不存在：{}", root.display()));
            continue;
        }
        let mut paths = Vec::new();
        collect(root, &mut paths, &mut warnings, cancel, &rules)?;
        paths.sort();
        let deepseek = if agent == crate::deepseek::AGENT {
            let index = crate::deepseek::index(root, &paths, cancel)?;
            for message in &index.warnings {
                warn(&mut warnings, message.clone());
            }
            Some(index)
        } else {
            None
        };
        for p in paths {
            ensure!(!cancel.load(Ordering::Relaxed), "扫描已取消");
            ensure!(files.len() < MAX_FILES, "达到文件上限，保留上次索引");
            let (allocated, identity) = match file_info(&p) {
                Ok(x) => x,
                Err(e) => {
                    warn(&mut warnings, format!("文件未计入：{} ({e})", p.display()));
                    continue;
                }
            };
            if !seen.insert(identity.clone()) {
                continue;
            }
            let bytes = match fs::metadata(&p) {
                Ok(m) => m.len(),
                Err(e) => {
                    warn(
                        &mut warnings,
                        format!("文件在扫描时变化：{} ({e})", p.display()),
                    );
                    continue;
                }
            };
            let rel = p.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
            let mut category = "shared".to_string();
            let mut owner = agent.clone();
            if let Some(index) = &deepseek {
                if let Some(session) = index.files.get(&p) {
                    category = "unknown".into();
                    if let Some(session) = session {
                        category = "exclusive".into();
                        owner = session.owner.clone();
                        metas.entry(owner.clone()).or_insert((
                            Meta {
                                id: session.id.clone(),
                                cwd: session.cwd.clone(),
                                timestamp: session.created_at.clone(),
                                agent: crate::deepseek::AGENT.into(),
                            },
                            session.path.clone(),
                        ));
                    }
                }
            }
            let transcript = rel.ends_with(".jsonl")
                && if agent == "codex" {
                    rel.starts_with("sessions/") || rel.starts_with("archived_sessions/")
                } else {
                    agent == "claude" && rel.starts_with("projects/")
                };
            if transcript {
                let parsed = if agent == "codex" {
                    codex_meta(&p)
                } else {
                    claude_meta(&p)
                };
                match parsed {
                    Ok(Some(meta)) => {
                        owner = if agent == "codex" {
                            meta.id.clone()
                        } else {
                            format!("claude:{}", meta.id)
                        };
                        category = "exclusive".into();
                        metas
                            .entry(owner.clone())
                            .or_insert((meta, p.display().to_string()));
                    }
                    _ => {
                        category = "unknown".into();
                        warn(
                            &mut warnings,
                            format!("未识别会话格式，仅统计文件大小：{rel}"),
                        );
                    }
                }
            }
            files.push(FileEntry {
                path: p.display().to_string(),
                bytes,
                allocated,
                identity,
                category,
                owner,
                agent: agent.clone(),
                volume: actual_volume(&p),
            });
            progress(files.len());
        }
    }
    let mut project_paths = BTreeMap::new();
    let mut project_sessions = HashMap::<String, usize>::new();
    let mut cwd_cache = HashMap::<String, Option<PathBuf>>::new();
    for (meta, _) in metas.values() {
        let path = cwd_cache.entry(meta.cwd.clone()).or_insert_with(|| {
            let p = PathBuf::from(&meta.cwd);
            if project_candidate(&p, roots) {
                p.canonicalize().ok().filter(|p| !p.starts_with(&lab.root))
            } else {
                None
            }
        });
        if let Some(p) = path {
            let k = key(p);
            project_paths.insert(k.clone(), p.clone());
            *project_sessions.entry(k).or_default() += 1;
        }
    }
    let mut project_roots: Vec<_> = project_paths.into_values().collect();
    project_roots.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    let mut projects = Vec::new();
    for root in project_roots {
        let id = format!("project-{:x}", Sha256::digest(key(&root).as_bytes()));
        let mut paths = Vec::new();
        collect(&root, &mut paths, &mut warnings, cancel, &rules)?;
        let mut project_bytes = 0;
        let mut project_allocated = 0;
        for p in paths {
            ensure!(!cancel.load(Ordering::Relaxed), "扫描已取消");
            ensure!(files.len() < MAX_FILES, "达到文件上限，保留上次索引");
            let Ok((allocated, identity)) = file_info(&p) else {
                warn(&mut warnings, format!("项目文件不可读：{}", p.display()));
                continue;
            };
            if !seen.insert(identity.clone()) {
                continue;
            }
            let Ok(m) = fs::metadata(&p) else {
                continue;
            };
            project_bytes += m.len();
            project_allocated += allocated;
            files.push(FileEntry {
                path: p.display().to_string(),
                bytes: m.len(),
                allocated,
                identity,
                category: "associated".into(),
                owner: id.clone(),
                agent: "workspace".into(),
                volume: actual_volume(&p),
            });
            progress(files.len());
        }
        projects.push(Entity {
            id: id.clone(),
            name: root
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            agent: "workspace".into(),
            path: root.display().to_string(),
            bytes: project_bytes,
            allocated: project_allocated,
            category: "associated".into(),
            sessions: *project_sessions.get(&key(&root)).unwrap_or(&0),
            updated: "来自会话元数据".into(),
            note: "项目去重后单列；cwd 关联不代表该对话生成了这些文件。嵌套项目独立归集。".into(),
        });
    }
    let mut owner_totals = HashMap::<String, (u64, u64)>::new();
    for f in &files {
        let total = owner_totals
            .entry(format!("{}:{}", f.agent, f.owner))
            .or_default();
        total.0 += f.bytes;
        total.1 += f.allocated;
    }
    let sessions=metas.iter().map(|(id,(m,path))|{
        let title=if m.agent=="codex"{titles.get(&m.id)}else{None};
        let label=match m.agent.as_str(){"codex"=>"Codex","claude"=>"Claude Code","deepseek"=>"DeepSeek Harness",_=>"Agent"};
        let (bytes,allocated)=owner_totals.get(&format!("{}:{}",m.agent,id)).copied().unwrap_or_default();
        Entity{id:id.clone(),name:title.map(|t|t.0.clone()).unwrap_or_else(||format!("{label} 会话 {}",m.id.chars().take(16).collect::<String>())),agent:m.agent.clone(),path:path.clone(),bytes,allocated,category:"exclusive".into(),sessions:0,updated:title.map(|t|t.1.clone()).unwrap_or_else(||m.timestamp.clone()),note:if m.agent=="deepseek" {format!("会话 ID：{}；cwd：{}。仅读取最高已知格式的会话头；独占空间包含该目录已提交的历史代文件及压缩日志，未分摊共享缓存或推断附件归属。",m.id,m.cwd)} else {format!("元数据 cwd：{}；独占大小只含识别的会话文件，未推断附件归属，不分摊共享数据库。",m.cwd)}}
    }).collect::<Vec<_>>();
    let mut defs = BTreeMap::new();
    for (id, name, root) in roots {
        defs.entry(id.clone())
            .or_insert((name.clone(), root.display().to_string()));
    }
    let agents = defs
        .into_iter()
        .map(|(id, (name, path))| Entity {
            id: id.clone(),
            name,
            agent: id.clone(),
            path,
            bytes: files
                .iter()
                .filter(|f| f.agent == id)
                .map(|f| f.bytes)
                .sum(),
            allocated: files
                .iter()
                .filter(|f| f.agent == id)
                .map(|f| f.allocated)
                .sum(),
            category: "shared".into(),
            sessions: sessions.iter().filter(|s| s.agent == id).count(),
            updated: "本机扫描".into(),
            note:
                "本地文件统计；已登记迁移入口只跟随其验证后的目标，保留副本单列。云端聊天不计入。"
                    .into(),
        })
        .collect();
    let sum = |category: &str| {
        files
            .iter()
            .filter(|f| f.category == category)
            .map(|f| f.bytes)
            .sum()
    };
    let snapshot = Snapshot {
        mode: "native-local".into(),
        root: lab.root(),
        scanned_at: timestamp(),
        total_bytes: files.iter().map(|f| f.bytes).sum(),
        allocated_bytes: files.iter().map(|f| f.allocated).sum(),
        file_count: files.len(),
        exclusive_bytes: sum("exclusive"),
        shared_bytes: sum("shared"),
        project_bytes: sum("associated"),
        unknown_bytes: sum("unknown"),
        backup_bytes,
        agents,
        projects,
        sessions,
        jobs: lab.jobs()?,
        warnings,
        volumes: volume_usage(&files),
    };
    ensure!(!cancel.load(Ordering::Relaxed), "扫描已取消");
    lab.store_snapshot(&snapshot, &files)?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deepseek_generations_are_one_session_and_keep_physical_bytes() -> Result<()> {
        let t = crate::lab::test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        let root = t.path().join("dsh");
        let session = root.join("sessions/--fixture--/short");
        let project = t.path().join("project");
        fs::create_dir_all(&session)?;
        fs::create_dir(&project)?;
        fs::write(project.join("package.json"), "{}")?;
        fs::write(
            project.join("output.bin"),
            b"PROJECT_ASSOCIATED_NOT_CONVERSATION_OWNED",
        )?;
        let header = |version| {
            serde_json::json!({"type":"session","version":version,"id":"short","createdAt":1794000000000u64,"cwd":project,"isSeeded":false,"delegationDepth":0}).to_string()+"\nUNPARSED MESSAGE BODY\n"
        };
        let old = header(0);
        let current = zstd::stream::encode_all(header(4).as_bytes(), 1)?;
        fs::write(session.join("session.jsonl"), &old)?;
        fs::write(session.join("session.v4.jsonl.zstd"), &current)?;
        fs::create_dir(root.join("cache"))?;
        fs::write(root.join("cache/models.bin"), b"SHARED_CACHE")?;
        fs::write(root.join("auth.json"), b"INVALID_JSON_NEVER_PARSED")?;
        let roots = vec![("deepseek".into(), "DeepSeek Harness".into(), root.clone())];
        let s = scan_roots(&lab, &roots, &AtomicBool::new(false), |_| {})?;
        assert_eq!(s.sessions.len(), 1);
        assert!(s.sessions[0].name.contains("DeepSeek Harness"));
        assert_eq!(s.sessions[0].bytes, (old.len() + current.len()) as u64);
        assert_eq!(s.exclusive_bytes, s.sessions[0].bytes);
        assert_eq!(s.projects.len(), 1);
        assert_eq!(s.projects[0].sessions, 1);
        assert!(s.projects[0].bytes > 0);
        assert_eq!(
            s.total_bytes,
            s.exclusive_bytes + s.shared_bytes + s.project_bytes + s.unknown_bytes
        );
        let indexed:u64=lab.db()?.query_row("SELECT SUM(bytes) FROM files WHERE category='exclusive' AND agent='deepseek' AND owner=?1",[&s.sessions[0].id],|r|r.get(0))?;
        assert_eq!(indexed, s.sessions[0].bytes);
        let future = header(5);
        fs::write(session.join("session.v5.jsonl"), &future)?;
        let s = scan_roots(&lab, &roots, &AtomicBool::new(false), |_| {})?;
        assert!(s.sessions.is_empty());
        assert_eq!(
            s.unknown_bytes,
            (old.len() + current.len() + future.len()) as u64
        );
        assert!(!s.warnings.is_empty());
        Ok(())
    }
    #[test]
    fn codex_and_claude_metadata_partition_without_body_analysis() -> Result<()> {
        let t = crate::lab::test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        let codex = t.path().join("codex");
        let claude = t.path().join("claude");
        let project = t.path().join("project");
        fs::create_dir_all(codex.join("sessions/2026/11/06"))?;
        fs::create_dir_all(claude.join("projects/project"))?;
        fs::create_dir(&project)?;
        fs::write(project.join("package.json"), "{}")?;
        fs::write(project.join("source.ts"), "source")?;
        let id = "019f0986-2794-7af1-aad1-8c96afb880f1";
        let meta = serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":project,"timestamp":"2026-11-06"}});
        fs::write(
            codex.join("sessions/2026/11/06/rollout.jsonl"),
            format!("{meta}\nNOT JSON MESSAGE BODY\n"),
        )?;
        fs::write(codex.join("auth.json"), "NOT PARSED")?;
        fs::write(
            codex.join("session_index.jsonl"),
            format!("{{\"id\":\"{id}\",\"thread_name\":\"Codex title\"}}\n"),
        )?;
        let header = serde_json::json!({"type":"user","sessionId":id,"cwd":project,"timestamp":"2026-11-06","message":{"content":"do not analyze"}});
        fs::write(
            claude.join("projects/project/session.jsonl"),
            format!("{{\"type\":\"file-history-snapshot\"}}\n{header}\nNOT JSON BODY"),
        )?;
        let roots = vec![
            ("codex".into(), "Codex".into(), codex),
            ("claude".into(), "Claude Code".into(), claude),
        ];
        let s = scan_roots(&lab, &roots, &AtomicBool::new(false), |_| {})?;
        assert_eq!(s.sessions.len(), 2);
        assert_ne!(s.sessions[0].id, s.sessions[1].id);
        assert_eq!(s.projects.len(), 1);
        assert_eq!(s.projects[0].sessions, 2);
        assert_eq!(
            s.total_bytes,
            s.exclusive_bytes + s.shared_bytes + s.project_bytes + s.unknown_bytes
        );
        assert!(s.sessions.iter().any(|e| e.name == "Codex title"));
        Ok(())
    }
}
