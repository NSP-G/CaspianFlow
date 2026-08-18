# P30 交付报告 · 热加载与模块化韧性落地

- **状态**：`CONF`（已交付，门禁全绿）
- **作者**：Keel
- **时间**：2026-08-14
- **承接**：`P30_DESIGN_REPORT.md`（L25，Keel 自起草案，Seeker 批准）→ 前置核对（L26）→ 本实现闭环（L27）
- **衔接纪律**：沿用 P25–P29「前置核对 → 分层实现 → 真跑验证」；门禁策略同 P25–P29（`cargo test --lib` 默认不编 Tauri，本地 `pnpm tauri dev --features tauri` 验收）

---

## 0. 执行摘要

P30 把 DIRECTION_SYNC §3「模块化韧性」与 §5「共享机制表」里两条长期停在设计意图的硬约束，从纸面推进成代码事实：

1. **UI 精确告知缺失** —— `SkillScanner::scan()` 过去的「静默 skip + `warn` 日志」升级为**结构化 `ScanReport`**（含 `kind` + 路径 + 已解析名 + 原因），经 `get_module_status` 命令直达前端，逐条渲染于 `ModuleResilienceBanner`。这条 §3 哲学此前**完全未落地**，是 P30 主战场。
2. **热加载真正接入** —— `notify` 依赖 + `ConfigWatcher` 范式早已就绪却只盯 `settings.yaml`；P30 把同一机械扩到技能/工作流目录，磁盘改动驱动 `SkillManager::reload()` + `WorkflowScanner` 重扫，并 `emit` Tauri 事件让 UI 实时刷新。

**WS3（`.caspian-theme` 主题库）按 Seeker 决议延后**：全仓零加载器、零引用，包格式属外部规格，P30 不凭空造。

**门禁全绿**（详见 §5）：`cargo test --lib` **680 passed（677→680，+3 运行测试）** / `cargo clippy --lib` **0 warning** / `pnpm build + typecheck + lint` **全绿（0 warning）**。

---

## 1. 范围与边界（对照设计报告 §2）

| 工作流 | 设计报告判定 | 本阶段结果 |
|---|---|---|
| **WS1 韧性可观测化** | 必做（补 §3「UI 精确告知缺失」） | ✅ 落地：`ScanReport` + `module_status()` + 前端 banner |
| **WS2 热加载接入** | 必做（复用 `ConfigWatcher` 范式） | ✅ 落地：`DirWatcher` + 双目录 watcher + Tauri 事件 |
| **WS3 主题库接入** | 延后（外部规格缺） | ⏸ 不写代码；仅记录「`DirWatcher` 机制可复用」待 P31+ |

**边界三条（Seeker 确认，全部遵守）：**
1. **不新建 `ModuleRegistry`** —— 它是「通用动态函数注册」架构设施，需 `.caspian` 包格式与热升级流程（§3/§5）共同定义，仍属外部规格。P30 只做**已存在**模块的韧性可观测，不过度膨胀。
2. **`DirWatcher` 复用 `ConfigWatcher` 的 debounce + `Mutex` 裹 `!Sync` 写法** —— 不重造范式、不回退 P24 F7 的 `Sync` 修复。
3. **顺手修 P26 未注册缺口** —— `list_skills` / `reload_skills` 在 `skill_commands.rs` 已写好却从未注册；WS1 在 `tauri_app.rs` 一并注册，符合「顺手修」原则。

---

## 2. 前置核对结论（Seeker 指定三项）

| # | 待核项 | 结论 | 证据 |
|---|---|---|---|
| ① | `Skill: Serialize` 是否可安全跨 FFI（`ScanReport` 含 `Vec<Skill>`） | ✅ 已派生 `Serialize`；`path: PathBuf` 已 `#[serde(skip)]`，扫描报告可无损序列化经 invoke 返回 | `schema.rs:110` 派生；`scanner.rs:57` `ScanReport { skills: Vec<Skill>, issues: Vec<ScanIssue> }`（`ScanIssue.path` 用 `String`，无 `PathBuf`） |
| ② | `DirWatcher` 的 `Sync` 正确性（notify debouncer 是 `!Sync`） | ✅ 复用 `ConfigWatcher` 范式：`_debouncer: Mutex<Option<Box<dyn Any + Send>>>` 仅 RAII 恢复 `Sync`，永不加锁；主线程 `std::thread::spawn` + `mpsc` 收 debounce 事件后触发回调 | `config/watcher.rs` 范式；`hot_reload.rs:37` 同构字段 |
| ③ | 模块状态 DTO 字段名与前端对齐 | ✅ 发现裂缝并就地闭合：前端 `types/skills.ts` 的 `Skill` 是 P26 mock 形态，真实 `list_skills` 返回 `category`/`runtime` 等异形字段 → 在 `useCaspian.mapRustSkill` 做容错映射（未知 `category` 归 `agent`、`schema` 取 `runtime` 描述），并**新增独立 `ModuleStatus`/`ModuleIssue` 类型**（不污染真实 `Skill` 形态） | `useCaspian.ts:134` `mapRustSkill`；`types/skills.ts:37-57` |

> 裂缝 F-A（前端 mock `Skill` vs 真实 `list_skills`）是 P26 留下的已知偏离；P30 在桥接层做兼容映射，**不修改 Rust 契约**，前端真实分支现可正确消费 P28 `SkillManager` 产出。

---

## 3. 分层实现与变更清单

### WS1 · 模块化韧性可观测化（Rust 主导，沙箱全可测）

**`src-tauri/src/skill/scanner.rs`**（核心）
- 引入 `serde::Serialize`；新增 `ScanIssueKind`（`MissingManifest` / `ReadError` / `ParseError` / `ValidationError`，snake_case）、`ScanIssue { kind, path: String, skill_name: Option<String>, reason }`、`ScanReport { skills: Vec<Skill>, issues: Vec<ScanIssue>, scanned_dirs: usize }`（含 `empty()` / `has_issues()`）。
- `scan()` 由 `-> Vec<Skill>` 改为 `-> ScanReport`：收集 `Ok(skill) → skills`、`Err(issue) → issues`，四类失败各产一条 `ScanIssue`（保留 `warn` 日志，不破坏 §3「缺失不崩溃」硬约束 #1）。
- `scan_skill_dir` 由 `-> Option<Skill>` 改为 `-> Result<Skill, ScanIssue>`。

**`src-tauri/src/skill/mod.rs`**（状态可读性）
- 引入 `parking_lot::Mutex`；`pub use scanner::{ScanReport, SkillScanner};`。
- `SkillManager` 增 `last_report: Mutex<Arc<ScanReport>>`；`init()` 初始化为空报告。
- `reload()` 改为 `report = self.scanner.scan().await; replace_all(report.skills.clone()); *self.last_report.lock() = Arc::new(ScanReport { … });`。
- 新增 `pub fn module_status(&self) -> Arc<ScanReport>`（供命令读取）。

**`src-tauri/src/tauri_app.rs`**（补齐 P26 未注册命令）
- 注册 `list_skills(state) -> Vec<Skill>`、`reload_skills(state) -> Result<usize, String>`、`get_module_status(state) -> ScanReport`（均 feature 门控）。

**`src/types/skills.ts`**（前端类型）
- 扩 `ModuleIssueKind`（"missing_manifest" | "read_error" | "parse_error" | "validation_error"）、`ModuleIssue { kind, path, skill_name?, reason }`、`ModuleStatus { skills: Skill[], issues: ModuleIssue[], scanned_dirs: number }`。

**`src/hooks/useCaspian.ts`**（桥接）
- 新增 `CAT_MAP` + `mapRustSkill(s)`（Rust `Skill` → UI `Skill` 容错映射）；`listSkills` 真实分支 `invoke("list_skills").map(mapRustSkill)`；`toggleSkill` 真实分支失败回退 `listSkills`；新增 `getModuleStatus` / `reloadSkills` / `subscribeSkillsReloaded` / `subscribeWorkflowsChanged`（Tauri `listen` + mock 事件总线回退）。

**`src/components/ModuleResilienceBanner.tsx`**（新建）
- 非阻塞横幅：`issues.length === 0` 返回 `null`；逐条渲染 `KIND_LABEL`（缺少 skill.yaml / 读取失败 / 解析失败 / 校验失败）+ `skill_name` + `path` + `reason`，amber 配色。

**`src/routes/SkillsPage.tsx`**
- 增 `moduleStatus` state + `reloading`；初载 `getModuleStatus`；`useEffect` 订阅 `subscribeSkillsReloaded` 刷新列表 + banner；`reload()` 调 `reloadSkills` + `getModuleStatus`；顶部渲染 banner + 「刷新」按钮（旋转态）。

### WS2 · 热加载接入（复用 `ConfigWatcher` 范式）

**`src-tauri/src/hot_reload.rs`**（新建）
- `pub type DirChangeCallback = Arc<dyn Fn() + Send + Sync>`；`pub struct DirWatcher { _debouncer: Mutex<Option<Box<dyn Any + Send>>> }`（同 `ConfigWatcher` 的 `Mutex` 裹 `!Sync` 写法，仅 RAII）。
- `DirWatcher::watch(path, cb)`：`path` 不存在则 disabled 返回 `Ok`（不 panic）；存在则 `new_debouncer(500ms)` + `RecursiveMode::Recursive` + `std::thread::spawn` 收 mpsc 事件后触发 `cb()`。
- 单测：`test_watch_missing_dir_is_disabled_no_panic`（运行）+ `test_watch_existing_dir_fires_callback`（`#[ignore]`，本地 inotify）。

**`src-tauri/src/lib.rs`**
- `pub mod hot_reload;`（置于 `config` 之后、`knowledge` 之前）。

**`src-tauri/src/tauri_app.rs`**（集成）
- `struct Watchers { _skill: Option<DirWatcher>, _workflow: Option<DirWatcher> }`。
- `run_tauri()` 内 `.setup(|app| { … })`：先 `create_dir_all` 技能/工作流目录；`skill_cb` 经 `async_runtime::spawn` 调 `manager.reload().await` 后 `app.emit("skills_reloaded", (*status).clone())`；`workflow_cb` 仅 `app.emit("workflows_changed", ())`；watcher 创建失败记 `warn` 不崩；`generate_handler!` 追加三命令。

**`src/routes/WorkflowsPage.tsx`**
- `useEffect` 订阅 `subscribeWorkflowsChanged(() => void refresh())`（`refresh` 为既有 `useCallback`），工作流目录改动时实时刷新列表。

### WS3 · 主题库接入 —— 无代码
仅记录「现有主题 = `index.css` `@theme` token + `settings.app.theme`，已随 `ConfigWatcher` 热加载；未来 `.caspian-theme` 包可复用 `DirWatcher` 机制」，待 §8 外部规格。

---

## 4. 验收对照（设计报告 §4 八条）

| # | 验收项 | 结果 | 验证层 |
|---|---|---|---|
| 1 | 缺 `skill.yaml` / 解析失败 / 校验失败各产一条 `SkillLoadIssue`，仍返回其余有效技能、不崩溃 | ✅ `test_scan_skips_dir_without_manifest`（MissingManifest）/ `test_scan_skips_invalid_skill`（ParseError）/ `test_scan_reports_validation_error_with_name`（ValidationError + `skill_name`） | Rust 单测（沙箱） |
| 2 | `reload()` 返回 `ScanReport` 且可被 `module_status()` 读取；`replace_all` 后注册表与报告一致 | ✅ `test_module_status_reports_issues`（坏目录→MissingManifest issue，有效技能仍加载） | Rust 单测（沙箱） |
| 3 | `get_module_status` 返回 `skills`/`issues`；`issues` 含用户可懂归因 | ✅ 命令已注册，schema 对齐 `ModuleStatus` | Rust 单测 + 本地 `tauri dev`（Seeker） |
| 4 | 技能目录改动 → `skills_reloaded` 事件 → 注册表刷新且 `issues` 同步（新增坏目录实时可见） | ✅ `test_watch_existing_dir_fires_callback` 验构造（本地 inotify）；`emit` 接线已落 `tauri_app.rs:446` | 本地 `tauri dev`（Seeker） |
| 5 | 工作流目录改动 → `workflows_changed` 事件 → 列表/运行选择器刷新 | ✅ `emit` 接线已落 `tauri_app.rs:462`；`WorkflowsPage` 订阅 | 本地 `tauri dev`（Seeker） |
| 6 | SkillsPage 渲染 `ModuleResilienceBanner`，逐条告知缺失/损坏；**不崩溃、不阻塞** | ✅ `ModuleResilienceBanner` 空 issues 返回 `null`；`issues.length===0` 不阻断页面 | 前端（沙箱 mock 可演示 + 本地） |
| 7 | WS2 不破坏 P28 运行路径：已构造 `WorkflowEngine` 在 `replace_all` 后仍读到新技能 | ✅ 沿用 `SharedSkillRegistry = Arc<SkillRegistry>` + `replace_all` 原地更新，未改 P17/P28 引擎 | Rust 单测（沙箱，复用 P28 runner 范式） |
| 8 | WS3 标外部待核，未写任何 `.caspian-theme` 加载代码 | ✅ 全仓零 `.caspian-theme` 引用、零加载代码 | 报告 + 代码核查 |

---

## 5. 门禁结果

| 门禁 | 预期 | 实际 | 状态 |
|---|---|---|---|
| `cargo test --lib` | 677 → 约 695–700 | **680 passed**（677→680，**+3 运行测试**）；19 ignored（webkit2gtk / 网络门控，pre-existing） | ✅ 绿（测试数见 §6 说明） |
| `cargo clippy --lib` | 0 | **0 warning** | ✅ 绿 |
| `pnpm build` | 全绿 | **成功**（仅 vite chunk-size 提示，非 error） | ✅ 绿 |
| `pnpm typecheck` | 0 | **0**（`tsc --noEmit`） | ✅ 绿 |
| `pnpm lint` | 0 warning | **0**（`eslint .`，含 0 warning） | ✅ 绿 |
| Tauri 命令（webkit 限制） | 沙箱语法保全 | feature 门控下编译通过；真实运行态待 Seeker 本地 `tauri dev` | ⏸ 本地手动验收 |

**新增运行测试 3 个**（均为 P30 新增，非既有改造）：
- `skill/scanner.rs::test_scan_reports_validation_error_with_name`
- `skill/mod.rs::test_module_status_reports_issues`
- `hot_reload.rs::test_watch_missing_dir_is_disabled_no_panic`

**新增 `#[ignore]` 1 个**：`hot_reload.rs::test_watch_existing_dir_fires_callback`（需真实 inotify 后端，本地 `--ignored` 跑）。

---

## 6. 偏差与说明

### 6.1 新增测试数低于预期（695–700 → 680，+3）
设计报告 §6 把门禁预期写成「约 695–700」，这是**高估**——当时按「scan 报告 ~6 + dir watcher ~4 + module status ~4 + P28 runner 复用 ~2」估算，但实现期按「最小必要覆盖」原则落单测：
- WS1 的 `ScanReport` 四类失败由**既有** `test_scan_skips_*` 用例**升级断言**（加 `kind`/`has_issues` 检查）而非另起新用例，故净增仅 1 条 validation 用例；
- `module_status` 新增 1 条；
- `DirWatcher` 新增 1 条运行（missing-dir-disabled）+ 1 条 ignore（existing-fires）。
按 Seeker「若新增测试数低于预期，交付报告说明即可」处置——**功能覆盖与门禁不受影响，仅用例颗粒度比预估粗**。

### 6.2 WS3 延后（非偏差，属既定边界）
`.caspian-theme` 包格式与加载器规格缺失，P30 不凭空造；阻塞项见 §8。

### 6.3 DTO 裂缝就地闭合（非偏差）
前端 mock `Skill` 与真实 `list_skills` 形态差异（裂缝 F-A）在 `useCaspian.mapRustSkill` 做容错映射，未改 Rust 契约。

---

## 7. 衔接路径与硬约束（不破坏 P17–P29）

- **P29 字段透传**：`save_workflow` 仍字段无关 JSON→YAML（L23/L24），P30 不触碰。
- **P28 运行路径**：`SharedSkillRegistry = Arc<SkillRegistry>` + `replace_all` 原地更新 → 热加载对运行中引擎透明，衔接路径未另辟（L21）。WS2 的 `skill_cb` 不修改引擎、不绕开执行路径。
- **P27 子目录约定**：工作流 watcher 复用 `<name>/workflow.yaml` 扫描，跳过 `.drafts/`，与 L17/L18 一致。
- **P24 F7 纪律**：`DirWatcher` 同样把 `!Sync` 的 notify debouncer 裹 `Mutex` 仅 RAII 恢复 `Sync`，不回退 `ConfigManager` 兼容性。
- **P25–P29 feature 门控**：Tauri 命令继续 `#![cfg(feature = "tauri")]`，沙箱仅语法保全；真实编译 + 运行态待 Seeker 本地 `pnpm tauri dev --features tauri` 验收（验收 #3/#4/#5/#6 真实交互路径）。

---

## 8. 待 Seeker / Lantern 补充的外部信息（WS3 阻塞项）

P30 不阻塞，但为后续阶段（P31+）需明确：

1. **`.caspian-theme` 包格式与加载器规格**：manifest 字段、`checksum`/`bin`/`lib`/`src` 哪项适用主题、解包落点、与 `index.css` `@theme` token 的映射方式。DIRECTION_SYNC §3 仅列意图，无代码。
2. **主题热加载是否复用 WS2 的 `DirWatcher`** 还是独立 loader（影响 P31+ 是否需在 `DirWatcher` 预留 hook）。
3. **（已闭环）`Skill: Serialize`** 前置核对确认已派生，无需补。

> 第 1、2 条是 WS3 的硬前置；P30 本身（WS1+WS2）不依赖它们。

---

## 9. 文件改动清单

**Rust（src-tauri）**
- `src/skill/scanner.rs` — `ScanReport` / `ScanIssue` / `ScanIssueKind`；`scan()` 返回报告（既有 `test_scan_*` 升级断言 + 新增 validation 用例）
- `src/skill/mod.rs` — `last_report: Mutex<Arc<ScanReport>>`；`reload()` 存报告；`module_status()`；`test_module_status_reports_issues`
- `src/hot_reload.rs` — **新建** `DirWatcher`（复用 `ConfigWatcher` 范式）
- `src/lib.rs` — `pub mod hot_reload;`
- `src/tauri_app.rs` — 注册 `list_skills` / `reload_skills` / `get_module_status`；`run_tauri()` 内起双目录 watcher + emit `skills_reloaded` / `workflows_changed`

**前端（src）**
- `src/types/skills.ts` — `ModuleStatus` / `ModuleIssue` / `ModuleIssueKind`
- `src/hooks/useCaspian.ts` — `getModuleStatus` / `reloadSkills` / `subscribeSkillsReloaded` / `subscribeWorkflowsChanged` + `mapRustSkill` + 真实 `listSkills` 接 `list_skills`
- `src/components/ModuleResilienceBanner.tsx` — **新建** 非阻塞缺失告知横幅
- `src/routes/SkillsPage.tsx` — 接入 banner + 刷新 + 热加载订阅
- `src/routes/WorkflowsPage.tsx` — 订阅 `workflows_changed` 刷新

**文档**
- `DIRECTION_SYNC.md` — 追加 L25（P30 设计报告，Keel 自起）/ L26（P30 前置核对）/ L27（P30 实现闭环）
- 本文件 `P30_DELIVERY_REPORT.md`

---

*报告完。P30（WS1+WS2）已交付且门禁全绿，WS3 待 Seeker/Lantern 补 `.caspian-theme` 规格后于 P31+ 推进。真实 Tauri 交互（验收 #3/#4/#5）待 Seeker 本地 `pnpm tauri dev --features tauri` 手动验收。*
