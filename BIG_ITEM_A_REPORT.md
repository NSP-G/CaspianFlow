# 大件 A · 执行与集成 — 最终报告

> 模式：大件自主推进 + 内部检查点记录（L28 生效）。工作纪律「不问 / 不管 / 记录」。
> 周期：2026-08-16 起，4 个子项（A1–A4）全部收口。
> 作者：Keel（自主推进，未逐阶段等反馈）。

---

## 〇、门禁总览（沙箱侧，可复跑）

| 维度 | A1 基线 | A2 | A3 | A4 | 大件 A 终值 |
|---|---|---|---|---|---|
| `cargo test --lib` | 680 | 680 | 686 | **694** | **694 passed / 0 failed / 19 ignored** |
| `cargo clippy --lib` | 0 | 0 | 0 | 0 | **0 warning** |
| `pnpm build` | — | — | 全绿 | (后端零改动) | 全绿 |
| `pnpm typecheck` | — | — | 0 | (后端零改动) | 0 |
| `pnpm lint` | — | — | 0 | (后端零改动) | 0 |

> 注：A1 起沙箱 `webkit2gtk-4.1/4.0` 均 ABSENT，真实 Tauri 编译/运行不在沙箱能力内（L29）。
> 所有「真实 Tauri 交互 / 真实文件系统执行路径」均记录为 **Seeker 本地门禁**，不谎报已交付。

---

## 一、A1 · P28 收尾（0.5d）

**目标**：本地 `pnpm tauri dev --features tauri` 验证工作流真实执行状态/结果/失败处理 + RunStore 持久化。

**结论**：沙箱缺 webkit2gtk，真实 Tauri 无法运行。沙箱侧跑等效 Rust 单测闭环（`workflow::runner` 2/2 + `workflow::store` 8/8 + 全 lib 680 绿）→ **P28 沙箱侧已闭环**。真实执行路径（UI 运行态/结果/失败展示、RunStore 真实环境）待 Seeker 本地 `tauri dev` 验收。

**记录**：见 `P28_DELIVERY_REPORT.md` §七。

---

## 二、A2 · 核心边界重定义 + ModuleRegistry 草案（1d）

**产出**：`core-modules.md` + `ModuleRegistry_DESIGN.md`。

**核心边界**：
- **编译期代码模块**（config / logging / types / guardian / router / skill / workflow / commands）：必编，无运行期缺失概念。
- **运行期内容模块**（skills / workflows / knowledge / themes）：磁盘发现，§3 韧性直接适用——缺失/损坏不得致崩，UI 精确告知。

**ModuleRegistry 定位**：**编排 + 聚合层**，复用既有子注册表（`Arc` + `replace_all`，P28）+ P30 `ScanReport`/`DirWatcher`。**不新建独立动态函数注册表**（守 L25/L26 边界，不凭空造 ModuleRegistry 设施）。

**检查点 A-1**：核心边界方向稳定，P31 主题库直接落入 `ModuleCategory::Themes` + `DirWatcher` 扩 themes，无冲突。✅ 已闭环。

**偏离记录**：D1–D3 见 `core-modules.md`。

---

## 三、A3 · P31 主题库接入（1d）

**产出**：Rust `theme/mod.rs` + paths 字段 + Tauri 命令 + `DirWatcher` 扩 themes；前端 types/lib/store/hook/`SettingsPage`。

**关键机制**：
- `ThemeManager`：扫描 `~/.caspian/themes/`，`validate_theme_css` 禁 `!important`、禁 `@import`、限 `backdrop-filter`≤2、限选择器层级≤2 且无子/兄弟组合符。
- 主题包格式 = 纯 CSS 变量覆盖（无 JS）+ `manifest.yaml`（name/author/version/deps）；active 持久化到 `themes/_active.json`。
- 前端：`lib/theme.ts` 注入 CSS 变量覆盖 + `document.documentElement` 切 `data-theme`；`useAppStore` 持 `customTheme/customThemeCss`。

**检查点 A-1 闭环**：主题库落入 `ModuleCategory::Themes`，无冲突。✅

**门禁**：cargo test --lib 686（680+6）/ clippy 0 / 前端三绿。

**偏差**：主题包格式按 Seeker 规格（纯 CSS + manifest.yaml）落地，**未实现 `.caspian` 包加载器**（外部规格，守 L25/L26）。WS3 待 Seeker/Lantern 规格，见 `WS3_READINESS.md`。

---

## 四、A4 · P32 安全沙箱（1.5d）

### 4.1 代码事实调查（检查点 A-2 前）

读 `skill/schema.rs` + `skill/executor/mod.rs` + 各 runtime adapter 后确认：

1. **`permissions` 字段早已存在**（非 Seeker 指令所述「新增」）：
   `SkillPermissions { fs: Vec<FsPermission{read, write}>, network: bool, shell: bool }`，
   `test_permissions_parsing` 已验证解析正确。**A4 实为「复用既有字段 + 加强制执行 + 加临时目录隔离」**。
2. `Executor::execute()` 已有完整 subprocess 引擎（build_args / ulimit 内存限 / timeout kill / 并发池），但**未做按技能独立临时目录、未强制执行 permissions**（grep 确认 `permissions` 仅被 schema 解析 + 测试消费）。
3. `entry_path()` 返回绝对路径，故将子进程 CWD 改为沙箱临时目录**不会破坏入口脚本解析**。

### 4.2 检查点 A-2 结论

**与 P28 / P30 无冲突**——A4 仅叠加「隔离 + 权限」层：
- 不触 P28 运行路径（`run_workflow` → `Workflow::load` → `WorkflowEngine::execute` → `RunStore` 持久化，L21 链路原样保留）；
- 不触 P30 `ScanReport` / `DirWatcher`；
- P28 三处运行 Shell 技能的测试夹具补 `shell: true` 即恢复（真实 `skill.yaml` 本就该声明所需权限）。✅

### 4.3 实现

| 落点 | 内容 |
|---|---|
| `src-tauri/src/skill/executor/sandbox.rs`（新建） | `SkillSandbox`（`tempfile::TempDir` 私有沙箱目录，持有至 `execute` 返回自动清理）+ `check_runtime_permissions`（shell 门控）+ `apply_sandbox_env`（注入 `CASPIAN_SANDBOX` / `CASPIAN_SKILL_DIR` / `CASPIAN_NETWORK_ALLOWED` / `CASPIAN_FS_READ` / `CASPIAN_FS_WRITE`） |
| `src-tauri/src/skill/executor/mod.rs` | `execute()` 入口加 `check_runtime_permissions` + `SkillSandbox::new()`；CWD 设为沙箱目录；`apply_sandbox_env` 注入策略 env |
| `src-tauri/src/types/error.rs` | 加 `ExecutorError::PermissionDenied { skill_name, reason }` |
| `src-tauri/Cargo.toml` | `tempfile` 由 `[dev-dependencies]` 升为 `[dependencies]`（运行时使用） |
| 测试夹具（3 处） | `workflow/engine.rs` / `workflow/runner.rs` / `commands/executor_commands.rs`：Shell 技能补 `shell: true` |

### 4.4 权限执行矩阵（诚实分级）

| 权限 | 机制 | 本轮状态 |
|---|---|---|
| **写隔离** | 每技能独立 `tempfile::TempDir` 作 CWD，相对写不漏出技能目录，`execute` 返回即自动清理（成败皆然） | ✅ **真执行** |
| **shell 门控** | `shell: false` 且 runtime=Shell → 拒绝 spawn（`PermissionDenied`） | ✅ **真执行** |
| `network` | 声明 + `CASPIAN_NETWORK_ALLOWED` 标记 + 告警；真阻断需 seccomp / Landlock / 网络命名空间 | ⚠️ **声明，OS 级留待 harness** |
| `fs` 读/写路径 | 声明 + `CASPIAN_FS_READ` / `CASPIAN_FS_WRITE` 标记；真限权需 Landlock（Linux）/ sandbox 权限 | ⚠️ **声明，OS 级留待 harness** |

> 写隔离是**不依赖 root/seccomp、可跨平台交付的最高价值保证**，本轮已实装并单测覆盖。

### 4.5 新增测试（8 个，全绿）

- 纯函数门控：`denies_shell_when_false` / `allows_shell_when_true` / `ignores_shell_flag_for_python`
- 沙箱：`dir_is_absolute_and_unique` / `cleanup_on_drop`（验证成败皆清理）
- 集成：`execute_shell_denied_when_shell_false` / `execute_shell_allowed_when_shell_true` / `execute_writes_to_sandbox_not_skill_dir`（验证相对写落在沙箱而非技能目录）

### 4.6 偏差（已记录，非设计违规）

- D-A4-1：`permissions` 字段非「新增」，实为复用既有 `SkillPermissions`（表达力甚至强于指令的 `fs_read/fs_write` 扁平字段——用 `fs:[{read, write}]` 列表）。
- D-A4-2：网络 / fs 路径限权需 OS 级原语，本轮仅声明 + env 标记，真执行留待未来 OS 沙箱 harness（WS 待排期）。

### 4.7 门禁

`cargo test --lib 694`（686→694，+8）/ `cargo clippy --lib 0`。

---

## 伍、内部检查点汇总

| 检查点 | 触发时机 | 结论 |
|---|---|---|
| **A-1** | 主题库接入前确认核心边界方向稳定 | ✅ 方向稳定，P31 落入 `ModuleCategory::Themes` 无冲突（A2/A3 闭环） |
| **A-2** | 沙箱实现前确认与 P28/P30 无冲突 | ✅ 仅叠加隔离+权限层，不触 P28 运行路径 / P30 扫描器，无冲突（A4 闭环） |

---

## 陆、大件 A 偏离清单（纪律：全部记录，未问）

| 编号 | 子项 | 偏离 | 处置 |
|---|---|---|---|
| D-A2-1~3 | A2 | 核心/可选边界划分、ModuleRegistry 定为聚合层不新建设施 | `core-modules.md` 记录 |
| D-A3-1 | A3 | 主题包未实现 `.caspian` 加载器（守 L25/L26 外部规格依赖） | WS3 待规格，`WS3_READINESS.md` |
| D-A4-1 | A4 | `permissions` 非新增，复用既有 `SkillPermissions` | 本报告 §四.6 |
| D-A4-2 | A4 | 网络/fs 限权需 OS 原语，本轮仅声明 | 留待未来 harness |

---

## 柒、待 Seeker 本地验收（非沙箱能力内，不谎报）

1. **A1 / P28**：`pnpm tauri dev --features tauri` 跑通真实工作流执行态 / 结果 / 失败展示 + RunStore 真实环境。
2. **A3 / P31**：`pnpm tauri dev` 真实切主题包（CSS 变量覆盖 + `data-theme` 切换生效）。
3. **A4 / P32**：真实进程确认每技能独立沙箱目录 + `shell:false` 拒绝 + 相对写隔离；OS 级网络/fs 限权待 harness 排期。

## 捌、后续建议

- WS 排期：OS 级沙箱 harness（seccomp/Landlock/网络命名空间）以真正收敛 `network`/`fs` 限权。
- WS3：主题包 `.caspian` 格式规格敲定后补加载器（当前纯 CSS + manifest 已可交付）。
