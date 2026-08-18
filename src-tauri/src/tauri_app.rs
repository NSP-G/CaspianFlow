//! Tauri v2 GUI runtime (P25). Compiled only with `--features tauri`.
//!
//! P25 wires the desktop shell + mock IPC so the frontend's `invoke` path is
//! live. The three commands mirror the frontend `useCaspian` contract
//! (P25 §九): `send_message` streams via `agent_status` / `chat_stream_chunk`
//! events; `list_sessions` / `get_data_path` return placeholder data. These are
//! mocks — real implementations are swapped in when P21/P22 modules land.

#![cfg(feature = "tauri")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;

use caspian_flow::config::CaspianPaths;
use caspian_flow::startup::StartupTimer;
use caspian_flow::hot_reload::{DirWatcher, DirChangeCallback};
use caspian_flow::skill::{ScanReport, Skill, SkillManager};
use caspian_flow::theme::{ThemeManager, ThemeListResult};
use caspian_flow::workflow::store::{RunRecord, RunStatus, RunStore};
use caspian_flow::memory;
use caspian_flow::workflow::{delete_workflow, list_entries, read_raw, save_draft, save_workflow, Workflow, WorkflowEngine, WorkflowListEntry, WorkflowRunResult};

#[derive(Clone, Serialize)]
struct SessionDto {
    id: String,
    title: String,
    updated_at: i64,
}

#[derive(Clone, Serialize)]
struct AgentStatusEvent {
    session_id: String,
    status: String,
    label: Option<String>,
}

#[derive(Clone, Serialize)]
struct StreamChunk {
    session_id: String,
    chunk: String,
}

const MOCK_DATA_PATH: &str = "~/.caspian/";

/// Data directory shorthand (P25 §六 / 验收 #5).
#[tauri::command]
fn get_data_path() -> String {
    MOCK_DATA_PATH.to_string()
}

/// Placeholder conversation list (P25 §四.2 / 验收 #2).
#[tauri::command]
fn list_sessions() -> Vec<SessionDto> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    vec![
        SessionDto {
            id: "s_demo".into(),
            title: "工作流引擎怎么实现的？".into(),
            updated_at: now - 180_000,
        },
        SessionDto {
            id: "s_2".into(),
            title: "本地的数据存在哪里".into(),
            updated_at: now - 2_520_000,
        },
        SessionDto {
            id: "s_3".into(),
            title: "帮我总结这段记忆".into(),
            updated_at: now - 18_000_000,
        },
    ]
}

/// Mock send: drives THINKING → STREAMING_ANSWER → IDLE and pushes chunked
/// tokens through events (P25 §五). Real backend replaces the body later.
#[tauri::command]
async fn send_message(
    app: tauri::AppHandle,
    session_id: String,
    text: String,
) -> Result<(), String> {
    let emit_status = |status: &str, label: Option<&str>| {
        app.emit(
            "agent_status",
            AgentStatusEvent {
                session_id: session_id.clone(),
                status: status.to_string(),
                label: label.map(str::to_string),
            },
        )
        .map_err(|e| e.to_string())
    };

    emit_status("THINKING", Some("规划任务"))?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    emit_status("STREAMING_ANSWER", Some("回答中"))?;

    let answer = format!(
        "收到：「{}」\n\n（P25 演示响应）CaspianFlow 以本地优先方式运行，所有数据落在 ~/.caspian/。\
         当前通道为 Rust mock command，真实推理将在 P21/P22 模块就绪后接入。",
        text.trim()
    );

    for part in answer.split('\n') {
        tokio::time::sleep(Duration::from_millis(120)).await;
        app.emit(
            "chat_stream_chunk",
            StreamChunk {
                session_id: session_id.clone(),
                chunk: format!("{part}\n"),
            },
        )
        .map_err(|e| e.to_string())?;
    }

    emit_status("IDLE", Some("就绪"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Workflow canvas IPC (P27, 模式 C)
// ---------------------------------------------------------------------------

/// Resolve the default CaspianFlow paths (`~/.caspian/`).
fn caspian_paths() -> CaspianPaths {
    CaspianPaths::resolve(None)
}

/// List workflow definitions for the canvas list view (P27 验收 #1).
#[derive(Clone, Serialize)]
struct WorkflowListEntryDto {
    name: String,
    display_name: String,
    description: String,
    modified: u64,
    step_count: usize,
}

#[tauri::command]
fn list_workflows() -> Vec<WorkflowListEntryDto> {
    list_entries(&caspian_paths())
        .unwrap_or_default()
        .into_iter()
        .map(|e: WorkflowListEntry| WorkflowListEntryDto {
            name: e.name,
            display_name: e.display_name,
            description: e.description,
            modified: e.modified,
            step_count: e.step_count,
        })
        .collect()
}

/// Load a workflow definition as a JSON document (with `ui` layout preserved)
/// plus its mtime. The frontend works purely in JSON; Rust converts the stored
/// P17 YAML to/from JSON so no browser YAML parser is needed.
#[derive(Clone, Serialize)]
struct WorkflowFileDto {
    doc: String,
    modified: u64,
}

#[tauri::command]
fn load_workflow(name: String) -> Result<WorkflowFileDto, String> {
    let (yaml, modified) = read_raw(&caspian_paths(), &name).map_err(|e| e.to_string())?;
    let value: Value = serde_yaml::from_str(&yaml).map_err(|e| e.to_string())?;
    let doc = serde_json::to_string(&value).map_err(|e| e.to_string())?;
    Ok(WorkflowFileDto { doc, modified })
}

/// Explicitly save a workflow (atomic write, draft cleared). `expected_mtime`
/// enables conflict detection against an external edit (P27 验收 #4/#5). The
/// `doc` is a JSON document; Rust converts it to P17 YAML and validates it.
#[tauri::command]
fn save_workflow(
    name: String,
    doc: String,
    expected_mtime: Option<u64>,
) -> Result<u64, String> {
    let value: Value = serde_json::from_str(&doc).map_err(|e| e.to_string())?;
    let yaml = serde_yaml::to_string(&value).map_err(|e| e.to_string())?;
    save_workflow(&caspian_paths(), &name, &yaml, expected_mtime).map_err(|e| e.to_string())
}

/// Write a draft (auto-save, debounced by the frontend). Isolated under
/// `.drafts/` — the engine never reads it (P27 验收 #3/#6).
#[tauri::command]
fn save_workflow_draft(name: String, doc: String) -> Result<(), String> {
    let value: Value = serde_json::from_str(&doc).map_err(|e| e.to_string())?;
    let yaml = serde_yaml::to_string(&value).map_err(|e| e.to_string())?;
    save_draft(&caspian_paths(), &name, &yaml).map_err(|e| e.to_string())
}

/// Delete a workflow definition and any stale draft.
#[tauri::command]
fn delete_workflow(name: String) -> Result<(), String> {
    delete_workflow(&caspian_paths(), &name).map_err(|e| e.to_string())
}

/// Build + run the Tauri application (called from `src/main.rs` under the
/// `tauri` feature).

// ---------------------------------------------------------------------------
// Workflow execution IPC (P28): trigger P17 engine + surface run state
// ---------------------------------------------------------------------------

/// Shared application state, managed by Tauri (`tauri::Builder::manage`).
///
/// Holds the disk-populated skill registry and the run store so `run_workflow`
/// can construct a `WorkflowEngine` without re-scanning on every invocation.
/// The path formula mirrors P27/P17: `workflows/<name>/workflow.yaml`.
struct AppState {
    paths: CaspianPaths,
    manager: SkillManager,
    store: Arc<RunStore>,
    themes: Arc<ThemeManager>,
}

/// Live hot-reload watchers, held by Tauri so they are not dropped (P30 WS2).
/// Stored in managed state so the `notify` debouncer threads keep running for
/// the lifetime of the app. `None` entries mean that watcher failed to start
/// (e.g. notify backend unavailable) — core functionality is unaffected.
struct Watchers {
    _skill: Option<DirWatcher>,
    _workflow: Option<DirWatcher>,
    _theme: Option<DirWatcher>,
}

/// Run handle returned synchronously by `run_workflow` (execution runs in the
/// background; the frontend follows progress via events).
#[derive(Clone, Serialize)]
struct RunResponse {
    run_id: String,
    status: RunStatus,
}

/// Emitted right after the run record is created (验收 #2: 执行中).
#[derive(Clone, Serialize)]
struct WorkflowRunStarted {
    run_id: String,
    workflow_name: String,
    started_at: u64,
}

/// Per-step result carried in `WorkflowRunFinished` (验收 #3/#5).
#[derive(Clone, Serialize)]
struct StepResultDto {
    step_id: String,
    skill: String,
    output: Value,
    duration_ms: u64,
}

/// Final run result (验收 #3 摘要 + #4/#5 逐步骤输入/输出).
#[derive(Clone, Serialize)]
struct RunResultDto {
    run_id: String,
    workflow_name: String,
    status: RunStatus,
    duration_ms: u64,
    terminated: bool,
    skipped_steps: usize,
    steps: Vec<StepResultDto>,
    outputs: HashMap<String, Value>,
}

impl RunResultDto {
    fn from_result(r: &WorkflowRunResult) -> Self {
        Self {
            run_id: r.run_id.clone(),
            workflow_name: r.workflow_name.clone(),
            status: RunStatus::Completed,
            duration_ms: r.duration_ms,
            terminated: r.terminated,
            skipped_steps: r.skipped_steps,
            steps: r
                .steps
                .iter()
                .map(|s| StepResultDto {
                    step_id: s.step_id.clone(),
                    skill: s.skill.clone(),
                    output: s.output.clone(),
                    duration_ms: s.duration_ms,
                })
                .collect(),
            outputs: r.outputs.clone(),
        }
    }
}

/// Emitted when execution fails (验收 #4).
#[derive(Clone, Serialize)]
struct WorkflowRunErrored {
    run_id: String,
    error: String,
}

/// Trigger a workflow run (验收 #1).
///
/// Loads the saved formal manifest (`workflows/<name>/workflow.yaml` — the P27
/// write path), creates a `RunStore` record, then executes the P17 engine on a
/// background task. Progress reaches the frontend via `workflow_run_started`
/// / `workflow_run_finished` / `workflow_run_errored` events (最小闭环, F3-a:
/// the engine returns only the final result, so no per-step streaming).
#[tauri::command]
async fn run_workflow(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<RunResponse, String> {
    let manifest = state.paths.workflows.join(&name).join("workflow.yaml");
    let workflow = Workflow::load(&manifest).map_err(|e| e.to_string())?;

    let run = state.store.create_run(&name).map_err(|e| e.to_string())?;
    let run_id = run.run_id.clone();

    app.emit(
        "workflow_run_started",
        WorkflowRunStarted {
            run_id: run_id.clone(),
            workflow_name: name.clone(),
            started_at: run.started_at,
        },
    )
    .map_err(|e| e.to_string())?;

    let engine =
        WorkflowEngine::with_defaults(state.manager.shared_registry(), Arc::clone(&state.store));
    let store = Arc::clone(&state.store);
    let app_for_task = app.clone();

    tauri::async_runtime::spawn(async move {
        match engine.execute(&workflow, &HashMap::new()).await {
            Ok(result) => {
                if let Ok(Some(mut rec)) = store.get_run(&run_id) {
                    rec.finish(RunStatus::Completed);
                    let _ = store.update_run(&rec);
                }
                let _ = app_for_task.emit("workflow_run_finished", RunResultDto::from_result(&result));
            }
            Err(e) => {
                if let Ok(Some(mut rec)) = store.get_run(&run_id) {
                    rec.finish(RunStatus::Failed);
                    let _ = store.update_run(&rec);
                }
                let _ = app_for_task.emit(
                    "workflow_run_errored",
                    WorkflowRunErrored {
                        run_id,
                        error: e.to_string(),
                    },
                );
            }
        }
    });

    Ok(RunResponse {
        run_id,
        status: run.status,
    })
}

/// Current status of a run (验收 #2/#6).
#[tauri::command]
fn get_run_status(state: tauri::State<'_, AppState>, run_id: String) -> Result<RunStatus, String> {
    state
        .store
        .get_run(&run_id)
        .map_err(|e| e.to_string())?
        .map(|r| r.status)
        .ok_or_else(|| "run not found".to_string())
}

/// List run history, optionally filtered by workflow (验收 #6/#7).
#[tauri::command]
fn list_runs(
    state: tauri::State<'_, AppState>,
    workflow_name: Option<String>,
) -> Result<Vec<RunRecord>, String> {
    let mut runs = state.store.list_runs().map_err(|e| e.to_string())?;
    if let Some(wf) = workflow_name {
        runs.retain(|r| r.workflow_name == wf);
    }
    Ok(runs)
}

/// Memory baseline snapshot (P35) — structural counts + real RSS on Linux.
///
/// Lets the GUI surface the Rust core's contribution to the process footprint
/// (the 200/500 MB budgets are GUI-dominated; this isolates the core's share).
#[tauri::command]
fn memory_report(state: tauri::State<'_, AppState>) -> Value {
    let skills = state.manager.registry().count();
    let runs = state.store.list_runs().map(|r| r.len()).unwrap_or(0);
    let rss = memory::current_rss_bytes();
    serde_json::json!({
        "skills": skills,
        "runs": runs,
        "rss_bytes": rss,
        "summary": memory::MemoryBaseline {
            skills,
            runs,
            sessions: 0,
            estimated_bytes: 0,
            rss_bytes: rss,
        }.summary(),
    })
}

/// Export the live state into a `.caspian` bundle at `dest` (P36).
#[tauri::command]
fn export_bundle(state: tauri::State<'_, AppState>, dest: String) -> Result<String, String> {
    let opts = caspian_flow::package::ExportOptions {
        include_sessions: true,
        include_knowledge: true,
    };
    caspian_flow::package::export_bundle(&state.paths, std::path::Path::new(&dest), &opts)
        .map(|m| serde_json::to_string(&m).unwrap_or_default())
        .map_err(|e| e.to_string())
}

/// Import a `.caspian` bundle from `src` into the live state (P36).
/// `policy` is one of `skip` (default) / `overwrite` / `rename`.
#[tauri::command]
fn import_bundle(
    state: tauri::State<'_, AppState>,
    src: String,
    policy: Option<String>,
) -> Result<String, String> {
    let policy = match policy.as_deref().unwrap_or("skip") {
        "overwrite" => caspian_flow::package::ConflictPolicy::Overwrite,
        "rename" => caspian_flow::package::ConflictPolicy::Rename,
        _ => caspian_flow::package::ConflictPolicy::Skip,
    };
    caspian_flow::package::import_bundle(std::path::Path::new(&src), &state.paths, policy)
        .map(|r| serde_json::to_string(&r).unwrap_or_default())
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Module status + skill (re)load IPC (P30 WS1)
// ---------------------------------------------------------------------------

/// List all loaded skills (P26/P30 — wires real registry data into the UI).
#[tauri::command]
fn list_skills(state: tauri::State<'_, AppState>) -> Vec<Skill> {
    state.manager.registry().list_all()
}

/// Re-scan skills from disk and return the new count (P26/P30).
///
/// Returns the post-reload skill count; the reload also refreshes the module
/// status surfaced by `get_module_status`.
#[tauri::command]
async fn reload_skills(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    state
        .manager
        .reload()
        .await
        .map_err(|e| e.to_string())?;
    Ok(state.manager.registry().count())
}

/// Current module status: loaded skills plus any missing/broken issues (P30 WS1 §3).
///
/// Drives the UI resilience banner — the UI can tell the user exactly which
/// skill directories failed to load and why.
#[tauri::command]
fn get_module_status(state: tauri::State<'_, AppState>) -> ScanReport {
    (*state.manager.module_status()).clone()
}

// ---------------------------------------------------------------------------
// Theme packages IPC (P31 · A3)
// ---------------------------------------------------------------------------

/// Broadcast when the active theme — or any theme package on disk — changes.
/// The UI applies `css` as a `<style>` override and flips `data-theme`.
#[derive(Clone, Serialize)]
struct ThemeChanged {
    name: Option<String>,
    css: String,
}

/// List installed theme packages plus any load issues (P31).
#[tauri::command]
fn list_themes(state: tauri::State<'_, AppState>) -> ThemeListResult {
    state.themes.list()
}

/// Get a theme package's CSS variable overrides by name (P31).
#[tauri::command]
fn get_theme_css(state: tauri::State<'_, AppState>, name: String) -> Result<String, String> {
    state.themes.get_css(&name).map_err(|e| e.to_string())
}

/// Currently-active theme name, if any (P31).
#[tauri::command]
fn get_active_theme(state: tauri::State<'_, AppState>) -> Option<String> {
    state.themes.active()
}

/// Activate a theme, persist the selection, and return its CSS. Broadcasts
/// `theme_changed` so other windows apply it without a round-trip (P31).
#[tauri::command]
fn apply_theme(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<String, String> {
    let css = state.themes.apply(&name).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "theme_changed",
        ThemeChanged {
            name: Some(name),
            css: css.clone(),
        },
    );
    Ok(css)
}

pub fn run_tauri() {
    let mut startup = StartupTimer::new();
    let paths = CaspianPaths::resolve(None);
    startup.mark("paths");
    let manager = tauri::async_runtime::block_on(SkillManager::init(&paths.skills))
        .expect("failed to initialize skill registry");
    startup.mark("skills");
    let store = Arc::new(RunStore::from_paths(&paths));
    let themes = Arc::new(ThemeManager::new(paths.themes.clone()));
    startup.mark("runstore");
    tracing::info!(report = %startup.report(), "CaspianFlow core initialized");
    let state = AppState {
        paths,
        manager,
        store,
        themes: themes.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .setup(|app| {
            let paths = app.state::<AppState>().paths.clone();
            // Ensure dirs exist so watchers stay live even before content is created.
            let _ = std::fs::create_dir_all(&paths.skills);
            let _ = std::fs::create_dir_all(&paths.workflows);
            let _ = std::fs::create_dir_all(&paths.themes);

            // Skill watcher: re-scan + reload registry, then announce to the UI.
            let app_for_reload = app.handle().clone();
            let skill_cb: DirChangeCallback = Arc::new(move || {
                let app = app_for_reload.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<AppState>();
                    if state.manager.reload().await.is_ok() {
                        let status = state.manager.module_status();
                        let _ = app.emit("skills_reloaded", (*status).clone());
                    }
                });
            });
            let skill_watcher = match DirWatcher::watch(&paths.skills, skill_cb) {
                Ok(w) => Some(w),
                Err(e) => {
                    tracing::warn!(error = %e, "skill hot-reload watcher failed to start");
                    None
                }
            };

            // Workflow watcher: just tell the UI to re-fetch the list.
            let app_for_wf = app.handle().clone();
            let workflow_cb: DirChangeCallback = Arc::new(move || {
                let app = app_for_wf.clone();
                let _ = app.emit("workflows_changed", ());
            });
            let workflow_watcher = match DirWatcher::watch(&paths.workflows, workflow_cb) {
                Ok(w) => Some(w),
                Err(e) => {
                    tracing::warn!(error = %e, "workflow hot-reload watcher failed to start");
                    None
                }
            };

            // Theme watcher: re-broadcast the active theme's CSS on any change
            // (P31). If no theme is active, an empty css is sent so the UI can
            // revert to the built-in dark/light tokens.
            let app_for_theme = app.handle().clone();
            let theme_mgr = themes.clone();
            let theme_cb: DirChangeCallback = Arc::new(move || {
                let app = app_for_theme.clone();
                let mgr = theme_mgr.clone();
                let active = mgr.active();
                let css = active
                    .as_ref()
                    .and_then(|n| mgr.get_css(n).ok())
                    .unwrap_or_default();
                let _ = app.emit("theme_changed", ThemeChanged { name: active, css });
            });
            let theme_watcher = match DirWatcher::watch(&paths.themes, theme_cb) {
                Ok(w) => Some(w),
                Err(e) => {
                    tracing::warn!(error = %e, "theme hot-reload watcher failed to start");
                    None
                }
            };

            if skill_watcher.is_some() || workflow_watcher.is_some() || theme_watcher.is_some() {
                app.manage(Watchers {
                    _skill: skill_watcher,
                    _workflow: workflow_watcher,
                    _theme: theme_watcher,
                });
            } else {
                tracing::warn!("hot-reload disabled: no directory watchers started");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_message,
            list_sessions,
            get_data_path,
            list_workflows,
            load_workflow,
            save_workflow,
            save_workflow_draft,
            delete_workflow,
            run_workflow,
            get_run_status,
            list_runs,
            list_skills,
            reload_skills,
            get_module_status,
            list_themes,
            get_theme_css,
            get_active_theme,
            apply_theme,
            memory_report,
            export_bundle,
            import_bundle,
            caspian_flow::updater::check_for_update,
            caspian_flow::updater::install_update
        ])
        .run(tauri::generate_context!())
        .expect("error while running CaspianFlow GUI");
}
