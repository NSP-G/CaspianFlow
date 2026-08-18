# P28 前置核对报告（Keel 2026-08-14）

> 阶段：P28 | 文档：`P28_DESIGN_DOC`（`_AP`）| 方向锁定：L19 | 衔接路径核验：用户指定重点
> 核验原则：不对 P17 引擎另辟执行通路；以「正式写盘 → `Workflow::load` 重载」为唯一运行入口

## 〇、衔接路径核验（用户指定重点）—— **PASS，未另辟通路**

设计文档§三/§四声称的运行入口，逐点对照 P17 真实代码：

| 设计文档调用点 | P17 真实代码 | 结论 |
|---|---|---|
| `run_workflow` 经 `Workflow::load` 加载 `workflows/<name>/workflow.yaml` | `schema.rs:360` `pub fn load(manifest_path: &Path)`；路径约定 `<name>/workflow.yaml` 与 P27 子目录一致（`path` 取 manifest 父目录） | ✅ 完全一致 |
| 调 `WorkflowEngine::execute()` | `engine.rs:111` `pub async fn execute(&self, workflow: &Workflow, inputs: &HashMap<String,Value>) -> WorkflowResult<WorkflowRunResult>` | ✅ 签名一致 |
| 运行态走 `RunStore` 持久化 | `store.rs` 提供 `create_run`/`get_run`/`list_runs`/`update_run`，盘持久化 `<temp>/workflows/<run_id>/run.json` + `steps/<id>.json` | ✅ 不绕开现有路径 |

→ **核心约束达标**：运行 = 读已保存的正式文件（`Workflow::load`）+ 引擎执行（`execute`），全程复用 P17，无第二条执行通路。设计文档§二「不修改 P17 引擎内部逻辑」与代码事实相容。

---

## 一、F 系列裂缝（均为"实现决策层" Gap，非设计意图违规）

### F1 〔硬 · 实现决策〕`AppState` 不存在，需补建

设计文档命令签名假设 `state: State<AppState>`（持有 `registry`/`store`/`executor`/`guardian`），
但**全仓无 `AppState`**；P27 的 `tauri_app.rs` 命令用模块级 `caspian_paths()` 自由函数，不持有引擎。
要真正执行，`run_workflow` 必须构造 `WorkflowEngine`，而它要求：

- `SharedSkillRegistry`（`= Arc<SkillRegistry>`，`registry.rs:404`）
- `Arc<RunStore>`
- `Executor::with_defaults()` / `Guardian::with_defaults()`（`engine.rs:91-92` 测试已用）

代码已提供这些协作者的**来源**：

- 技能注册表：`SkillManager::init(&paths.skills).await`（`skill/mod.rs:55`，异步扫描 `~/.caspian/skills` + 安装内置技能，产出 `SkillRegistry`）
- 运行存储：`RunStore::from_paths(&paths)`（`store.rs:96`）
- 引擎默认件：测试已证实 `Executor::with_defaults()` / `Guardian::with_defaults()`

**处置（推荐）**：P28 在 `run_tauri()` 引入 `AppState`（Tauri `manage`）持有 `SkillManager` + `Arc<RunStore>`；
并让 `SkillManager` 暴露 `SharedSkillRegistry`（把 `registry` 字段改为 `Arc<SkillRegistry>`，`skill/mod.rs` 一行级改动，**属 P28 范围、非 P17**）。
命令改为 `async fn run_workflow(app: AppHandle, state: State<AppState>, name: String)`，
后台 spawn 执行并 emit 事件。
这是对文档"假设已存在"的 Gap 补建，**不改文档意图、不另辟通路**。

### F2 〔中 · 文档与代码不符〕`RunStatus` 无「排队中 / Pending」

`store.rs:29` 引擎枚举 = `{ Running, Completed, Failed, Skipped, Terminated }`，**无 `Pending`**；`create_run` 立即置 `Running`。

设计文档 UI 四态（排队中 / 执行中 / 已完成 / 失败）中：

- 「排队中」在**单运行、无调度队列**（文档§二明确排除调度）前提下无引擎态可对应，只能作为「点击运行 → 收到 `workflow_run_started`」之间的**前端乐观瞬态**，不持久化。
- 文档漏列 `Terminated`（`end` 提前结束，非失败）与 `Skipped`（步骤级，非运行级）。

**处置**：UI 状态机以引擎 `RunStatus` 为准——`Running→执行中`、`Completed`/`Terminated→已完成`、`Failed→失败`、`Skipped`(步骤级)→节点详情展示。「排队中」降级为按钮乐观态。无需改文档范围，仅校准 UI 映射。

### F3 〔中 · 实现决策 · 需用户拍板〕逐步骤事件流引擎不产出

`execute()` 跑到结束才返回 `WorkflowRunResult`，**不发射任何 per-step 进度**（它是库、非 Tauri 命令），
且「P28 不修改 P17」→ 不能在引擎加进度回调。
文档§三期望 `status_update`/`step_update` 与「执行中（第 X/Y 步）」实时进度，但此数据执行期不可用。

两条诚实路径：

- **(a) 最小闭环**：命令 emit `workflow_run_started`（create_run 后）→ spawn `execute` → 结束 emit `workflow_run_finished`/`workflow_run_errored`（带 `WorkflowRunResult`）。UI 仅在 started→finished 间显示 Running；"第 X/Y 步" 仅在终态用最终 `steps.len()` 显示静态总数。**不改动 P17，最稳。**
- **(b) RunStore 轮询近似**：spawn `execute` 同时，后台线程每 ~200ms `get_run(run_id)` 读取已落盘 step 记录，对新增 step emit `step_update`，结束 emit 终态。观察持久化状态（**不改 P17**）即可取得逐步骤进度，但有轮询粒度/竞态近似。

**处置**：需用户拍板 (a)/(b)。推荐 (a) 作 P28 基线（沙箱亦无法实跑，UI 走 mock 网关同样演示形态）。

### F4 〔低 · 实现细节〕`list_runs` 无 `workflow_name` 过滤形参

`store.rs:211` `list_runs(&self) -> Vec<RunRecord>` 返回全量。文档 `list_runs(state, workflow_name: Option<String>)`
需在命令内按 `RunRecord.workflow_name` 内存过滤（trivial，字段已存在）。

### F5 〔低 · 命名与 DRY〕`RunStatus`/`RunResponse`/`RunSummary` DTO

- `RunStatus` 已在 `store.rs` 存在且 `Serialize` → `get_run_status` **直接复用引擎 `RunStatus`**，勿另定义同名 DTO（冲突）。
- `RunRecord` 已 `Serialize` → `list_runs` 命令可直接返回 `Vec<RunRecord>`（或瘦身 `RunSummary`），勿重复造。
- 仅 `RunResponse { run_id, status }` 需新建；`run_workflow` 同步返回它（run_id），执行后台跑。

### F6 〔中 · 契约决策 · 需用户拍板〕运行前必须存在正式写盘文件

衔接路径 = 「正式写盘 → load 重载」。`run_workflow(name)` 仅收 name，load `workflows/<name>/workflow.yaml`。
若用户未显式保存（仅草稿），正式文件缺失 → `Workflow::load` 以 `ParseError`（io 包成）失败，提示不友好。

两条处置：

- **(a) 前端 Run 前先调 `save_workflow(doc)` 再 `run_workflow(name)`**（自动保存，仍在正式写盘通路，无新路）。
- **(b)** 命令检测正式 manifest 缺失 → 返回明确「请先保存（Cmd+S）」错误，UI 提示。

**处置**：推荐 (a)（顺滑且守正式写盘前提）；仍需用户确认。

### F7 〔低 · 实现细节〕`execute()` 异步，命令须 spawn 不阻塞 invoke

`run_workflow` 应：`create_run` → emit started → `tauri::async_runtime::spawn` 执行 → 完成后 emit 终态；
同步返回 `run_id`。避免执行（可能秒级/分钟级、spawn 子进程）阻塞前端 invoke。F1 `AppState` 的自然配套。

### F8 〔信息〕`Workflow::name`(YAML) 与目录身份分离（沿用 P27）

`run_workflow(name)` 用目录名做路径键 + `create_run(name)`；manifest `display_name` 供 UI 展示。无冲突。

### F9 〔信息 · 沙箱闭环诚实〕文档§六沙箱策略正确

沙箱仅能闭环「写 `workflows/<name>/workflow.yaml` → `Workflow::load` 成功」（P27 已加 `CaspianPaths::workflows` 字段，确认存在）；
真正 `execute()`（子进程技能）沙箱不可跑，UI 运行态由 mock 网关兜底。无需改。

---

## 二、门禁预期（沿用 P25/P26/P27）

- `cargo test --lib` 不破：新增 P28 单测走纯 `Workflow::load` 闭环（`CaspianPaths::workflows` + 子目录约定）；feature 门控命令不进默认 `cargo test`。
- `cargo clippy --lib` 0。
- 前端 `pnpm build` / `pnpm typecheck` / `pnpm lint` 全绿。
- Tauri 命令编译需 webkit2gtk，沙箱仅语法保证（feature 门控），真实运行态待 Seeker 本地 `pnpm tauri dev`。

---

## 三、决议请求

- ✅ **衔接路径（用户指定核验项）**：PASS，未另辟通路。
- ⏳ **待 Seeker 拍板**：F3 (a/b 事件流模型)、F6 (a/b 保存前契约)。
- 🟢 **可落地实现决策（按推荐处置，无需回问）**：F1（补建 `AppState` + `SkillManager` 暴露 `SharedSkillRegistry`）、F2（UI 状态映射校准）、F4/F5（DTO 复用）、F7（spawn + emit）。

> 收到 Seeker 对 F3/F6 的拍板后，Keel 按老规矩（分层实现 → 真跑验证）开工。
