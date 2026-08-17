# ModuleRegistry 设计草案

- **作者**：Keel
- **时间**：2026-08-16
- **归属**：大件 A · 子项 2（核心边界重定义）
- **对齐**：DIRECTION_SYNC §3/§5（ModuleRegistry 概念）、P30 `ScanReport`/`DirWatcher`（L25–L27）、`core-modules.md`（分类基线）
- **状态**：`DRAFT`（设计草案，实装待 P31/P32 节奏；本文件不写实现代码，只定抽象与边界）
- **硬约束**：不实现 `.caspian` 包加载逻辑（§3/§5 热升级形态属外部规格，见 `core-modules.md` D2）；复用 P30 既有机制，不另辟通路。

---

## 0. 设计目标

把 P30 已经验证的**单类别**韧性机制（`SkillScanner` → `ScanReport` → `ModuleResilienceBanner`）**泛化为统一的多类别 ModuleRegistry**，使得：

1. skills / workflows / knowledge / themes **四类内容模块用同一套抽象扫描、注册、暴露状态**；
2. 启动扫描 + 运行时重扫（DirWatcher）共用一条路径（对齐 §5 共享机制表第一行「ModuleRegistry：启动时扫描可用模块 / 运行时重新扫描目录」）；
3. UI 拿到的是**聚合的 `ModuleStatus`**（哪些加载了、哪些缺失/损坏、原因），而非逐类别各自一份；
4. 对 P28 运行路径**零破坏**——各运行时注册表仍以 `Arc<...Registry>` + `replace_all` 原地更新。

---

## 1. 统一抽象

### 1.1 类别枚举

```rust
// 与 core-modules.md §3 的分类一一对应
pub enum ModuleCategory {
    Skills,
    Workflows,
    Knowledge,
    Themes,        // P31 新增
}
```

### 1.2 扫描器 trait（P30 ScanReport 的泛化）

P30 的 `ScanReport { skills, issues, scanned_dirs }` 是 skills 专用形态。泛化为：

```rust
// 每个类别一个扫描器，产出「该类别的项 + 该类别的遗漏/损坏」
pub trait ModuleScanner: Send + Sync {
    fn category(&self) -> ModuleCategory;
    // 失败一律转 ScanIssue，绝不 panic（沿用 P30 scan_skill_dir 范式）
    fn scan(&self) -> CategoryReport;
}

pub struct CategoryReport {
    pub items: Vec<ModuleItem>,     // 成功加载的模块（含 name/version/path）
    pub issues: Vec<ScanIssue>,     // 缺失/损坏（kind+path+name?+reason）
}
```

- **skills**：`SkillScanner::scan()` 已返回 `ScanReport`，加一层 `impl ModuleScanner` 适配即可（或把 `ScanReport` 直接复用为 `CategoryReport` 的 skills 特化）。
- **workflows**：`WorkflowScanner::list()` 当前仅返 `Vec<WorkflowSummary>`，**需扩为也产出 `issues`**（缺 `workflow.yaml` 的目录、解析失败的 manifest 各记一条）。这是 P30 WS1 的最小覆盖缺口（`core-modules.md` D1）。
- **knowledge**：新增 `KnowledgeScanner`（扫描 `paths.knowledge` 下语料目录，缺索引/损坏记 issue）。当前 knowledge 仅有 store，无目录扫描器。
- **themes**：P31 随主题加载器一并实装（扫描 `paths.themes`，manifest.yaml 解析）。

### 1.3 统一 issue 类型（复用 P30）

`ScanIssue { kind: ScanIssueKind, path: String, skill_name: Option<String>, reason: String }` 已定义。为跨类别复用，将 `skill_name` 泛化为 `module_name: Option<String>`，并给 `ScanIssueKind` 增 `MissingManifest`/`ReadError`/`ParseError`/`ValidationError`（P30 已有）+ 未来 `ChecksumMismatch`（`.caspian` 包用，见 D2）。

---

## 2. ModuleRegistry：编排者

```rust
pub struct ModuleRegistry {
    scanners: Vec<Box<dyn ModuleScanner>>,
    // 各运行时注册表（P28 已用 Arc 包装，原地 replace_all）
    skills: SharedSkillRegistry,            // = Arc<SkillRegistry>
    workflows: SharedWorkflowRegistry,      // 既有
    // knowledge / themes 的运行时索引由各自模块持有，Registry 只聚合状态
}

impl ModuleRegistry {
    pub fn scan_all(&self) -> ModuleStatus {
        let mut all_issues = vec![];
        let mut loaded = 0;
        for sc in &self.scanners {
            let r = sc.scan();              // 失败已转 issue，不崩
            loaded += r.items.len();
            all_issues.extend(r.issues);
            self.register_category(sc.category(), r.items);  // 调各注册表 replace_all
        }
        ModuleStatus { loaded, scanned_dirs: ..., issues: all_issues }
    }

    // 热加载：DirWatcher 触发时只重扫单类别
    pub fn rescan(&self, cat: ModuleCategory) { /* 重扫 + 更新对应注册表 */ }
}
```

- **注册（register_category）**：调各注册表的 `replace_all`/`unregister`，沿用 P28 `SharedSkillRegistry` 的 `Arc` + `RwLock` 原地更新——**已构造的 `WorkflowEngine` 热更新后透明看到新模块**，不破坏 P28 运行路径（沿用 L21 衔接纪律）。
- **不新建 ModuleRegistry 的"动态函数注册表"**：§5 表里"动态函数注册"在**当前架构**即各子注册表（`SkillRegistry`/`WorkflowRegistry`）。ModuleRegistry 是**编排 + 聚合**，不是另造一套函数注册机制（避免 P30 WS3 同款超范围）。

---

## 3. 状态暴露（对齐 P30 前端契约）

P30 已定义前端 `ModuleStatus { skills, issues, scanned_dirs }` 与 `ModuleResilienceBanner`。扩展：

- `get_module_status` 命令（P30 已注册，当前仅返 skills `ScanReport`）→ 改为返**聚合 `ModuleStatus`**（含 workflows/knowledge/themes 的 issues）。前端 `useCaspian.getModuleStatus` 已就位，类型加字段即可，banner 渲染逻辑不变。
- 新增 `ModuleStatus` 字段：`workflow_issues` / `knowledge_issues` / `theme_issues`（或统一 `issues: ModuleIssue[]` 带 `category` 标签，由 banner 分组渲染）。**决策（Keel 拍板，记入偏离 D3）**：用统一 `issues[]` + `category` 字段，避免类型膨胀。
- 热加载事件：P30 已有 `skills_reloaded` / `workflows_changed`；扩 `knowledge_changed` / `theme_changed`（P31 随主题加载器加）。前端 `useCaspian` 订阅后刷新对应面板——复用 P30 `subscribeSkillsReloaded` 同款 `listen` + mock 回退范式。

---

## 4. 与 §5 共享机制表的逐条对齐

| §5 机制 | ModuleRegistry 草案落点 | 现状 |
|---|---|---|
| ModuleRegistry：启动扫描 / 运行时重扫 | `scan_all()` / `rescan(cat)` | skills 已验；其余类别补扫描器 |
| 动态函数注册 | 各子注册表 `replace_all`（P28 `Arc`） | ✅ 已就位 |
| UI 状态查询 | 聚合 `ModuleStatus` + `ModuleResilienceBanner` | skills ✅；扩类别 |
| 文件系统监控 | `DirWatcher`（P30，复用 `ConfigWatcher` 范式） | skills+workflows ✅；扩 knowledge+themes |
| 版本号元数据 | `ModuleItem.version`（来自 manifest） | skills 有 `version`；其余待补 manifest 读取 |

---

## 5. 实装节奏（不在此草案内写代码，仅排期）

| 阶段 | 动作 | 依赖 |
|---|---|---|
| **P31（主题库，A3）** | 实装 themes 扫描器 + `paths.themes` + `DirWatcher` 扩 themes + `theme_changed` 事件 + 设置面板切换 | 本草案 §1.2/§3 |
| **A2 收尾补齐** | workflows `WorkflowScanner` 扩 issues；新增 `KnowledgeScanner`；`get_module_status` 返聚合 `ModuleStatus` | 本草案 §1.2/§3 |
| **未来 `.caspian` 包（外部规格）** | 仅扩 `ModuleRegistry::rescan` 接包加载器的"重扫+替换"，加 `ChecksumMismatch` issue 类型 | 规格到位（core-modules.md D2） |

> 本草案刻意**不绑定 P31/P32 的具体代码**——P31 主题库是独立可交付单元（A3），P32 安全沙箱（A4）改的是 Skill **执行**层而非扫描/注册层（见 A-2 检查点：不冲突）。ModuleRegistry 是横切抽象，随各子项逐步填满上表。

---

## 6. 偏离与决策（子项 2，记入大件 A 最终报告）

- **D1**：P30 WS1 仅 skills 有 `ScanReport`，workflows/knowledge 缺 → 本草案将其列为 ModuleRegistry 首批补齐，非回归。
- **D2**：`.caspian` 包加载逻辑不在此实现（外部规格）；ModuleRegistry 仅预留 `rescan` + `ChecksumMismatch` 接口。
- **D3**：`ModuleStatus.issues` 用统一数组 + `category` 标签（非每类别独立字段），避免前端类型膨胀；与 P30 `ModuleResilienceBanner` 渲染兼容。
- **D4**：ModuleRegistry 定位为**编排/聚合层**，复用既有子注册表（`Arc`+`replace_all`），**不新建独立动态函数注册表**——避免 P30 WS3 同款超范围，严守 §3「不凭空造 ModuleRegistry 设施，只做已存在模块的韧性可观测」的既定边界（L25/L26 已 CONF）。

---

*检查点 A-1（主题库接入前确认核心边界方向稳定）：本草案与 `core-modules.md` 共同给出稳定的 core/optional 分类与统一扫描抽象，P31 主题库将直接落入 `ModuleCategory::Themes` + `DirWatcher` 扩 themes，方向无冲突、无需调整。A-1 结论：**方向稳定，P31 可按本草案推进**。*
