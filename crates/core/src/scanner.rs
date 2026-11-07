use crate::{
    lab::{reparse, Lab},
    models::{timestamp, FileEntry},
    Entity, Snapshot,
};
use anyhow::{ensure, Result};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) fn walk(root: &Path) -> Result<Vec<std::path::PathBuf>> {
    ensure!(
        !reparse(root)?,
        "Link traversal refused: {}",
        root.display()
    );
    let mut files = Vec::new();
    for e in fs::read_dir(root)? {
        let p = e?.path();
        ensure!(!reparse(&p)?, "Link traversal refused: {}", p.display());
        if p.is_dir() {
            files.extend(walk(&p)?);
        } else {
            files.push(p);
        }
    }
    files.sort();
    Ok(files)
}

pub(crate) fn file_info(path: &Path) -> Result<(u64, String)> {
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::{GetLastError, SetLastError, HANDLE, WIN32_ERROR},
                Storage::FileSystem::{
                    FileStandardInfo, GetCompressedFileSizeW, GetFileInformationByHandle,
                    GetFileInformationByHandleEx, BY_HANDLE_FILE_INFORMATION, FILE_STANDARD_INFO,
                },
            },
        };
        // Metadata only: do not request data-read access to credentials or locked databases.
        let f = fs::OpenOptions::new()
            .read(true)
            .access_mode(0x80)
            .share_mode(7)
            .open(path)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(f.as_raw_handle()), &mut info)?;
        }
        let allocated = if info.dwFileAttributes & (0x800 | 0x200) != 0 {
            let canonical = path.canonicalize()?;
            let wide: Vec<u16> = canonical.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut high = 0;
            unsafe {
                SetLastError(WIN32_ERROR(0));
            }
            let low = unsafe { GetCompressedFileSizeW(PCWSTR(wide.as_ptr()), Some(&mut high)) };
            ensure!(
                low != u32::MAX || unsafe { GetLastError().0 } == 0,
                "Cannot read compressed/sparse allocation"
            );
            ((high as u64) << 32) | low as u64
        } else {
            let mut standard = FILE_STANDARD_INFO::default();
            unsafe {
                GetFileInformationByHandleEx(
                    HANDLE(f.as_raw_handle()),
                    FileStandardInfo,
                    &mut standard as *mut _ as *mut std::ffi::c_void,
                    std::mem::size_of::<FILE_STANDARD_INFO>() as u32,
                )?;
            }
            ensure!(standard.AllocationSize >= 0, "Invalid allocation size");
            standard.AllocationSize as u64
        };
        Ok((
            allocated,
            format!(
                "{}:{}:{}",
                info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
            ),
        ))
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(path)?;
        Ok((m.blocks() * 512, format!("{}:{}", m.dev(), m.ino())))
    }
}
pub(crate) fn actual_volume(path: &Path) -> String {
    let Ok(p) = path.canonicalize() else {
        return "未识别".into();
    };
    let text = p.to_string_lossy();
    let text = text.strip_prefix("\\\\?\\").unwrap_or(&text);
    if text.as_bytes().get(1) == Some(&b':') {
        text[..2].to_uppercase()
    } else {
        "本地磁盘".into()
    }
}
pub(crate) fn volume_usage(files: &[FileEntry]) -> Vec<crate::DriveUsage> {
    let mut totals = BTreeMap::<String, (u64, u64, usize)>::new();
    for f in files {
        let n = totals.entry(f.volume.clone()).or_default();
        n.0 += f.bytes;
        n.1 += f.allocated;
        n.2 += 1;
    }
    totals
        .into_iter()
        .map(
            |(volume, (bytes, allocated_bytes, files))| crate::DriveUsage {
                volume,
                bytes,
                allocated_bytes,
                files,
            },
        )
        .collect()
}

fn classify(rel: &str) -> (String, String, String) {
    let v: Vec<_> = rel.split('/').collect();
    if v.first() == Some(&"projects") && v.len() > 2 {
        return ("associated".into(), v[1].into(), "workspace".into());
    }
    if v.first() == Some(&"agents") && v.len() > 3 {
        let agent = v[1].to_string();
        if v[2] == "sessions" {
            return (
                "exclusive".into(),
                v[3].trim_end_matches(".jsonl").into(),
                agent,
            );
        }
        if v[2] == "attachments" && v.len() > 4 {
            return ("exclusive".into(), v[3].into(), agent);
        }
        return ("shared".into(), agent.clone(), agent);
    }
    ("unknown".into(), "unknown".into(), "unknown".into())
}

pub(crate) fn scan(
    lab: &Lab,
    cancelled: &AtomicBool,
    progress: impl Fn(usize),
) -> Result<Snapshot> {
    let mut files = Vec::new();
    let source = lab.path("source")?;
    // Traverse explicitly. Only migration-recorded cache junctions may be substituted.
    let jobs = lab.jobs()?;
    fn visit(
        lab: &Lab,
        p: &Path,
        rel: &str,
        jobs: &[crate::Job],
        out: &mut Vec<(std::path::PathBuf, String)>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        ensure!(
            !cancel.load(Ordering::Relaxed),
            "扫描已取消；保留上次完整结果"
        );
        if reparse(p)? {
            let job = jobs
                .iter()
                .find(|j| j.source == p.display().to_string() && j.status != "rolledBack")
                .ok_or_else(|| anyhow::anyhow!("Unregistered link; refusing scan"))?;
            lab.verify_link(job)?;
            let target = lab.path(&format!("destination/{}/cache", job.agent))?;
            for f in walk(&target)? {
                let tail = f
                    .strip_prefix(&target)?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((
                    f,
                    format!("{rel}/{tail}").trim_start_matches('/').to_string(),
                ));
            }
        } else if p.is_dir() {
            for e in fs::read_dir(p)? {
                let e = e?;
                let name = e.file_name().to_string_lossy().to_string();
                visit(lab, &e.path(), &format!("{rel}/{name}"), jobs, out, cancel)?;
            }
        } else {
            out.push((p.to_path_buf(), rel.trim_start_matches('/').into()));
        }
        Ok(())
    }
    let mut paths = Vec::new();
    visit(lab, &source, "", &jobs, &mut paths, cancelled)?;
    let mut seen = HashSet::new();
    let mut warnings = Vec::new();
    let mut sessions = BTreeMap::new();
    for (p, rel) in paths {
        ensure!(
            !cancelled.load(Ordering::Relaxed),
            "扫描已取消；保留上次完整结果"
        );
        let (allocated, identity) = file_info(&p)?;
        if !seen.insert(identity.clone()) {
            continue;
        }
        let bytes = fs::metadata(&p)?.len();
        let (category, owner, agent) = classify(&rel);
        if rel.contains("/sessions/") && rel.ends_with(".jsonl") {
            let contents = fs::read_to_string(&p)?;
            let parsed = contents
                .lines()
                .next()
                .and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok());
            match parsed {
                Some(v)
                    if v["type"] == "agent_space_demo_session"
                        && v["schemaVersion"] == 1
                        && v["id"] == owner =>
                {
                    sessions.insert(
                        owner.clone(),
                        (
                            v["title"].as_str().unwrap_or("未命名模拟对话").to_string(),
                            v["project"].as_str().unwrap_or("").to_string(),
                            agent.clone(),
                        ),
                    );
                }
                _ => warnings.push(format!("无法解析 {rel}，仅统计文件大小")),
            }
        }
        files.push(FileEntry {
            path: p.display().to_string(),
            bytes,
            allocated,
            identity,
            category,
            owner,
            agent,
            volume: actual_volume(&p),
        });
        progress(files.len());
    }
    // An unrecognized transcript must not imply ownership of its attachments.
    for f in &mut files {
        if f.category == "exclusive" && !sessions.get(&f.owner).is_some_and(|s| s.2 == f.agent) {
            f.category = "unknown".into();
        }
    }
    let sum = |category: &str| {
        files
            .iter()
            .filter(|f| f.category == category)
            .map(|f| f.bytes)
            .sum::<u64>()
    };
    let entity = |id: &str,
                  name: &str,
                  agent: &str,
                  path: &str,
                  category: &str,
                  filtered: Vec<&FileEntry>,
                  count: usize,
                  note: &str| Entity {
        id: id.into(),
        name: name.into(),
        agent: agent.into(),
        path: path.into(),
        bytes: filtered.iter().map(|f| f.bytes).sum(),
        allocated: filtered.iter().map(|f| f.allocated).sum(),
        category: category.into(),
        sessions: count,
        updated: "2026-11-06 10:30 · 模拟".into(),
        note: note.into(),
    };
    let agents = [
        ("codex", "Codex · 模拟适配器"),
        ("claude", "Claude Code · 模拟适配器"),
    ]
    .iter()
    .map(|(id, name)| {
        entity(
            id,
            name,
            id,
            &format!("source/agents/{id}"),
            "shared",
            files.iter().filter(|f| f.agent == *id).collect(),
            sessions.values().filter(|x| x.2 == *id).count(),
            "仅支持项目内 fixture 格式；未接入真实 Agent",
        )
    })
    .collect();
    let projects = [
        ("atlas", "Atlas 工作台"),
        ("orbit", "Orbit 索引服务"),
        ("notes", "个人知识库"),
    ]
    .iter()
    .map(|(id, name)| {
        entity(
            id,
            name,
            "workspace",
            &format!("source/projects/{id}"),
            "associated",
            files
                .iter()
                .filter(|f| f.category == "associated" && f.owner == *id)
                .collect(),
            sessions.values().filter(|x| x.1 == *id).count(),
            "会话元数据关联；项目空间只计算一次，不推断文件由哪个对话生成",
        )
    })
    .collect();
    let sessions = sessions
        .iter()
        .map(|(id, (name, project, agent))| {
            entity(
                id,
                name,
                agent,
                &format!("source/agents/{agent}/sessions/{id}.jsonl"),
                "exclusive",
                files
                    .iter()
                    .filter(|f| f.category == "exclusive" && f.owner == *id && f.agent == *agent)
                    .collect(),
                0,
                &format!("关联项目：{project}；只计会话文件与明确归属附件，不分摊共享数据库"),
            )
        })
        .collect();
    let mut backup_bytes = 0;
    for j in &jobs {
        if Path::new(&j.backup).exists() {
            let rel = format!("backups/{}", j.id);
            for p in walk(&lab.path(&rel)?)? {
                backup_bytes += fs::metadata(p)?.len();
            }
        }
    }
    let snapshot = Snapshot {
        mode: "native-lab".into(),
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
        jobs,
        warnings,
        volumes: volume_usage(&files),
    };
    ensure!(!cancelled.load(Ordering::Relaxed), "扫描已取消");
    lab.store_snapshot(&snapshot, &files)?;
    Ok(snapshot)
}
