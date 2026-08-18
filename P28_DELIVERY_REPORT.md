# P28 交付报告（工作流执行入 UI）

> 日期：2026-08-14 | 方向锁定：L19 | 前置核对：L20（衔接路径 PASS，未另辟通路）
> 决策（Seeker 拍板）：F3 → (a) 最小闭环事件流；F6 → (a) 运行前自动保存

## 一、衔接路径核验结论（用户指定重点）

**设计文档未另辟通路。** `run_workflow` 的真实入口就是 P27 约定的「正式写盘 → `Workflow::load` 重载」：

- 命令经 `state.paths.workflows.join(name).join("workflow.yaml")` 加载正式文件（P27 子目录约定，沿用 P17 `<name>/workflow.yaml`）；
- 加载后交给 P17 `WorkflowEngine::execute` 执行；
- 运行态写入 P17 `RunStore`（`<temp>/workflows/<run_id>/`），不绕开、不复制执行逻辑。

全程复用 P17，**未修改引擎内部逻辑**（禁改约束满足）。

## 二、前置核对裂缝处置（F1–F9）

| 裂缝 | 处置 | 是否偏离文档意图 |
|---|---|---|
| F1 `AppState` 不存在 | `tauri_app.rs` 引入 `AppState`(`manage`)；`SkillManager` 暴露 `shared_registry()`（字段改 `Arc<SkillRegistry>`） | 否，补建文档假设的 managed state |
| F2 `RunStatus` 无 `Pending` | UI 状态机以引擎 `RunStatus` 为准；「排队中」降级为按钮乐观瞬态；覆盖 `Terminated`/`Skipped` | 否，校准映射 |
| F3 逐步骤事件流引擎不产出 | **(a) 最小闭环**：emit `started`/`finished`/`errored`，无 per-step 流 | 否，Seeker 拍板 |
| F4 `list_runs` 无 name 过滤 | 命令内按 `workflow_name` 内存过滤 | 否 |
| F5 DTO 命名 | 复用引擎 `RunStatus`/`RunRecord`，仅新建 `RunResponse` | 否 |
| F6 运行前须正式写盘 | **(a) 运行前先 `saveWorkflow` 自动保存**再 `run_workflow` | 否，Seeker 拍板 |
| F7 命令须 spawn | `run_workflow` `async_runtime::spawn` 执行，同步返回 `run_id` | 否 |
| F8/F9 | `Workflow::name` 与目录身份分离沿用 P27；沙箱闭环诚实 | 否 |

## 三、交付内容

### Rust（沙箱可测 + feature 门控命令）

- **`skill/mod.rs`**：`SkillManager.registry` 字段改为 `Arc<SkillRegistry>`（`SharedSkillRegistry`），新增 `shared_registry()`。调用方经 `Arc` 自动解引用，既有测试无缝。
- **`workflow/runner.rs`（新增）**：`workflow::runner::tests` 沙箱端到端单测——
  - `run_path_executes_and_persists`：`SkillManager::init`（扫描真实 shell 技能）+ `WorkflowEngine::with_defaults` + `execute` + 断言 `RunStore` 持久化为 `Completed` 且逐步骤输出落盘。**这是 P28 核心声明的真实闭环**（设计文档 §六承诺的「序列化↔引擎加载」扩展为「load→execute→持久化」）。
  - `run_path_manifest_location_matches_command`：锁定 `run_workflow` 的路径公式（不依赖执行，纯 load）。
- **`tauri_app.rs`**：`AppState`（`manage` 持 `paths`/`SkillManager`/`Arc<RunStore>`）+ 三命令 `run_workflow`/`get_run_status`/`list_runs`（feature 门控）。`run_workflow`：`Workflow::load` → `create_run` → emit `workflow_run_started` → spawn `execute` → 终态 emit `workflow_run_finished`(带 `RunResultDto`)/`workflow_run_errored`；DTO 复用引擎 `RunStatus`/`RunRecord`。

### 前端

- **`types/workflow.ts`**：`RunStatus`/`RunResponse`/`RunResult`/`RunRecord`/`RunStepResult`。
- **`useCaspian.ts`**：`runWorkflow`（运行前先 `saveWorkflow` 自动保存 F6-a）、`getRunStatus`、`listRuns`、`subscribeWorkflowRun`（真实 Tauri `listen` / mock 事件总线）。
- **`WorkflowEditorPage.tsx`**：「运行」按钮（运行中禁用）、状态指示器（排队中乐观态 / 执行中 / 已完成 / 失败，映射引擎 `RunStatus` 含 `Terminated`/`Skipped`，新增 `--color-success` token）、结果面板（摘要 + 逐步骤输出展开，验收 #3/#4/#5）、底部最近运行列表（验收 #6/#7）。
- **`index.css`**：加 `--color-success`（暗/浅两态，与 `--color-danger` 同构）。

## 四、门禁结果

| 门禁 | 结果 |
|---|---|
| `cargo test --lib` | ✅ **677 passed / 0 failed**（675 基线 + 2 新增 P28 runner 单测）|
| `cargo clippy --lib` | ✅ 0 |
| `pnpm build` | ✅ |
| `pnpm typecheck` | ✅ 0 |
| `pnpm lint` | ✅ 0（0 warning）|

## 五、未解风险 / 待本地验收

- **Tauri 命令沙箱仅语法保全**：`tauri_app.rs` 经 `tauri` feature 门控，沙箱缺 `webkit2gtk` 无法编译/启动 Tauri，故命令**未经验证编译**。其写法严格沿用 P25/P26/P27 既有 mock command 模式（feature 门控、`State<AppState>`、`invoke`/`listen`），但需 Seeker 本地 `pnpm tauri dev -- --features tauri` 实编实跑确认。
- **真实执行路径**（验收 #1/#2/#4/#5/#7 中依赖实际技能运行的部分）依赖本地环境：沙箱闭环已用 shell 技能证明 `load→execute→持久化` 全链路，但 UI 端真实 fs 行为 + 真实技能执行流需本地验证。
- 运行态可视化采用 F3-(a) 最小闭环：UI 仅在「运行中」与「终态」间切换，无实时「第 X/Y 步」进度（终态用 `steps.len()` 显示静态总数）。若后续要实时进度，走 F3-(b) RunStore 轮询（不改 P17）。

## 六、验收对照

| # | 验收项 | 沙箱覆盖 | 本地待验 |
|---|---|---|---|
| 1 | 点击运行，P17 引擎加载并执行 | runner 单测闭环 | Tauri 命令实编实跑 |
| 2 | 状态指示器正确更新 | UI mock 网关演示 | 真实事件流 |
| 3 | 运行完成显示输出摘要 | UI mock 结果面板 | 真实 |
| 4 | 失败显示错误节点/原因 | UI failed 面板 + `workflow_run_errored` | 真实失败路径 |
| 5 | 逐步骤输入/输出展开 | UI per-step 展开（mock 有数据） | 真实 |
| 6 | 列表页显示最近运行状态 | `listRuns` + 底部历史（mock 落盘） | 真实 RunStore |
| 7 | 重启后可查运行历史 | runner 单测验证 RunStore 持久化 | 真实 |

> 同步：DIRECTION_SYNC.md L20 决策已拍板、L21 P28 实现闭环已记录。P28 收口完成。

---

## 七、大件 A · A1 本地验证复查（2026-08-16，Keel 自主推进）

**背景**：大件 A 子项 1 要求「本地 `pnpm tauri dev --features tauri` 验证工作流真实执行状态/结果/失败处理 + RunStore 持久化」。

**环境硬约束（已实证）**：沙箱 `pkg-config` 确认 `webkit2gtk-4.1` 与 `webkit2gtk-4.0` **均 ABSENT**，`cargo build --features tauri` 在链接期失败，真实 Tauri 运行时**无法在沙箱启动**。此约束自 P25 起一致存在，非新风险。

**沙箱侧等价闭环（已实跑）**：
| 验证项 | 结果 |
|---|---|
| `cargo test --lib workflow::runner` | ✅ 2/2（`run_path_executes_and_persists` / `run_path_manifest_location_matches_command`）|
| `cargo test --lib workflow::store::tests`（RunStore 持久化） | ✅ 8/8 |
| `cargo test --lib`（全 lib 套件） | ✅ 680 passed / 0 failed / 19 ignored |
| `cargo clippy --lib` | ✅ 0 warning |

**状态判定（诚实，不谎报）**：
- **P28 沙箱侧 = 已闭环**：Rust 端 `load→execute→持久化` 全链路经单测证明；命令接线严格沿用 P25–P27 既有 feature 门控模式。
- **P28 真实 Tauri 执行路径 = 待 Seeker 本地验收（未关闭）**：UI 运行状态/结果/失败展示、RunStore 在真实 Tauri 环境中的行为，需 Seeker 本地 `pnpm tauri dev --features tauri` 跑通后确认。**此门禁不在 Keel 沙箱能力内，记录为 Seeker 本地任务，不标记已交付。**

> 处置符合大件 A「不问 / 不管 / 记录」节奏：不阻断、不假装通过，记录约束后直接进入子项 2（核心边界重定义）。
