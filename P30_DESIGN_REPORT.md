# P30 开工设计报告 · 热加载与模块化韧性落地

- **状态**：`PEND`（Keel 自起草案，待 Seeker 批准或要求调整）
- **作者**：Keel（基于读项目文件自定范围，非 Seeker 提供文档）
- **时间**：2026-08-14
- **基线**：DIRECTION_SYNC.md（L19–L24 已 CONF）；P17–P29 交付物；Cargo.toml / package.json 现状
- **衔接纪律**：沿用 P25–P29「前置核对 → 分层实现 → 真跑验证」老规矩；本报告批准后再进前置核对与实现。

---

## 0. 阅读范围与代码事实基线（本报告的事实来源）

本报告的范围判定**全部来自读代码事实**，非凭记忆或设计意图。已读文件与关键事实：

| 文件 | 事实 |
|---|---|
| `Cargo.toml` | `notify = "7"` + `notify-debouncer-mini = "0.5"` 为**默认依赖**（非 feature 门控）；`tauri` 为 optional + `tauri` feature |
| `src-tauri/src/config/watcher.rs` | `ConfigWatcher` 已存在：debounce 500ms + `ArcSwap<Settings>` 原子替换 + `validate` 拒错保旧值 + `on_reload` 回调 + `_debouncer` 裹 `Mutex` 恢复 `Sync`（P24 F7 修过）。**这是热加载的成熟范式，可直接复用** |
| `src-tauri/src/skill/registry.rs` | `SkillRegistry` 纯内存索引，`register_all`/`replace_all`/`unregister` 齐全；无"从磁盘重扫"入口（热更新须靠 `SkillManager::reload()`） |
| `src-tauri/src/skill/mod.rs` | `SkillManager::reload()` 已存在：`scanner.scan()` → `registry.replace_all()`。但**从不返回扫描遗留信息**，且未被任何 watcher 调用 |
| `src-tauri/src/skill/scanner.rs` | `SkillScanner::scan()` 容忍坏技能（缺 `skill.yaml`/解析失败/校验失败 → `warn` 后 `skip`），但 **`scan()` 只返回 `Vec<Skill>`，把被跳过的目录与原因丢在日志里**，调用方/UI 拿不到"缺了什么" |
| `src-tauri/src/commands/skill_commands.rs` | `list_skills` / `reload_skills` / `enable_skill` 等已是写好的 async 函数，但**未标 `#[tauri::command]`、未在 `tauri_app.rs` 注册** → 真实 `invoke("list_skills")` 当前会失败（P26 只建了 mock） |
| `src-tauri/src/workflow/scanner.rs` | `WorkflowScanner::list()` 已跳过 `.drafts/` 与隐藏目录、跳过不可解析 manifest；同样**不返回"被跳过的工作流"清单** |
| `src-tauri/src/tauri_app.rs` | `AppState { paths, manager: SkillManager, store: Arc<RunStore> }`；命令清单里**无 `list_skills` / `reload_skills` / 任何模块状态命令** |
| `src/hooks/useCaspian.ts` | 技能/知识库均为 `MOCK_*` 内存数据；`listSkills` 真实分支调 `invoke("list_skills")`（未注册） |
| `src/index.css` | 主题为纯 CSS `@theme` token，`settings.app.theme`（light/dark）已由 `ConfigWatcher` 热加载驱动；**全仓零 `.caspian-theme` 引用** |
| `DIRECTION_SYNC.md` L15/L17 | 代码事实预警：**`ModuleRegistry` 不存在；`.caspian`/`.caspian-theme` 加载器不存在**（截至 P22，P23–P29 未改变） |

**门禁基线（L24）**：`cargo test --lib` 677 绿；`cargo clippy --lib` 0；`pnpm build/typecheck/lint` 全绿（0 warning）。

> 纪律声明：DIRECTION_SYNC §3「模块化韧性」与 §5「共享机制表」是**设计意图**，本报告核验其当前**未落地**——缺口在 `scan()` 丢弃缺失信息 + `reload()` 不触发/不暴露 + UI 无感知通道。

---

## 1. 目标

把 DIRECTION_SYNC §3 的两条硬约束从"设计意图"变成"代码事实"：

1. **非核心模块缺失/损坏不得致启动失败或崩溃** —— 这条部分已满足（扫描容错），但需补"精确告知缺失内容"。
2. **UI 必须能感知模块缺失，并以用户可理解方式告知** —— 这条**完全未落地**，是 P30 主战场。
3. **把已就绪的热加载机械（`notify` + `ConfigWatcher` 范式）真正接入技能/Skill 注册表与工作流目录**，使磁盘改动在运行期反映到 UI，而非仅 `settings.yaml` 独享。

---

## 2. 范围边界（基于三条线自定）

### 线 1 · 模块化韧性 / ModuleRegistry —— `ModuleRegistry` 不存在，但 **不新建 ModuleRegistry**
- **判定**：DIRECTION_SYNC L15 确认 `ModuleRegistry` 不存在；它是一个"通用动态函数注册"架构设施，需要 `.caspian` 包格式与热升级 5 步流程（§3/§5）共同定义——这些仍是设计意图。P30 **不凭空造 ModuleRegistry**（超范围、需外部规格）。
- **P30 实际补的"韧性"子集**：让**现有**技能/工作流扫描的"缺失/损坏"成为**可观测、可上报、可 UI 呈现**的一等事实。这精准对应 §3「UI 精确告知缺失了什么」而不过度膨胀。

### 线 2 · 热加载（notify） —— **落地**，复用 `ConfigWatcher` 范式
- **判定**：机械就绪（依赖+范式都在）。P30 把 watcher 从"只盯 `settings.yaml`"扩到"盯技能目录 + 工作流目录"，触发 `SkillManager::reload()` 与 `WorkflowScanner` 重扫，并 emit Tauri 事件让 UI 刷新。

### 线 3 · 主题库接入（`.caspian-theme`） —— **延迟 / 需外部规格**
- **判定**：全仓零 `.caspian-theme` 引用、零加载器；包格式仅是 §3 设计意图。**不能凭空造格式**。P30 仅确认"热加载路径（notify+debounce+event）对主题可复用"，并把"需要 `.caspian-theme` 包规格"列为待 Seeker/Lantern 补充的外部信息（详见 §8）。不写任何 `.caspian-theme` 加载代码。

**范围一句话**：WS1 韧性可观测化（必做）+ WS2 热加载接入（必做）+ WS3 主题库接入（延迟，标外部待核）。

---

## 3. 设计与落地方案

### WS1 · 模块化韧性可观测化（Rust 主导，沙箱全可测）

**3.1.1 扫描报告（单一事实来源）**
- `skill/scanner.rs`：新增 `SkillLoadIssue { dir: PathBuf, name: Option<String>, kind: SkillLoadIssueKind, reason: String }`，`kind` ∈ `{ MissingManifest, ReadError, ParseError, ValidationError, SkippedUnknown }`。
- `SkillScanner::scan()` 改为返回 `ScanReport { skills: Vec<Skill>, issues: Vec<SkillLoadIssue> }`；每个被跳过的目录生成一条 `SkillLoadIssue`（带精确路径 + 原因 + 已解析出的 name 若可得）。**保留 skip-don't-crash**（§3 硬约束 #1 不破坏）。
- `workflow/scanner.rs`：`list()` 同步产出 `WorkflowScanReport { workflows: Vec<WorkflowSummary>, issues: Vec<WorkflowLoadIssue> }`（缺 `workflow.yaml` 的目录、解析失败的 manifest 各记一条）。

**3.1.2 管理器持有报告**
- `skill/mod.rs`：`SkillManager` 新增 `last_report: ArcSwap<ScanReport>`；`reload()` 返回 `ScanReport` 并存入；新增 `report() -> ScanReport`（供命令读取）。

**3.1.3 Tauri 命令暴露（补齐 P26 未注册的真实技能命令）**
- 在 `tauri_app.rs`（tauri feature）注册：`get_module_status() -> ModuleStatusDto`、`list_skills()`、`reload_skills()`。DTO 复用 `Skill`/`WorkflowSummary` 序列化（precheck 验证 `Skill: Serialize`）。
- `ModuleStatusDto { loaded: usize, skipped: usize, issues: Vec<IssueDto>, skills_light: Vec<SkillLight> }`，`IssueDto` 含 `dir`/`name?`/`kind`/`reason`（用户可懂的中文归因）。

**3.1.4 前端呈现（§3 硬约束 #2 落地）**
- `useCaspian.ts`：新增 `getModuleStatus()`（真实 `invoke` + mock 返回**合成 issue**，使沙箱可演示"缺失告知"）；`listSkills()`/`toggleSkill()` 真实分支接 `list_skills`/`set_skill_enabled`（顺手修 P26 未注册缺口）。
- `routes/SkillsPage.tsx`：顶部非阻塞 `ModuleResilienceBanner`，逐条列出"X 技能目录缺失 skill.yaml / Y 解析失败：<原因>"，**不阻塞页面、不崩溃**（§3 硬约束 #1/#2 同时达标）。

### WS2 · 热加载接入（复用 `ConfigWatcher` 范式，tauri feature 驱动）

**3.2.1 通用目录 watcher**
- 新建 `src-tauri/src/hot_reload/mod.rs`：`DirWatcher { dir, _debouncer: Mutex<Option<Box<dyn Any+Send>>>, reload: Arc<dyn Fn() + Send + Sync> }`，复用 `ConfigWatcher` 的 debounce(500ms) + 父/目标目录 `watch` + `_debouncer` 裹 `Mutex` 保 `Sync` 写法。差异点：回调是一个**无参 reload 闭包**（调用方注入 `manager.reload()` 或工作流重扫），且支持 `RecursiveMode::Recursive` 以捕获子目录内 `skill.yaml`/`workflow.yaml` 改动。
- `start(handle: tokio::runtime::Handle)`：在 notify 事件线程里用 `handle.block_on(reload())` 驱动 async reload（Skills 的 `reload()` 是 async）；保证不阻塞 invoke、不破坏 P28 `run_workflow` 的 `spawn` 约定。

**3.2.2 接线（仅 tauri feature，`run_tauri()` 内）**
- skills watcher：`|| { let r = block_on(manager.reload()); 存 last_report; emit skills_reloaded(ModuleStatusDto) }`。
- workflows watcher：`|| { 重扫 WorkflowScanner; emit workflows_changed }`，使 `/workflows` 列表与 P28 运行选择器实时刷新。
- `DirWatcher` 结构体放**非 tauri 模块**，默认 `cargo test --lib` 可编译；`start()` 只在 tauri feature 调用（同 `ConfigWatcher` 只编译不跑）。

**3.2.3 事件（UI 刷新）**
- emit `skills_reloaded`（含 `ModuleStatusDto`）、`workflows_changed`；前端 `useCaspian` 新增 `subscribeModuleStatus`/`subscribeWorkflowsChanged`（真实 `listen` + mock 事件总线，沿用 P28 模式）。SkillsPage 订阅后自动刷新列表与 banner；WorkflowsPage 订阅后刷新列表。

**3.2.4 不破坏既有硬约束**
- P28 运行路径持有 `SharedSkillRegistry = Arc<SkillRegistry>`，`replace_all` 原地改 RwLock 内状态 → 已构造的 `WorkflowEngine` 热更新后也能看到新技能，**不另辟路径、不破坏 P17/P28**（L21/L24 衔接保持）。

### WS3 · 主题库接入（延迟，外部待核）
- 不写代码。仅记录：现有主题 = `index.css` `@theme` token + `settings.app.theme`，已随 `ConfigWatcher` 热加载；未来 `.caspian-theme` 包可复用 WS2 的 `DirWatcher` 机制。所需的**包格式/校验/解包规格**缺，见 §8。

---

## 4. 验收标准

| # | 验收项 | 验证层 |
|---|---|---|
| 1 | `SkillScanner` 对"缺 skill.yaml / 解析失败 / 校验失败"目录各产出一条 `SkillLoadIssue`（精确路径+原因），且仍返回其余有效技能、不崩溃 | Rust 单测（沙箱） |
| 2 | `SkillManager::reload()` 返回 `ScanReport` 且可被 `report()` 读取；`replace_all` 后注册表与报告一致 | Rust 单测（沙箱） |
| 3 | `get_module_status` 命令返回 `loaded`/`skipped`/`issues`；`issues` 含用户可懂归因 | Rust 单测 + 本地 `tauri dev`（Seeker） |
| 4 | 技能目录改动 → `skills_reloaded` 事件 → `SkillManager` 注册表刷新且 `ModuleStatus.issues` 同步更新（新增坏目录实时可见） | 本地 `tauri dev`（Seeker）；watcher 构造/manual_trigger 单测（沙箱） |
| 5 | 工作流目录改动 → `workflows_changed` 事件 → 画布列表/运行选择器刷新 | 本地 `tauri dev`（Seeker） |
| 6 | SkillsPage 在真实或 mock 下渲染 `ModuleResilienceBanner`，逐条告知缺失/损坏内容；**页面不崩溃、不阻塞** | 前端（沙箱 mock 可演示 + 本地） |
| 7 | WS2 不破坏 P28 运行路径：已构造的 `WorkflowEngine` 在 `replace_all` 后仍读到新技能 | Rust 单测（沙箱，复用 P28 runner 范式） |
| 8 | 主题库接入标记为外部待核，未写任何 `.caspian-theme` 加载代码 | 报告 + 代码核查 |

---

## 5. 沙箱策略

- **Rust 默认构建**：`DirWatcher`/`ScanReport` 均为普通模块（notify+tokio 是默认依赖），`cargo test --lib` 编译并运行；watcher 的 **OS 线程监听不在此断言**（同 `ConfigWatcher` 仅测 `manual_trigger`/`new` 不 panic），实际 inotify 行为由 Seeker 本地 `pnpm tauri dev` 验收。
- **Tauri 命令**：继续 feature 门控（`#![cfg(feature = "tauri")]`），沙箱仅语法保全；真实编译+运行态待 Seeker 本地验收（webkit2gtk 限制同 P25–P29）。
- **前端**：`getModuleStatus` 真实分支走 `invoke`，mock 分支返回合成 issue，使 §3「UI 精确告知缺失」在沙箱即可演示验收 #6。
- **不新增系统依赖**：notify/tokio 已就位；不引 webkit/新 crate。

---

## 6. 门禁预期

| 门禁 | 预期 |
|---|---|
| `cargo test --lib` | 677 → **约 695–700**（新增：scan 报告 ~6、dir watcher manual_trigger/reload ~4、module status/reload ~4、P28 runner 复用校验 #7 ~2） |
| `cargo clippy --lib` | **0** |
| `pnpm build` / `pnpm typecheck` / `pnpm lint` | 全绿，**0 warning** |
| Tauri 命令（webkit 限制） | 沙箱语法保全；本地 `tauri dev` 验收 #3/#4/#5 |

---

## 7. 衔接路径与硬约束（不破坏 P17–P29）

- **P29 字段透传**：`save_workflow` 仍字段无关 JSON→YAML（L23/L24），P30 不触碰。
- **P28 运行路径**：`SharedSkillRegistry = Arc<SkillRegistry>` + `replace_all` 原地更新 → 热加载对运行中的引擎透明，衔接路径未另辟（L21）。
- **P27 子目录约定**：工作流 watcher 复用 `<name>/workflow.yaml` 扫描，跳过 `.drafts/`，与 L17/L18 一致。
- **P24 F7 纪律**：新 watcher 同样把 `!Sync` 的 notify debouncer 裹 `Mutex` 仅 RAII，恢复 `Sync`，不回退 ConfigManager 的兼容性。

---

## 8. 待 Seeker / Lantern 补充的外部信息（WS3 阻塞项）

P30 不阻塞，但为后续阶段需明确：

1. **`.caspian-theme` 包格式与加载器规格**：manifest 字段、`checksum`/`bin`/`lib`/`src` 哪项适用主题、解包落点、与 `index.css` `@theme` token 的映射方式。DIRECTION_SYNC §3 仅列意图，无代码。
2. **主题热加载是否复用 WS2 的 `DirWatcher`** 还是独立 loader（影响 P31+ 是否先在 WS2 预留 hook）。
3. （可选确认）`Skill: Serialize` 当前是否已派生——precheck 会先核；若缺，P30 实现期补（小改动）。

> 第 1、2 条是 WS3 的硬前置；P30 本身（WS1+WS2）不依赖它们。

---

## 9. 预期文件改动清单（批准后的实现期）

**Rust（src-tauri）**
- `src/skill/scanner.rs`：`ScanReport`/`SkillLoadIssue`/`SkillLoadIssueKind`；`scan()` 返回报告（含单测）。
- `src/skill/mod.rs`：`last_report: ArcSwap<ScanReport>`；`reload()` 返回报告；`report()`。
- `src/workflow/scanner.rs`：`WorkflowScanReport`/`WorkflowLoadIssue`（含单测）。
- `src/hot_reload/mod.rs`（新建）：`DirWatcher`（复用 ConfigWatcher 范式 + `start(handle)`）。
- `src/tauri_app.rs`：注册 `get_module_status`/`list_skills`/`reload_skills` + `skills_reloaded`/`workflows_changed` emit；`run_tauri()` 内起两个 watcher。

**前端（src）**
- `src/hooks/useCaspian.ts`：`getModuleStatus()` + 真实 `listSkills`/`toggleSkill` + `subscribeModuleStatus`/`subscribeWorkflowsChanged` + mock 合成 issue。
- `src/types/skills.ts` / `src/types/workflow.ts`：补 `ModuleStatus`/`Issue`/`WorkflowLoadIssue` 类型。
- `src/routes/SkillsPage.tsx`：`ModuleResilienceBanner`（逐条缺失告知）。
- `src/routes/WorkflowsPage.tsx`：订阅 `workflows_changed` 刷新。

**文档**
- 更新 `DIRECTION_SYNC.md`：L25 P30 前瞻 → L26 前置核对 → L27 实现闭环（沿用 L19–L24 节奏）。

---

*报告完。待 Seeker 批准或指出需调整的范围/优先级后，Keel 进前置核对（逐项核 `Skill: Serialize`、watcher Sync 复用、模块状态 DTO 字段名），再分层实现 + 真跑验证。*
