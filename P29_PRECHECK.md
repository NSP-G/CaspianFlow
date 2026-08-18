# P29 前置核对报告（Keel · 2026-08-14）

> 阶段：P29 | 前置：P27（画布 模式 C）、P28（执行入 UI）、P17（DAG 引擎）
> 设计文档状态：`_AP` | 用户指令：**读 `schema.rs` 的 `WorkflowStep` 真实结构逐字段对照 → 直接开工，不要问**

## 一、衔接路径核验（PASS，未另辟通路）

P29 的落盘路径与 P27/P28 完全一致：

- 编辑器 `nodesEdgesToDoc` → `WorkflowDoc.steps[]` → 前端 `saveWorkflow`/`saveWorkflowDraft` → Tauri `save_workflow`（`tauri_app.rs:179`）。
- `save_workflow` 是 **字段无关的 JSON→YAML 透传**：`serde_json::from_str(doc)` → `serde_yaml::to_string` → `manifest::save_workflow` 原子写。**不做任何字段映射**，前端 `WorkflowStepDoc` 里的字段名原样落到 `workflow.yaml` 的 `steps[].*`。
- P17 `Workflow::load`（`schema.rs:360`）按真实字段名读回；P28 `run_workflow` 加载后交给 `WorkflowEngine::execute`。

结论：**「Rust 零改动」可成立，当且仅当前端用 P17 真实 `WorkflowStep` 字段名**。`save_workflow` 无需改（保持透传），`Workflow::load` 无需改（字段本就认识）。这条约束决定了下面所有裂缝的处置方向。

## 二、裂缝（F 系列）—— 文档 §三 字段表 vs P17 `WorkflowStep` 真实结构

读 `src-tauri/src/workflow/schema.rs:127-187` 的 `WorkflowStep`：

| 文档 §三 可编辑项 | 类型/控件 | P17 真实字段（`schema.rs`） | 结论 |
|---|---|---|---|
| `params` | object | `input: serde_json::Value`（自由 JSON 模板表达式） | **名称不符**：P17 用 `input` 承载技能入参，无 `params` |
| `inputs` | name+type+required 组 | **无此字段** | P17 步骤级无 inputs schema；`input` 即实际入参 |
| `outputs` | name+type 对象 | `output: Option<String>`（**单个**输出变量名） | **名称+结构不符**：P17 输出是单个变量名字符串，非对象 |
| `condition` | 表达式字符串 | `condition: Option<String>` | ✅ 吻合 |
| `timeout` | number(秒) | `timeout: Option<u64>` | ✅ 吻合（`serde_yaml`/serde 接受整数秒） |
| `retry` | number(次) | `retry_count: Option<usize>` | **名称不符**：P17 叫 `retry_count` |
| `on_error` | 枚举 stop/continue/retry | **无此字段** | P17 步骤级**无** `on_error`；仅工作流级 `error_handling.on_step_failure`（abort/continue/retry）|

**F1〔硬·重大〕**：`params`/`inputs`/`outputs`/`retry`/`on_error` 五项里，仅 `condition`/`timeout` 名字对得上 P17；`input`(=params)、`output`(单名)、`retry_count` 才是真实载体，**`on_error` 在 P17 步骤结构里根本不存在**。
**F2〔硬〕**：若照文档字面发射 `params`/`inputs`/`outputs`/`retry`/`on_error`，serde 默认 `无 deny_unknown_fields` → `Workflow::load` 不报错但**这些编辑在回读时全部丢失**，P28 跑起来仍用默认值 → P29「能配了再跑」目标落空，且违反验收 #6「不引入新字段」。
**F3〔中〕**：`inputs` 的 name+type+required 子结构与 `outputs` 的 name+type 对象，在 P17 步骤级无对应；P17 `input` 是自由 JSON（模板表达式），`output` 是单变量名。
**F4〔中〕**：`condition` 控件写 Monaco 表达式编辑器 —— 引入 `monaco-editor`（~5MB+）重依赖，与 P25 扁平设计（零 box-shadow/渐变/backdrop-filter）及 P25-P28 零重依赖纪律冲突；`condition` 实为单行/短表达式，Monaco 过重。
**F5〔中〕**：§四 D2「`save_workflow` 写入前调 `Workflow::load` 校验」与 §六「Rust 无需改动」自相矛盾——真要在 Rust 加 load 校验就得改 `save_workflow`。
**F6〔低〕**：§四 D3/D4（每节点草稿隔离、保存互斥）是前端行为，P27 编辑器有 500ms 防抖但缺「切换节点取消防抖」「保存期间暂停草稿」两处。

## 三、拍板决策（用户授权「不要问」→ Keel 直接定）

> 硬约束优先级：**编辑后 YAML 必须能被 `Workflow::load` 合法加载 + 编辑须真实作用于执行（P28 跑的是这份配置）**。二者共同要求：只发射 P17 真实字段名。

- **D-F1/F2/F3**：严格映射到 P17 `WorkflowStep` 真实字段，`WorkflowStepDoc` 只含 `{ id, skill, input, output, condition, timeout, retry_count, depends_on }`。
  - 文档 `params` → P17 `input`（技能入参对象，结构化表单 + JSON 双模式，承载「params/inputs」意图）。
  - 文档 `outputs` → P17 `output`（单个输出变量名文本，承载「outputs」意图）。
  - 文档 `retry` → P17 `retry_count`（usize，0–5）。
  - 文档 `condition`/`timeout` → 同名透传。
  - **文档 `on_error`（步骤级）直接删除**：P17 步骤级无此字段；工作流级 `error_handling.on_step_failure` 不在 P29 步骤面板范围（留待工作流级设置，后续阶段）。不发明 P17 不存在的字段。
- **D-F4**：`condition` 用**等宽 textarea（Geist Mono，已装 `@fontsource/geist-mono`）**而非 Monaco——理由：扁平设计纪律、零重依赖、condition 是短表达式。在验收与体验上等价于「表达式编辑 + 等宽 + 校验」。
- **D-F5**：`save_workflow` **不改**（保 Rust 零改动）。「保存前预校验」在**前端结构性校验**实现：① 字段名只发 P17 已知项（天然不引入新字段）；② `timeout`∈[1,300] 整数、`retry_count`∈[0,5] 整数、`input` 须为 JSON 对象——违规则红字 + 阻止正式保存。最终 load 门控仍是 P17 `Workflow::load`（P28 run / 再次 load_workflow 时触发）。
- **D-F6**：实现 D3（切换选中节点 `clearTimeout(draftTimer)`）+ D4（保存期间 `savingRef` 置位，`scheduleDraft` 跳过）。
- **§四 D1 字段映射契约表**：在 `lib/workflow.ts` 落地为单一事实来源 `STEP_FIELDS`（P17 字段名 / 控件 / 范围 / 必填），面板由它派生，不在多处重复定义。

## 四、验收映射（沙箱可达性）

| # | 验收项 | 沙箱可达性 | 说明 |
|---|---|---|---|
| 1 | 点击节点显示全部可编辑字段 | ✅ 前端可构建 | 面板读 `node.data` |
| 2 | 改字段 500ms 草稿自动保存，重开显示新值 | ✅ localStorage fallback | updateNodeData→scheduleDraft |
| 3 | Cmd+S 后 `Workflow::load` 可加载 | ⚠️ 沙箱无 webkit；结构校验 + Rust 透传保证字段名合法 ⇒ 等价 load-safe；真编待本地 `tauri dev` | 关键：只发 P17 字段名 |
| 4 | 非法输入（timeout:-5）红字阻止保存 | ✅ 前端校验门控 | doSave 前 `docHasErrors` |
| 5 | 切换节点面板更新无残留 | ✅ 由 `selected` 派生 + D3 |  |
| 6 | 编辑后 YAML 不引入新字段、不破坏必填 | ✅ 单一事实来源约束 | 只发 P17 字段名 |

## 五、门禁预期（同 P25-P28）

- 前端：`pnpm build` + `pnpm typecheck` + `pnpm lint` 全绿。
- Rust：**零改动** → `cargo test --lib` 维持 677、`cargo clippy --lib` 0（无需重跑，但交付前复跑确认）。
- Tauri 命令：仅语法保全（webkit 限制，同 P25-P28），真实编译待本地。
