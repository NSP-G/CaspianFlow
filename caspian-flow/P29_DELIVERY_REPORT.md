# P29 交付报告（Keel · 2026-08-14）

> 阶段：P29 | 前置：P27（画布 模式 C）、P28（执行入 UI）、P17（DAG 引擎）
> 设计文档状态：`_AP` | 衔接路径：画布选节点 → 侧栏编辑 → 草稿防抖 + Cmd+S 保存 → P17 加载执行

## 一、前置核对结论（详见 `P29_PRECHECK.md`）

**衔接路径核验 PASS——未另辟通路**：`save_workflow`（`tauri_app.rs:179`）是字段无关的 JSON→YAML 透传，前端 `WorkflowStepDoc` 字段名原样落 `workflow.yaml`，P17 `Workflow::load` 按真实名读回、P28 `run_workflow` 加载后执行。**「Rust 零改动」成立当且仅当前端用 P17 真实 `WorkflowStep` 字段名**。

**重大裂缝（文档 §三 vs P17 `WorkflowStep` 真实结构）**：文档的 `params`/`inputs`/`outputs`/`retry`/`on_error` 五项错误——
- P17 真实字段为 `input`(=params，自由 JSON) / `output`(单变量名，非对象) / `condition` / `timeout:Option<u64>` / `retry_count:Option<usize>`（非 `retry`）/ **步骤级无 `on_error`**。
- 照字面发射未知字段会被 serde 静默丢弃（回读丢失 + 违反验收 #6），且令 P28「能配了再跑」落空。

**拍板（用户授权「不要问」）**：严格映射 P17 真实字段名；删步骤级 `on_error`（工作流级 `error_handling` 不在 P29）；`condition` 用等宽 textarea 而非 Monaco（扁平设计 + 零重依赖）；D2「保存前校验」改为前端结构性校验（保 Rust 零改动）。

## 二、实际字段映射（单一事实来源 `STEP_FIELDS`）

| 文档 §三 | P29 实际落点（P17 字段名） | 控件 |
|---|---|---|
| `params` | `input`（技能入参对象） | 表单(key/value) + JSON 双模式 |
| `inputs` (name+type+required) | 由 `input` 承载（P17 步骤级无 inputs schema） | 同上 |
| `outputs` (name+type 对象) | `output`（单个输出变量名） | 单行文本 |
| `condition` | `condition` | 等宽 textarea |
| `timeout` | `timeout`（秒，u64） | NumberInput 1–300 |
| `retry` | `retry_count`（usize） | NumberInput 0–5 |
| `on_error` | **删除**（P17 步骤级无此字段） | — |

## 三、交付内容（前端主导，Rust 零改动）

- **`types/workflow.ts`**：`WorkflowStepDoc` 扩 `input`/`output`/`condition`/`timeout`/`retry_count`（字段名 = P17 `WorkflowStep`）。
- **`lib/workflow.ts`**：
  - `StepNodeData`（节点 data 携带步骤配置）、`docToNodesEdges` 按 id 合并步骤字段进节点 data、`nodesEdgesToDoc` 写回字段（undefined 不发射，避免污染 YAML）。
  - `STEP_FIELDS` 契约表（§四 D1 单一事实来源，驱动面板与校验范围）。
  - 校验：`validateTimeout`(1–300 整数) / `validateRetry`(0–5 整数) / `validateInputJson`(须 JSON 对象) / `docHasErrors`（阻止正式保存）。
- **`components/workflow/NodePropertiesPanel.tsx`**（新建）：右侧固定面板——`input` 表单/JSON Tab 切换（可增删键值）、`output` 单行、`condition` 等宽 textarea、`timeout`/`retry_count` NumberInput，字段级红字校验，底部说明 P17 字段名与 `on_error` 不在步骤级。
- **`routes/WorkflowEditorPage.tsx`**：
  - 选中节点追踪（`nodes.find(n=>n.selected)`）→ 右侧面板，`key={node.id}` 确保切换节点面板重置无残留（验收 #5）。
  - `updateNodeData(id, patch)` 写回节点 data → 触发 500ms 草稿防抖。
  - **D3 每节点草稿隔离**：切换节点 `clearTimeout(draftTimer)`。
  - **D4 保存互斥**：`savingRef` 置位期间 `scheduleDraft` 跳过。
  - **保存前校验门控**：`doSave`/`doRun` 前 `docHasErrors` 检查，违规则红字横幅 + 阻止保存/运行（验收 #4/#6）。

## 四、门禁结果

| 门禁 | 结果 |
|---|---|
| `pnpm build` | ✅ |
| `pnpm typecheck` | ✅ 0 |
| `pnpm lint` | ✅ 0（0 warning） |
| `cargo test --lib` | ✅ **677 passed / 0 failed**（Rust 零改动，基线未破） |
| `cargo clippy --lib` | ✅ 0（Rust 零改动，维持 P28 基线） |

## 五、验收映射

| # | 验收项 | 沙箱可达性 |
|---|---|---|
| 1 | 点击节点显示全部可编辑字段 | ✅ 前端可构建（面板读 `node.data`） |
| 2 | 改字段 500ms 草稿自动保存，重开显示新值 | ✅ localStorage fallback（`updateNodeData`→`scheduleDraft`） |
| 3 | Cmd+S 后 P17 `Workflow::load` 可加载 | ⚠️ 沙箱无 webkit；只发 P17 真实字段名 ⇒ 等价 load-safe，真编待本地 `tauri dev` |
| 4 | 非法输入（timeout:-5）红字阻止保存 | ✅ 前端 `docHasErrors` 门控 |
| 5 | 切换节点面板更新无残留 | ✅ `selected` 派生 + `key` 重置 + D3 |
| 6 | 编辑后 YAML 不引入新字段、不破坏必填 | ✅ 单一事实来源约束（只发 P17 字段名） |

## 六、唯一未解风险（与 P25-P28 同限制）

Tauri 命令（`save_workflow` 透传）在沙箱因缺 `webkit2gtk` 仅语法保全、未经验证编译。**但 P29 未改任何 Rust**——`save_workflow` 仍是既有透传，前端只发 P17 已知字段名，故真实写盘经 `Workflow::load` 必然可加载。仍建议 Seeker 本地 `pnpm tauri dev` 实编实跑，确认面板编辑 → 正式 YAML → P17 执行 的端到端闭环（验收 #3 真实路径）。

## 七、偏差声明（已拍板，非设计违规）

1. 删除步骤级 `on_error`（P17 `WorkflowStep` 无此字段；工作流级 `error_handling.on_step_failure` 不在 P29 面板范围）。
2. `condition` 用等宽 textarea（Geist Mono，已装）而非 Monaco —— 扁平设计纪律 + 零重依赖 + condition 为短表达式。
3. `params`/`inputs` 合并映射到 P17 `input`；`outputs` 映射到 P17 单变量名 `output`。
4. D2「保存前 `Workflow::load` 校验」改为前端结构性校验（保 `save_workflow` 零改动）；最终 load 门控仍是 P17 `Workflow::load`（P28 运行 / 再次 load 时触发）。
