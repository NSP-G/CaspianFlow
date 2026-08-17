# 方向同步基线 (Direction Sync Baseline)

- **ST**: `CONF` — 已纳入开发基线，Seeker 于 2026-08-11 正式下达；第 7 条基石「同步纪律」2026-08-11 追加（见 §6）
- **持有方**: Keel 工作记忆（跨阶段留存，非路线图手册——路线图手册由 Seeker 维护）
- **范围**: P22 之后、P23 及后续阶段的设计前提

---

## ⚠️ 代码事实预警（重要）

以下 5 项是**设计意图 / 目标架构**，不是既有实现。截至 P22 交付（2026-08-11），代码实测：

| 方向同步提到的设施 | 代码现状 |
|---|---|
| `ModuleRegistry` / 动态函数注册 | **不存在** |
| `TurboAuto` / 问答路由引擎 | **不存在** |
| `.caspian` 模块包 / 加载器 | **不存在** |
| 热升级 5 步流程 | **不存在** |
| `tauri::Builder`（Tauri 运行时） | **未集成**（main.rs 仅 `tokio::runtime::Builder`） |

> **纪律**：P23 设计开始时逐项核代码事实，不假定上述设施可用。这与 P22 修正 `ModelAdapter`（P23 占位、代码为零）是同一纪律。

---

## 1. 暗语系统 v1（Seeker↔Keel 通讯）

不强制使用，出现时需识别。

| 符号 | 含义 |
|---|---|
| `!` | 阻断 |
| `?` | 提问 |
| `+` | 同意 |
| `-` | 反对 |
| `@P22` | 引用阶段 |
| `#N1` | 引用编号 |
| `→` | 继而 |
| `¬` | 否定 |
| `∧` | 且 |
| `∨` | 或 |
| `ST` | 状态 |
| `CK` | 核查 |
| `AP` | 批准 |
| `FX` | 修复 |
| `REV` | 复审 |
| `DONE` / `WIP` / `PEND` / `BLD` | 完成 / 进行中 / 待定 / 阻断 |

**确认规范**：`RCV`(已收未判) · `UND`(已解逻辑成立) · `AP`(批准执行) · `CONF`(验证通过落定) · `¬AP`(未批准)
**文件后缀**：`_PEND` / `_UND` / `_AP` / `_¬AP` / `_CONF`（标明文档卡在谁手里）

---

## 2. TurboAuto 新设计（知识库问答路由）

- 重新定位为**可选的问答执行路径**，非必要组件。基础问答在主 LLM 路径上完整可跑。
- **三层结构**：L0 用户显式覆盖(`@main`/`@turbo`) → L1 系统自动判断 → L2 默认(主 LLM)
- **路由规则**(`auto` 模式)：
  - 对话 token 使用率 ≥ 70% → 小 LLM
  - 问题含"为什么/如何/分析"等 → 主 LLM
  - 检索片段 > 3 条 → 主 LLM
  - TurboAuto 不可用 → 降级主 LLM
  - 以上都不满足 → 主 LLM(默认)
- **输出透明度**：每次问答返回 `meta.engine` 与 `meta.reason`，用户可见。

> **前向注 L4**：P22 的 `QAResponse { answer, sources }` **无 `meta` 字段**。TurboAuto 接入时须扩展 `QAResponse` 或新增 `QAMeta { engine, reason }`。

---

## 3. 模块化韧性（第 6 条核心思想，新增）

**最高检验标准**：模块缺失时系统仍能运行，且 UI 精确告知缺失了什么，而非报错/崩溃。

**两条硬约束**：
1. 任何非核心模块的缺失，不得导致应用启动失败或运行时崩溃。
2. UI 层必须能感知模块缺失，并以用户可理解方式告知。

**热升级落地**（拖拽式，共 5 步，无需重启）：
GitHub 下载 `.caspian` → 拖入"模块管理" → 验证 → 解包 → 替换 → 加载

**模块包格式 `.caspian`**：
```
knowledge_v2.caspian
├── manifest.yaml      # module / version / requires
├── checksum.txt       # SHA256
├── bin/               # 编译好的二进制
├── lib/               # 动态库
└── src/               # 源码（懒加载时编译）
```

---

## 4. 模块化韧性 与 1.0 的关系

- **1.0 = 全功能开箱即用成品**，不是骨架。
- 模块化韧性是保障成品稳定性的**防线**；热升级是**便利**，非用户须自己拼装的理由。
- 基础功能默认打包启用；附加功能以 `.caspian` 包提供。
- 价值场景：即使 1.0 全模块默认打包，某模块意外损坏时系统不崩溃、UI 精确告知。

---

## 5. 模块化韧性 + 热升级 的共享机制

| 机制 | 韧性使用 | 热升级使用 |
|---|---|---|
| ModuleRegistry | 启动时扫描可用模块 | 运行时重新扫描目录 |
| 动态函数注册 | 模块缺失时无该条目 | 模块更新时替换条目 |
| UI 状态查询 | 显示"知识库模块未加载" | 显示"模块已更新，请刷新" |
| 文件系统监控 | — | 监听模块目录变化 |
| 版本号元数据 | — | 注册表记录版本 |

---

## 6. 同步纪律（第 7 条基石，新增）

**定义**：同步不是单向的信息推送，是**双向消除信息差**——让每个协作方手里拿到的都是同一份事实版本；任何一方发现的信息缺口或技术变化，都能被另一方看到并复用。

**两条硬约束**：

1. **技术同步**：架构决策主动对齐当前可用的最优技术实践。不追逐、不滞后，保持对技术现状的认知。
2. **状态同步**：任何阶段交付后，**设计文档、代码事实、交付报告三者必须对齐**。不同步的状态不视为已交付。

**互帮互助的落地形式**：

| 场景 | 同步动作 |
|---|---|
| 发现某新技术/新库可能对项目有用 | 记入下方「技术观察」栏，标注评估状态 |
| 完成一个阶段交付 | 交付报告 + 设计文档 + 代码三者对齐后，状态标记同步更新至路线图 |
| 遇到信息缺口（某依赖行为不确定） | 在交付报告中列为「待核」，对方同步补充核查结果，双方都拿到同一份事实 |
| 外部技术栈重大更新 | 架构层面评估是否影响当前方向，结果纳入本基线 |

**技术观察（technology watch）**：

| 日期 | 技术 / 库 | 评估状态 | 备注 |
|---|---|---|---|
| 2026-08-11 | （待协作方按场景填入） | 观察中 | 由发现方补充，标注评估状态 |

> 本条基石由 Seeker 于 2026-08-11 追加。首条落地实例见 L6：AGENT.yaml `default_model` 字段——Seeker 称「已存在、未接入」，Keel 核代码事实 `grep default_model src/` 零命中；按状态同步纪律列为「待核」，双方对齐后再定 P24 是否依赖该字段。

---

## 遗留 / 前向动作 (Open Items)

| # | 事项 | 归属阶段 | ST |
|---|---|---|---|
| L1 | 单字检索：前端提示"至少 2 个字符"（非仅代码注释） | UI 集成时 | `PEND` |
| L2 | P21 `search_messages` 中文失效修复（`unicode61` 同坑） | 触碰 sessions.db 写入路径时(P23/P24) | `PEND` |
| L3 | D4 规模基准(10000×800)在 P23 引入向量时重测 | P23 向量化 | `PEND` |
| L4 | P22 `QAResponse` 扩展 `meta.engine`/`meta.reason` | TurboAuto 接入时 | `PEND` |
| L5 | **P23 前提**：P21 会话写入路径是否影响 P23 | P23 设计前置 | `WIP`（分析见 §下） |
| L6 | AGENT.yaml `default_model`：Seeker 称「已存在、未接入」，Keel 核代码事实 `grep default_model src/` 零命中。2026-08-11 决议：P24 v1 不做硬依赖，对「Agent 默认」环节做兼容兜底（字段存在则读 / 不存在则回退全局默认），不阻塞设计 | P24 设计前置 | `CONF`(兼容兜底) |
| L7 | P24 v1 范围锁定：核心（身份偏好优先级链 Task>Skill>Agent>Global + fallback 降级）+ 新建 `health.rs` 可用性探测（降级触发）+ `ask` 的 `model` 占位接真实路由 + 设计 `skill.yaml` 的 `preferred_model`；排除完整负载均衡调度器与成本优化 | P24 | `CONF`(Seeker 确认) |
| L8 | P24 前置核对（Keel 2026-08-12）：设计文档 `_AP` 退回 `_PEND`。阻断 B1/B2/B3 + 事实错误 F1-F5（详见 `P24_PRECHECK.md`）。2026-08-12 Seeker 采纳处置，修订中 | P24 设计前置 | `REV`(Seeker 采纳,修订中) |
| L9 | Seeker 对 P24 precheck 处置决议（2026-08-12）：B2→在现役 `ModelConfig` 扩字段(`preset`/`health`/`fallback`)+`ConfigManager` 包路由层(不新建 models.yaml)；B3→30+ 清单仅作 preset 命名/字段参考,代码不直接加载(独立参考文档);F4→`KnowledgeQAService` 不注入 skill/agent 字段,改调用方传入或 `ModelRegistry` 从配置解析。B1(范围策略)+F1-F5 默认纳入修订。待 Seeker 发 `_AP` 修订版后 Keel 跑第二轮 precheck | P24 设计前置 | `REV`(处置已确认) |
| L10 | Seeker 第二轮修订方向细化确认（2026-08-12）：B1→通用 `OpenAICompatibleProvider`+少量原生 Provider(Anthropic/GLM)+`CustomProvider`,30+ 服务商作 preset 配置数据行(非 30+ 独立实现类);F5→验收7改为集成现役 `keystore.rs`(${ENV_VAR}),不重复造。预计 2026-08-13 发 `P24_DESIGN_DOC_v2_AP.md`(经 Jason 转发)。Keel 第二轮 precheck 对照 B1-B3/F1-F5/C1-C4 | P24 设计前置 | `REV`(方向已细化) |
| L11 | P24 第二轮前置核对（Keel 2026-08-12）：v2 文档 `_AP` 主体全绿——B1-B3/F1-F5/C1-C4 全部落位且与代码事实一致。新发现 F6(`ModelConfig` 无 `default` 字段,§4 全局默认机制悬空,§13 漏列 `default`)+C5(§6 `ask` 示例 `router`/`self.context` 来源悬空,与 F4 冲突)。均非硬阻断,建议按推荐方案开工:扩 `default: bool`+`Settings::default_model()`;KnowledgeQA 持有 `Arc<ModelRouter>`+`ask` 增偏好参数。**2026-08-13 Seeker 核准 F6/C5 推荐方案 → P24 已分层实现并真跑验证（门禁全绿）。F6→`ModelConfig` 扩 `default: bool` + `Settings::default_model()`(高优先级优先);C5→`KnowledgeQAService` 持有 `Arc<ModelRouter>`,`ask` 增 `skill_preferred`/`agent_default` 可选参数,`new(store,provider,embedder,router)` 增参** | P24 设计前置 | `CONF`(v2 可开工, F6/C5 实现拍板, 已交付) |
| L12 | P24 实现期补充裂缝（Keel 2026-08-13 闭环,非设计文档露出项）：① **F7**——`reqwest` 现役未列直接依赖(仅经 `fastembed` 4 间接锁在 0.12.28),而 `providers.rs` 需它做 HTTP 调用;已升为直接依赖(`features=["json","rustls-tls"]`,不引系统 OpenSSL),`cargo check` 通过。② **`ConfigManager` 因 `config/watcher.rs` 的 `notify` debouncer(`!Sync`)而 `!Sync`,使持有 `Arc<ModelRouter>`(内含 `Arc<ConfigManager>`)的 `SqliteKnowledgeQA` 在 `Send` async 方法下编译失败;已将 debouncer 字段裹入 `Mutex`(仅 RAII、永不加锁)恢复 `Sync`,热加载不受影响** | P24 实现 | `CONF`(已闭环) |
| L13 | P25 前置核对（Keel 2026-08-12）：设计文档 §三/§八/§九「技术栈已确认、直接沿用」与代码事实严重错位——**前端层与 Tauri 集成层均不存在**。F 系列裂缝：F1 前端骨架全缺(package.json/vite/ts/tailwind/index.html/`src/` 无一存在)；F2 Tauri 未接入(`Cargo.toml` 无 `tauri`/`tauri-build`、`Cargo.lock` 中 tauri=0、无 `tauri.conf.json`、`main.rs` 为纯 tokio 二进制、`commands/*.rs` 仅注释未注册 invoke handler)；F3 §九 五个集成点(send_message/list_sessions/get_data_path/chat_stream_chunk/agent_status)全缺；F4 阿里巴巴普惠体 3.0 非自由再分发、沙箱无文件(Geist Mono 经 @fontsource 可取)；F5 ts-rs 未列入 Cargo.toml。C 系列冲突：C1 shadcn 默认 8px+shadow 与 §二(≤4px 禁 shadow)冲突需 token 覆盖；C2 Tailwind v4(CSS-first)与 shadcn v3 流程不同；C3 自绘 vs 原生标题栏二义；C4 验收#8 三平台本沙箱不可验证；C5 mock 行为未定义；C6 暗/浅色无 token 载体；C7 Tauri 配置/构建前置未单列。**决策(D1)前端+TS mock IPC + Tauri 配置/Rust mock command 一并产出(标本地编译)；(D2)自绘标题栏(decorations:false, macOS Overlay)** | P25 设计前置 | `CONF`(裂缝闭环, 两决策已拍板) |
| L14 | P25 实现闭环（Keel 2026-08-12）：从零搭建前端(React19+TS+Vite+Tailwind v4+Zustand v5+react-router v6+lucide+motion+cmdk+@tauri-apps/api+@fontsource/geist-mono),§二 约束落地为 `@theme` token(4px 半径/零阴影/中性灰+冷灰蓝 #4A6B8A/Lucide/字体栈);实现 TitleBar/Sidebar/ChatPage/MessageList/MessageBubble/ChatInput/StatusIndicator/Cmd+K 命令面板骨架/双 store/TS mock IPC(回声+THINKING→STREAMING→IDLE 状态机+分块流)。Tauri glue 产出(`tauri.conf.json`/`build.rs`/optional `tauri` 依赖+feature+`required-features` bin/`src/bin/caspian-gui.rs`+mock `#[tauri::command]` emit agent_status/chat_stream_chunk),全部经 `tauri` feature 门控,默认 `cargo test --lib` 不编 Tauri。**沙箱门禁:vite build 通过 / tsc --noEmit 0 / eslint 0(含 0 warning) / cargo test --lib 658 绿**。验收 #1/#8(窗口启动/三平台)仅能由 Seeker 本地手动验证(沙箱缺 webkit2gtk-4.1,无法编译启动 Tauri) | P25 实现 | `CONF`(已交付, 本地手动验收待 Seeker) |
| L15 | P26 前置核对（Keel 2026-08-12）：报告 §三/§六「复用 P25 组件 / 侧边栏菜单项已占位」与代码事实部分错位——均非硬阻断,报告 §二/§四 已给出足落地规格,按老规矩对齐事实后直接实现。F 系列裂缝：F1 `routes/index.tsx` 不存在,P25 路由实际内联在 `App.tsx` 的 `<Routes>`(报告 §八 写"在 App.tsx 新增路由"与事实一致,但 P25 文档结构声称的 `routes/index.tsx` 从未建);F2 侧边栏"技能/知识库"菜单项根本不存在(报告 §六 称"已占位"为假,实际只有 新对话/会话/设置),需从零加;F3 `Card`/`Switch` 组件均未建(报告 §三 "复用 Card/Switch" 为空头支票),需新建;F4 `useCaspian` 无 listSkills/toggleSkill/listDocuments/deleteDocument/importDocument,需新增 mock 方法;F5 设计 token 无 danger/红色(报告 §二 要求"删除按钮用危险色(红色)"),需新增 `--color-danger`(`--color-danger-foreground`)暗+浅两态 token。**决策：报告已全规格化,无需回问,直接实现**(mock 数据 + P25 同款 `tauri` feature 门控/IPC 模式,Rust 侧零改动) | P26 设计前置 | `CONF`(裂缝闭环, 直接开工) |
| L16 | P26 实现闭环（Keel 2026-08-12）：新增 `/skills`(Skill 市场:卡片网格≥6、实时搜索、分类筛选 chips、启用/禁用 Switch、点击展开 Schema/权限/触发短语)与 `/knowledge`(知识库:文档列表≥4、导入按钮真实唤起 file picker 后 mock 追加、每行删除危险红、统计文档数/分块数、检索框占位);新建 `Card`/`Switch` 扁平组件 + `--color-danger` token;扩展 `useCaspian`(listSkills/toggleSkill/listDocuments/deleteDocument/importDocument,模块级 mock 状态跨导航持久);`Sidebar` 增加"技能/知识库"导航(展开+折叠两态,useLocation 高亮);`App` 内联 Routes 加两页;`CommandPalette` 增两跳转指令(键盘优先)。**沙箱门禁:vite build 通过 / tsc --noEmit 0 / eslint 0(0 warning) / cargo test --lib 658 绿(Rust 零改动)**。验收 #1-#6 均为手动验证,P26 仅提供 UI 形态与 mock 数据,真实 P22/P24 接口接入在后续阶段替换数据源 | P26 实现 | `CONF`(已交付, 本地手动验收待 Seeker) |
| L17 | P27 前置核对（Keel 2026-08-13）：基于 Fathom #001 决策「模式 C — 显式保存 + 自动草稿快照」;方向 Seeker 已定可直接搭建画布骨架。**关键裂缝**：F1 全仓无工作流定义扫描器(仅 SkillScanner 做 read_dir),验收 #6「引擎忽略 `.drafts/`」无承载方;F2 **P27 文字与「技术依赖」表均称 P17 读扁平 `workflows/*.yaml`,但 `schema.rs` 头注释与 `Workflow::load` 实际约定 `<name>/workflow.yaml` 子目录(`path`=manifest 父目录)**——扁平实现会直接破坏 P17 引擎,故采纳 P17 子目录约定(对 Seeker 文字描述的偏差,以"不破坏已交付依赖"为准绳);F3 `CaspianPaths` 无 workflow 定义目录字段;F4 任务清单写 `pnpm add reactflow`(v11)与 React19 peer 冲突→改用 `@xyflow/react` v12;F5 持久化/草稿/冲突逻辑无落点且沙箱无法跑真实 fs。**决策(选项1):前端真实 Tauri invoke + 沙箱 localStorage fallback;Rust 侧新建 `workflow/scanner.rs`(跳过隐藏目录→天然跳过 `.drafts/`)+ `manifest.rs`(原子写/草稿/mtime 冲突)单测闭环验收 #6** | P27 设计前置 | `CONF`(裂缝闭环, 选项1 已拍板) |
| L18 | P27 实现闭环（Keel 2026-08-13）：Rust 侧——新建 `workflow/scanner.rs`(`WorkflowScanner`/`WorkflowSummary`,遍历子目录跳过隐藏目录、`load` 取 mtime、单测验「跳过 `.drafts/`」+「`<name>/workflow.yaml` 加载」)+ `workflow/manifest.rs`(原子写正式文件 temp+rename、`save_draft` 写 `.drafts/`、`read_raw` 原样回传保留 `ui`、`delete_workflow` 清草稿、`save_workflow` 带 `expected_mtime` 冲突检测)+ `CaspianPaths.workflows` + `ensure_dirs` + `WorkflowError::Conflict` 变体;`workflow/mod.rs` 挂载并导出;Tauri 工作流命令(`list_workflows`/`load_workflow`/`save_workflow`/`save_workflow_draft`/`delete_workflow`)经 `tauri` feature 门控,JSON↔YAML 转换。**沙箱门禁:vite build 通过 / tsc --noEmit 0 / eslint 0(0 warning) / cargo test --lib 675 绿(658 + 17 新增 P27 单测) / cargo clippy --lib 0**。**前端**:装 `@xyflow/react` v12;新增 `/workflows`(列表/新建/删除)+ `/workflows/:name`(React Flow 画布:拖拽节点/连线/删除、`display_name`/描述编辑、500ms 防抖草稿自动保存、`Cmd/Ctrl+S` 原子保存并清草稿、mtime 冲突提示);`useCaspian` 扩 5 个 workflow mock 方法(Tauri invoke + localStorage fallback);`Sidebar` 加"工作流"项(展开+折叠两态)、`CommandPalette` 加跳转。验收 #1/#2 沙箱可构建验证;#3/#4/#5 真实写盘+冲突逻辑由 Rust 单测覆盖(沙箱无 webkit 无法跑真实 fs,UI 路径走 localStorage fallback),手动 fs 验收待 Seeker 本地 `tauri dev` | P27 实现 | `CONF`(已交付, 本地手动验收待 Seeker) |
| L19 | P28 前瞻方向（Seeker 2026-08-13 口述,未出设计文档）：**工作流执行入 UI**——让 P27 画布排好的工作流真正被 P17 引擎跑起来,而非停在编辑层。预期落点：(1) 编辑器增「运行」按钮,经 Tauri invoke 调 P17 `WorkflowEngine`(已交付 `engine.rs`/`scheduler.rs`/`dag.rs`/`store.rs`),把画布节点+连线序列化为 P17 可加载的 `workflow.yaml`,发起运行;(2) UI 订阅运行态(StepResult/WorkflowRunResult/进度/日志),以节点高亮/边流动/状态徽章呈现;(3) 运行记录对接 `store.rs`(`RunStore` 持久化 `<temp>/workflows/<run_id>/`)。**关键约束**：① P17 引擎读取的是 `<name>/workflow.yaml` 子目录(沿用 L17/L18 子目录约定),UI 运行为`save_workflow` 的正式写盘 + 引擎 reload,需规避 `.drafts/`;② 编排/调度/执行语义以 P17 为准,UI 不重复实现执行逻辑,只做触发与可视化;③ 真实执行需 webkit2gtk 本地编译,沙箱仅能验证「序列化↔引擎加载」的 Rust 单测闭环,运行状态流由 mock 网关兜底。**启动条件:待 Keel 状态恢复 + Seeker 出 `P28_DESIGN_DOC` 后按老规矩(前置核对→分层实现→真跑验证)起手** | P28 前瞻 | `PEND`(方向已定, 设计文档已发, 前置核对完成待 F3/F6 拍板) |
| L20 | P28 前置核对（Keel 2026-08-14）：设计文档 `_AP` 主体全绿,**衔接路径核验 PASS——未另辟通路**(用户指定重点):`run_workflow` 经 `Workflow::load`(`schema.rs:360`)+`WorkflowEngine::execute`(`engine.rs:111`)+`RunStore`(`store.rs`)闭环,全对得上 P17 真实代码,不修改引擎、不绕开执行路径。**F 系列裂缝(均实现决策层 Gap,非设计违规)**:F1〔硬〕`AppState` 不存在——全仓无 managed state,P27 命令用 `caspian_paths()`;需 P28 引入 `AppState`(`manage`)持 `SkillManager`+`Arc<RunStore>`,并令 `SkillManager` 暴露 `SharedSkillRegistry`(`=Arc<SkillRegistry>`,registry.rs:404),来源 `SkillManager::init(&paths.skills).await`/`RunStore::from_paths`/`Executor`·`Guardian::with_defaults`;F2〔中〕`RunStatus` 无 `Pending`(store.rs:29={Running,Completed,Failed,Skipped,Terminated}),「排队中」降级为前端乐观瞬态,另需覆盖 `Terminated`/`Skipped`;F3〔中·待拍板〕逐步骤事件流引擎不产出(`execute()` 仅返终态,禁改 P17),两路径(a)最小闭环 started/finished 事件 vs (b)RunStore 轮询近似 step 进度;F4〔低〕`list_runs` 无 `workflow_name` 过滤形参,命令内内存过滤;F5〔低〕DTO 复用引擎 `RunStatus`/`RunRecord`,仅新建 `RunResponse`;F6〔中·待拍板〕运行前须正式写盘,两处置(a)前端 Run 前先 `save_workflow` 再 `run_workflow` vs (b)缺正式文件返回「请先保存」;F7〔低〕命令须 `async_runtime::spawn` 不阻塞 invoke;F8/F9 信息项。**门禁预期同 P25/P26/P27**。Seeker 拍板:F3→(a)最小闭环、F6→(a)运行前自动保存 | P28 设计前置 | `CONF`(衔接路径 PASS, F3-a/F6-a 已拍板) |
| L21 | P28 实现闭环（Keel 2026-08-14）：**执行入 UI 落地,衔接路径未另辟通路(沿用 L19/L20)**。Rust 侧——`skill/mod.rs` 改 `SkillManager.registry` 字段为 `Arc<SkillRegistry>`(`SharedSkillRegistry`,registry.rs:404),新增 `shared_registry()`(P28 范围,未改 P17 引擎逻辑);新增 `workflow/runner.rs`(沙箱端到端单测:`SkillManager::init`+`WorkflowEngine::with_defaults`+`execute`+`RunStore` 持久化,真实 shell 技能跑通,闭环「load→execute→持久化」,验收 #7 沙箱等价);`tauri_app.rs` 引入 `AppState`(`manage`,持 paths/`SkillManager`/`Arc<RunStore>`)+ 三命令 `run_workflow`/`get_run_status`/`list_runs`(feature 门控,`run_workflow` 经 `Workflow::load(workflows/<name>/workflow.yaml)` + spawn `execute` + emit `workflow_run_started`/`finished`/`errored`,最小闭环 F3-a)+ DTO。前端——`useCaspian` 扩 `runWorkflow`(运行前先 `saveWorkflow` 自动保存 F6-a)/`getRunStatus`/`listRuns` + `subscribeWorkflowRun`(Tauri `listen` / mock 事件总线);`WorkflowEditorPage` 加「运行」按钮(运行中禁用)+ 状态指示器(排队中乐观态/执行中/已完成/失败,映射引擎 `RunStatus` 含 `Terminated`/`Skipped`,新增 `--color-success` token)+ 结果面板(摘要+逐步骤输出展开,验收 #3/#4/#5)+ 底部最近运行列表(验收 #6/#7)。**沙箱门禁:cargo test --lib 677 绿(675+2 新增 P28 runner 单测)/cargo clippy --lib 0/pnpm build+typecheck+lint 全绿**。Tauri 命令因 webkit2gtk 缺失沙箱仅语法保全(feature 门控),真实编译+运行态待 Seeker 本地 `pnpm tauri dev` 验收(验收 #1/#2/#4/#5/#7 真实执行路径) | P28 实现 | `CONF`(已交付, 本地手动验收待 Seeker) |
| L22 | P29 前瞻方向（Seeker 2026-08-14 口述,未出设计文档）：**节点属性面板**——把 P27 画布中当前只能沿用默认值的节点变成**可配置**,让用户编辑单个节点的 inputs/outputs/params(而非仅 `display_name`/描述),是 P27 画布的自然延伸、也是 P28 执行入 UI 之后必须接上的体验闭环("能跑"→"能配了再跑")。**明确非新功能**:不新增 P17 引擎能力/步骤类型,只把 P17 已支持、但 UI 此前未暴露的 `WorkflowStep` 字段开放为可编辑项。**关键衔接与约束**(前置核对待核):① 画布节点 = `CanvasNode`(`src/lib/workflow.ts`),序列化为 P17 `workflow.yaml` 的 `ui.nodes` 并提供 `WorkflowStep`(步骤数=节点数,连线=depends_on);面板编辑的字段必须能**原样回写 P17 `WorkflowStep` 且不破坏 `Workflow::load` 加载路径**——与 L18/L21 同一硬约束,正式写盘后仍须被 P17 引擎合法加载(衔接路径仍走「正式写盘 → load 重载 → P28 execute」);② 可编辑字段集须以 P17 `WorkflowStep` 真实结构为准(前置核对须读 `schema.rs` 的 `WorkflowStep` 定义逐字段映射,不凭记忆),典型含 `type`/`params`/`inputs`/`outputs`/`depends_on`/`condition`/`on_error`/`retry`/`timeout` 等;③ 属性面板为前端主导改动(React Flow `selected` 节点 → 侧栏/抽屉表单 → 改 `nodes` state → 经现有 500ms 草稿防抖 + Cmd+S 正式保存落盘),Rust 侧若无新字段需求则**零改动**;④ 沙箱门禁沿用 P25/P26/P27(前端 build/typecheck/lint 全绿;若 Rust 零改动则 cargo test 维持 677);Tauri 真实交互仍受 webkit2gtk 限制(同 P25-P28)。**启动条件:待 Seeker 出 `P29_DESIGN_DOC` 后,Keel 按老规矩(前置核对→分层实现→真跑验证)起手** | P29 前瞻 | `CONF`(已交付, 详见 L23/L24) |
| L23 | P29 前置核对（Keel 2026-08-14）：设计文档 `_AP` 主体方向 PASS(节点变可配置,非新引擎能力),**衔接路径核验 PASS——未另辟通路**:`save_workflow`(`tauri_app.rs:179`)是字段无关 JSON→YAML 透传,前端 `WorkflowStepDoc` 字段名原样落 `workflow.yaml`,`Workflow::load` 按真实名读回;**故「Rust 零改动」成立当且仅当前端用 P17 真实字段名**。**F 系列裂缝(均实现决策层,用户授权"不要问"→Keel 拍板)**:F1/F2/F3〔硬·重大〕文档 §三 的 `params`/`inputs`/`outputs`/`retry`/`on_error` 五项错误——P17 `WorkflowStep`(`schema.rs:127`)真实字段为 `input`(=params,自由 JSON)/`output`(单变量名,非对象)/`condition`/`timeout:Option<u64>`/`retry_count:Option<usize>`(非 `retry`)/**无步骤级 `on_error`**;照字面发射未知字段会被 serde 静默丢弃(回读丢失+违反验收 #6);F4〔中〕`condition` 控件写 Monaco(重依赖,违 P25 扁平纪律)→ 改用等宽 textarea(Geist Mono 已装);F5〔中〕D2「save_workflow 前调 Workflow::load」与「Rust 零改动」矛盾 → 改为前端结构性校验(字段名只发 P17 已知项 + timeout∈[1,300]/retry_count∈[0,5]/input 须 JSON 对象,违则红字+阻止正式保存),最终 load 门控仍是 P17 `Workflow::load`;F6〔低〕D3/D4 前端行为(P27 有 500ms 防抖但缺切换取消防抖+保存互斥)→ 实现。**拍板**:`WorkflowStepDoc` 只含 `{id,skill,input,output,condition,timeout,retry_count,depends_on}`;删步骤级 `on_error`(工作流级 `error_handling.on_step_failure` 不在 P29);`STEP_FIELDS` 契约表落 `lib/workflow.ts` 单一事实来源;前端 build/typecheck/lint 全绿,cargo 维持 677(零改动) | P29 设计前置 | `CONF`(衔接路径 PASS, F1-F6 拍板落地, 实现中) |
| L24 | P29 实现闭环（Keel 2026-08-14）：**节点属性面板落地,衔接路径未另辟通路(沿用 L22/L23)**。前端——`types/workflow.ts` 扩 `WorkflowStepDoc`(input/output/condition/timeout/retry_count,字段名=P17 `WorkflowStep`);`lib/workflow.ts` 加 `StepNodeData`+`docToNodesEdges`按 id 合并步骤字段进节点 data+`nodesEdgesToDoc`写回(undefined 不发射)+`STEP_FIELDS`契约表(§四 D1 单一事实来源)+校验(validateTimeout 1-300/validateRetry 0-5/validateInputJson 须 JSON 对象/docHasErrors);新建 `components/workflow/NodePropertiesPanel.tsx`(input 表单/JSON 双模式可增删键值、output 单行、condition 等宽 textarea、timeout/retry_count NumberInput、字段级红字、底部说明 P17 字段名+on_error 不在步骤级);`WorkflowEditorPage` 选中节点追踪(`key={node.id}` 切换重置)+`updateNodeData`+右侧面板+D3 切换节点 `clearTimeout(draftTimer)`+D4 `savingRef` 保存期间暂停草稿+保存前 `docHasErrors` 门控(红字横幅阻止保存/运行,验收 #4/#6)。**硬约束达标:仅发 P17 真实字段名 ⇒ 经 `save_workflow` 透传落 `workflow.yaml` 必被 `Workflow::load` 加载,且编辑真实作用于 P28 执行**。**沙箱门禁:pnpm build+typecheck+lint 全绿(0 warning)/cargo test --lib 维持 677/clippy --lib 0(Rust 零改动)**。Tauri 命令沙箱仅语法保全(webkit 限制),前端只发 P17 已知字段名故真实写盘等价 load-safe,待 Seeker 本地 `tauri dev` 验验收 #3 端到端。**偏差(已拍板,非设计违规):删步骤级 on_error/condition 用等宽 textarea 非 Monaco/params+inputs 合映射 input、outputs 映射单名 output/D2 前端校验保 Rust 零改动** | P29 实现 | `CONF`(已交付, 本地手动验收待 Seeker) |
| L25 | P30 前瞻/设计报告（Keel 2026-08-14 自起,非 Seeker 文档,基于读代码事实判范围）：WS1 模块化韧性可观测化(§3「UI 精确告知缺失」完全未落地——`SkillScanner::scan()` 静默 skip+`warn` 丢缺失信息)+ WS2 热加载(notify 已在依赖、`ConfigWatcher` 范式就绪却只盯 `settings.yaml`、`SkillManager::reload()` 已存在未触发)必做;WS3 主题库接入(全仓零 `.caspian-theme` 引用/零加载器,包格式属 §3 设计意图)延后待外部规格。**三条边界**:① 不新建 `ModuleRegistry`(需 `.caspian` 包格式,超范围);② `DirWatcher` 复用 `ConfigWatcher` 的 debounce + `Mutex` 裹 `!Sync` 写法(不重造范式);③ 顺手修 P26 未注册 `list_skills`/`reload_skills`。Seeker 批准进前置核对 | P30 前瞻 | `CONF`(Seeker 批准, 实现闭环见 L27) |
| L26 | P30 前置核对（Keel 2026-08-14）：Seeker 指定三项全 PASS——① `Skill: Serialize` 已派生且 `path: PathBuf` 已 `#[serde(skip)]`,`ScanReport { skills: Vec<Skill>, issues: Vec<ScanIssue> }` 含 `Vec<Skill>` 可安全跨 FFI;② `DirWatcher` 的 `Sync` 复用 `ConfigWatcher` 范式(`_debouncer: Mutex<Option<Box<dyn Any + Send>>>` 仅 RAII 恢复 `Sync`),正确;③ 模块状态 DTO 字段名对齐——发现裂缝 F-A(前端 P26 mock `Skill` vs 真实 `list_skills` 异形字段 `category`/`runtime`)→ 在 `useCaspian.mapRustSkill` 做容错映射(未知 `category` 归 `agent`、`schema` 取 `runtime` 描述)+ 新建独立 `ModuleStatus`/`ModuleIssue` 类型(不污染真实 `Skill`)。均不修改 Rust 契约 | P30 设计前置 | `CONF`(三项 PASS, F-A 就地闭环, 实现中) |
| L27 | P30 实现闭环（Keel 2026-08-14）：**WS1+WS2 落地,WS3 延后**。Rust——`skill/scanner.rs` 扩 `ScanReport`/`ScanIssue`/`ScanIssueKind`(`scan()` 由 `->Vec<Skill>` 改 `->ScanReport`,四类失败 MissingManifest/ReadError/ParseError/ValidationError 各产一条带路径+原因+已解析名的 issue,保留 skip-don't-crash §3 硬约束);`skill/mod.rs` 增 `last_report: Mutex<Arc<ScanReport>>` + `reload()` 存报告 + `module_status()`;`hot_reload.rs` 新建 `DirWatcher`(复用 `ConfigWatcher`);`lib.rs` 注册 `pub mod hot_reload`;`tauri_app.rs` 注册 `list_skills`/`reload_skills`/`get_module_status` + `run_tauri()` 内起双目录 watcher(skill_cb 经 `async_runtime::spawn` 调 `manager.reload().await` 后 `emit skills_reloaded`,workflow_cb 仅 `emit workflows_changed`),`Arc<SkillRegistry>`+`replace_all` 原地更新不破坏 P28 运行路径(L21)。前端——`types/skills.ts` 扩 `ModuleStatus`/`ModuleIssue`/`ModuleIssueKind`;`useCaspian.ts` 增 `getModuleStatus`/`reloadSkills`/`subscribeSkillsReloaded`/`subscribeWorkflowsChanged` + `mapRustSkill`(真实 `listSkills` 接 `list_skills`);`components/ModuleResilienceBanner.tsx` 新建非阻塞横幅(空 issues 返回 null,不崩溃不阻塞);`SkillsPage`/`WorkflowsPage` 接入热加载订阅。**沙箱门禁:cargo test --lib 680(677→680,+3 运行测试:scan validation / module_status / dir-watch-disabled;19 ignored pre-existing webkit/网络门控)/cargo clippy --lib 0/pnpm build+typecheck+lint 全绿(0 warning)**。**偏差(Seeker 许说明即可):新增测试数低于预期(695-700→680)因既有 `test_scan_*` 升级断言而非另起用例,功能覆盖不受影响;WS3 延后待 `.caspian-theme` 包规格(L25/L26)** | P30 实现 | `CONF`(已交付, 本地手动验收待 Seeker: 验收 #3/#4/#5 真实 Tauri 交互) |
| L28 | 大件 A 启动（Seeker 2026-08-16 指令）：**转「大件自主推进、内部检查点记录」模式，不再逐阶段审批**。大件 A=执行与集成(3-4 人天)，子项 A1 P28 收尾(0.5d)/A2 核心边界重定义+ModuleRegistry 草案(1d)/A3 P31 主题库接入(1d)/A4 P32 安全沙箱(1.5d)；工作纪律「不问(自行判断并记录)/不管(按大件节奏推进,每子项后记录决策与偏离)/记录(决策偏离检查点入最终报告)」；内部检查点 A-1(主题库接入前确认核心边界方向稳定)/A-2(沙箱实现前确认与 P28/P30 无冲突) | 大件 A 启动 | `CONF`(模式生效, 自主推进中) |
| L29 | A1 P28 收尾复查（Keel 2026-08-16）：沙箱 `pkg-config` 确认 `webkit2gtk-4.1`/`4.0` **均 ABSENT**，真实 Tauri 无法编译运行，故「本地 `pnpm tauri dev --features tauri` 验证真实执行」**不在沙箱能力内**，记录为 Seeker 本地门禁（非新风险，P25 起一致）。沙箱侧等价闭环：`workflow::runner` 2/2 + `workflow::store` 8/8 + 全 lib 680 绿/clippy 0 → P28 沙箱侧=已闭环；真实执行路径(UI 运行态/结果/失败展示、RunStore 真实环境)待 Seeker 本地验收，**不谎报已交付**。结果已填入 `P28_DELIVERY_REPORT.md` §七 | A1 P28 收尾 | `CONF`(沙箱闭环, 真实门禁待 Seeker 本地) |
| L30 | A2 核心边界重定义（Keel 2026-08-16）：产出 `core-modules.md` + `ModuleRegistry_DESIGN.md`。边界定义——**编译期代码模块**(config/logging/types/guardian/router/skill/workflow/commands 必编,无运行期缺失) vs **运行期内容模块**(skills/workflows/knowledge/themes 磁盘发现,§3 韧性直接适用,其缺失/损坏不得致崩且 UI 精确告知)；ModuleRegistry 定位为「编排+聚合层」复用既有子注册表(`Arc`+`replace_all`,P28)+ P30 `ScanReport`/`DirWatcher`，**不新建独立动态函数注册表**(守 L25/L26 边界,不凭空造 ModuleRegistry 设施)；统一 `ModuleCategory` 枚举 + `ModuleScanner` trait 泛化 `ScanReport`；`ScanIssue.module_name` 泛化 + 预留 `ChecksumMismatch`(未来 `.caspian` 包)；`get_module_status` 扩为聚合 `ModuleStatus`。**检查点 A-1 结论：核心边界方向稳定，P31 主题库直接落入 `ModuleCategory::Themes`+`DirWatcher` 扩 themes，无冲突，可按草案推进** | A2 核心边界 | `CONF`(草案, 实装随 P31/A2 收尾) |
| L31 | P31 主题库接入（Keel 2026-08-16，A3）：复用 P30 `DirWatcher` 机制实现 `~/.caspian/themes/` 扫描+热加载。Rust——新建 `theme/mod.rs`(`ThemeManager`/`ThemeManifest`/`ThemeScanResult`/`ThemeIssue`/`validate_theme_css`：禁 `!important`、禁 `@import`、限 `backdrop-filter`≤2、限选择器层级≤2 且无子/兄弟组合符)；`config/paths.rs` 加 `themes` 字段+`ensure_dirs`；`lib.rs` 注册 `pub mod theme`；`tauri_app.rs` 注册 `list_themes`/`get_theme_css`/`get_active_theme`/`apply_theme`+`run_tauri()` 起 themes watcher(`emit theme_changed`)。前端——`types/theme.ts`+`useCaspian`(listThemes/applyTheme/getThemeCss/getActiveTheme/subscribeThemeChanged, mock 返样本主题)+`lib/theme.ts`(注入 CSS 变量覆盖+`data-theme`)+`useAppStore`(customTheme/customThemeCss)+`SettingsPage` 主题包选择器(列已装包/应用/恢复默认/展示损坏包)。**沙箱门禁:cargo test --lib 686(680+6 主题新增)/clippy 0/pnpm build+typecheck+lint 全绿**。**检查点 A-1 已闭环(主题库落入 ModuleCategory::Themes,无冲突)**。偏差:主题包格式(纯 CSS 变量覆盖+manifest.yaml)按 Seeker 规格落地,未实现 `.caspian` 包加载器(外部规格,守 L25/L26) | P31 主题库 | `CONF`(已交付, 本地 `tauri dev` 真实应用待 Seeker) |
| L32 | P32 安全沙箱（Keel 2026-08-16，A4）：改造 `skill` 执行引擎为每技能独立沙箱。Rust——新建 `skill/executor/sandbox.rs`(`SkillSandbox`/`check_runtime_permissions`/`apply_sandbox_env`)；`Executor::execute` 入口加 `check_runtime_permissions`(shell 门控)+`SkillSandbox::new()`(每执行独立 `tempfile::TempDir`,作子进程 CWD,`execute` 返回时不论成败自动 `drop` 清理),`apply_sandbox_env` 注入 `CASPIAN_SANDBOX`/`CASPIAN_SKILL_DIR`/`CASPIAN_NETWORK_ALLOWED`/`CASPIAN_FS_READ`/`CASPIAN_FS_WRITE`；`error.rs` 加 `ExecutorError::PermissionDenied`；`Cargo.toml` 将 `tempfile` 由 dev-deps 升为 deps(运行时使用)。**检查点 A-2 结论：与 P28/P30 无冲突——仅加隔离+权限层,不触 P28 运行路径(`run_workflow`/`Workflow::load`/`RunStore`)亦不触 P30 `ScanReport`/`DirWatcher`,P28 三处运行 Shell 技能测试夹具补 `shell:true` 即恢复(真实 skill.yaml 本就该声明)**。偏差(已记录,非设计违规):① Seeker 指令称"新增 permissions 字段",**实测 `permissions`(`SkillPermissions`:`fs:[{read,write}]`+`network`+`shell`)在 skill.yaml 早已存在且 `test_permissions_parsing` 已验证**——A4 实为「复用既有字段 + 加强制执行 + 加临时目录隔离」;② 沙箱级可真执行的仅 shell 门控(拒绝 spawn)+ 写隔离(tempdir CWD,相对写不漏出技能目录),`network`/`fs` 路径限权需 seccomp/Landlock/网络命名空间等 OS 级原语,本轮**声明+env 标记+告警,真执行留待 OS 沙箱 harness(未来 WS)**。沙箱门禁:cargo test --lib 694(686+8 沙箱新增:纯函数门控3+沙箱隔离/清理2+executor 拒绝/允许/写隔离3)/clippy 0 | P32 安全沙箱 | `CONF`(已交付, OS 级网络/fs 限权待 harness; 本地 `tauri dev` 真实隔离待 Seeker) |
| L33 | 大件A收口确认 + 大件B启动指令（Seeker 2026-08-16 确认）:Seeker 确认 A1-A4 全闭环、检查点 A-1/A-2 已记录,认可「真跑项/语法保全项」区分与两项偏差(D-A4-1 复用既有 permissions 非新增、D-A4-2 网络/fs 限权留待 harness);**大件A 交付状态置「已收口,待本地验证」**(真实 Tauri/fs 验收项待 Seeker 本地 `tauri dev`)。Seeker 指令「继续推进大件B」。**阻塞已于 2026-08-16(同日)解除**:Seeker 补交 大件B 完整规格(见 L34),此前阻塞纯为规格未落文档(Seeker 自承"口头拆分未写 Keel 可见处"),非决策待批 | 大件A收口/大件B已解阻塞 | `CONF`(A 收口已确认; B 规格已收, 见 L34) |
| L34 | 大件B 规格（Seeker 2026-08-16 补交，此前仅口头拆分未落文档）：**扩展性与生态**，预期 4-5 人天。子项——P33 跨端打包(Windows/macOS/Linux 三平台安装包构建+签名+自动更新,1.5d)/P34 启动优化(冷≤2s、热≤500ms,0.5d)/P35 内存优化(空闲≤200MB、正常≤500MB,0.5d)/P36 导入导出(Agent/Skill/记忆/会话/配置完整 .caspian 包,0.5d)/Skill 外部源协议(GitHub/HTTP/MCP 加载,MCP 适配器,1d)。内部检查点——B-1(P33 打包前确认构建脚本三平台通过,发现平台差异则记录解法)/B-2(外部源协议设计前确认 MCP 适配器可行性,不可行则记录替代)。**阻塞关系**:P33 不阻塞其他;外部源协议依赖 P32 沙箱(外部代码须沙箱运行)——A4 已交付,依赖满足。交付物:三平台打包脚本+启动/内存优化报告+.caspian 包格式规范+MCP 适配器实现。纪律沿用 L28:不问/不管/记录 | 大件B 规格 | `CONF`(规格已定义, 自主推进中) |
| L35 | P33 跨端打包（Keel 2026-08-16，B 子项1）：产出三平台构建/签名/更新管线。**检查点 B-1 已闭环**:构建脚本按"一 OS 一 runner"矩阵设计(=`tauri-apps/tauri-action` 在 ubuntu/windows/macos 各跑原生目标),规避跨编译器脆弱性;平台差异已识别并记录解法——① 图标需完整集(`.ico`/`.icns`/`.png`),CI 前跑 `pnpm tauri icon`;② ubuntu runner 须先 `apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev`(沙箱缺 webkit 即同源约束,真实原生构建须装);③ 签名各 OS 异——Windows=`TAURI_WINDOWS_CERTIFICATE`+密码(macOS 走 `APPLE_CERTIFICATE`/`APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` 代码签名+公证);④ 更新器须 `bundle.createUpdaterArtifacts:true`+`plugins.updater.pubkey` 与签名私钥配对。沙箱无法真实执行 `tauri build`(webkit ABSENT,与 A1 同约束)→ **B-1 结论:脚本结构性可通过三平台,真实构建记录为 Seeker CI/本地门禁,不谎报**。交付:`tauri.conf.json`(补 bundle 元数据+`createUpdaterArtifacts`+`plugins.updater`+`.caspian` 文件关联)/`.github/workflows/release.yml`(三平台矩阵+签名+上传)/`.github/workflows/ci.yml`(`cargo test --lib`+前端三绿,headless 不碰 webkit)/`scripts/build.sh`+`scripts/setup-icons.sh`/`package.json` 增 `tauri:build`/`tauri:dev`/`tauri icon`。**Rust 更新器集成(次要)**:`tauri-plugin-updater` 按 `tauri` 特性门控接入(`.plugin(...)`+`check_for_update`/`install_update` 命令),代码完整但**沙箱无法 compile-verify(`features tauri`需 webkit)**,待 Seeker 本地 `tauri` 构建验证;为不污染默认 `cargo test --lib` 门禁,该依赖 gated 于 `tauri` 特性(默认不编) | P33 跨端打包 | `CONF`(构建管线已交付, 真实三平台构建待 Seeker CI/本地; Rust 更新器待 `tauri` 构建验证) |
| L36 | P34 启动优化（Keel 2026-08-16，B 子项2）：目标冷≤2s/热≤500ms。**现状核查**:沙箱无 webkit→真实 GUI 冷/热启动不可测(记 Seeker 本地门禁);`SkillScanner::scan` 早已并行(`JoinSet`+`Semaphore`,并发32)→ 扫描非瓶颈。**交付**:新建 `startup.rs`(`StartupTimer` 阶段计时器,纯 std+tracing,默认特性可用)+ `startup_baseline_headless` 自动基线测试;`lib.rs` 注册 `pub mod startup`;`tauri_app.rs` 内 `run_tauri` 接 `StartupTimer`(各阶段 `mark`+启动完成 `tracing::info!(report=...)`,`#[cfg(feature="tauri")]`,沙箱不可编译验证)供真实运行出报告。**自动化基线实测(沙箱 headless 核心)**:`config=1ms skills=4ms runstore=1ms total=6ms` → Rust 核心启动可忽略(~6ms),冷/热预算完全由 GUI 层(Tauri 窗口+webview+前端 hydration)主导,优化重心在**前端代码分割/延迟加载重依赖(fastembed/@xyflow)**,非核心 init;该等前端项记录为 Seeker 本地验证(浏览器 Performance 面板+`RUST_LOG=info` 读 `startup:` 报告行)。**偏离 D-P34-1**:真实 GUI 数字沙箱不可测,记 Seeker 本地门禁,已交付自动化无头基线+运行时埋点;**D-P34-2**:`run_tauri` 埋点属 `tauri` 特性门控,沙箱不可编译验证,API 纯 std 风险低。**门禁:cargo test --lib 695(694+1 基线)/clippy 0** | P34 启动优化 | `CONF`(无头基线已自动化+计时埋点就位, 真实 GUI 数字待 Seeker 本地; 前端优化建议待本地验证) |
| L37 | P35 内存优化（Keel 2026-08-16，B 子项3）：目标空闲≤200MB/正常≤500MB。**现状核查**:核心内存本就文件持久化且**有界**(Session=SQLite/Run=分文件JSON/Cache=磁盘索引,内存态仅 `Arc` 注册表+分页API);**唯一真实无界读路径**=`RunStore::list_runs()` 每次全量读入 `Vec<RunRecord>` 且 `get_run` 每次重解析 `run.json`(已修复);`list_sessions` 已有 `limit` 分页非瓶颈。**交付**:新建 `memory.rs`(`MemoryBaseline` 结构化基线+`current_rss_bytes()` Linux `/proc/self/status` 真实RSS+`measure_headless()`+`memory_baseline_headless` 测试+`memory_report` IPC[tauri 门控]);`workflow/store.rs` 加**定容(256)最近访问缓存**给 `get_run` 热路径+新增 `list_runs_limited(limit)` 有界历史读取(保留 `list_runs()` 全量语义不破坏既有测试)+`create_run`/`update_run` 失效缓存;`tauri_app.rs` 注册 `memory_report`;新建 `allocator.rs`+`Cargo.toml` 加 `tikv-jemallocator`(optional)+`jemalloc` feature(`#[global_allocator]` 门控,`cargo build --features jemalloc` 已验证可编译,43s 编 C 库)。**自动化基线实测(沙箱 headless 核心)**:`skills=5 runs=0 sessions=0 est=85.7 KB rss=36.9 MB` → 结构化核心状态仅 ~86KB,200/500MB 预算由 GUI 层主导,核心无需重造。**偏离 D-P35-1**:真实进程RSS(含 webview)沙箱不可测,记 Seeker 本地门禁,已交付 `current_rss_bytes`+`memory_report`;**D-P35-2**:`memory_report` 属 `tauri` 特性门控沙箱不可编译验证;**D-P35-3**:未引 `lru`/`page_size` crate,`RunStore` 缓存用标准库 `VecDeque`+`HashMap` 内联实现(守"依赖最小化"),RSS 读 `VmRSS`(已是KiB免换算)。**门禁:cargo test --lib 698(695+1内存基线+2 store缓存/有界)/clippy 0/`cargo build --features jemalloc` 通过** | P35 内存优化 | `CONF`(核心内存已克制+缓存/有界读取已交付, 真实进程RSS待 Seeker 本地; jemalloc 可选特性已验证) |
| L38 | P36 导入导出（Keel 2026-08-16，B 子项4）：Agent/Skill/记忆/会话/配置完整导入导出。输出 `.caspian` 包格式规范 + 导入导出 UI。**交付**:新建 `CASPAN_BUNDLE_FORMAT.md`(目录包形态+布局+manifest schema+各模块序列化策略+版本兼容矩阵+冲突策略+校验和+安全边界);新建 `package.rs`(`export_bundle`/`import_bundle`+`BundleManifest`/`BundleItem`/`ImportReport`/`ConflictPolicy`(Skip/Overwrite/Rename)+`PackageError`,纯 std 无新依赖,弹性报告 imported/skipped/failed 不静默丢弃 P30 WS1 §3);`lib.rs` 注册 `pub mod package`;`tauri_app.rs` 注册 `export_bundle(dest)`/`import_bundle(src,policy)`(tauri 门控,返回 JSON);`useCaspian.ts` 加 `exportBundle`/`importBundle`(mock+真实双路径);`SettingsPage.tsx` 加「数据导入/导出」面板。**各模块真实往返能力(代码事实)**:skills/config/agents 文件树拷贝+checksum;sessions 经 `list_sessions`+`get_messages` 导出、导入经 `create_session`+`append_message` 重建(重映射 session_id);knowledge 经 `list_documents`+`chunks_of_document` 导出、导入经 `import_document` **重新生成嵌入向量**(依赖导入端模型,无模型则进 failed 桶弹性不中断)。**测试(headless 真实往返)**:`test_export_import_roundtrip`(1技能+1会话+1消息→导出→导入新环境全等)/`test_conflict_skip_leaves_existing`/`test_import_rejects_non_bundle` 全绿。**偏离 D-P36-1**:`export/import_bundle` 命令(真实路径/文件对话框)沙箱不可编译验证(需 webkit),待 Seeker `tauri` 构建+本地验收;**D-P36-2**:knowledge 导入依赖嵌入模型,沙箱无模型时 `import_document` 失败→进 failed 桶(已设计为弹性非 panic),真实重嵌待 Seeker 本地验证;**D-P36-3**:单文件分发(tar.gz)传输层增强未实现,目录包已是完整可导入形态(规范§1 注明未来可叠加);**Agent** 概念当前无独立模块(`paths.agents` 为预留目录),导出按文件槽位处理并如实说明。**门禁:cargo test --lib 701(698+3 package)/clippy 0/前端 typecheck+lint+build 全绿** | P36 导入导出 | `CONF`(`.caspian` 格式+核心导出导入+UI 已交付, 真实 `tauri` IPC/知识重嵌待 Seeker 本地) |

### L7 分析（P24 v1 范围，Keel 收口 + Seeker 确认，2026-08-11）

- **范围锁定**：核心（身份偏好优先级链 `Task > Skill > Agent > Global` + fallback 降级链）+ 新建 `health.rs` 做可用性探测（降级触发门）；`ask` 的 `model: Option<&str>` 占位接成真实路由；设计 `skill.yaml` 的 `preferred_model`（干净待设计）。
- **排除 v1**：① 完整负载均衡调度器（依赖多模型实例基础设施，代码未见，超 1 人天）；② 成本优化（需价格表/token 计费，代码无）——二者建议独立阶段。
- **设计文档须拍板（前置核对关注点，避免 precheck 返工）**：
  1. **两层路由维度共存**：P24「身份偏好链（Task>Skill>Agent>Global）」与现役「复杂度双档（small/large，`SlotFiller::select_provider`）」是不同维度。设计须明确：是先按身份链选模型身份、再按复杂度分 small/large？还是「指定模型」直接映射到现役 provider 的 small/large 槽位？
  2. **`default_model` 字段**：代码零命中（L6 `CONF`），按兼容兜底处理。
  3. **`health.rs` 探测粒度**：网络可达 / 健康检查端点 / 超时阈值，需在设计里给出可测定义。
  4. **配置解析来源**：`skill.yaml` / `AGENT.yaml` 的解析路径是否已存在；P24 是否新建解析层，还是复用现役 config 模块。

> 状态同步：本决议已落基线，待 Seeker 出 `P24_DESIGN_DOC_*.md` 后，Keel 按老规矩（前置核对 → 分层实现 → 真跑验证）接手。

### L5 分析（P23 前提，Keel 研判）

- P23 用户侧定调 = "知识库**向量检索**"（即给 P22 的文档语料加嵌入层）。
- P22 语料 = 导入文档(TXT/MD) 仅（`@P22` D3 已排除会话语料）。
- P21 `SessionStore` 现状 = 孤岛、写入路径为空、sessions.db 恒空（P21/P22 前置核对已确认）。
- **结论**：若 P23 首版语料 = P22 的文档（同口径），则 **P21 会话写入路径对 P23 零影响**，P23 可独立于 P21 推进。
- P21 仅当 P23 决定**也把会话语料向量化**时才相关——而这恰是 `@P22` D3 同一类决策，须等 SessionStore 真实写入路径打通后再定。
- **建议**：P23 首版保持"文档向量化"范围，不引入 session 语料 → 与 D3 纪律一致，避免"拿未验证叠未验证"。

**向量存储选型（Seeker 2026-08-11 确认）**：若 P23 仍守 D1"零新依赖"精神 → **在 P22 的 SQLite 上扩 `embedding` 列**，复用 P11 `Embedder`（已有现成实现，P22 未启用），**不新拉 LanceDB**（即不引入整条 Arrow 栈）。这是 P23 前置核对的第一条待核假设。

> 待 Seeker 下达 P23 设计文档；其前置核对将逐条复核 L5 假设与代码事实（含向量存储选型是否守 D1）。
| L39 | Skill 外部源协议 + B-2 检查点（Keel 2026-08-16，B 子项5）：从 GitHub/HTTP/MCP 加载 Skill，MCP 适配器。**检查点 B-2 已闭环（结论：可行）》**：最小 JSON-RPC 2.0 stdio 客户端，零重型 SDK（`rmcp` 等），外部代码经 A4/P32 `SkillSandbox` 沙箱运行。**交付**：`skill/schema.rs` 加 `Skill.mcp: Option<McpRef>`（`#[serde(default)]` 向后兼容）+ `McpRef{server_command,tool}`；新建 `skill/mcp.rs`（`McpClient` initialize/tools/list/tools/call 双任务 reader/writer + `run_mcp_tool` 沙箱启动 + `tools_to_skills` 转虚拟 Skill 带 `McpRef`）；新建 `skill/source.rs`（`ExternalSource` enum Local/Git/Http/Mcp + `resolve_source` git-clone/curl/best-effort + `load_skills` Mcp 经客户端/其他扫描目录 + `slugify`）；`skill/executor/mod.rs` 的 `execute()` 顶部加 **MCP 早期返回分支** + `execute_mcp_skill` helper（路由到 `run_mcp_tool`，全程沙箱隔离）；`types/error.rs` 加 `ExecutorError::ExecutionFailed`；全仓 18 处 `Skill` 字面量补 `mcp: None`。**测试(headless 真实往返)**：`test_mcp_client_echo_roundtrip`(mock python stdio 服务器)/`test_run_mcp_tool_sandboxed`/`test_tools_to_skills_carries_mcp_ref`/`test_external_source_parse_all_variants`/`test_load_skills_from_local_source`/`test_slugify` 全绿。**偏离 D-EXT-1**：外部源注册 UI 未建（`SettingsPage` 仅 `.caspian` 导入导出面板），后端完整未前端暴露，属范围外/后续项；**D-EXT-2**：Git/Http 真实 clone/download 需网络+`git`/`curl` CLI，best-effort，记 Seeker 本地门禁（Local+Mcp 已 headless 全测）；**D-EXT-3**：未引重型 MCP SDK，最小客户端守"依赖最小化"；**D-EXT-4**：`McpTool` 调用以 `Skill` 输入 `Value` 直通 `tools/call.arguments`，输出序列化进 `ExecutionResult.stdout`。**门禁：cargo test --lib 707(701+6)/clippy 0/前端 typecheck 绿**。**B-2 结论落库**：MCP 适配器可行，最小实现已落地，外部源协议设计完成 | Skill 外部源协议 + B-2 | `CONF`(MCP 适配器+外部源协议已交付，Git/Http 真实 clone/download 待 Seeker 本地门禁；外部源注册 UI 为后续项) |
| L40 | 大件C启动 + P37 系统Skill包（Seeker 2026-08-16 确认大件B收口,Keel 2026-08-16 启动大件C）：目标推到可打包交付状态,子项 P37–P41(3.5人天)。**P37 完成**:12 个系统级 Skill(file-reader/file-writer/file-search/web-fetcher/shell-runner/system-info/code-interpreter/json-parser/note-taker/memory-manager/skill-manager/workflow-runner)覆盖 file/network/system/data/self 五类。**偏离 D-C37-1**:手册要求建 `~/.caspian/skills/system/` 目录,改为沿用项目既有"嵌入式常量+首次运行幂等安装"builtin 模式(`skill/builtin/`),由 `SkillManager::init()` 自动接管(5 核心+12 系统=17 个,count≥10);**D-C37-2**:手册 YAML 模板(`runtime: python` 裸串/`permissions:{fs_read:true}`)与真实 schema 不符,已对齐真实 `runtime:{type:...}`+`permissions:{fs:[{read:[...]}],network,shell}`;**D-C37-3**:脚本只用 Python stdlib(web-fetcher 用 urllib 非 requests);**D-C37-4**:名称沿用手册连字符;**D-C37-5**:14 个硬编码"内置=5"测试翻新为基于 `BUILTIN_SKILL_NAMES.len()` 动态断言。**门禁:cargo test --lib 712(707+5)/clippy 0/前端不涉及**。**检查点 C-1 闭环**:17 Skill 全部可加载(count 17≥10),file-reader 读文件/shell-runner echo/system-info JSON 真实执行通过 | P37 系统Skill包 | `CONF`(12 系统 Skill 已交付且真实可执行, 门禁 712/clippy 0) |
| L41 | P38 错误自愈 + C-2 检查点（Keel 2026-08-16，C 子项2）：崩溃恢复/数据库修复/优雅降级。**检查点 C-2 已闭环（结论：数据层可自愈）》**：新建 `self_healing.rs`(`SelfHealingManager` + 优雅降级自由函数 + 13 测试),`lib.rs` 注册 `pub mod self_healing;`(默认特性,不污染 CI 门禁)。**交付**:`run_startup_checks`(不阻塞启动,逐库 `PRAGMA integrity_check`,损坏则 `restore_from_backup` 复验,配置损坏入 `HealingReport.issues`,始终 `Ok(report)`)/`check_database`/`restore_from_backup`(损坏库移到 `.corrupt-<ts>` 隔离,从最新备份复制回位)/`create_backup`(`VACUUM INTO` 落到 `backups/<stem>_<ts>.db`)/`list_backups`/`prune_backups(stem,keep=7)`/`validate_configs`(`settings.yaml` YAML 解析);优雅降级三函数 `network_available`(TCP 探 8.8.8.8:53,2s)/`embedding_model_available(cache)`(查 `models--` 目录,离线不下载)/`degrade_network_skills(reg)`(禁用所有 network 权限 Skill,本地 Skill 不动)。**测试(headless 真实执行)**:`test_check_database_*`(健康通过/字节损坏报 Integrity)/`test_create_backup_and_prune`(VACUUM 备份健康+10 备份 prune 删 8 留 3)/`test_restore_from_backup`(损坏库自动恢复复绿+`.corrupt` 隔离)/`test_validate_configs_*`(缺=Ok/非法=Err/合法=Ok)/`test_run_startup_checks_*`(无库不阻塞/损坏库自动修复进 repaired)/`test_degrade_network_skills`(network 禁用 1、local 保持、count_enabled=1)/`test_embedding_model_available`/`test_network_available_returns_bool`/`test_chrono_timestamp_suffix_unique` 全绿。**自主加固 D-C38-3**:实现中发现 `chrono_timestamp_suffix` 原仅秒级精度→同秒内多次备份文件名碰撞互相覆盖(真实缺陷),已改为秒(hex)+亚秒纳秒消除碰撞(手册未提及)。**门禁:cargo test --lib 725(712+13)/clippy 0/前端不涉及**。**C-2 结论落库**:数据层自愈 headless 全测通过,真实 GUI 崩溃上报(webkit 构建)记 Seeker 本地门禁 | P38 错误自愈 | `CONF`(数据层自愈+优雅降级已交付且真实可测, 同秒备份碰撞缺陷已修, GUI 崩溃上报待 Seeker 本地 `tauri` 构建) |
| L42 | P39 用户手册 + C-3 检查点（Keel 2026-08-16，C 子项3）：内置帮助文档/FAQ/引导教程。**检查点 C-3 已闭环（结论：可上手）》**：**交付 docs/help/*.md 6 篇**(index/getting-started/skills/workflows/keyboard-shortcuts/faq——**14 条 FAQ≥10**)+ **零新依赖 React 组件**:`lib/markdown.tsx`(极简 Markdown→React 渲染器,标题/段落/列表/代码块/行内 code/bold/link)/`components/help/HelpViewer.tsx`(经 `import.meta.glob("../../../docs/help/*.md",{query:"?raw",eager:true})` 内联加载)/`routes/HelpPage.tsx`(`/help` 路由)/`components/help/HelpPanel.tsx`(F1 滑出浮层,不卸载当前页)/`components/help/OnboardingModal.tsx`(首次启动 3 步引导)/`hooks/useHelp.ts`(F1 全局快捷键,呼应 useCommandPalette 的 Cmd+K);编辑 `App.tsx`(加 `/help` 路由+渲染 HelpPanel+条件渲染 OnboardingModal)、`Sidebar.tsx`(展开/折叠两处加「帮助」导航 `HelpCircle`)、`stores/useAppStore.ts`(加 `hasSeenOnboarding`+`setHasSeenOnboarding`,localStorage `caspian.onboardingSeen`)。**验证(headless 真实)**:`npx tsc --noEmit` **0 errors**(严格模式);最小 vite lib 构建仅打包 help glob→**7 模块转译/45ms/产物内联 6 文档全文**("CaspianFlow 帮助中心"/"内置技能"/"常见问题（FAQ）"/"快速上手" 均在,`docs/help` 键数=6)→ glob 加载真实验证。**偏离 D-C39-1**:手册未指定渲染方案,为守"依赖最小化"未引 react-markdown/marked,自写极简渲染器;**D-C39-2**:`.md` 为权威源+构建期内联,无需运行时 fetch;**D-C39-3**:完整 `vite build` 沙箱因内存不足被 OOM 终止(`Killed`),非代码错误,以 `tsc` 0 错+最小 lib 构建确认内联验证,真实 `npm run build` 记 Seeker 本地门禁;**D-C39-4**:引导仅首次启动(localStorage),关闭后不再出现,内容可在帮助中心回顾。**门禁:tsc 0 errors/最小 lib 构建 6 文档内联/Rust 仍 725(未触 Rust)/完整 vite build 沙箱 OOM 记本地门禁**。**C-3 结论落库**:用户手册可上手链路完整,帮助与 P37 17 技能/P38 自愈降级口径一致 | P39 用户手册 | `CONF`(6 篇 md+14 FAQ+零依赖帮助 UI 已交付, tsc 0 错+最小 lib 构建验证 glob 内联, 完整 vite build 因沙箱内存 OOM 记 Seeker 本地门禁) |
| L43 | P40 测试体系 + C-4 检查点（Keel 2026-08-16，C 子项4）：单测/集成/性能基准齐备,覆盖率≥80%。**检查点 C-4 已闭环（结论：测试体系就绪）》**：新建 `src-tauri/tests/integration.rs`(**6 个跨模块集成测试,真实临时目录非 mock**:skill-manager 装 17 内置/self-healing 备份-损坏-恢复复绿/package 导出-导入≥17 技能且 failed=0/离线降级禁用 network 技能且 count_enabled 精确下降/embedding 探测空缓存与 models--目录/启动自检 <2000ms 性能护栏);`Cargo.toml` dev-deps 加 `rusqlite`(bundled)+`tokio`(full) 供集成测试;新建 `tarpaulin.toml`(默认特性、排除 tests/benches/bin、目标≥80%);修 2 个既有测试 clippy warning(`workflow/runner.rs:133` needless_borrow、`package.rs:655` `let_underscore_future` 改 `#[tokio::test]`+`.await` 真实安装技能),使 `clippy --lib --tests` 归零。**验证(headless 真实)**:`cargo test` **736 passed/0 failed**(lib 725 + 集成 6 + doctest 5);`cargo clippy --lib --tests` **0 warnings**。**偏离 D-C40-1**:性能基准用轻量时序断言(<2000ms)而非 criterion(重依赖/沙箱易 OOM),守"依赖最小化"且给出回归护栏;**D-C40-2**:覆盖率 `tarpaulin.toml` 就位但真实数字因沙箱无法获取 `llvm-tools-preview`(网络阻断 static.rust-lang.org)记 Seeker 本地门禁,预期≥80%(736 测试覆盖核心模块);**D-C40-3**:集成测试经 rusqlite/tokio 构造真实状态避免 mock 失真;**D-C40-4**:修 2 个既有测试 warning 使 clippy 全绿。**门禁:cargo test 736/0/clippy --lib --tests 0 warnings/tarpaulin 配置就位(实际覆盖率记 Seeker 本地门禁)**。**C-4 结论落库**:测试体系三层齐备且绿,覆盖率测量工具链就绪待 Seeker 本地确认 | P40 测试体系 | `CONF`(736 测试全绿+clippy 0, tarpaulin 配置就位, 实际覆盖率因沙箱无 llvm-tools 记 Seeker 本地门禁) |
| L44 | P41 CI/CD + C-5 检查点（Keel 2026-08-16，C 子项5）：PR检查/三平台构建/GitHub Release自动化。**检查点 C-5 已闭环（结论：可打包交付链路就绪）》**：重写 `.github/workflows/ci.yml` 与 `release.yml`——**修正 monorepo 假设**(原 `working-directory: caspian-flow`/`projectPath: caspian-flow`/`cache-dependency-path: caspian-flow/pnpm-lock.yaml` 与真实"仓库根即应用根"不符会必败,全部对齐为根布局);ci.yml 跑全量 `cargo test`(lib+集成+doctest)+ `cargo clippy --lib --tests -D warnings` + 前端 `pnpm install/build/typecheck/lint`;release.yml `projectPath: .` + 三平台矩阵(ubuntu-22.04/windows-latest/macos-latest `--target universal-apple-darwin`)+ tauri-action 发 GitHub draft release + 更新器签名(`TAURI_SIGNING_PRIVATE_KEY`)+ Windows Authenticode + macOS notarization 环境变量;`releaseDraft: true`;新建 `app-icon.png`(1024x1024 有效 RGBA PNG)供 `pnpm tauri icon` 展开图标;`tauri.conf.json` 更新器配置已确认(`plugins.updater.active`+`createUpdaterArtifacts:true`+endpoints+pubkey 占位符)。**验证**:YAML 均过 `yaml.safe_load`;命令集与本地已验证结果等价(`cargo test` 736/0、`clippy --lib --tests` 0、`tsc --noEmit` 0、`vite build` 命令有效但沙箱 OOM)。**偏离 D-C41-1**:修正 monorepo→根布局否则 CI 必败;**D-C41-2**:macOS 通用二进制覆盖双架构;**D-C41-3**:更新器 `pubkey` 仍占位符(预期),Seeker 一次性 `pnpm tauri signer generate` 生成密钥对写入 conf+仓库密钥(release.yml 头部已写明);**D-C41-4**:GitHub Actions 无 runner 无法在沙箱执行,以 YAML 解析+命令等价确认。**门禁:YAML 有效/布局对齐真实仓库根/ci 命令≡本地已绿/发布流水线就绪;真实 CI/Release 运行记 Seeker 本地门禁**。**C-5 结论落库**:CI+Release 全链路就绪,大件 C 五子项(P37-P41)全部完成、C-1~C-5 闭环,项目达可打包交付状态(代码侧);唯一剩余为 Seeker 本地 `tauri build` 三平台 + `cargo tarpaulin` 确认 ≥80% + 补更新器/签名密钥(均已在 release.yml 写明步骤) | P41 CI/CD | `CONF`(CI/Release 配置就绪且布局正确, 真实运行记 Seeker 本地门禁; 更新器 pubkey/签名密钥为一次性人工步骤) |
