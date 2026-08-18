# P27 前置核对报告 — 工作流画布（模式 C：显式保存 + 自动草稿快照）

> 状态：`_AP`（已对齐，进入实现）
> 决策依据：Fathom 推演 #001 → 模式 C
> 方案：选项 1（前端真实 Tauri invoke + 沙箱 localStorage fallback；Rust 侧新建 WorkflowScanner + 单元测试闭环验收 #6）

## 0. 已对齐的前提（无需重建）

| 项 | 状态 | 来源 |
|---|---|---|
| React 19 + Vite + Tailwind v4 + Zustand 5 + react-router 6 | ✅ 已交付 | P25 |
| Card / Switch / Button(primary) / danger token | ✅ 已交付 | P26 |
| 侧边栏骨架（技能/知识库项已占位） | ✅ 已交付 | P26 |
| Tauri feature 门控（默认 `cargo test --lib` 不编译 webview） | ✅ 已交付 | P25 |
| P17 工作流引擎（DAG/执行/校验） | ✅ 已交付 | P17 |

## 1. 裂缝与决议（F1–F5 + 路径约定冲突）

| # | 裂缝 | 决议 |
|---|---|---|
| F1 | P27 设计里**没有** `<name>/workflow.yaml` 子目录扫描器（全仓仅 SkillScanner 做 read_dir）。验收 #6「引擎忽略 `.drafts/`」无承载方。 | 新建 `workflow/scanner.rs`：`WorkflowScanner` 遍历 `~/.caspian/workflows/` 子目录、跳过隐藏目录（`.drafts` 自然跳过）、加载 `workflow.yaml`。纯 Rust，单测闭环 #6。 |
| F2 | P27 写正式文件为扁平 `workflows/<name>.yaml`，**但 P17 `schema.rs` 头注释与 `Workflow::load` 明确约定 `<name>/workflow.yaml` 子目录**，`path` 取 manifest 父目录。扁平结构会直接破坏 P17 引擎。 | **采纳 P17 既有约定**：正式文件 `~/.caspian/workflows/<name>/workflow.yaml`；草稿 `~/.caspian/workflows/.drafts/<name>.yaml`。引擎兼容零破损。 |
| F3 | `CaspianPaths` 无 workflow 定义目录字段（只有 P17 运行态 `temp/workflows`）。 | `paths.rs` 新增 `workflows: PathBuf`，`ensure_dirs` 一并创建。 |
| F4 | 任务清单写 `pnpm add reactflow`（v11），与 React 19 的 peer 冲突。 | 改用 **`@xyflow/react` v12**（React Flow 官方 React19 兼容包），API 等价。 |
| F5 | 持久化/草稿/冲突逻辑无落点，且沙箱无法跑真实 fs（无 webkit）。 | 纯 Rust `workflow/manifest.rs` 承载原子写/草稿/mtime 冲突（单测可验）；前端 `useCaspian` 在 Tauri 内 invoke、沙箱回退 localStorage；真实 fs 行为由 Rust 单测覆盖，手动验收留本地 `tauri dev`。 |

> 注：路径约定（F2）是**与用户 P27 文字描述的偏差**。理由：P27 文字与「技术依赖」表都假设 P17 读扁平 `workflows/*.yaml`，但 `schema.rs` 实际是子目录结构。若按扁平实现会令 P17 引擎找不到工作流，因此以「不破坏已交付依赖」为准绳，采用 P17 子目录约定。验收 #6 在子目录下依然成立（引擎只扫非隐藏目录）。

## 2. 数据模型（前端↔Rust 契约）

- 画布编辑器在**前端**持有 JSON 文档：`{ name, display_name, description, steps:[{id,skill,input,output,depends_on}], ui:{ nodes:[{id,x,y,skill}], edges:[{id,source,target}] } }`。
- `ui` 段为画布布局，**P17 `Workflow` 无 `deny_unknown_fields`，会安全忽略**，不污染执行引擎。
- 真实 Tauri 路径：前端发 JSON 字符串 → Rust 命令 `serde_json::Value` → `serde_yaml` 写出 `workflow.yaml`（保存时先 `Workflow::from_yaml` 校验）；读取时 YAML → JSON 回传。沙箱回退路径：localStorage 直接存 JSON。

## 3. 冲突检测（验收 #5）

- `save_workflow(root, name, yaml, expected_mtime: Option<u64>)`：若 `expected_mtime` 与当前正式文件 mtime 不一致 → 返回 `WorkflowError::Conflict`。
- 前端加载时记录 mtime；`Cmd+S`/保存按钮带上；不一致则弹提示、不静默覆盖。
- Rust 单测覆盖「外部修改后 mtime 变化 → 冲突返回」。

## 4. 门禁

- `pnpm build` + `pnpm typecheck` + `pnpm lint` 全绿。
- `cargo test --lib` 保持绿（基线 658 passed / 0 failed / 18 ignored）。新增 scanner + manifest 单测只增不减。
- `cargo build --features tauri` 不在门禁内（需 webkit2gtk，沙箱无），但命令代码按可编译标准编写。

## 5. 验收映射

| 验收 | 承载 |
|---|---|
| #1 列表显示 | WorkflowsPage + `list_workflows`（scanner） |
| #2 新建进入画布、可拖拽 | WorkflowEditor + React Flow |
| #3 500ms 草稿自动保存 + 刷新恢复 | 500ms 防抖 `saveWorkflowDraft` + 加载草稿 |
| #4 显式保存原子写 + 清草稿 | `save_workflow` 原子写 temp+rename + 删 `.drafts/<name>.yaml` |
| #5 外部修改冲突提示 | `expected_mtime` 比对 + `WorkflowError::Conflict` |
| #6 引擎忽略 `.drafts/` | `WorkflowScanner` 跳过隐藏目录（单测） |
