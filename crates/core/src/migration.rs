use crate::{
    lab::{reject_links, reparse, Lab},
    models::timestamp,
    scanner::walk,
    Job, Plan,
};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

fn hashes(root: &Path) -> Result<Vec<(String, String)>> {
    let mut result = Vec::new();
    for p in walk(root)? {
        let mut f = fs::File::open(&p)?;
        let mut h = Sha256::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        result.push((
            p.strip_prefix(root)?.to_string_lossy().replace('\\', "/"),
            format!("{:x}", h.finalize()),
        ));
    }
    result.sort();
    Ok(result)
}

fn agent_id(agent: &str) -> Result<()> {
    ensure!(
        matches!(agent, "codex" | "claude"),
        "Only built-in fixture adapters are allowed"
    );
    Ok(())
}

impl Lab {
    pub fn plan(&self, agent: &str) -> Result<Plan> {
        ensure!(
            !self.local_enabled(),
            "真实 Agent 当前仅开放只读分析；目录迁移仍限模拟测试区"
        );
        agent_id(agent)?;
        let source = self.path(&format!("source/agents/{agent}/cache"))?;
        ensure!(source.is_dir(), "请先扫描模拟数据；或该目录已迁移");
        let destination = self.path(&format!("destination/{agent}/cache"))?;
        ensure!(!destination.exists(), "目标目录已存在，请先恢复上次演练");
        let files = walk(&source)?;
        let bytes = files.iter().try_fold(0u64, |n, p| {
            Ok::<_, anyhow::Error>(n + fs::metadata(p)?.len())
        })?;
        Ok(Plan {agent:agent.into(),source:source.display().to_string(),destination:destination.display().to_string(),bytes,file_count:files.len(),steps:vec!["检查项目测试区边界".into(),"复制缓存并逐文件 SHA-256 校验".into(),"保留原副本，建立目录 Junction".into(),"验证入口，写入恢复记录".into()],warning:"只操作项目内模拟缓存；保留原副本，因此本轮不会释放磁盘空间。来源和目标都在同一测试区，跨卷操作需要另一块本地磁盘。".into()})
    }

    fn job_paths(&self, j: &Job) -> Result<(PathBuf, PathBuf, PathBuf)> {
        agent_id(&j.agent)?;
        uuid::Uuid::parse_str(&j.id)?;
        let parent = self.path(&format!("source/agents/{}", j.agent))?;
        let source = parent.join("cache"); // source alone may be our recorded junction
        let dest = self.path(&format!("destination/{}/cache", j.agent))?;
        let backup = self.path(&format!("backups/{}", j.id))?;
        ensure!(
            j.source == source.display().to_string()
                && j.destination == dest.display().to_string()
                && j.backup == backup.display().to_string(),
            "Recorded paths do not match fixture boundary"
        );
        Ok((source, dest, backup))
    }

    pub(crate) fn verify_link(&self, j: &Job) -> Result<()> {
        let (source, dest, _) = self.job_paths(j)?;
        ensure!(reparse(&source)?, "Expected recorded junction");
        ensure!(
            source.canonicalize()? == dest.canonicalize()?,
            "Junction target mismatch"
        );
        Ok(())
    }

    pub fn execute(&self, agent: &str) -> Result<Job> {
        let _lock = self.lock()?;
        let plan = self.plan(agent)?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut job = Job {
            id: id.clone(),
            agent: agent.into(),
            source: plan.source,
            destination: plan.destination,
            backup: self.path(&format!("backups/{id}"))?.display().to_string(),
            status: "copying".into(),
            bytes: plan.bytes,
            created_at: timestamp(),
            message: "复制前已记录任务；中断后可从迁移记录恢复".into(),
            hashes: vec![],
        };
        let (source, dest, backup) = self.job_paths(&job)?;
        job.hashes = hashes(&source)?;
        self.save_job(&job)?;
        let result = (|| -> Result<()> {
            fs::create_dir_all(&dest)?;
            for (rel, _) in &job.hashes {
                let input = source.join(rel);
                let output = dest.join(rel);
                reject_links(&input)?;
                reject_links(&output)?;
                fs::create_dir_all(output.parent().unwrap())?;
                fs::copy(input, output)?;
            }
            ensure!(
                hashes(&dest)? == job.hashes && hashes(&source)? == job.hashes,
                "复制期间文件发生变化，拒绝切换"
            );
            job.status = "switching".into();
            self.save_job(&job)?;
            fs::create_dir_all(backup.parent().unwrap())?;
            fs::rename(&source, &backup)?;
            #[cfg(windows)]
            junction::create(&dest, &source)?;
            #[cfg(not(windows))]
            std::os::unix::fs::symlink(&dest, &source)?;
            self.verify_link(&job)?;
            job.status = "completed".into();
            job.message = "演练完成：入口已切换，原副本已保留；释放空间 0 B".into();
            self.save_job(&job)?;
            Ok(())
        })();
        if let Err(e) = result {
            job.status = "failed".into();
            job.message = format!("操作未完成，可尝试恢复：{e}");
            self.save_job(&job)?;
            return Err(e.context(format!("任务 {} 已保留恢复记录", job.id)));
        }
        Ok(job)
    }

    pub fn rollback(&self, id: &str) -> Result<Job> {
        let _lock = self.lock()?;
        let mut job = self
            .jobs()?
            .into_iter()
            .find(|j| j.id == id)
            .context("Unknown job")?;
        ensure!(job.status != "rolledBack", "任务已恢复");
        let (source, dest, backup) = self.job_paths(&job)?;
        // Never remove user changes. Partial copies must be a hash-matching subset.
        if dest.exists() {
            for entry in hashes(&dest)? {
                ensure!(
                    job.hashes.contains(&entry),
                    "目标文件已变化，拒绝自动恢复；原副本和目标均保留"
                );
            }
        }
        if backup.exists() {
            ensure!(
                hashes(&backup)? == job.hashes,
                "原副本发生变化，拒绝自动恢复"
            );
        }
        if fs::symlink_metadata(&source).is_ok() && reparse(&source)? {
            self.verify_link(&job)?;
            ensure!(backup.exists(), "原副本缺失，保留目标数据");
        } else if source.exists() {
            reject_links(&source)?;
            if backup.exists() {
                ensure!(
                    fs::read_dir(&source)?.next().is_none(),
                    "原入口与备份同时存在且非空，需人工检查"
                );
            } else {
                ensure!(hashes(&source)? == job.hashes, "原目录变化，拒绝恢复");
            }
        } else {
            ensure!(backup.exists(), "原入口与备份均缺失，保留目标数据");
        }
        job.status = "restoring".into();
        self.save_job(&job)?;
        if fs::symlink_metadata(&source).is_ok() && reparse(&source)? {
            #[cfg(windows)]
            junction::delete(&source)?;
            #[cfg(not(windows))]
            fs::remove_file(&source)?;
        }
        if backup.exists() {
            // Removing a Windows reparse point can leave an empty plain directory.
            if source.exists() {
                reject_links(&source)?;
                fs::remove_dir(&source)?;
            }
            fs::rename(&backup, &source)?;
        }
        // Destination was validated above, is inside the lab, and contains no links.
        if dest.exists() {
            reject_links(&dest)?;
            walk(&dest)?;
            fs::remove_dir_all(&dest)?;
        }
        job.status = "rolledBack".into();
        job.message = "已恢复原缓存目录，移除本次演练的目标副本".into();
        self.save_job(&job)?;
        Ok(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    #[test]
    fn migration_and_rollback_preserve_bytes_and_attribution() -> Result<()> {
        let t = crate::lab::test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        let before = lab.scan(&AtomicBool::new(false), |_| {})?;
        let job = lab.execute("codex")?;
        let after = lab.scan(&AtomicBool::new(false), |_| {})?;
        assert_eq!(before.total_bytes, after.total_bytes);
        assert_eq!(before.shared_bytes, after.shared_bytes);
        assert_eq!(after.backup_bytes, job.bytes);
        assert!(lab.plan("codex").is_err());
        lab.rollback(&job.id)?;
        assert_eq!(lab.scan(&AtomicBool::new(false), |_| {})?.backup_bytes, 0);
        assert!(lab.plan("codex").is_ok());
        Ok(())
    }
    #[test]
    fn changed_destination_blocks_destructive_rollback() -> Result<()> {
        let t = crate::lab::test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        lab.seed()?;
        let job = lab.execute("claude")?;
        fs::write(Path::new(&job.destination).join("new-user-file"), "keep")?;
        assert!(lab.rollback(&job.id).is_err());
        assert!(Path::new(&job.backup).exists());
        assert!(Path::new(&job.destination).join("new-user-file").exists());
        Ok(())
    }
    #[test]
    fn external_junction_is_never_followed() -> Result<()> {
        let t = crate::lab::test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        lab.seed()?;
        let outside = t.path().join("outside");
        fs::create_dir(&outside)?;
        let link = lab.path("source/unclassified/link")?;
        #[cfg(windows)]
        junction::create(&outside, &link)?;
        #[cfg(not(windows))]
        std::os::unix::fs::symlink(&outside, &link)?;
        assert!(lab.scan(&AtomicBool::new(false), |_| {}).is_err());
        assert!(lab.path("source/unclassified/link/file").is_err());
        Ok(())
    }
    #[test]
    fn interrupted_switch_can_be_recovered() -> Result<()> {
        let t = crate::lab::test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        lab.seed()?;
        let job = lab.execute("codex")?;
        #[cfg(windows)]
        junction::delete(&job.source)?;
        #[cfg(not(windows))]
        fs::remove_file(&job.source)?;
        lab.rollback(&job.id)?;
        assert!(Path::new(&job.source).is_dir());
        Ok(())
    }
}
