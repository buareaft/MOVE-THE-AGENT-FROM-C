#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use agent_space_core::{Lab, Progress};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::{ipc::Channel, State};

#[derive(Default)]
struct AppState {
    scan: Mutex<Option<Arc<AtomicBool>>>,
    movement: Mutex<Option<Arc<AtomicBool>>>,
}

#[tauri::command]
async fn get_storage_settings() -> Result<agent_space_core::StorageSettings, String> {
    tauri::async_runtime::spawn_blocking(|| Lab::open()?.storage_settings())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn save_storage_settings(
    settings: agent_space_core::StorageSettings,
) -> Result<agent_space_core::StorageSettings, String> {
    tauri::async_runtime::spawn_blocking(|| Lab::open()?.save_storage_settings(settings))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn virtual_disk(action: String) -> Result<agent_space_core::VirtualDiskStatus, String> {
    tauri::async_runtime::spawn_blocking(move || Lab::open()?.virtual_disk(&action))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn configured_move_target() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| Lab::open()?.configured_move_target())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_runtime_info() -> Result<agent_space_core::RuntimeInfo, String> {
    Lab::open()
        .and_then(|lab| lab.runtime_info())
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn get_overview(volume: String) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || Lab::open()?.overview(&volume))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn get_snapshot() -> Result<agent_space_core::Dashboard, String> {
    tauri::async_runtime::spawn_blocking(|| Lab::open()?.snapshot().map(Into::into))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn start_scan(
    state: State<'_, AppState>,
    on_progress: Channel<Progress>,
) -> Result<agent_space_core::Dashboard, String> {
    let flag = Arc::new(AtomicBool::new(false));
    {
        let mut active = state.scan.lock().map_err(|e| e.to_string())?;
        if active.is_some() {
            return Err("扫描正在运行".into());
        }
        *active = Some(flag.clone());
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
        let lab = Lab::open()?;
        let s = lab.scan(&flag, |n| {
            if n % 128 == 0 || n == 1 {
                let _ = on_progress.send(Progress {
                    phase: "正在索引文件".into(),
                    files: n,
                    done: false,
                });
            }
        })?;
        let _ = on_progress.send(Progress {
            phase: "扫描完成".into(),
            files: s.file_count,
            done: true,
        });
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(s)
    })
    .await;
    *state.scan.lock().map_err(|e| e.to_string())? = None;
    result
        .map_err(|e| e.to_string())?
        .map(Into::into)
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn cancel_scan(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(f) = state.scan.lock().map_err(|e| e.to_string())?.as_ref() {
        f.store(true, Ordering::Relaxed);
    }
    Ok(())
}
#[tauri::command]
async fn list_entities(
    kind: String,
    search: String,
    offset: usize,
    limit: usize,
    sort: Option<String>,
    descending: Option<bool>,
) -> Result<agent_space_core::Page, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Lab::open()?.list_sorted(
            &kind,
            &search,
            offset,
            limit,
            sort.as_deref().unwrap_or("bytes"),
            descending.unwrap_or(true),
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
#[tauri::command]
async fn get_space_breakdown(
    kind: String,
    id: String,
    volume: Option<String>,
) -> Result<Vec<agent_space_core::Breakdown>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Lab::open()?.breakdown_on_volume(&kind, &id, volume.as_deref().unwrap_or(""))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
#[tauri::command]
async fn create_migration_plan(agent: String) -> Result<agent_space_core::Plan, String> {
    tauri::async_runtime::spawn_blocking(move || Lab::open()?.plan(&agent))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn execute_migration(agent: String) -> Result<agent_space_core::Job, String> {
    tauri::async_runtime::spawn_blocking(move || Lab::open()?.execute(&agent))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn rollback_migration(id: String) -> Result<agent_space_core::Job, String> {
    tauri::async_runtime::spawn_blocking(move || Lab::open()?.rollback(&id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn get_move_candidates() -> Result<Vec<agent_space_core::MoveCandidate>, String> {
    tauri::async_runtime::spawn_blocking(|| Lab::open()?.move_candidates())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn get_move_jobs() -> Result<Vec<agent_space_core::MoveJob>, String> {
    tauri::async_runtime::spawn_blocking(|| Lab::open()?.move_jobs())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn prepare_move(
    candidate: String,
    target: String,
) -> Result<agent_space_core::MovePlan, String> {
    tauri::async_runtime::spawn_blocking(move || Lab::open()?.prepare_move(&candidate, &target))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn run_move(
    state: State<'_, AppState>,
    id: String,
    action: String,
    on_progress: Channel<agent_space_core::MoveProgress>,
) -> Result<agent_space_core::MoveJob, String> {
    let flag = Arc::new(AtomicBool::new(false));
    {
        let mut running = state.movement.lock().map_err(|e| e.to_string())?;
        if running.is_some() {
            return Err("已有迁移正在运行".into());
        }
        *running = Some(flag.clone());
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
        let last = Mutex::new(std::time::Instant::now());
        let progress = |p| {
            if let Ok(mut sent) = last.lock() {
                if sent.elapsed().as_millis() >= 100 {
                    let _ = on_progress.send(p);
                    *sent = std::time::Instant::now();
                }
            }
        };
        let lab = Lab::open()?;
        match action.as_str() {
            "execute" => lab.execute_move(&id, &flag, progress),
            "restore" => lab.restore_move(&id, &flag, progress),
            "finalize" => lab.finalize_move_cancellable(&id, &flag),
            _ => Err(std::io::Error::other("未知迁移操作").into()),
        }
    })
    .await;
    *state.movement.lock().map_err(|e| e.to_string())? = None;
    result
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn cancel_move(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(f) = state.movement.lock().map_err(|e| e.to_string())?.as_ref() {
        f.store(true, Ordering::Relaxed);
    }
    Ok(())
}
#[tauri::command]
async fn export_report(format: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || Lab::open()?.export_report(&format))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn open_export(path: String) -> Result<(), String> {
    let lab = Lab::open().map_err(|e| e.to_string())?;
    let p = lab
        .validated_export_path(&path)
        .map_err(|e| e.to_string())?;
    let shell_path = p.to_string_lossy();
    let shell_path = shell_path.strip_prefix("\\\\?\\").unwrap_or(&shell_path);
    #[cfg(windows)]
    std::process::Command::new("explorer.exe")
        .arg("/select,")
        .arg(shell_path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
fn reveal_entity(kind: String, id: String) -> Result<(), String> {
    let path = Lab::open()
        .and_then(|lab| lab.validated_entity_path(&kind, &id))
        .map_err(|e| e.to_string())?;
    let shell_path = path.to_string_lossy();
    let shell_path = shell_path.strip_prefix("\\\\?\\").unwrap_or(&shell_path);
    #[cfg(windows)]
    std::process::Command::new("explorer.exe")
        .arg("/select,")
        .arg(shell_path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .setup(|app| {
            let data = Lab::open()?.webview_dir()?;
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .data_directory(data)
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_runtime_info,
            get_storage_settings,
            save_storage_settings,
            virtual_disk,
            configured_move_target,
            get_snapshot,
            get_overview,
            start_scan,
            cancel_scan,
            list_entities,
            get_space_breakdown,
            create_migration_plan,
            execute_migration,
            rollback_migration,
            get_move_candidates,
            get_move_jobs,
            prepare_move,
            run_move,
            cancel_move,
            export_report,
            open_export,
            reveal_entity
        ])
        .run(tauri::generate_context!())
        .expect("Unable to start Agent Space");
}
