# P27 交付报告 — 工作流画布（模式 C：显式保存 + 自动草稿快照）

> 状态：`_AP` 已交付 · 沙箱门禁全绿 · 本地手动验收待 Seeker
> 决策依据：Fathom 推演 #001 → 模式 C
> 方案：**选项 1**（前端真实 Tauri invoke + 沙箱 localStorage fallback；Rust 侧新建 WorkflowScanner + 单元测试闭环验收 #6）

## 1. 概要

按 Fathom #001 决策落地「模式 C」持久化策略，交付工作流画布的完整骨架：

- **正式文件** `~/.caspian/workflows/<name>/workflow.yaml` —— 仅 `Cmd+S`/保存按钮原子写入。
- **草稿文件** `~/.caspian/workflows/.drafts/<name>.yaml` —— 每次编辑 500ms 防抖自动写入。
- **冲突检测** —— 保存比对加载时记录的正式文件 mtime，外部修改则提示、不静默覆盖。
- **引擎隔离** —— P17 扫描器跳过 `.drafts/`，画布崩溃不污染可执行集。

> 路径结构沿用 **P17 既有子目录约定**（见 §2 F2），未采用 P27 文字里的扁平 `workflows/<name>.yaml`。

## 2. 前置核对裂缝与处置（详见 `P27_PRECHECK.md`）

| # | 裂缝 | 处置 |
|---|---|---|
| F1 | 全仓无工作流定义扫描器，验收 #6 无承载方 | 新建 `workflow/scanner.rs` |
| F2 | P27 文字称 P17 读扁平 `workflows/*.yaml`，但 `schema.rs` 实为该目录 `<name>/workflow.yaml` 子目录。扁平实现会破坏 P17 引擎 | **采纳 P17 子目录约定**（偏差已记录，以不破坏已交付依赖为准） |
| F3 | `CaspianPaths` 无 workflow 定义目录字段 | `paths.rs` 新增 `workflows` + 纳入 `ensure_dirs` |
| F4 | 任务清单 `pnpm add reactflow`(v11) 与 React19 peer 冲突 | 改用 `@xyflow/react` v12 |
| F5 | 持久化/草稿/冲突逻辑无落点，沙箱无 webkit 跑不了真实 fs | 纯 Rust `manifest.rs`（单测可验）+ 前端 localStorage fallback |

## 3. 交付文件

### Rust（`src-tauri/src/`）

| 文件 | 内容 |
|---|---|
| `workflow/scanner.rs` | `WorkflowScanner` / `WorkflowSummary`；遍历 `workflows/` 子目录、跳过隐藏目录（`.drafts` 自然跳过）、`Workflow::load` + 取 mtime；8 个单测 |
| `workflow/manifest.rs` | `save_workflow`（原子写 + `expected_mtime` 冲突检测 + 清草稿）、`save_draft`、``read_raw``（`ui` 原样回传）、`read_workflow`、`delete_workflow`、`list_entries`；9 个单测 |
| `config/paths.rs` | `CaspianPaths.workflows` 字段 + `resolve`/`ensure_dirs` |
| `types/error.rs` | `WorkflowError::Conflict { name, reason }` 变体 |
| `workflow/mod.rs` | 挂载 `manifest`/`scanner` 并导出 |
| `tauri_app.rs` | `list_workflows` / `load_workflow` / `save_workflow` / `save_workflow_draft` / `delete_workflow`（feature 门控，JSON↔YAML） |

### 前端（`src/`）

| 文件 | 内容 |
|---|---|
| `types/workflow.ts` | `WorkflowDoc` / `WorkflowListEntry` / `WorkflowLoadResult` / `SaveResult` 等类型 |
| `lib/workflow.ts` | `docToNodesEdges` / `nodesEdgesToDoc` / `blankDoc`（JSON 文档 ⇄ React Flow） |
| `hooks/useCaspian.ts` | 扩 `listWorkflows` / `loadWorkflow` / `saveWorkflow` / `saveWorkflowDraft` / `deleteWorkflow`（invoke + localStorage fallback） |
| `components/workflow/WorkflowCanvas.tsx` | React Flow 画布（自定义 StepNode、连线、删除键） |
| `routes/WorkflowsPage.tsx` | 列表（名称/修改时间/步数/删除）+ 新建 |
| `routes/WorkflowEditorPage.tsx` | 编辑器：500ms 防抖草稿、Cmd/Ctrl+S 原子保存、mtime 冲突提示 |
| `components/layout/Sidebar.tsx` | 新增「工作流」导航（展开+折叠两态） |
| `App.tsx` | 新增 `/workflows` 与 `/workflows/:name` 路由 |
| `components/command/CommandPalette.tsx` | 新增「工作流」跳转指令 |
| `index.css` | React Flow 扁平化（controls/handle/edge 配色 + 4px 半径，零阴影） |
| `package.json` | 新增 `@xyflow/react` 12.11.3 |

## 4. 数据契约

前端持有 `WorkflowDoc`（`steps` + `ui.{nodes,edges}`）。`ui` 段为画布布局，P17 `Workflow` 无 `deny_unknown_fields` 故安全忽略，不污染执行引擎。

- **真实 Tauri 路径**：前端发 JSON → Rust `serde_json::Value` → `serde_yaml` 写盘（保存前 `Workflow::from_yaml` 校验）；读取时 YAML → JSON 回传。
- **沙箱 fallback**：localStorage 直接存 JSON（`caspian.wf.<name>` 正式 / `caspian.wfdraft.<name>` 草稿）；加载优先取草稿以恢复刷新前的编辑。

## 5. 验收映射与门禁

| 验收 | 承载 | 验证方式 |
|---|---|---|
| #1 列表显示 | `WorkflowsPage` + `list_workflows` | 构建可验；手动验收待本地 |
| #2 新建进入画布、可拖拽 | `WorkflowEditorPage` + React Flow | 构建可验 |
| #3 500ms 草稿自动保存 + 刷新恢复 | 防抖 `saveWorkflowDraft` + 加载取草稿 | Rust 单测 `test_draft_isolated_from_list` 等；UI 走 localStorage fallback |
| #4 显式保存原子写 + 清草稿 | `save_workflow` temp+rename + 删 `.drafts` | Rust 单测 `test_atomic_save_no_partial_on_crash` / `test_explicit_save_clears_draft` |
| #5 外部修改冲突提示 | `expected_mtime` 比对 + `WorkflowError::Conflict` | Rust 单测 `test_conflict_when_external_edit_changes_mtime`；前端冲突横幅 |
| #6 引擎忽略 `.drafts/` | `WorkflowScanner` 跳过隐藏目录 | Rust 单测 `test_list_skips_drafts_dir` |

**门禁结果（沙箱）**

| 门禁 | 结果 |
|---|---|
| `pnpm build` | ✅ 通过 |
| `pnpm typecheck` | ✅ 0 错误 |
| `pnpm lint` | ✅ 0 错误 0 warning |
| `cargo test --lib` | ✅ **675 passed / 0 failed / 18 ignored**（658 基线 + 17 新增 P27 单测） |
| `cargo clippy --lib` | ✅ 0 warning |

> Tauri `tauri` feature 不在沙箱门禁内（需 webkit2gtk，沙箱无）；命令代码按可编译标准编写并 feature 门控，与 P25/P26 一致。

## 6. 已知限制 / 本地手动验收项（待 Seeker）

- 验收 #3/#4/#5 的**真实文件系统行为**（原子写、mtime 冲突、`.drafts` 隔离）由 Rust 单测在沙箱闭环；**UI 端真实写盘**需在本地 `pnpm tauri dev -- --features tauri` 跑，沙箱无 webkit 无法启动。
- 节点属性面板、工作流执行、版本历史为后续阶段（P27 范围外，已排除）。
- 画布当前映射「节点=步骤、连线=依赖」的最小骨架；P17 的 `condition`/`vars`/`iterate`/`error_handling` 等高级字段暂未进入编辑器 UI。
