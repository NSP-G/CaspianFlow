# core-modules.md · 核心模块与可选模块的边界定义

- **作者**：Keel
- **时间**：2026-08-16
- **归属**：大件 A · 子项 2（核心边界重定义）
- **对齐**：DIRECTION_SYNC §3（模块化韧性）、§5（共享机制表）；P30 `ScanReport`（L25–L27）
- **状态**：`DRAFT`（设计草案，待大件 A 收口时并入最终报告）

---

## 0. 为什么要划这条线

DIRECTION_SYNC §3 两条硬约束：

1. 任何**非核心模块**的缺失，不得导致应用启动失败或运行时崩溃。
2. UI 层必须能感知模块缺失，并以用户可理解方式告知。

§4 进一步澄清：「1.0 = 全功能开箱即用成品」——核心模块随 1.0 默认打包启用，附加功能以 `.caspian` 包提供。模块化韧性是**防线**：即使 1.0 全模块默认打包，某模块意外损坏时系统不崩溃、UI 精确告知。

本文件给这条线一个**可落到代码的事实定义**，为 `ModuleRegistry`（见 `ModuleRegistry_DESIGN.md`）提供分类依据。

---

## 1. 两类模块的事实区分

当前架构（P17–P30）里「模块」有两种完全不同的含义，必须分开处理，否则边界定义会自相矛盾：

| 维度 | **编译期代码模块** | **运行期内容模块** |
|---|---|---|
| 形态 | Rust crate/模块（lib.rs 的 `pub mod`） | 磁盘上被扫描发现的**包/资源**（skills、workflows、knowledge 语料、themes） |
| 缺失场景 | 编译进二进制，要么全在要么编译失败——**运行期不存在"缺失"概念** | 目录可为空、包可损坏/缺 manifest——**运行期真实存在"缺失/损坏"** |
| §3 韧性适用 | 间接：其*必需的磁盘资产*（如内置 shell 技能）损坏时不应崩 | **直接适用**：缺失/损坏不得致崩，UI 精确告知 |
| 当前扫描机制 | 无（编译期绑定） | skills → `SkillScanner::scan()`→`ScanReport`（P30 已落地）；其余类别待补 |

> **结论**：§3「模块化韧性」与 §5「ModuleRegistry」真正要管的是**运行期内容模块**。编译期代码模块的"缺失"不是运行时风险，其*必需资产*的损坏才是——而后者恰好已被 P30 的 `ScanReport` 机制覆盖（skills 已落地，workflows/knowledge/themes 待扩）。

---

## 2. 编译期代码模块分类（参考，非 ModuleRegistry 管辖）

用于判断「AppState 里哪些必须有、哪些可降级」：

| 模块 | 分类 | 理由 |
|---|---|---|
| `config`（settings/paths/validation/watcher/migration） | **core** | 无配置/路径，应用无法初始化 |
| `logging` | **core** | 启动即需 |
| `types` | **core** | 共享类型，编译期绑定 |
| `guardian` | **core** | 安全闸门，P17 执行依赖 |
| `router`（providers/health/routing） | **core** | 任何 agent 动作需 LLM 路由 |
| `skill`（engine + registry + scanner） | **core** | 执行引擎本体 |
| `workflow`（engine + scanner + store + runner） | **core** | 执行引擎本体 |
| `commands` / `tauri_app` | **core**（feature 门控） | Tauri 胶水，缺则无 GUI，但 Rust lib 仍可用 |
| `hot_reload` | **utility** | P30 `DirWatcher`；启动失败不影响核心（watcher `Option`，None 即降级） |
| `session` | **core 代码 / optional 内容** | 代码必编；但其*会话数据*缺失不致命 |
| `knowledge` | **core 代码 / optional 内容** | 代码必编；但*语料索引*缺失只是 RAG 不可用，不致命 |

> 代码模块一律编译进二进制，**不存在运行期缺失**。ModuleRegistry 不管辖它们；它只管辖下面的运行期内容模块。

---

## 3. 运行期内容模块分类（ModuleRegistry 管辖对象）

这是边界定义的**主战场**。分类依据：缺失时，应用是否仍能提供"开箱即用成品"的基本能力（§4）。

### 3.1 core-content（必需内容资产，损坏须韧性处理）

随 1.0 默认打包、缺失会显著削弱核心体验，但**仍不得致崩**（§3 硬约束 #1 对所有非代码模块生效）：

| 类别 | 路径（`CaspianPaths`） | 当前扫描/报告现状 | 缺失影响 |
|---|---|---|---|
| **内置技能（built-in skills）** | `paths.skills` | ✅ `SkillScanner::scan()`→`ScanReport`（P30） | 特定能力不可用，UI banner 告知 |
| **内置工作流（built-in workflows）** | `paths.workflows` | ⚠️ `WorkflowScanner::list()` 仅返 `Vec<WorkflowSummary>`，**无 issue 报告**（P30 WS1 未扩，见偏离 D1） | 工作流库空，但执行引擎仍可用 |
| **知识库语料（knowledge corpora）** | `paths.knowledge` | ❌ 无目录扫描器/报告 | RAG 不可用，不致命 |
| **默认主题（default theme）** | `src/index.css` `@theme`（硬编码） | ✅ 内置，不依赖磁盘 | 无（内置兜底） |

### 3.2 optional-content（附加模块，缺失完全无碍）

以 `.caspian` 包或目录形式提供，纯增强，缺失/损坏均不降级核心（§3/§4 的"附加功能"）：

| 类别 | 路径 | 当前现状 | 说明 |
|---|---|---|---|
| **用户/第三方技能包** | `paths.skills/<name>/` | 同上 ScanReport | 与内置技能同目录扫描，统一处理 |
| **用户/第三方工作流** | `paths.workflows/<name>/` | 同上（无报告） | 同上 |
| **自定义主题包** | `paths.themes/<name>/`（P31 新增） | ❌ 待 P31 实现 | 纯 CSS 变量覆盖，缺失回退默认主题 |
| **未来 `.caspian` 功能包** | 待规格 | ❌ 外部规格（§3/§5） | ModuleRegistry 的"运行时重扫"为其预留 |

---

## 4. 边界判定的可操作规则

为让 `ModuleRegistry` 与 `ScanReport` 落地，给出可编码的规则：

1. **启动不失败**：所有内容模块扫描均 `catch` 失败→转 `ScanIssue`，绝不 `panic`/`exit`（P30 `scan_skill_dir` 已是此范式）。
2. **core-content 缺失 → 降级 + 告知**：内置技能/工作流/语料缺失时，应用照常启动，UI（ModuleResilienceBanner / 设置页）精确告知"X 缺失/损坏 + 原因"。
3. **optional-content 缺失 → 静默忽略或轻告知**：第三方包缺失不进 banner 红区，仅在对应管理面板标注"未加载"。
4. **asset 损坏 ≠ 代码缺失**：代码模块永远在；只有其*磁盘资产*可损坏，归 core-content 韧性处理。
5. **默认兜底**：主题永远有内置默认（`src/index.css`）；技能引擎永远有 `SkillManager`（空注册表也可运行）；工作流引擎无工作流也可接收运行请求（返回"无此工作流"而非崩）。

---

## 5. 当前实现状态矩阵（设计草案的事实基线）

| 内容类别 | 扫描器 | 结构化报告(ScanIssue) | 注册表 | 热加载(DirWatcher) | UI 告知 |
|---|---|---|---|---|---|
| skills | ✅ `SkillScanner` | ✅ `ScanReport` | ✅ `SkillRegistry`(`Arc`) | ✅ P30 | ✅ `ModuleResilienceBanner` |
| workflows | ✅ `WorkflowScanner` | ❌ 仅 `Vec<Summary>` | ✅ `WorkflowRegistry` | ✅ P30 | ⚠️ 列表级，无缺失告知 |
| knowledge | ❌ 无 | ❌ | ❌（store 级） | ❌ | ❌ |
| themes | ❌（P31 加） | ❌（P31 加） | ❌（P31 加） | ❌（P31 加） | ❌（P31 加） |

> ModuleRegistry 草案（见 `ModuleRegistry_DESIGN.md`）的目标：把上表逐行补到 ✅，且用**统一抽象**而非每类各写一套。

---

## 6. 偏离与决策记录（子项 2）

- **D1（已记录于 P30 交付报告 §6.1）**：P30 WS1 仅给 skills 加了 `ScanReport`，workflows/knowledge 未扩。本草案将其列为 ModuleRegistry 的**首批补齐项**（不视为回归，是 P30 最小覆盖的自然延伸）。
- **D2**：`.caspian` 功能包（§3/§5 的热升级形态）仍属外部规格，ModuleRegistry 仅**预留"运行时重扫 + 注册表替换"接口**，不实现包加载逻辑（与 P30 WS3 延后口径一致）。
- **D3**：core/optional 的"内容资产"分类不进入 Rust 编译期类型系统（那是代码模块职责），只体现在扫描器的 `ScanIssue` 语义与 UI 呈现层级（banner 红区 vs 面板灰标）。

---

*本文件与大件 A 子项 2 的 `ModuleRegistry_DESIGN.md` 配套。所有分类均来自读代码事实（lib.rs / tauri_app.rs AppState / scanner.rs / paths.rs / P30 交付），非凭设计意图。*
