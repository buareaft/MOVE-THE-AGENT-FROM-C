use crate::{lab::reject_links, Lab};
use anyhow::{ensure, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageSettings {
    pub backend: String,
    pub directory: String,
    pub image_path: String,
    pub capacity_gb: u32,
    pub drive_letter: String,
    pub auto_mount: bool,
    pub disk_id: String,
}
impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            backend: "directory".into(),
            directory: "D:\\AgentData".into(),
            image_path: "D:\\MoveYourAgent\\AgentData.vhdx".into(),
            capacity_gb: 64,
            drive_letter: "V".into(),
            auto_mount: false,
            disk_id: uuid::Uuid::new_v4().to_string(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualDiskStatus {
    pub state: String,
    pub message: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub free: u64,
    #[serde(default)]
    pub image_bytes: u64,
    #[serde(default)]
    pub backing_free: u64,
    #[serde(default)]
    pub auto_mount: bool,
}
fn validate_path(path: &str) -> Result<()> {
    let b = path.as_bytes();
    ensure!(
        b.len() > 3 && b[0].is_ascii_alphabetic() && &b[1..3] == b":\\",
        "请输入本地磁盘的绝对路径，例如 D:\\AgentData"
    );
    ensure!(
        !path
            .chars()
            .any(|c| c.is_control() || "\"<>|?*'`$;".contains(c)),
        "路径包含不支持的字符"
    );
    ensure!(
        !path[3..].contains(':')
            && path[3..]
                .split(['\\', '/'])
                .all(|s| !s.is_empty() && s != "." && s != ".." && !s.ends_with([' ', '.'])),
        "路径包含无效目录或相对路径"
    );
    Ok(())
}
impl StorageSettings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.backend.as_str(), "directory" | "vhdx"),
            "未知存储方案"
        );
        validate_path(&self.directory)?;
        validate_path(&self.image_path)?;
        ensure!(
            self.image_path.to_ascii_lowercase().ends_with(".vhdx"),
            "镜像扩展名必须为 .vhdx"
        );
        ensure!(
            (1..=2048).contains(&self.capacity_gb),
            "虚拟容量范围为 1–2048 GB"
        );
        ensure!(
            self.drive_letter.len() == 1 && matches!(self.drive_letter.as_bytes()[0], b'D'..=b'Z'),
            "盘符范围为 D–Z"
        );
        ensure!(
            !self
                .image_path
                .to_ascii_uppercase()
                .starts_with(&format!("{}:", self.drive_letter)),
            "镜像文件不能放在自身挂载盘内"
        );
        uuid::Uuid::parse_str(&self.disk_id)?;
        Ok(())
    }
}
impl Lab {
    pub fn storage_settings(&self) -> Result<StorageSettings> {
        let db = self.db()?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY,json TEXT NOT NULL)",
        )?;
        if let Some(json) = db
            .query_row("SELECT json FROM settings WHERE key='storage'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
        {
            return Ok(serde_json::from_str(&json)?);
        }
        let s = StorageSettings::default();
        db.execute(
            "INSERT OR IGNORE INTO settings VALUES ('storage',?1)",
            [serde_json::to_string(&s)?],
        )?;
        Ok(serde_json::from_str(&db.query_row(
            "SELECT json FROM settings WHERE key='storage'",
            [],
            |r| r.get::<_, String>(0),
        )?)?)
    }
    pub fn save_storage_settings(&self, settings: StorageSettings) -> Result<StorageSettings> {
        let _lock = self.lock()?;
        settings.validate()?;
        let previous = self.storage_settings()?;
        ensure!(previous.disk_id == settings.disk_id, "磁盘标识不能修改");
        // Once reserved, even an interrupted creation must keep its recovery identity.
        let reserved = self
            .db()?
            .query_row(
                "SELECT json FROM settings WHERE key='disk-reserved'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .is_some();
        ensure!(
            !reserved
                || (previous.image_path == settings.image_path
                    && previous.drive_letter == settings.drive_letter
                    && previous.capacity_gb == settings.capacity_gb),
            "磁盘已创建或创建待恢复，不能修改镜像位置、容量或盘符"
        );
        // Auto-mount is changed only by its explicit privileged action.
        ensure!(
            previous.auto_mount == settings.auto_mount,
            "请通过自动挂载按钮修改登录任务"
        );
        self.db()?.execute(
            "UPDATE settings SET json=?1 WHERE key='storage'",
            [serde_json::to_string(&settings)?],
        )?;
        Ok(settings)
    }
    pub fn virtual_disk(&self, action: &str) -> Result<VirtualDiskStatus> {
        let _lock = self.lock()?;
        self.disk_action(action)
    }
    fn disk_action(&self, action: &str) -> Result<VirtualDiskStatus> {
        ensure!(
            self.local_enabled(),
            "虚拟磁盘操作仅在 Windows 本机模式提供；测试区不会操作磁盘"
        );
        ensure!(
            matches!(
                action,
                "status" | "create" | "mount" | "detach" | "enable-auto" | "disable-auto"
            ),
            "未知磁盘操作"
        );
        let mut settings = self.storage_settings()?;
        settings.validate()?;
        reject_links(std::path::Path::new(&settings.image_path))?;
        let active = self.move_jobs()?.iter().any(|j| {
            !matches!(j.phase.as_str(), "restored" | "cancelled")
                && j.plan
                    .destination
                    .to_ascii_uppercase()
                    .starts_with(&format!("{}:\\", settings.drive_letter))
        });
        ensure!(
            !(active && matches!(action, "detach" | "disable-auto")),
            "仍有迁移目录使用此磁盘，请先恢复原位置；不能卸载或关闭自动挂载"
        );
        let result = self.path(&format!("disk-result-{}.json", uuid::Uuid::new_v4()))?;
        let request = serde_json::json!({"settings":settings,"action":action,"result":result.to_string_lossy().trim_start_matches(r"\\?\")});
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(include_bytes!("virtual-disk.ps1"))?;
        let body = STANDARD.encode(gzip.finish()?);
        let source=format!("$request = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{}')) | ConvertFrom-Json\n$z=[IO.MemoryStream]::new([Convert]::FromBase64String('{body}'));$gz=[IO.Compression.GZipStream]::new($z,[IO.Compression.CompressionMode]::Decompress);$rd=[IO.StreamReader]::new($gz,[Text.Encoding]::UTF8);$diskBody=$rd.ReadToEnd();$rd.Dispose(); & ([ScriptBlock]::Create($diskBody))",STANDARD.encode(serde_json::to_vec(&request)?));
        let encoded = STANDARD.encode(
            source
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        ensure!(
            encoded.len() < 30000,
            "磁盘配置路径过长，超出 Windows 命令长度限制"
        );
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let shell = std::path::PathBuf::from(
                std::env::var_os("SystemRoot")
                    .ok_or_else(|| anyhow::anyhow!("SystemRoot unavailable"))?,
            )
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
            let output = std::process::Command::new(shell)
                .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
                .creation_flags(0x08000000)
                .output()?;
            if action == "create" {
                let marker = std::path::PathBuf::from(format!(
                    "{}.move-your-agent.json",
                    settings.image_path
                ));
                reject_links(&marker)?;
                if marker.is_file() && std::fs::metadata(&marker)?.len() < 65536 {
                    let text = std::fs::read_to_string(&marker)?;
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(
                        text.trim_start_matches('\u{feff}'),
                    ) {
                        if value["id"] == settings.disk_id && value["image"] == settings.image_path
                        {
                            self.db()?.execute(
                                "INSERT OR IGNORE INTO settings VALUES ('disk-reserved',?1)",
                                [serde_json::to_string(&settings)?],
                            )?;
                        }
                    }
                }
            }
            reject_links(&result)?;
            let answer = if result.is_file() {
                let data = std::fs::read_to_string(&result)?;
                std::fs::remove_file(&result)?;
                data
            } else {
                anyhow::bail!(
                    "磁盘操作未完成（可能取消了管理员授权）：{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            };
            let value: serde_json::Value =
                serde_json::from_str(answer.trim_start_matches('\u{feff}'))?;
            ensure!(
                value["ok"] == true,
                "{}",
                value["error"].as_str().unwrap_or("磁盘操作失败")
            );
            let status: VirtualDiskStatus = serde_json::from_value(value["status"].clone())?;
            if matches!(action, "enable-auto" | "disable-auto" | "status") {
                settings.auto_mount = status.auto_mount;
                self.db()?.execute(
                    "UPDATE settings SET json=?1 WHERE key='storage'",
                    [serde_json::to_string(&settings)?],
                )?;
            }
            return Ok(status);
        }
        #[cfg(not(windows))]
        {
            let _ = (encoded, result);
            anyhow::bail!("Windows only")
        }
    }
    pub fn configured_move_target(&self) -> Result<String> {
        let s = self.storage_settings()?;
        if s.backend == "directory" {
            return Ok(s.directory);
        }
        let status = self.virtual_disk("status")?;
        ensure!(
            status.state == "mounted",
            "虚拟磁盘尚未就绪，请先在设置中创建或挂载"
        );
        Ok(status.target)
    }
    // Caller owns the relocation lock. Verify both the mounted identity and the
    // backing volume's space; the virtual capacity alone is not sufficient.
    pub(crate) fn check_virtual_target(
        &self,
        target: &std::path::Path,
        required: u64,
    ) -> Result<()> {
        if !self.local_enabled() {
            return Ok(());
        }
        let s = self.storage_settings()?;
        let target = target
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_ascii_uppercase();
        if !target.starts_with(&format!("{}:\\", s.drive_letter)) {
            return Ok(());
        }
        let reserved = self
            .db()?
            .query_row(
                "SELECT json FROM settings WHERE key='disk-reserved'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .is_some();
        if !reserved {
            return Ok(());
        }
        let status = self.disk_action("status")?;
        ensure!(
            status.state == "mounted",
            "已登记的虚拟磁盘未挂载或身份不匹配"
        );
        ensure!(
            status.backing_free >= required + 64 * 1024 * 1024,
            "镜像所在磁盘空间不足，需要迁移数据及 64 MB 余量"
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_disk_plans() {
        let mut s = StorageSettings::default();
        s.validate().unwrap();
        for path in [
            r"\\server\a.vhdx",
            r"D:\..\a.vhdx",
            r"D:\a\x:stream.vhdx",
            r"D:\x\a`bad.vhdx",
            r"D:\x\a.vhd",
            r"V:\x.vhdx",
            r"v:\x.vhdx",
        ] {
            s.image_path = path.into();
            assert!(s.validate().is_err(), "{path}");
        }
        s = StorageSettings::default();
        s.drive_letter = "C".into();
        assert!(s.validate().is_err());
        s = StorageSettings::default();
        s.capacity_gb = 0;
        assert!(s.validate().is_err());
    }
    #[test]
    fn settings_survive_reopen_and_fixture_disk_actions_refused() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".lab/agent-space/settings-tests")
            .join(uuid::Uuid::new_v4().to_string());
        let lab = Lab::at(root.clone()).unwrap();
        let mut s = lab.storage_settings().unwrap();
        s.backend = "vhdx".into();
        lab.save_storage_settings(s.clone()).unwrap();
        assert_eq!(Lab::at(root).unwrap().storage_settings().unwrap(), s);
        for action in ["status", "create", "mount", "detach", "enable-auto"] {
            assert!(lab.virtual_disk(action).is_err());
        }
        s.disk_id = uuid::Uuid::new_v4().to_string();
        assert!(lab.save_storage_settings(s).is_err());
    }
    #[test]
    fn reserved_geometry_and_task_state_cannot_be_overwritten_by_settings() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".lab/agent-space/settings-tests")
            .join(uuid::Uuid::new_v4().to_string());
        let lab = Lab::at(root).unwrap();
        let s = lab.storage_settings().unwrap();
        lab.db()
            .unwrap()
            .execute(
                "INSERT INTO settings VALUES ('disk-reserved',?1)",
                [serde_json::to_string(&s).unwrap()],
            )
            .unwrap();
        let mut changed = s.clone();
        changed.capacity_gb += 1;
        assert!(lab.save_storage_settings(changed).is_err());
        let mut changed = s.clone();
        changed.image_path = "E:\\Other.vhdx".into();
        assert!(lab.save_storage_settings(changed).is_err());
        let mut changed = s.clone();
        changed.drive_letter = "W".into();
        assert!(lab.save_storage_settings(changed).is_err());
        let mut changed = s.clone();
        changed.auto_mount = true;
        assert!(lab.save_storage_settings(changed).is_err());
        let mut changed = s;
        changed.directory = "E:\\AgentData".into();
        assert!(lab.save_storage_settings(changed).is_ok());
    }
}
