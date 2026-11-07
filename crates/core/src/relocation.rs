//! Journaled local Windows directory relocation.
//! A destination is always a new, owned UUID directory; existing data is never merged.
use crate::{
    lab::{reject_links, reparse},
    models::timestamp,
    scanner::file_info,
    Lab,
};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveCandidate {
    pub id: String,
    pub agent: String,
    pub label: String,
    pub source: String,
    pub available: bool,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePlan {
    pub id: String,
    pub candidate: String,
    pub agent: String,
    pub label: String,
    pub source: String,
    pub destination: String,
    pub bytes: u64,
    pub allocated_bytes: u64,
    pub file_count: usize,
    pub cross_volume: bool,
    pub warning: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveJob {
    #[serde(flatten)]
    pub plan: MovePlan,
    pub status: String,
    pub phase: String,
    pub message: String,
    pub backup: String,
    pub retained_bytes: u64,
    pub freed_bytes: u64,
    pub created_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveProgress {
    pub phase: String,
    pub files: usize,
    pub bytes: u64,
    pub total_bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Entry {
    rel: String,
    bytes: u64,
    modified: u128,
    identity: String,
    hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Manifest {
    files: Vec<Entry>,
    dirs: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    job: MoveJob,
    original: Manifest,
    restore_manifest: Option<Manifest>,
    source_identity: String,
    destination_identity: String,
    restore_identity: String,
    target_root: String,
}

fn cancelled(flag: &AtomicBool) -> Result<()> {
    ensure!(
        !flag.load(Ordering::Relaxed),
        "操作已取消；恢复记录和已有文件已保留"
    );
    Ok(())
}
fn crash_checkpoint(r: &Record, name: &str) {
    if r.job.plan.candidate == "diagnostic"
        && std::env::var("AGENT_SPACE_TEST_CRASH").is_ok_and(|v| v == name)
    {
        std::process::exit(81);
    }
}
fn hash_file(path: &Path) -> Result<String> {
    hash_file_cancellable(path, &AtomicBool::new(false))
}
fn hash_file_cancellable(path: &Path, flag: &AtomicBool) -> Result<String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("文件被占用或不可读：{}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 256 * 1024];
    loop {
        cancelled(flag)?;
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn identity(path: &Path) -> Result<String> {
    #[cfg(windows)]
    {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let f = fs::OpenOptions::new()
            .read(true)
            .access_mode(0x80)
            .share_mode(7)
            .custom_flags(0x02000000)
            .open(path)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(f.as_raw_handle()), &mut info)?;
        }
        Ok(format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::metadata(path)?;
        Ok(format!("{}:{}", m.dev(), m.ino()))
    }
}
fn plain_path(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "请输入本地绝对目录路径");
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        let text = text.strip_prefix("\\\\?\\").unwrap_or(&text);
        ensure!(
            text.as_bytes().get(1) == Some(&b':') && text.as_bytes().get(2) == Some(&b'\\'),
            "不支持网络、共享目录或设备路径"
        );
        ensure!(
            text[2..]
                .chars()
                .all(|c| !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')),
            "目录包含不支持的字符"
        );
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW},
        };
        let root: Vec<_> = std::ffi::OsStr::new(&text[..3])
            .encode_wide()
            .chain(Some(0))
            .collect();
        ensure!(
            unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == 3,
            "仅支持本地固定磁盘"
        );
        let mut filesystem = [0u16; 32];
        unsafe {
            GetVolumeInformationW(
                PCWSTR(root.as_ptr()),
                None,
                None,
                None,
                None,
                Some(&mut filesystem),
            )?;
        }
        let end = filesystem
            .iter()
            .position(|x| *x == 0)
            .unwrap_or(filesystem.len());
        ensure!(
            String::from_utf16_lossy(&filesystem[..end]) == "NTFS",
            "首发版仅支持 NTFS 文件系统"
        );
    }
    reject_links(path)?;
    Ok(())
}
fn free_bytes(path: &Path) -> Result<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDiskFreeSpaceExW};
        let p: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0;
        unsafe {
            GetDiskFreeSpaceExW(PCWSTR(p.as_ptr()), Some(&mut available), None, None)?;
        }
        Ok(available)
    }
    #[cfg(not(windows))]
    {
        Ok(fs2::available_space(path)?)
    }
}
fn same_volume(a: &Path, b: &Path) -> Result<bool> {
    Ok(identity(a)?.split(':').next() == identity(b)?.split(':').next())
}
fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        text.strip_prefix("\\\\?\\")
            .unwrap_or(&text)
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    }
    #[cfg(not(windows))]
    {
        text.trim_end_matches('/').to_string()
    }
}
fn within(path: &Path, parent: &Path) -> bool {
    let path = path_key(path);
    let parent = path_key(parent);
    let separator = if cfg!(windows) { "\\" } else { "/" };
    path == parent || path.starts_with(&(parent + separator))
}
fn validate_separate_roots(source: &Path, target: &Path) -> Result<()> {
    let source = source.canonicalize()?;
    let target = target.canonicalize()?;
    ensure!(
        !within(&target, &source) && !within(&source, &target),
        "来源和目标根目录不能相同或互相包含"
    );
    Ok(())
}
fn supported_file(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        };
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::HANDLE,
                Storage::FileSystem::{
                    FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard,
                    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, WIN32_FIND_STREAM_DATA,
                },
            },
        };
        let m = fs::metadata(path)?;
        ensure!(
            m.file_attributes() & (0x4000 | 0x200 | 0x400) == 0,
            "不迁移加密、稀疏或链接文件：{}",
            path.display()
        );
        let f = fs::OpenOptions::new()
            .read(true)
            .access_mode(0x80)
            .share_mode(7)
            .open(path)?;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(f.as_raw_handle()), &mut info)?;
        }
        ensure!(
            info.nNumberOfLinks == 1,
            "目录含硬链接，不能保证跨盘语义，已拒绝迁移"
        );
        let canonical = path.canonicalize()?;
        let p: Vec<_> = canonical.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut stream = WIN32_FIND_STREAM_DATA::default();
        if let Ok(handle) = unsafe {
            FindFirstStreamW(
                PCWSTR(p.as_ptr()),
                FindStreamInfoStandard,
                &mut stream as *mut _ as *mut _,
                None,
            )
        } {
            let result = (|| -> Result<()> {
                loop {
                    let end = stream
                        .cStreamName
                        .iter()
                        .position(|c| *c == 0)
                        .unwrap_or(stream.cStreamName.len());
                    ensure!(
                        String::from_utf16_lossy(&stream.cStreamName[..end]) == "::$DATA",
                        "文件含附加数据流，已拒绝迁移"
                    );
                    if unsafe { FindNextStreamW(handle, &mut stream as *mut _ as *mut _) }.is_err()
                    {
                        break;
                    }
                }
                Ok(())
            })();
            unsafe {
                let _ = FindClose(handle);
            }
            result?;
        }
    }
    Ok(())
}
fn inventory(root: &Path, hashes: bool, flag: &AtomicBool) -> Result<Manifest> {
    plain_path(root)?;
    ensure!(root.is_dir(), "来源目录不存在");
    fn visit(
        root: &Path,
        dir: &Path,
        hashes: bool,
        flag: &AtomicBool,
        out: &mut Manifest,
    ) -> Result<()> {
        cancelled(flag)?;
        for item in fs::read_dir(dir)? {
            cancelled(flag)?;
            let p = item?.path();
            ensure!(
                !reparse(&p)?,
                "目录含未经登记的链接，已拒绝迁移：{}",
                p.display()
            );
            let rel = p
                .strip_prefix(root)?
                .to_str()
                .context("不支持的路径编码")?
                .replace('\\', "/");
            ensure!(
                Path::new(&rel)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_))),
                "非法相对路径"
            );
            if p.is_dir() {
                out.dirs.push(rel);
                visit(root, &p, hashes, flag, out)?;
            } else {
                ensure!(p.is_file(), "仅支持普通文件");
                supported_file(&p)?;
                let m = fs::metadata(&p)?;
                out.files.push(Entry {
                    rel,
                    bytes: m.len(),
                    modified: m
                        .modified()?
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos(),
                    identity: identity(&p)?,
                    hash: if hashes {
                        hash_file_cancellable(&p, flag)?
                    } else {
                        String::new()
                    },
                });
                ensure!(out.files.len() <= 100_000, "迁移目录超过 100000 文件上限");
            }
        }
        Ok(())
    }
    let mut result = Manifest {
        files: vec![],
        dirs: vec![],
    };
    visit(root, root, hashes, flag, &mut result)?;
    result.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    result.dirs.sort();
    Ok(result)
}
fn content_matches(actual: &Manifest, expected: &Manifest, subset: bool) -> bool {
    (subset || actual.files.len() == expected.files.len() && actual.dirs == expected.dirs)
        && actual
            .dirs
            .iter()
            .all(|d| expected.dirs.binary_search(d).is_ok())
        && actual.files.iter().all(|f| {
            expected
                .files
                .binary_search_by(|e| e.rel.cmp(&f.rel))
                .is_ok_and(|i| {
                    expected.files[i].bytes == f.bytes && expected.files[i].hash == f.hash
                })
        })
}
fn copy_acl(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::{LocalFree, HLOCAL},
                Security::{
                    Authorization::{GetNamedSecurityInfoW, SetNamedSecurityInfoW, SE_FILE_OBJECT},
                    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
                    PSECURITY_DESCRIPTOR,
                },
            },
        };
        let source_path = source.canonicalize()?;
        let destination_path = destination.canonicalize()?;
        let a: Vec<_> = source_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let b: Vec<_> = destination_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let mut dacl = std::ptr::null_mut();
        unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(a.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                &mut descriptor,
            )
            .ok()
            .with_context(|| format!("无法读取来源权限：{}", source.display()))?;
        }
        let result = if dacl.is_null() {
            Err(anyhow::anyhow!("来源权限不明确，拒绝创建副本"))
        } else {
            unsafe {
                SetNamedSecurityInfoW(
                    PCWSTR(b.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(dacl),
                    None,
                )
            }
            .ok()
            .map_err(Into::into)
        };
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
        result.with_context(|| format!("无法复制文件权限：{}", destination.display()))?;
    }
    Ok(())
}

pub(crate) fn blockers(agent: &str) -> Result<Vec<String>> {
    #[cfg(windows)]
    {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
        };
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)? };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut result = Vec::new();
        let mut next = unsafe { Process32FirstW(handle, &mut entry) };
        while next.is_ok() {
            let end = entry
                .szExeFile
                .iter()
                .position(|x| *x == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            let lower = name.to_lowercase();
            let blocked = if agent == crate::deepseek::AGENT {
                matches!(
                    lower.as_str(),
                    "deepseek harness.exe"
                        | "deepseek-harness.exe"
                        | "deepseekharness.exe"
                        | "deepseek.exe"
                        | "dsh-desktop.exe"
                        | "dsh.exe"
                        | "dsh-host.exe"
                        | "node.exe"
                )
            } else if agent == "claude" {
                lower == "claude.exe" || lower == "node.exe"
            } else {
                lower == "codex.exe"
                    || lower == "chatgpt.exe"
                    || (lower.starts_with("codex-") && lower != "codex-windows-sandbox-service.exe")
            };
            if blocked {
                result.push(format!("{name} (PID {})", entry.th32ProcessID));
            }
            next = unsafe { Process32NextW(handle, &mut entry) };
        }
        unsafe {
            let _ = CloseHandle(handle);
        }
        Ok(result)
    }
    #[cfg(not(windows))]
    {
        let _ = agent;
        Ok(vec![])
    }
}

impl Lab {
    fn move_gate(&self, candidate: &str) -> Result<()> {
        if candidate == "diagnostic" {
            return Ok(());
        }
        ensure!(self.local_enabled(), "样本模式不提供本机目录迁移");
        Ok(())
    }
    fn candidate_paths(&self) -> Result<Vec<(String, String, String, PathBuf)>> {
        ensure!(self.local_enabled(), "样本模式不发现本机目录");
        let profile = PathBuf::from(std::env::var_os("USERPROFILE").context("没有用户目录")?);
        let codex = std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| profile.join(".codex"));
        let claude = std::env::var_os("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| profile.join(".claude"));
        let deepseek = crate::deepseek::current_home(&profile);
        let mut rows = vec![
            (
                "deepseek-sessions".into(),
                crate::deepseek::AGENT.into(),
                "DeepSeek Harness 会话目录".into(),
                deepseek.join("sessions"),
            ),
            (
                "deepseek-cache".into(),
                crate::deepseek::AGENT.into(),
                "DeepSeek Harness 共享缓存".into(),
                deepseek.join("cache"),
            ),
            (
                "codex-sessions".into(),
                "codex".into(),
                "Codex 会话记录".into(),
                codex.join("sessions"),
            ),
            (
                "codex-archive".into(),
                "codex".into(),
                "Codex 归档会话".into(),
                codex.join("archived_sessions"),
            ),
            (
                "claude-projects".into(),
                "claude".into(),
                "Claude Code 会话目录".into(),
                claude.join("projects"),
            ),
        ];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let root = PathBuf::from(local).join("OpenAI/Codex");
            for sub in ["bin", "runtimes"] {
                rows.push((
                    format!("codex-{sub}"),
                    "codex".into(),
                    format!("Codex 桌面运行数据 / {sub}"),
                    root.join(sub),
                ));
            }
        }
        Ok(rows)
    }
    fn candidate_source(&self, id: &str) -> Result<(String, String, PathBuf)> {
        if id == "diagnostic" {
            return Ok((
                "diagnostic".into(),
                "跨盘测试样本".into(),
                self.path("relocation-fixture")?.join("source"),
            ));
        }
        let (_, agent, label, p) = self
            .candidate_paths()?
            .into_iter()
            .find(|r| r.0 == id)
            .context("不支持的迁移适配器")?;
        Ok((agent, label, p))
    }
    pub fn move_candidates(&self) -> Result<Vec<MoveCandidate>> {
        if !self.local_enabled() {
            return Ok(vec![]);
        }
        let jobs = self.move_jobs()?;
        self.candidate_paths()?
            .into_iter()
            .filter(|(_, _, _, p)| {
                p.parent().is_some_and(|parent| plain_path(parent).is_ok())
                    && fs::symlink_metadata(p).is_ok()
            })
            .map(|(id, agent, label, p)| {
                let prior = jobs.iter().any(|j| {
                    j.plan.candidate == id
                        && !matches!(j.status.as_str(), "restored" | "cancelled" | "planned")
                });
                let reason = if prior {
                    "已有迁移记录，请继续或恢复该任务".into()
                } else {
                    plain_path(&p)
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_default()
                };
                Ok(MoveCandidate {
                    id,
                    agent,
                    label,
                    source: p.display().to_string(),
                    available: reason.is_empty(),
                    reason,
                })
            })
            .collect()
    }
    pub fn seed_move_fixture(&self) -> Result<String> {
        let _lock = self.lock()?;
        let root = self.path("relocation-fixture/source")?;
        ensure!(!root.exists(), "测试样本已存在；先恢复上次测试");
        fs::create_dir_all(root.join("empty"))?;
        fs::write(root.join("sample.bin"), vec![0x37; 4 * 1024 * 1024])?;
        fs::write(root.join("多语言.txt"), "Agent Space 迁移测试\n")?;
        Ok(root.display().to_string())
    }
    fn move_save(&self, r: &Record) -> Result<()> {
        self.db()?.execute(
            "INSERT OR REPLACE INTO relocations VALUES (?1,?2)",
            params![r.job.plan.id, serde_json::to_string(r)?],
        )?;
        Ok(())
    }
    fn move_records(&self) -> Result<Vec<Record>> {
        let db = self.db()?;
        let mut stmt = db.prepare("SELECT json FROM relocations ORDER BY rowid DESC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn move_jobs(&self) -> Result<Vec<MoveJob>> {
        Ok(self.move_records()?.into_iter().map(|r| r.job).collect())
    }
    fn move_record(&self, id: &str) -> Result<Record> {
        uuid::Uuid::parse_str(id)?;
        self.move_records()?
            .into_iter()
            .find(|r| r.job.plan.id == id)
            .context("找不到迁移记录")
    }
    fn move_paths(&self, r: &Record) -> Result<(PathBuf, PathBuf, PathBuf, PathBuf)> {
        self.move_gate(&r.job.plan.candidate)?;
        uuid::Uuid::parse_str(&r.job.plan.id)?;
        let source = PathBuf::from(r.job.plan.source.clone());
        let (_, _, expected) = self.candidate_source(&r.job.plan.candidate)?;
        ensure!(source == expected, "记录来源与适配器不符");
        plain_path(source.parent().context("来源父目录缺失")?)?;
        let root = PathBuf::from(&r.target_root);
        if !self.local_enabled() {
            ensure!(
                root == self.path("relocation-fixture/destination")?,
                "样本模式只允许生成样本目录"
            );
        }
        plain_path(&root)?;
        let destination = root.join(format!("agent-space-{}", r.job.plan.id));
        ensure!(
            destination.to_string_lossy() == r.job.plan.destination,
            "目标与记录不符"
        );
        plain_path(&destination)?;
        let name = source
            .file_name()
            .context("来源不能是卷根目录")?
            .to_str()
            .context("来源编码无效")?;
        let backup = source.with_file_name(format!(".{name}.agent-space-{}.backup", r.job.plan.id));
        let restore =
            source.with_file_name(format!(".{name}.agent-space-{}.restore", r.job.plan.id));
        ensure!(
            backup.to_string_lossy() == r.job.backup,
            "备份路径与记录不符"
        );
        plain_path(&backup)?;
        plain_path(&restore)?;
        // Resolve the plain parent, not the source itself: after cutover the source
        // is our junction and must retain its original logical boundary.
        let origin = source
            .parent()
            .context("来源父目录缺失")?
            .canonicalize()?
            .join(name);
        let target = root.canonicalize()?;
        ensure!(
            !within(&target, &origin) && !within(&origin, &target),
            "来源和目标根目录不能相同或互相包含"
        );
        Ok((source, destination, backup, restore))
    }
    pub fn prepare_move(&self, candidate: &str, target: &str) -> Result<MovePlan> {
        self.move_gate(candidate)?;
        let _lock = self.lock()?;
        let (agent, label, source) = self.candidate_source(candidate)?;
        plain_path(&source)?;
        let target = PathBuf::from(target);
        if !self.local_enabled() {
            ensure!(
                target == self.path("relocation-fixture/destination")?,
                "样本模式只允许项目内样本迁移"
            );
        }
        plain_path(&target)?;
        ensure!(target.is_dir(), "目标根目录不存在，请先创建并选择目录");
        validate_separate_roots(&source, &target)?;
        ensure!(
            !within(&self.root, &source.canonicalize()?),
            "不能迁移程序自身或包含测试区的目录"
        );
        ensure!(
            !self
                .move_jobs()?
                .iter()
                .any(|j| j.plan.candidate == candidate
                    && !matches!(j.status.as_str(), "planned" | "restored" | "cancelled")),
            "该目录有未完成的迁移，请先继续或恢复"
        );
        let original = inventory(&source, false, &AtomicBool::new(false))?;
        let bytes = original.files.iter().map(|f| f.bytes).sum();
        let allocated_bytes = original.files.iter().try_fold(0, |n, f| {
            Ok::<_, anyhow::Error>(n + file_info(&source.join(&f.rel))?.0)
        })?;
        let reserve = original
            .files
            .iter()
            .map(|f| f.bytes.div_ceil(4096) * 4096)
            .sum::<u64>()
            + 16 * 1024 * 1024;
        ensure!(
            free_bytes(&target)? >= reserve,
            "目标磁盘空间不足，需要文件大小加 16 MB 余量"
        );
        self.check_virtual_target(&target, reserve)?;
        let id = uuid::Uuid::new_v4().to_string();
        let destination = target.join(format!("agent-space-{id}"));
        let name = source.file_name().unwrap().to_string_lossy();
        let backup = source.with_file_name(format!(".{name}.agent-space-{id}.backup"));
        let plan=MovePlan{id,candidate:candidate.into(),agent,label,source:source.display().to_string(),destination:destination.display().to_string(),bytes,allocated_bytes,file_count:original.files.len(),cross_volume:!same_volume(&source,&target)?,warning:"复制校验后切换目录入口并保留原副本；确认验证通过后可清理副本。迁移和恢复前请退出相关 Agent。不会改动凭据或配置文件。".into()};
        let record = Record {
            job: MoveJob {
                plan: plan.clone(),
                status: "planned".into(),
                phase: "planned".into(),
                message: "方案已生成，尚未修改来源".into(),
                backup: backup.display().to_string(),
                retained_bytes: 0,
                freed_bytes: 0,
                created_at: timestamp(),
            },
            original,
            restore_manifest: None,
            source_identity: identity(&source)?,
            destination_identity: String::new(),
            restore_identity: String::new(),
            target_root: target.display().to_string(),
        };
        self.move_paths(&record)?;
        self.move_save(&record)?;
        Ok(plan)
    }
    fn check_processes(&self, r: &Record) -> Result<()> {
        if r.job.plan.candidate != "diagnostic" {
            let busy = blockers(&r.job.plan.agent)?;
            ensure!(
                busy.is_empty(),
                "请先退出相关 Agent，再重试：{}",
                busy.join("、")
            );
        }
        Ok(())
    }
    fn phase(&self, r: &mut Record, phase: &str, message: &str) -> Result<()> {
        r.job.phase = phase.into();
        r.job.status = phase.into();
        r.job.message = message.into();
        self.move_save(r)?;
        crash_checkpoint(r, phase);
        Ok(())
    }
    fn fail(&self, r: &mut Record, error: &anyhow::Error) -> Result<()> {
        r.job.status = "paused".into();
        r.job.message = format!("{error:#}。文件和阶段记录已保留，可以继续或恢复");
        self.move_save(r)
    }
    fn verify_move_link(&self, r: &Record) -> Result<()> {
        let (source, dest, _, _) = self.move_paths(r)?;
        ensure!(reparse(&source)?, "来源不再是登记的 Junction");
        ensure!(
            source.canonicalize()? == dest.canonicalize()?,
            "目录链接被替换，拒绝操作"
        );
        ensure!(
            identity(&dest)? == r.destination_identity,
            "目标目录身份已变化"
        );
        Ok(())
    }
    fn copy_manifest(
        &self,
        r: &mut Record,
        source: &Path,
        dest: &Path,
        manifest: &Manifest,
        flag: &AtomicBool,
        progress: &impl Fn(MoveProgress),
        restore: bool,
    ) -> Result<()> {
        cancelled(flag)?;
        if !dest.exists() {
            fs::create_dir(dest)?;
            if restore {
                r.restore_identity = identity(dest)?;
            } else {
                r.destination_identity = identity(dest)?;
            }
            self.move_save(r)?;
        }
        if if restore {
            r.restore_identity.is_empty()
        } else {
            r.destination_identity.is_empty()
        } {
            ensure!(
                fs::read_dir(dest)?.next().is_none(),
                "未登记目标含有文件，保留来源并拒绝覆盖"
            );
            if restore {
                r.restore_identity = identity(dest)?;
            } else {
                r.destination_identity = identity(dest)?;
            }
            self.move_save(r)?;
        }
        ensure!(
            identity(dest)?
                == if restore {
                    r.restore_identity.clone()
                } else {
                    r.destination_identity.clone()
                },
            "目标目录身份已变化"
        );
        copy_acl(source, dest)?;
        // Check existing completed files; interrupted .part files are owned scratch files.
        for item in fs::read_dir(dest)? {
            let p = item?.path();
            ensure!(!reparse(&p)?, "目标中存在未知链接");
        }
        let mut files = 0;
        let mut bytes = 0;
        for d in &manifest.dirs {
            let p = dest.join(d);
            reject_links(&p)?;
            if !p.exists() {
                fs::create_dir_all(&p)?;
                copy_acl(&source.join(d), &p)?;
            }
        }
        for entry in &manifest.files {
            cancelled(flag)?;
            let input = source.join(&entry.rel);
            let output = dest.join(&entry.rel);
            reject_links(&input)?;
            reject_links(&output)?;
            if output.exists() {
                ensure!(
                    hash_file(&output)? == entry.hash,
                    "目标文件被修改，拒绝覆盖：{}",
                    entry.rel
                );
            } else {
                let temp = output.with_file_name(format!(".agent-space-{}.part", r.job.plan.id));
                reject_links(&temp)?;
                let mut options = fs::OpenOptions::new();
                options.read(true);
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    options.share_mode(1);
                }
                let mut input_file = options
                    .open(&input)
                    .with_context(|| format!("来源被占用：{}", entry.rel))?;
                let existed = temp.exists();
                let mut output_file = fs::OpenOptions::new()
                    .create_new(!existed)
                    .read(true)
                    .write(true)
                    .open(&temp)?;
                copy_acl(&input, &temp)?;
                let mut hash = Sha256::new();
                let mut buffer = [0u8; 256 * 1024];
                let mut remaining = output_file.metadata()?.len();
                ensure!(remaining <= entry.bytes, "中断文件出现未知内容，拒绝覆盖");
                while remaining > 0 {
                    cancelled(flag)?;
                    let n = remaining.min(buffer.len() as u64) as usize;
                    input_file.read_exact(&mut buffer[..n])?;
                    let mut saved = vec![0; n];
                    output_file.read_exact(&mut saved)?;
                    ensure!(saved == buffer[..n], "中断文件已变化，拒绝覆盖");
                    hash.update(&buffer[..n]);
                    remaining -= n as u64;
                }
                loop {
                    cancelled(flag)?;
                    let n = input_file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    output_file.write_all(&buffer[..n])?;
                    crash_checkpoint(r, "copy-chunk");
                    hash.update(&buffer[..n]);
                    progress(MoveProgress {
                        phase: if restore { "恢复复制" } else { "复制" }.into(),
                        files,
                        bytes: bytes + output_file.stream_position()?,
                        total_bytes: r.job.plan.bytes,
                    });
                }
                output_file.sync_all()?;
                ensure!(
                    format!("{:x}", hash.finalize()) == entry.hash,
                    "复制期间来源发生变化"
                );
                let modified = fs::metadata(&input)?.modified()?;
                output_file.set_times(fs::FileTimes::new().set_modified(modified))?;
                drop(output_file);
                drop(input_file);
                fs::rename(&temp, &output)?;
                fs::set_permissions(&output, fs::metadata(&input)?.permissions())?;
            }
            fs::set_permissions(&output, fs::metadata(&input)?.permissions())?;
            files += 1;
            bytes += entry.bytes;
            progress(MoveProgress {
                phase: "校验".into(),
                files,
                bytes,
                total_bytes: r.job.plan.bytes,
            });
        }
        ensure!(
            content_matches(&inventory(dest, true, flag)?, manifest, false),
            "目标包含未知文件或校验不一致，拒绝切换"
        );
        Ok(())
    }
    pub fn execute_move(
        &self,
        id: &str,
        flag: &AtomicBool,
        progress: impl Fn(MoveProgress),
    ) -> Result<MoveJob> {
        let _lock = self.lock()?;
        let mut r = self.move_record(id)?;
        self.check_virtual_target(
            Path::new(&r.target_root),
            if matches!(r.job.phase.as_str(), "planned" | "copying") {
                r.job.plan.bytes
            } else {
                0
            },
        )?;
        let (source, dest, backup, _) = self.move_paths(&r)?;
        self.check_processes(&r)?;
        ensure!(
            !matches!(r.job.phase.as_str(), "committed" | "restored" | "cancelled"),
            "该任务已结束"
        );
        let result = (|| -> Result<()> {
            if r.job.phase == "planned" {
                ensure!(
                    identity(&source)? == r.source_identity,
                    "来源目录身份已变化"
                );
                let current = inventory(&source, false, flag)?;
                ensure!(current == r.original, "来源已变化，请重新预览迁移方案");
                ensure!(
                    free_bytes(Path::new(&r.target_root))? >= r.job.plan.bytes + 16 * 1024 * 1024,
                    "目标磁盘空间不足"
                );
                r.original = inventory(&source, true, flag)?;
                self.phase(&mut r, "copying", "正在复制，来源保持原样")?;
            }
            if r.job.phase == "copying" {
                ensure!(
                    identity(&source)? == r.source_identity,
                    "来源目录身份已变化"
                );
                ensure!(
                    content_matches(&inventory(&source, true, flag)?, &r.original, false),
                    "来源在暂停期间发生变化，拒绝切换"
                );
                let manifest = r.original.clone();
                self.copy_manifest(&mut r, &source, &dest, &manifest, flag, &progress, false)?;
                self.phase(&mut r, "copied", "副本校验通过，准备切换入口")?;
            }
            cancelled(flag)?;
            self.check_processes(&r)?;
            if r.job.phase == "copied" {
                ensure!(
                    content_matches(&inventory(&source, true, flag)?, &r.original, false),
                    "来源变化，拒绝切换"
                );
                self.phase(&mut r, "renaming", "准备保留原目录")?;
            }
            if r.job.phase == "renaming" {
                if source.exists() {
                    ensure!(!backup.exists(), "来源和备份同时存在，请检查");
                    ensure!(identity(&source)? == r.source_identity, "来源身份变化");
                    fs::rename(&source, &backup)?;
                    crash_checkpoint(&r, "after-rename");
                }
                ensure!(identity(&backup)? == r.source_identity, "原副本身份变化");
                r.job.retained_bytes = r.job.plan.bytes;
                self.phase(&mut r, "linking", "原副本已保留，准备创建入口")?;
            }
            if r.job.phase == "linking" {
                ensure!(
                    content_matches(&inventory(&backup, true, flag)?, &r.original, false),
                    "原目录切换时发生变化，保留副本以便恢复"
                );
                ensure!(
                    content_matches(&inventory(&dest, true, flag)?, &r.original, false),
                    "目标校验不一致"
                );
                if fs::symlink_metadata(&source).is_err() {
                    create_link(&dest, &source)?;
                    crash_checkpoint(&r, "after-link");
                }
                self.verify_move_link(&r)?;
                self.phase(
                    &mut r,
                    "active",
                    "入口已切换，原副本保留。验证后可清理副本或恢复原位置",
                )?;
            }
            ensure!(r.job.phase == "active", "请使用恢复入口继续当前阶段");
            Ok(())
        })();
        if let Err(e) = result {
            self.fail(&mut r, &e)?;
            return Err(e);
        }
        Ok(r.job)
    }
    pub fn finalize_move(&self, id: &str) -> Result<MoveJob> {
        self.finalize_move_cancellable(id, &AtomicBool::new(false))
    }
    pub fn finalize_move_cancellable(&self, id: &str, flag: &AtomicBool) -> Result<MoveJob> {
        let _lock = self.lock()?;
        let mut r = self.move_record(id)?;
        let (_, dest, backup, _) = self.move_paths(&r)?;
        self.check_processes(&r)?;
        ensure!(
            matches!(r.job.phase.as_str(), "active" | "purging"),
            "任务尚未完成入口验证"
        );
        self.verify_move_link(&r)?;
        ensure!(
            content_matches(&inventory(&dest, true, flag)?, &r.original, false),
            "目标已有更新；为避免丢失恢复依据，请保留原副本"
        );
        if backup.exists() {
            ensure!(
                identity(&backup)? == r.source_identity,
                "原副本目录身份变化"
            );
            ensure!(
                content_matches(
                    &inventory(&backup, true, &flag)?,
                    &r.original,
                    r.job.phase == "purging"
                ),
                "原副本发生变化，拒绝清理"
            );
        } else {
            ensure!(r.job.phase == "purging", "原副本意外缺失");
        }
        cancelled(flag)?;
        self.check_processes(&r)?;
        self.phase(&mut r, "purging", "已验证目标，正在清理原副本")?;
        if r.job.plan.candidate == "diagnostic"
            && std::env::var("AGENT_SPACE_TEST_CRASH").is_ok_and(|v| v == "purge-file")
        {
            if let Some(f) = r
                .original
                .files
                .iter()
                .find(|f| backup.join(&f.rel).exists())
            {
                fs::remove_file(backup.join(&f.rel))?;
            }
            crash_checkpoint(&r, "purge-file");
        }
        if backup.exists() {
            remove_verified(&backup, &r.original, true, flag)?;
        }
        r.job.retained_bytes = 0;
        r.job.freed_bytes = if r.job.plan.cross_volume {
            r.job.plan.allocated_bytes
        } else {
            0
        };
        self.phase(
            &mut r,
            "committed",
            "原副本已清理；目录入口继续指向目标，可将最新数据恢复到原位置",
        )?;
        Ok(r.job)
    }
    pub fn restore_move(
        &self,
        id: &str,
        flag: &AtomicBool,
        progress: impl Fn(MoveProgress),
    ) -> Result<MoveJob> {
        let _lock = self.lock()?;
        let mut r = self.move_record(id)?;
        let (source, dest, backup, restore) = self.move_paths(&r)?;
        if r.job.phase == "planned" {
            self.phase(&mut r, "cancelled", "方案已取消，来源未修改")?;
            return Ok(r.job);
        }
        self.check_processes(&r)?;
        ensure!(
            !matches!(r.job.phase.as_str(), "restored" | "cancelled"),
            "任务已恢复或取消"
        );
        let result = (|| -> Result<()> {
            // Before the source is renamed, nothing requires a destructive rollback.
            if matches!(
                r.job.phase.as_str(),
                "planned" | "copying" | "copied" | "renaming"
            ) && source.exists()
                && !reparse(&source)?
                && !backup.exists()
            {
                ensure!(identity(&source)? == r.source_identity, "来源身份已变化");
                if dest.exists() {
                    ensure!(
                        identity(&dest)? == r.destination_identity,
                        "目标目录身份变化"
                    );
                    // Partial or changed data is retained; never discard an unverified file.
                    ensure!(
                        content_matches(&inventory(&source, true, flag)?, &r.original, false),
                        "来源已变化，请先继续检查；目标副本保留"
                    );
                    clean_partials(&source, &dest, &r.original, &r.job.plan.id)?;
                    let current = inventory(&dest, true, flag)?;
                    if content_matches(&current, &r.original, true) {
                        remove_verified(&dest, &r.original, true, flag)?;
                    } else {
                        bail!("目标有未验证的中断文件或新内容，保留来源和目标，请先检查目标");
                    }
                }
                self.phase(&mut r, "cancelled", "来源一直保持原样，已取消迁移")?;
                return Ok(());
            }
            if r.job.phase == "linking" && fs::symlink_metadata(&source).is_err() {
                ensure!(identity(&backup)? == r.source_identity, "原副本身份变化");
                fs::rename(&backup, &source)?;
                r.job.retained_bytes = 0;
                self.phase(&mut r, "cancelled", "已恢复原目录；目标副本保留供检查")?;
                return Ok(());
            }
            if !matches!(
                r.job.phase.as_str(),
                "restoreCopy" | "restoreSwitch" | "restoreCleanup"
            ) {
                self.verify_move_link(&r)?;
                let current = inventory(&dest, true, flag)?;
                let required = current
                    .files
                    .iter()
                    .map(|f| f.bytes.div_ceil(4096) * 4096)
                    .sum::<u64>()
                    + 16 * 1024 * 1024;
                ensure!(
                    free_bytes(source.parent().unwrap())? >= required,
                    "来源磁盘没有足够空间恢复最新数据，入口保持不变"
                );
                if backup.exists() {
                    ensure!(identity(&backup)? == r.source_identity, "原副本身份变化");
                    ensure!(
                        content_matches(
                            &inventory(&backup, true, flag)?,
                            &r.original,
                            r.job.phase == "purging"
                        ),
                        "原副本已变化，请先检查"
                    );
                }
                r.restore_manifest = Some(current);
                self.phase(&mut r, "restoreCopy", "正在把目标最新数据复制回来源磁盘")?;
            }
            let manifest = r.restore_manifest.clone().context("恢复清单缺失")?;
            if r.job.phase == "restoreCopy" {
                self.verify_move_link(&r)?;
                ensure!(
                    content_matches(&inventory(&dest, true, flag)?, &manifest, false),
                    "目标在恢复期间变化，保留目录入口"
                );
                self.copy_manifest(&mut r, &dest, &restore, &manifest, flag, &progress, true)?;
                self.phase(&mut r, "restoreSwitch", "恢复副本校验通过，准备还原入口")?;
            }
            cancelled(flag)?;
            if r.job.phase == "restoreSwitch" {
                if !restore.exists()
                    && source.exists()
                    && !reparse(&source)?
                    && identity(&source)? == r.restore_identity
                {
                    ensure!(
                        content_matches(&inventory(&source, true, flag)?, &manifest, false),
                        "来源恢复后变化，保留副本"
                    );
                    self.phase(&mut r, "restoreCleanup", "恢复入口已完成，继续清理副本")?;
                }
            }
            if r.job.phase == "restoreSwitch" {
                ensure!(
                    identity(&restore)? == r.restore_identity,
                    "恢复副本身份变化"
                );
                ensure!(
                    content_matches(&inventory(&restore, true, flag)?, &manifest, false),
                    "恢复副本校验失败"
                );
                ensure!(
                    content_matches(&inventory(&dest, true, flag)?, &manifest, false),
                    "目标出现新变化，拒绝替换入口"
                );
                self.check_processes(&r)?;
                if fs::symlink_metadata(&source).is_ok() {
                    self.verify_move_link(&r)?;
                    delete_link(&source)?;
                    crash_checkpoint(&r, "after-unlink");
                }
                if source.exists() {
                    ensure!(
                        fs::read_dir(&source)?.next().is_none(),
                        "原入口非空，拒绝覆盖"
                    );
                    fs::remove_dir(&source)?;
                }
                fs::rename(&restore, &source)?;
                crash_checkpoint(&r, "after-restore-rename");
                self.phase(&mut r, "restoreCleanup", "原位置已恢复，正在清理已验证副本")?;
            }
            if r.job.phase == "restoreCleanup" {
                ensure!(
                    identity(&source)? == r.restore_identity,
                    "恢复后的来源身份变化"
                );
                ensure!(
                    content_matches(&inventory(&source, true, flag)?, &manifest, false),
                    "恢复后的来源发生变化，保留其他副本"
                );
                if dest.exists() {
                    ensure!(identity(&dest)? == r.destination_identity, "目标身份变化");
                    remove_verified(&dest, &manifest, true, flag)?;
                }
                if backup.exists() {
                    ensure!(identity(&backup)? == r.source_identity, "原副本身份变化");
                    remove_verified(&backup, &r.original, true, flag)?;
                }
                r.job.retained_bytes = 0;
                r.job.freed_bytes = 0;
                self.phase(
                    &mut r,
                    "restored",
                    "最新数据已恢复至原位置，已验证的多余副本已清理",
                )?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.fail(&mut r, &e)?;
            return Err(e);
        }
        Ok(r.job)
    }
    pub(crate) fn move_scan_rules(&self) -> Result<(Vec<(PathBuf, PathBuf)>, Vec<PathBuf>, u64)> {
        let mut links = Vec::new();
        let mut excluded = Vec::new();
        let mut retained = 0;
        for r in self.move_records()? {
            if r.job.plan.candidate == "diagnostic" {
                continue;
            }
            // Completed history must not require an old destination volume to
            // remain mounted, or inspect a source now used by a newer job.
            if matches!(r.job.phase.as_str(), "restored" | "cancelled" | "planned") {
                if r.job.phase != "planned" {
                    excluded.push(PathBuf::from(&r.job.plan.destination));
                    excluded.push(PathBuf::from(&r.job.backup));
                }
                continue;
            }
            let (source, dest, backup, restore) = self.move_paths(&r)?;
            excluded.extend([backup.clone(), restore, dest.clone()]);
            if backup.exists() {
                retained += r.job.plan.bytes;
            }
            if !matches!(r.job.phase.as_str(), "restored" | "cancelled" | "planned")
                && fs::symlink_metadata(&source).is_ok()
                && reparse(&source)?
            {
                self.verify_move_link(&r)?;
                links.push((source, dest));
            }
        }
        Ok((links, excluded, retained))
    }
}
fn create_link(dest: &Path, source: &Path) -> Result<()> {
    #[cfg(windows)]
    junction::create(dest, source)?;
    #[cfg(not(windows))]
    std::os::unix::fs::symlink(dest, source)?;
    Ok(())
}
fn delete_link(source: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        junction::delete(source)?;
        if source.exists() {
            ensure!(
                !reparse(source)? && fs::read_dir(source)?.next().is_none(),
                "移除链接后入口不为空，保留目录"
            );
            fs::remove_dir(source)?;
        }
    }
    #[cfg(not(windows))]
    fs::remove_file(source)?;
    Ok(())
}
fn clean_partials(source: &Path, dest: &Path, manifest: &Manifest, id: &str) -> Result<()> {
    for entry in &manifest.files {
        let output = dest.join(&entry.rel);
        if output.exists() {
            continue;
        }
        let temp = output.with_file_name(format!(".agent-space-{id}.part"));
        if !temp.exists() {
            continue;
        }
        reject_links(&temp)?;
        let size = fs::metadata(&temp)?.len();
        ensure!(size <= entry.bytes, "中断文件存在未知内容");
        let mut a = fs::File::open(source.join(&entry.rel))?;
        let mut b = fs::File::open(&temp)?;
        let mut remaining = size;
        let mut buf = [0u8; 65536];
        while remaining > 0 {
            let n = remaining.min(buf.len() as u64) as usize;
            a.read_exact(&mut buf[..n])?;
            let mut saved = vec![0; n];
            b.read_exact(&mut saved)?;
            ensure!(saved == buf[..n], "中断文件变化，拒绝清理");
            remaining -= n as u64;
        }
        drop(a);
        drop(b);
        fs::remove_file(temp)?;
    }
    Ok(())
}
fn remove_verified(
    root: &Path,
    manifest: &Manifest,
    subset: bool,
    flag: &AtomicBool,
) -> Result<()> {
    plain_path(root)?;
    let current = inventory(root, true, flag)?;
    ensure!(
        content_matches(&current, manifest, subset),
        "待清理副本包含新数据，拒绝删除"
    );
    for f in &current.files {
        cancelled(flag)?;
        let p = root.join(&f.rel);
        reject_links(&p)?;
        #[cfg(windows)]
        {
            let mut permissions = fs::metadata(&p)?.permissions();
            if permissions.readonly() {
                permissions.set_readonly(false);
                fs::set_permissions(&p, permissions)?;
            }
        }
        fs::remove_file(p)?;
    }
    let mut dirs = current.dirs;
    dirs.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for d in dirs {
        let p = root.join(d);
        reject_links(&p)?;
        fs::remove_dir(p)?;
    }
    fs::remove_dir(root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> Result<(tempfile::TempDir, Lab, MovePlan)> {
        let t = crate::lab::test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        lab.seed_move_fixture()?;
        let target = lab.path("relocation-fixture/destination")?;
        fs::create_dir(&target)?;
        let plan = lab.prepare_move("diagnostic", target.to_str().unwrap())?;
        Ok((t, lab, plan))
    }
    #[test]
    fn relocated_data_can_be_committed_and_restored_with_new_files() -> Result<()> {
        let (_t, lab, plan) = setup()?;
        let flag = AtomicBool::new(false);
        let job = lab.execute_move(&plan.id, &flag, |_| {})?;
        assert_eq!(job.phase, "active");
        assert!(reparse(Path::new(&job.plan.source))?);
        assert_eq!(job.retained_bytes, plan.bytes);
        let job = lab.finalize_move(&plan.id)?;
        assert_eq!(job.phase, "committed");
        assert_eq!(job.retained_bytes, 0);
        assert!(!Path::new(&job.backup).exists());
        fs::write(
            Path::new(&plan.source).join("after-migration.txt"),
            "new content",
        )?;
        let restored = lab.restore_move(&plan.id, &flag, |_| {})?;
        assert_eq!(restored.phase, "restored");
        assert!(!reparse(Path::new(&plan.source))?);
        assert_eq!(
            fs::read_to_string(Path::new(&plan.source).join("after-migration.txt"))?,
            "new content"
        );
        assert!(!Path::new(&plan.destination).exists());
        assert!(Path::new(&plan.source).join("empty").is_dir());
        Ok(())
    }
    #[test]
    fn cancelled_partial_copy_resumes_and_verifies_prefix() -> Result<()> {
        let (_t, lab, plan) = setup()?;
        let flag = AtomicBool::new(false);
        assert!(lab
            .execute_move(&plan.id, &flag, |p| {
                if p.phase == "复制" {
                    flag.store(true, Ordering::Relaxed);
                }
            })
            .is_err());
        assert!(!reparse(Path::new(&plan.source))?);
        flag.store(false, Ordering::Relaxed);
        let job = lab.execute_move(&plan.id, &flag, |_| {})?;
        assert_eq!(job.phase, "active");
        lab.restore_move(&plan.id, &flag, |_| {})?;
        Ok(())
    }
    #[test]
    fn cancelled_partial_copy_can_be_safely_aborted() -> Result<()> {
        let (_t, lab, plan) = setup()?;
        let flag = AtomicBool::new(false);
        assert!(lab
            .execute_move(&plan.id, &flag, |_| flag.store(true, Ordering::Relaxed))
            .is_err());
        flag.store(false, Ordering::Relaxed);
        let job = lab.restore_move(&plan.id, &flag, |_| {})?;
        assert_eq!(job.phase, "cancelled");
        assert!(!Path::new(&plan.destination).exists());
        assert!(Path::new(&plan.source).join("sample.bin").exists());
        Ok(())
    }
    #[test]
    fn preview_changes_and_unknown_targets_never_overwrite_data() -> Result<()> {
        let (_t, lab, plan) = setup()?;
        fs::write(Path::new(&plan.source).join("new.txt"), "new")?;
        assert!(lab
            .execute_move(&plan.id, &AtomicBool::new(false), |_| {})
            .is_err());
        assert!(!Path::new(&plan.destination).exists());
        lab.restore_move(&plan.id, &AtomicBool::new(false), |_| {})?;
        assert!(lab.prepare_move("diagnostic", lab.root().as_str()).is_err());
        Ok(())
    }
    #[test]
    fn updated_target_is_kept_and_restored_without_losing_updates() -> Result<()> {
        let (_t, lab, plan) = setup()?;
        let flag = AtomicBool::new(false);
        lab.execute_move(&plan.id, &flag, |_| {})?;
        fs::write(Path::new(&plan.source).join("sample.bin"), "updated")?;
        assert!(lab.finalize_move(&plan.id).is_err());
        let job = lab.restore_move(&plan.id, &flag, |_| {})?;
        assert_eq!(job.phase, "restored");
        assert_eq!(
            fs::read_to_string(Path::new(&plan.source).join("sample.bin"))?,
            "updated"
        );
        Ok(())
    }
    #[test]
    fn changed_backup_and_replaced_junction_block_cleanup() -> Result<()> {
        let (t, lab, plan) = setup()?;
        let flag = AtomicBool::new(false);
        let job = lab.execute_move(&plan.id, &flag, |_| {})?;
        fs::write(Path::new(&job.backup).join("keep.txt"), "keep")?;
        assert!(lab.finalize_move(&plan.id).is_err());
        assert!(lab.restore_move(&plan.id, &flag, |_| {}).is_err());
        assert!(Path::new(&job.backup).join("keep.txt").exists());
        delete_link(Path::new(&plan.source))?;
        let elsewhere = t.path().join("elsewhere");
        fs::create_dir(&elsewhere)?;
        create_link(&elsewhere, Path::new(&plan.source))?;
        assert!(lab.finalize_move(&plan.id).is_err());
        assert!(lab.restore_move(&plan.id, &flag, |_| {}).is_err());
        delete_link(Path::new(&plan.source))?;
        Ok(())
    }
    #[test]
    fn interrupted_rename_and_restore_switch_are_recoverable() -> Result<()> {
        let (_t, lab, plan) = setup()?;
        let flag = AtomicBool::new(false);
        let job = lab.execute_move(&plan.id, &flag, |_| {})?;
        let mut r = lab.move_record(&plan.id)?;
        delete_link(Path::new(&plan.source))?;
        r.job.phase = "linking".into();
        lab.move_save(&r)?;
        lab.execute_move(&plan.id, &flag, |_| {})?;
        let (source, dest, _, restore) = lab.move_paths(&r)?;
        let current = inventory(&dest, true, &flag)?;
        r.restore_manifest = Some(current.clone());
        r.job.phase = "restoreSwitch".into();
        lab.copy_manifest(&mut r, &dest, &restore, &current, &flag, &|_| {}, true)?;
        delete_link(&source)?;
        if source.exists() {
            fs::remove_dir(&source)?;
        }
        fs::rename(restore, &source)?;
        lab.move_save(&r)?;
        assert_eq!(lab.restore_move(&plan.id, &flag, |_| {})?.phase, "restored");
        assert!(!Path::new(&job.backup).exists());
        Ok(())
    }
    #[test]
    fn hardlinked_sources_are_rejected() -> Result<()> {
        let (_t, lab, plan) = setup()?;
        fs::hard_link(
            Path::new(&plan.source).join("sample.bin"),
            Path::new(&plan.source).join("alias.bin"),
        )?;
        assert!(lab
            .prepare_move(
                "diagnostic",
                Path::new(&plan.destination)
                    .parent()
                    .unwrap()
                    .to_str()
                    .unwrap()
            )
            .is_err());
        Ok(())
    }
    #[cfg(windows)]
    #[test]
    fn equivalent_and_overlapping_windows_paths_are_rejected() -> Result<()> {
        let (_t, _lab, plan) = setup()?;
        let source = PathBuf::from(&plan.source);
        let child = source.join("child-target");
        fs::create_dir(&child)?;
        let normal = PathBuf::from(source.to_string_lossy().trim_start_matches("\\\\?\\"));
        let uppercase = PathBuf::from(normal.to_string_lossy().to_uppercase());
        for target in [
            source.clone(),
            normal,
            uppercase,
            child,
            source.parent().unwrap().to_path_buf(),
        ] {
            assert!(
                validate_separate_roots(&source, &target).is_err(),
                "Accepted overlapping target {}",
                target.display()
            );
        }
        assert!(
            validate_separate_roots(&source, Path::new(&plan.destination).parent().unwrap())
                .is_ok()
        );
        Ok(())
    }
    #[test]
    fn completed_history_does_not_require_an_old_target_volume() -> Result<()> {
        let (_t, lab, plan) = setup()?;
        let mut record = lab.move_record(&plan.id)?;
        record.job.plan.candidate = "codex-sessions".into();
        record.job.phase = "restored".into();
        record.job.status = "restored".into();
        record.target_root = lab.path("missing-target-volume")?.display().to_string();
        lab.move_save(&record)?;
        let (links, _, retained) = lab.move_scan_rules()?;
        assert!(links.is_empty());
        assert_eq!(retained, 0);
        Ok(())
    }
    #[cfg(windows)]
    #[test]
    fn long_windows_paths_keep_dacl_and_readonly_attributes() -> Result<()> {
        let (_t, lab, plan) = setup()?;
        let mut dir = PathBuf::from(&plan.source);
        for _ in 0..7 {
            dir = dir.join("nested-directory-with-a-long-name");
        }
        fs::create_dir_all(&dir)?;
        let input = dir.join("权限.txt");
        fs::write(&input, "content")?;
        let output = dir.join("copied.txt");
        fs::write(&output, "content")?;
        let a = PathBuf::from(input.to_string_lossy().trim_start_matches("\\\\?\\"));
        let b = PathBuf::from(output.to_string_lossy().trim_start_matches("\\\\?\\"));
        copy_acl(&a, &b)?;
        let mut permissions = fs::metadata(&input)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&input, permissions)?;
        let target = Path::new(&plan.destination).parent().unwrap();
        let fresh = lab.prepare_move("diagnostic", target.to_str().unwrap())?;
        let flag = AtomicBool::new(false);
        lab.execute_move(&fresh.id, &flag, |_| {})?;
        lab.restore_move(&fresh.id, &flag, |_| {})?;
        assert!(input.is_file() && fs::metadata(&input)?.permissions().readonly());
        Ok(())
    }
}
