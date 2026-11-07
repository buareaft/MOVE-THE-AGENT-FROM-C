//! Local Windows data access. No hypervisor detection or enrollment.
use crate::{lab::reject_links, Lab, RuntimeInfo};
use anyhow::{ensure, Result};
use std::path::Path;

impl Lab {
    pub(crate) fn local_enabled(&self) -> bool {
        cfg!(windows) && self.local
    }
    pub fn runtime_info(&self) -> Result<RuntimeInfo> {
        Ok(RuntimeInfo {
            windows_supported: cfg!(windows),
            data_mode: if self.local_enabled() {
                "local"
            } else {
                "fixtures"
            }
            .into(),
            message: if self.local_enabled() {
                "本机数据；点击扫描后读取文件属性与会话元数据"
            } else {
                "生成样本；不访问本机 Agent"
            }
            .into(),
        })
    }
}
pub(crate) fn safe_local(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "只支持本地绝对路径");
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetDriveTypeW};
        let s = path
            .to_string_lossy()
            .replace("\\\\?\\", "")
            .replace('/', "\\");
        ensure!(
            s.as_bytes().get(1) == Some(&b':') && s.as_bytes().get(2) == Some(&b'\\'),
            "不访问网络或共享目录"
        );
        let root: Vec<u16> = Path::new(&s[..3])
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        ensure!(
            unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } == 3,
            "只支持本地固定磁盘"
        );
    }
    reject_links(path)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_mode_never_discovers_local_agents() -> Result<()> {
        let t = crate::lab::test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        assert!(!lab.local_enabled());
        assert!(lab.move_candidates()?.is_empty());
        assert!(lab.prepare_move("codex-sessions", "C:\\").is_err());
        assert_eq!(lab.runtime_info()?.data_mode, "fixtures");
        Ok(())
    }
    #[test]
    fn local_startup_has_no_automatic_discovery() -> Result<()> {
        if !cfg!(windows) {
            return Ok(());
        }
        let t = crate::lab::test_dir()?;
        let mut lab = Lab::at(t.path().join("lab"))?;
        lab.local = true;
        let s = lab.snapshot()?;
        assert_eq!(s.mode, "native-local");
        assert_eq!(s.file_count, 0);
        assert!(s.agents.is_empty());
        Ok(())
    }
}
