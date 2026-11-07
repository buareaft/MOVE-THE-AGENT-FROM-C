use crate::{Dashboard, Lab, Snapshot};
use anyhow::{ensure, Result};
use rusqlite::params;
use std::collections::BTreeMap;

impl Lab {
    /// All figures come from one indexed snapshot, never by rescanning paths.
    pub fn overview(&self, volume: &str) -> Result<serde_json::Value> {
        self.snapshot()?;
        let mut db = self.db()?;
        let tx = db.transaction()?;
        let raw: String =
            tx.query_row("SELECT json FROM snapshots WHERE id=1", [], |r| r.get(0))?;
        let mut s: Snapshot = serde_json::from_str(&raw)?;
        ensure!(
            volume.is_empty() || s.volumes.iter().any(|v| v.volume == volume),
            "未知磁盘，请重新扫描"
        );
        s.total_bytes = 0;
        s.allocated_bytes = 0;
        s.file_count = 0;
        s.shared_bytes = 0;
        s.project_bytes = 0;
        s.exclusive_bytes = 0;
        s.unknown_bytes = 0;
        let mut groups = tx.prepare("SELECT category,SUM(bytes),SUM(allocated),COUNT(*) FROM files WHERE (?1='' OR volume=?1) GROUP BY category")?;
        for row in groups.query_map([volume], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, usize>(3)?,
            ))
        })? {
            let (category, bytes, allocated, files) = row?;
            s.total_bytes += bytes;
            s.allocated_bytes += allocated;
            s.file_count += files;
            match category.as_str() {
                "shared" => s.shared_bytes += bytes,
                "associated" => s.project_bytes += bytes,
                "exclusive" => s.exclusive_bytes += bytes,
                _ => s.unknown_bytes += bytes,
            }
        }
        let mut counts = BTreeMap::<String, usize>::new();
        for agent in &mut s.agents {
            let (bytes, allocated, files): (u64,u64,usize) = tx.query_row(
                "SELECT COALESCE(SUM(bytes),0),COALESCE(SUM(allocated),0),COUNT(*) FROM files WHERE agent=?1 AND category!='associated' AND (?2='' OR volume=?2)",
                params![agent.id,volume], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
            agent.bytes = bytes;
            agent.allocated = allocated;
            counts.insert(agent.id.clone(), files);
        }
        s.agents.retain(|a| counts[&a.id] > 0);
        let entity_count = |kind: &str, category: &str| -> Result<usize> {
            Ok(tx.query_row("SELECT COUNT(*) FROM entities e WHERE kind=?1 AND EXISTS(SELECT 1 FROM files f WHERE f.category=?2 AND f.owner=e.id AND (?1='projects' OR f.agent=e.agent) AND (?3='' OR f.volume=?3))", params![kind,category,volume], |r| r.get(0))?)
        };
        let projects = entity_count("projects", "associated")?;
        let sessions = entity_count("sessions", "exclusive")?;
        let subtotal: u64 = s.agents.iter().map(|a| a.bytes).sum();
        let mut result = serde_json::to_value(Dashboard::from(s))?;
        result["projectCount"] = projects.into();
        result["sessionCount"] = sessions.into();
        result["agentFiles"] = serde_json::to_value(counts)?;
        result["agentSubtotal"] = subtotal.into();
        result["selectedVolume"] = volume.into();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    #[test]
    fn disk_scope_partitions_totals_and_agent_subtotals() -> Result<()> {
        let t = crate::lab::test_dir()?;
        let lab = Lab::at(t.path().join("lab"))?;
        lab.scan(&AtomicBool::new(false), |_| {})?;
        let all = lab.overview("")?;
        let volume = all["volumes"][0]["volume"].as_str().unwrap();
        assert_eq!(lab.overview(volume)?["totalBytes"], all["totalBytes"]);
        assert!(lab.overview("nonexistent").is_err());
        let db = lab.db()?;
        db.execute("UPDATE files SET volume='scope-B' WHERE agent='claude'", [])?;
        let mut s: Snapshot = serde_json::from_str(&db.query_row::<String, _, _>(
            "SELECT json FROM snapshots WHERE id=1",
            [],
            |r| r.get(0),
        )?)?;
        s.volumes.push(crate::DriveUsage {
            volume: "scope-B".into(),
            bytes: 0,
            allocated_bytes: 0,
            files: 0,
        });
        db.execute(
            "UPDATE snapshots SET json=?1 WHERE id=1",
            [serde_json::to_string(&s)?],
        )?;
        let b = lab.overview("scope-B")?;
        let a = lab.overview(volume)?;
        assert_eq!(
            a["totalBytes"].as_u64().unwrap() + b["totalBytes"].as_u64().unwrap(),
            all["totalBytes"].as_u64().unwrap()
        );
        assert_eq!(b["agents"].as_array().unwrap().len(), 1);
        assert_eq!(b["agents"][0]["id"], "claude");
        assert_eq!(b["agentSubtotal"], b["totalBytes"]);
        assert_eq!(
            lab.breakdown_on_volume("agents", "claude", "scope-B")?
                .iter()
                .map(|b| b.bytes)
                .sum::<u64>(),
            b["agentSubtotal"].as_u64().unwrap()
        );
        assert!(b["fileCount"].as_u64().unwrap() > 0);
        Ok(())
    }
}
