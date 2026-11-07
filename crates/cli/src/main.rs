use agent_space_core::Lab;
use anyhow::{bail, Result};
fn run() -> Result<()> {
    let args: Vec<_> = std::env::args()
        .skip(1)
        .filter(|arg| arg != "--fixtures")
        .collect();
    let lab = Lab::open()?;
    let value=match args.first().map(String::as_str).unwrap_or("runtime") {
        "runtime" => serde_json::to_value(lab.runtime_info()?)?,
        "settings" => serde_json::to_value(lab.storage_settings()?)?,
        "settings-save" => serde_json::to_value(lab.save_storage_settings(serde_json::from_str(std::fs::read_to_string(args.get(1).context("Provide settings JSON file")?)?.trim_start_matches('\u{feff}'))?)?)?,
        "disk" => serde_json::to_value(lab.virtual_disk(args.get(1).map(String::as_str).unwrap_or("status"))?)?,
        "configured-target" => serde_json::to_value(lab.configured_move_target()?)?,
        "scan" => serde_json::to_value(lab.scan(&std::sync::atomic::AtomicBool::new(false),|_|{})?)?,
        "plan" => serde_json::to_value(lab.plan(args.get(1).map(String::as_str).unwrap_or("codex"))?)?,
        "execute" => serde_json::to_value(lab.execute(args.get(1).map(String::as_str).unwrap_or("codex"))?)?,
        "rollback" => serde_json::to_value(lab.rollback(args.get(1).ok_or_else(||anyhow::anyhow!("Provide a recorded job ID"))?)?)?,
        "jobs" => serde_json::to_value(lab.jobs()?)?,
        "candidates"=>serde_json::to_value(lab.move_candidates()?)?,
        "moves"=>serde_json::to_value(lab.move_jobs()?)?,
        "seed-move-fixture"=>serde_json::to_value(lab.seed_move_fixture()?)?,
        "seed-deepseek-fixture"=>lab.seed_deepseek_fixture()?,
        "move-plan"=>serde_json::to_value(lab.prepare_move(args.get(1).context("Provide adapter ID")?,args.get(2).context("Provide existing target root")?)?)?,
        "move"=>serde_json::to_value(lab.execute_move(args.get(1).context("Provide plan ID")?,&std::sync::atomic::AtomicBool::new(false),|_|{})?)?,
        "move-finalize"=>serde_json::to_value(lab.finalize_move(args.get(1).context("Provide job ID")?)?)?,
        "move-restore"=>serde_json::to_value(lab.restore_move(args.get(1).context("Provide job ID")?,&std::sync::atomic::AtomicBool::new(false),|_|{})?)?,
        "report"=>serde_json::to_value(lab.export_report(args.get(1).map(String::as_str).unwrap_or("json"))?)?,
        "list"=>serde_json::to_value(lab.list_sorted(args.get(1).map(String::as_str).unwrap_or("sessions"),args.get(2).map(String::as_str).unwrap_or(""),args.get(5).and_then(|x|x.parse().ok()).unwrap_or(0),args.get(6).and_then(|x|x.parse().ok()).unwrap_or(8),args.get(3).map(String::as_str).unwrap_or("bytes"),args.get(4).map(String::as_str).unwrap_or("desc")!="asc")?)?,
        _=>bail!("Commands: runtime | scan | list KIND SEARCH SORT asc/desc OFFSET LIMIT | report json/csv | plan AGENT | execute AGENT | rollback ID | jobs | candidates | move-plan ADAPTER TARGET_ROOT | move ID | move-finalize ID | move-restore ID | moves | seed-move-fixture"),
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
use anyhow::Context;
fn main() {
    if let Err(e) = run() {
        eprintln!(
            "{}",
            serde_json::json!({"code":"operation_refused","message":format!("{e:#}")})
        );
        std::process::exit(1);
    }
}
