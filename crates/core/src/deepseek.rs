//! DeepSeek Harness metadata adapter. Only the first session record is decoded.
use anyhow::{ensure, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) const AGENT: &str = "deepseek";
const MAX_HEADER: u64 = 64 * 1024;

pub(crate) fn home(profile: &Path, configured: Option<&str>) -> PathBuf {
    let Some(value) = configured.map(str::trim).filter(|s| !s.is_empty()) else {
        return profile.join(".dsh");
    };
    if value == "~" {
        return profile.to_path_buf();
    }
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return profile.join(rest);
    }
    PathBuf::from(value)
}
pub(crate) fn current_home(profile: &Path) -> PathBuf {
    home(profile, std::env::var("DSH_HOME").ok().as_deref())
}

impl crate::Lab {
    /// Generate format fixtures only beneath this application's owned sandbox.
    pub fn seed_deepseek_fixture(&self) -> Result<serde_json::Value> {
        let _lock = self.lock()?;
        let root = self.path("deepseek-fixture")?;
        ensure!(!root.exists(), "DeepSeek 测试样本已存在");
        let home = root.join("home");
        let project = root.join("project");
        fs::create_dir_all(&project)?;
        fs::write(project.join(".agent-space-project"), b"GENERATED PROJECT")?;
        fs::write(project.join("source.ts"), b"// synthetic workspace")?;
        for (id, versions) in [
            ("dsh-short", vec![(0, false), (4, true)]),
            ("短会话", vec![(4, false)]),
            ("future", vec![(5, false)]),
        ] {
            let dir = home.join("sessions/--fixture--").join(encode_segment(id));
            fs::create_dir_all(&dir)?;
            for (version, compressed) in versions {
                let name = if version == 0 {
                    "session.jsonl".into()
                } else {
                    format!(
                        "session.v{version}.jsonl{}",
                        if compressed { ".zstd" } else { "" }
                    )
                };
                let data=serde_json::json!({"type":"session","version":version,"id":id,"createdAt":1794000000000u64,"cwd":project,"isSeeded":false,"delegationDepth":0}).to_string()+"\nSYNTHETIC_MESSAGE_BODY_NOT_ANALYZED\n";
                fs::write(
                    dir.join(name),
                    if compressed {
                        zstd::stream::encode_all(data.as_bytes(), 1)?
                    } else {
                        data.into_bytes()
                    },
                )?;
            }
        }
        fs::create_dir(home.join("cache"))?;
        fs::write(home.join("cache/models.bin"), vec![0x42; 128 * 1024])?;
        fs::create_dir_all(home.join("storages/session_projcache"))?;
        fs::write(
            home.join("storages/session_projcache/opaque.json"),
            b"SYNTHETIC_SHARED_NOT_PARSED",
        )?;
        fs::write(home.join("auth.json"), b"SYNTHETIC_AUTH_NOT_PARSED")?;
        Ok(
            serde_json::json!({"home":home,"project":project,"recognizedSessions":2,"unknownGenerations":1}),
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Session {
    pub owner: String,
    pub id: String,
    pub cwd: String,
    pub created_at: String,
    pub path: String,
}
pub(crate) struct Index {
    pub files: HashMap<PathBuf, Option<Session>>,
    pub warnings: Vec<String>,
}
fn generation(name: &str) -> Option<(u64, bool)> {
    let (raw, compressed) = if let Some(raw) = name.strip_suffix(".zstd") {
        (raw, true)
    } else {
        (name, false)
    };
    if raw == "session.jsonl" {
        return Some((0, compressed));
    }
    let n = raw.strip_prefix("session.v")?.strip_suffix(".jsonl")?;
    if n.starts_with('0') || n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((n.parse().unwrap_or(u64::MAX), compressed))
}
fn encode_segment(id: &str) -> String {
    if id == "." {
        return "~002E".into();
    }
    if id == ".." {
        return "~002E~002E".into();
    }
    let mut result = String::new();
    for unit in id.encode_utf16() {
        if unit < 128 && ((unit as u8).is_ascii_alphanumeric() || b"._-".contains(&(unit as u8))) {
            result.push(char::from_u32(unit as u32).unwrap());
        } else {
            result.push_str(&format!("~{unit:04X}"));
        }
    }
    result
}
fn header(path: &Path, version: u64, compressed: bool) -> Result<(String, String, String)> {
    ensure!(version <= 4, "未知 DeepSeek 会话版本");
    let file = fs::File::open(path)?.take(4 * 1024 * 1024);
    let input: Box<dyn Read> = if compressed {
        let mut decoder = zstd::stream::read::Decoder::new(file)?.single_frame();
        decoder.window_log_max(23)?;
        Box::new(decoder)
    } else {
        Box::new(file)
    };
    let mut line = Vec::new();
    BufReader::new(input)
        .take(MAX_HEADER + 1)
        .read_until(b'\n', &mut line)?;
    ensure!(
        !line.is_empty() && line.len() as u64 <= MAX_HEADER,
        "会话头缺失或过大"
    );
    let v: serde_json::Value = serde_json::from_slice(&line)?;
    ensure!(
        v["type"] == "session" && v["version"].as_u64() == Some(version),
        "会话头与文件版本不匹配"
    );
    let id = v["id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control))
        .ok_or_else(|| anyhow::anyhow!("无效会话标识"))?;
    let created = v["createdAt"]
        .as_u64()
        .filter(|n| *n <= 9_007_199_254_740_991)
        .ok_or_else(|| anyhow::anyhow!("无效创建时间"))?;
    let cwd = match v.get("cwd") {
        None => "",
        Some(value) => value
            .as_str()
            .filter(|s| Path::new(s).is_absolute())
            .ok_or_else(|| anyhow::anyhow!("无效工作目录"))?,
    };
    ensure!(
        path.parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == encode_segment(id).as_str()),
        "会话标识与目录不匹配"
    );
    Ok((id.into(), cwd.into(), created.to_string()))
}
pub(crate) fn index(root: &Path, paths: &[PathBuf], cancel: &AtomicBool) -> Result<Index> {
    let mut groups = BTreeMap::<PathBuf, Vec<(PathBuf, u64, bool)>>::new();
    let mut result = Index {
        files: HashMap::new(),
        warnings: Vec::new(),
    };
    for path in paths {
        ensure!(!cancel.load(Ordering::Relaxed), "扫描已取消");
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let components = rel.components().collect::<Vec<_>>();
        if components
            .first()
            .is_none_or(|c| c.as_os_str() != "sessions")
        {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.to_ascii_lowercase().ends_with(".jsonl")
            || name.to_ascii_lowercase().ends_with(".jsonl.zstd")
        {
            result.files.insert(path.clone(), None);
        }
        if components.len() != 4 {
            continue;
        }
        if let Some((version, compressed)) = generation(name) {
            groups
                .entry(path.parent().unwrap().into())
                .or_default()
                .push((path.clone(), version, compressed));
        }
    }
    for (dir, logs) in groups {
        ensure!(!cancel.load(Ordering::Relaxed), "扫描已取消");
        let max = logs.iter().map(|(_, v, _)| *v).max().unwrap();
        let current = logs
            .iter()
            .filter(|(_, v, _)| *v == max)
            .collect::<Vec<_>>();
        let session = (|| -> Result<Session> {
            ensure!(current.len() == 1, "同一版本同时存在多种会话编码");
            let (path, version, compressed) = current[0];
            let (id, cwd, created_at) = header(path, *version, *compressed)?;
            let stable = dir
                .to_string_lossy()
                .replace("\\\\?\\", "")
                .replace('\\', "/")
                .to_lowercase();
            Ok(Session {
                owner: format!("deepseek-{:x}", Sha256::digest(stable.as_bytes())),
                id,
                cwd,
                created_at,
                path: path.display().to_string(),
            })
        })();
        match session {
            Ok(session) => {
                for (path, _, _) in logs {
                    result.files.insert(path, Some(session.clone()));
                }
            }
            Err(error) => {
                if result.warnings.len() < 100 {
                    result.warnings.push(format!(
                        "DeepSeek 会话仅统计未归属文件：{}（{error}）",
                        dir.display()
                    ));
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn home_precedence_and_filename_encoding() {
        let profile = Path::new("C:\\Users\\Fixture");
        assert_eq!(home(profile, None), profile.join(".dsh"));
        assert_eq!(home(profile, Some("   ")), profile.join(".dsh"));
        assert_eq!(home(profile, Some("~\\custom")), profile.join("custom"));
        assert_eq!(
            home(profile, Some("E:\\Harness")),
            PathBuf::from("E:\\Harness")
        );
        assert_eq!(encode_segment("测试/😀"), "~6D4B~8BD5~002F~D83D~DE00");
        assert!(generation("session.v01.jsonl").is_none());
        assert_eq!(generation("session.v12.jsonl.zstd"), Some((12, true)));
    }
    #[test]
    fn duplicate_encodings_identity_mismatch_and_cancellation_are_not_guessed() -> Result<()> {
        let t = crate::lab::test_dir()?;
        let root = t.path().join("dsh");
        let dir = root.join("sessions/_no-cwd/a");
        fs::create_dir_all(&dir)?;
        let header = b"{\"type\":\"session\",\"version\":4,\"id\":\"a\",\"createdAt\":1}\n";
        let raw = dir.join("session.v4.jsonl");
        let compressed = dir.join("session.v4.jsonl.zstd");
        fs::write(&raw, header)?;
        fs::write(&compressed, zstd::stream::encode_all(&header[..], 1)?)?;
        let paths = vec![raw.clone(), compressed];
        let i = index(&root, &paths, &AtomicBool::new(false))?;
        assert!(i.files.values().all(Option::is_none));
        fs::write(
            &raw,
            b"{\"type\":\"session\",\"version\":4,\"id\":\"different\",\"createdAt\":1}\n",
        )?;
        assert!(index(&root, &[raw], &AtomicBool::new(false))?
            .files
            .values()
            .all(Option::is_none));
        assert!(index(&root, &paths, &AtomicBool::new(true)).is_err());
        Ok(())
    }
    #[test]
    fn versions_compression_and_unknown_headers_are_bounded() -> Result<()> {
        let t = crate::lab::test_dir()?;
        let root = t.path().join("dsh");
        let mut paths = Vec::new();
        for (id, version, compressed) in [
            ("plain", 0, false),
            ("compressed", 4, true),
            ("future", 999, false),
            ("bad", 4, true),
        ] {
            let dir = root.join("sessions/_no-cwd").join(id);
            fs::create_dir_all(&dir)?;
            let name = if version == 0 {
                "session.jsonl".into()
            } else {
                format!(
                    "session.v{version}.jsonl{}",
                    if compressed { ".zstd" } else { "" }
                )
            };
            let p = dir.join(name);
            let data=format!("{{\"type\":\"session\",\"version\":{version},\"id\":\"{id}\",\"createdAt\":1}}\nNOT A JSON MESSAGE BODY");
            fs::write(
                &p,
                if id == "bad" {
                    b"corrupt zstd".to_vec()
                } else if compressed {
                    zstd::stream::encode_all(data.as_bytes(), 1)?
                } else {
                    data.into_bytes()
                },
            )?;
            paths.push(p);
        }
        let i = index(&root, &paths, &AtomicBool::new(false))?;
        assert_eq!(i.files.values().filter(|v| v.is_some()).count(), 2);
        assert_eq!(i.warnings.len(), 2);
        let dir = root.join("sessions/_no-cwd/plain");
        let too_large = dir.join("session.v4.jsonl");
        fs::write(&too_large, vec![b' '; MAX_HEADER as usize + 2])?;
        paths.push(too_large);
        assert!(index(&root, &paths, &AtomicBool::new(false))?.files[&paths[0]].is_none());
        Ok(())
    }
}
