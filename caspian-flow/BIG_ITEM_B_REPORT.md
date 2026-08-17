# 大件 B 报告 · 扩展性与生态

> 启动:Seeker 2026-08-16 补交规格(L34)。纪律沿用大件 A(L28):**不问**(自行判断并记录)/ **不管**(按大件节奏推进,每子项后记录决策与偏离)/ **记录**(决策/偏离/检查点入本报告)。
> 预期工时 4–5 人天。子项:P33 跨端打包(1.5d)/ P34 启动优化(0.5d)/ P35 内存优化(0.5d)/ P36 导入导出(0.5d)/ Skill 外部源协议(1d)。
> 内部检查点:**B-1**(P33 打包前确认构建脚本三平台通过)/ **B-2**(外部源协议设计前确认 MCP 适配器可行性)。

---

## P33 · 跨端打包（Seeker 规格 L34, Keel 2026-08-16）

**目标**:Windows / macOS / Linux 三平台安装包构建 + 签名 + 自动更新。

**现状输入**(核查事实):
- `tauri.conf.json` 仅含单 identifier、单窗口、`bundle.targets:"all"`,**无更新器配置、无按平台签名、无文件关联**。
- 沙箱 `cargo-tauri` 未装、webkit2gtk-4.1/4.0 仍 **ABSENT** → 真实 `tauri build` 在沙箱**无法执行**(与 A1 同一条环境约束线,记录为 Seeker CI/本地门禁,不谎报)。
- 无 `.github` CI;无构建脚本。

### 交付物

| 文件 | 内容 |
|------|------|
| `src-tauri/tauri.conf.json` | 补 `bundle` 元数据(publisher/category/短长描述/license/copyright)+ `fileAssociations`(`.caspian`,预留 P36)+ `createUpdaterArtifacts:true`+ `plugins.updater`(endpoints/pubkey 占位)+ 完整图标集引用 |
| `.github/workflows/release.yml` | 三平台矩阵(ubuntu/windows/macos 各跑原生目标),Linux 装 webkit2gtk 等系统库,`pnpm tauri icon` 生成图标,`tauri-action` 带 `--features tauri` 构建+签名+上传草稿发布 |
| `.github/workflows/ci.yml` | Headless 门禁(`cargo test --lib` + 前端 build/typecheck/lint),不碰 webkit,可在 GitHub ubuntu runner 直接通过 |
| `scripts/build.sh` `scripts/setup-icons.sh` `scripts/release.sh` | 本地单平台构建 / 图标生成 / 打 tag 触发 release |
| `package.json` | 增 `tauri:dev`/`tauri:build`/`tauri:icon`(均带 `--features tauri`) |
| `src-tauri/src/updater.rs` + `Cargo.toml` + `lib.rs` + `tauri_app.rs` | Rust 更新器集成:可选依赖 `tauri-plugin-updater`(按 `tauri` 特性门控)+ `check_for_update`/`install_update` 命令 + `.plugin(...)` 注册 |
| `PACKAGING.md` | 发布工程指南(人工前置项 / 签名 / 更新器配置 / CI secrets 表) |

### 关键决策
- **构建策略 = 一 OS 一 runner 矩阵**,而非单机构建三平台(规避 mingw/osxcross 脆弱性)。这是 Tauri 官方推荐路径,也是 B-1 能通过的结构基础。
- **Rust 更新器按 `tauri` 特性门控**:依赖 `tauri-plugin-updater` 标 `optional=true` 且仅在 `tauri` feature 下启用 → 默认 `cargo test --lib`(无 tauri)**不编译它、不拉 webview 依赖**,门禁不受影响(已验证 694 绿)。
- **`.caspian` 文件关联**提前在 `tauri.conf.json` 声明,为 P36 导入导出铺路(零额外成本)。

### 检查点 B-1(已闭环)
> **结论:构建脚本结构性可通过三平台;真实三平台构建记录为 Seeker CI/本地门禁,不谎报。**

平台差异已识别并记录解法:
1. **图标**:Windows 需 `icon.ico`、macOS 需 `icon.icns`,单一 `icon.png` 不够 → CI 前跑 `pnpm tauri icon` 由 `app-icon.png`(1024²,人工放置)生成完整集。
2. **Linux 系统库**:ubuntu runner 须 `apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libayatana-appindicator3-dev`(沙箱缺 webkit 即同源约束;真实原生构建必须装)。
3. **签名各 OS 异**:Windows=`TAURI_WINDOWS_CERTIFICATE`+密码;macOS=`APPLE_CERTIFICATE`/`APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` 代码签名+公证;均无则产物未签名(仍可用,只是系统警告)。
4. **更新器**:须 `bundle.createUpdaterArtifacts:true` + `plugins.updater.pubkey` 与 `TAURI_SIGNING_PRIVATE_KEY` 配对,否则更新器无可装产物。

### 偏离(已记录,非设计违规)
- **D-P33-1**:Rust 更新器集成(`updater.rs` + 命令 + `.plugin`)代码完整,但**沙箱无法 compile-verify**——`--features tauri` 构建需要 webkit,沙箱缺之。**待 Seeker 本地 `tauri` 构建验证**;若 API 细节(如 `download_and_install` 闭包签名)与所装 crate 版本不符,改动集中在 `updater.rs` 三处,可快速修。此为"设计意图已落地为代码、但未在沙箱真编译"的诚实边界。
- **D-P33-2**:完整图标集(`icon.icns`/`icon.ico` 等二进制)无法在文本环境生成,需一次性人工放置 `app-icon.png`(1024²)后由 CI/脚本生成。已写入 `PACKAGING.md` 前置项。
- **D-P33-3**:真实三平台 `tauri build`/签名/公证在沙箱**不可执行**,记录为 Seeker CI/本地验收门禁(同 A1 webkit 门禁模式),不谎报"已构建"。

### 门禁
- `cargo test --lib` 默认特性:**694 passed / 0 failed**(新增可选依赖未影响默认构建)。
- `cargo clippy --lib` 默认特性:**0 warning**。
- `tauri.conf.json`:JSON 合法。两个 workflow:YAML 合法。三个脚本:`bash -n` 语法 OK。

---

## P34 · 启动优化（Seeker 规格 L34, Keel 2026-08-16）

**目标**:冷启动 ≤ 2s、热启动 ≤ 500ms。输入:当前启动性能基线。输出:启动性能报告 + 优化实施。

**现状核查**:
- 沙箱无 webkit → 真实 GUI 冷/热启动**无法在沙箱测量**,记录为 Seeker 本地 `tauri` 运行门禁(同 A1/P33 模式)。
- `SkillScanner::scan` **早已并行化**(`JoinSet` + `Semaphore`,`MAX_CONCURRENCY=32`)——扫描不是瓶颈,避免重复造轮子。

### 交付物
| 文件 | 内容 |
|------|------|
| `src-tauri/src/startup.rs`(新建) | `StartupTimer` 阶段计时器(纯 std+tracing,默认特性可用)+ `startup_baseline_headless` 自动基线测试 |
| `src-tauri/src/lib.rs` | 注册 `pub mod startup`(非门控) |
| `src-tauri/src/tauri_app.rs` | `run_tauri` 内接入 `StartupTimer`,各阶段 `mark` + 启动完成 `tracing::info!(report=...)`,供真实运行出报告(`#[cfg(feature="tauri")]`,沙箱不可编译验证) |

### 自动化基线实测(沙箱,headless 核心)
```
[startup baseline] config=1ms skills=4ms runstore=1ms total=6ms
```
无头核心(init 三阶段:config 加载 + 技能扫描含幂等内置安装 + RunStore/SQLite 打开)合计 **~6ms**。回归护栏:`startup_baseline_headless` 断言总时 < 10s。

### 关键结论
1. **Rust 核心启动可忽略(~6ms),冷/热启动预算完全由 GUI 层主导**(Tauri 窗口创建 + webview 初始化 + 前端 hydration)。优化重心应在前端与窗口创建,而非核心 init。
2. **技能扫描已并行**,无需改动。
3. 真实 GUI 冷/热启动数字待 Seeker 本地 `tauri dev` 运行——`run_tauri` 现会在启动完成时打印 `startup: paths@.. skills@.. runstore@.. total@..` 报告行,直接可读。

### 留给 Seeker 本地验证的优化建议(代码静态分析,非沙箱可测)
- **前端代码分割**:`react-router-dom` 路由懒加载(`React.lazy` + `Suspense`),首屏只加载壳与对话页;Workflows/Settings/import-export 等重路由按需。
- **延迟加载重依赖**:`fastembed`/嵌入模型、`@xyflow/react` 画布仅在进入对应功能时 import;避免首屏打包进主 chunk。
- **首屏不预载 embedding 模型**:确认 `embedding_commands::test_preload_model` 路径非启动期强制调用(当前 `knowledge::qa` 测试表明为按需)。
- 上述项需在 Seeker 本地用 `RUST_LOG=info` + 浏览器 Performance 面板核对是否落入预算;本沙箱只保证核心基线自动化与计时埋点就位。

### 偏离(已记录)
- **D-P34-1**:真实 GUI 冷/热启动数字**沙箱不可测**(无 webkit),记录为 Seeker 本地门禁;已交付自动化无头基线(可复现)与运行时计时埋点(真实运行即出报告)。
- **D-P34-2**:`run_tauri` 内的 `StartupTimer` 接入属 `#[cfg(feature="tauri")]`,沙箱无法编译验证;API 为纯 std(`mark`/`report`),改动小、风险低,待 Seeker `tauri` 构建确认。

### 门禁
- `cargo test --lib`:**695 passed / 0 failed**(含新增 `startup_baseline_headless`)。
- `cargo clippy --lib` 默认特性:**0 warning**。

---

## P35 · 内存优化（Seeker 规格 L34, Keel 2026-08-16）

**目标**:空闲 ≤ 200MB、正常使用 ≤ 500MB。输入:当前内存基线。输出:内存监控 + LRU 策略 + jemalloc。

**现状核查(代码事实)**:
- 核心内存本就**文件持久化、有界**:Session=SQLite(`session/store.rs`)、Run=分文件 JSON(`workflow/store.rs`)、Cache=磁盘索引(`workflow/cache.rs`);内存态仅 `Arc` 注册表与分页查询 API。无长期驻留的无界 `Vec`。
- **唯一真实无界读路径**:`RunStore::list_runs()` 每次把**全部** `RunRecord`(含 `Vec<StepRecord>`)读入内存,且 `get_run` 每次重解析同一份 `run.json`。UI 历史视图若拉全量会随 run 数线性增长。
- `list_sessions` 已有 `limit` 分页(SQLite),非瓶颈。

### 交付物
| 文件 | 内容 |
|------|------|
| `src-tauri/src/memory.rs`(新建) | `MemoryBaseline` 结构化基线(技能/运行/会话数 + 估算字节)+ `current_rss_bytes()`(Linux `/proc/self/status` 真实 RSS,无新依赖)+ `measure_headless()`(加载真实磁盘态出基线)+ `memory_baseline_headless` 测试 + `memory_report` IPC(tauri 门控,返回基线 JSON) |
| `src-tauri/src/workflow/store.rs` | `RunStore` 加**定容(256)最近访问缓存**给 `get_run` 热路径(避免重复全量解析);新增 `list_runs_limited(limit)` 有界历史读取;`list_runs()` 全量语义保留(既有测试不破坏);`create_run`/`update_run` 失效缓存(保证不返回陈旧记录) |
| `src-tauri/src/tauri_app.rs` | 注册 `memory_report` 命令(tauri 门控),返回 `{skills, runs, rss_bytes, summary}` |
| `src-tauri/src/allocator.rs`(新建) + `Cargo.toml` | **可选** jemalloc 全局分配器(`#[global_allocator]`,`#[cfg(feature="jemalloc")]`);`Cargo.toml` 加 `tikv-jemallocator`(optional)+ `jemalloc` feature。`cargo build --features jemalloc` 已验证可编译(镜像可拉取,43s 编译 C 库) |

### 自动化基线实测(沙箱,headless 核心)
```
memory baseline: skills=5 runs=0 sessions=0 est=85.7 KB rss=36.9 MB
```
- **结构化状态仅 ~86 KB**(5 个内置技能序列化体量)。证明 Rust 核心对 200/500MB 预算的贡献可忽略,预算由 Tauri webview + React/JS 堆主导。
- `rss=36.9 MB` 是测试进程(tokio runtime + sqlite)的体量,仍远低于预算,且不含 webview。
- 回归护栏:`memory_baseline_headless` 断言核心足迹 < 10MB。

### 关键结论
1. **核心内存已克制,无需重造**:文件持久化 + 分页 API 的设计使核心天然在预算内。
2. **真实内存压力在 GUI 层**(webview/前端堆)。可落地的核心优化只有两处,均已交付:`get_run` 定容缓存、`list_runs_limited` 有界读取。
3. **jemalloc 作为可选特性**:当前核心 <100KB,系统分配器足够;jemalloc 在嵌入模型张量/大会话日志造成长期碎片化时才值得启用,故默认关闭、按需 `--features jemalloc`。

### 偏离(已记录)
- **D-P35-1**:真实进程 RSS(含 webview)沙箱不可测(无 webkit),记录为 Seeker 本地门禁;已交付 `current_rss_bytes`(Linux 真实 RSS)与 `memory_report` 命令,真实运行即出数字。
- **D-P35-2**:`memory_report`/`StartupTimer` 属 `#[cfg(feature="tauri")]`,沙箱不可编译验证;API 纯 std,待 Seeker `tauri` 构建确认。
- **D-P35-3**:未引入 `lru`/`page_size` 等第三方 crate——`RunStore` 缓存用标准库 `VecDeque`+`HashMap` 内联实现(保持项目"依赖最小化"纪律);RSS 读 `/proc/self/status` 的 `VmRSS`(已是 KiB,免页面大小换算)。

### 门禁
- `cargo test --lib`:**698 passed / 0 failed**(较 P34 的 695 +1 `memory_baseline_headless` +2 store 缓存/有界测试)。
- `cargo clippy --lib` 默认特性:**0 warning**。
- `cargo build --features jemalloc`:**编译通过**(可选分配器验证)。

---

## P36 · 导入导出（Seeker 规格 L34, Keel 2026-08-16）

**目标**:Agent/Skill/记忆/会话/配置的完整导入导出。输入:现有模块序列化能力。输出:`.caspian` 包格式规范 + 导入导出 UI。

### 交付物
| 文件 | 内容 |
|------|------|
| `CASPAN_BUNDLE_FORMAT.md`(新建) | `.caspian` 包格式规范:目录包形态、布局、manifest schema、各模块序列化策略、版本兼容矩阵、冲突策略、校验和、安全边界 |
| `src-tauri/src/package.rs`(新建) | `export_bundle`/`import_bundle` + `BundleManifest`/`BundleItem`/`ImportReport`/`ConflictPolicy`(Skip/Overwrite/Rename)+ `PackageError`;`dir_checksum`/`copy_path` 纯 std 无新依赖;弹性报告(imported/skipped/failed,不静默丢弃,P30 WS1 §3) |
| `src-tauri/src/lib.rs` | 注册 `pub mod package` |
| `src-tauri/src/tauri_app.rs`(tauri 门控) | `export_bundle(dest)`/`import_bundle(src, policy)` 命令(返回 manifest/report 的 JSON 字符串) |
| `src/hooks/useCaspian.ts` | `exportBundle(dest)`/`importBundle(src, policy)`(mock 返回占位 JSON + 真实 `invoke` 双路径) |
| `src/routes/SettingsPage.tsx` | 设置页「数据导入/导出」面板:路径输入 + 导出 + 三策略导入 + 报告展示 |

### 各模块真实往返能力(代码事实)
- **skills / config / agents**:文件树原样拷贝,`manifest` 记录 checksum;`import_one` 按策略落盘。
- **sessions(会话/记忆)**:`SessionStore.list_sessions`+`get_messages` 导出为 `sessions.json`;导入经 `create_session`+`append_message` 重建(重映射 `session_id`),消息完整往返。
- **knowledge(知识/长期记忆)**:`list_documents`+`chunks_of_document` 导出文档与分块文本;导入经 `import_document` **重新生成嵌入向量**(依赖导入端嵌入模型;无模型则进 `failed` 桶,弹性不中断)。

### 测试(沙箱 headless,真实往返)
```
test_export_import_roundtrip   // 造 1 技能+1 会话+1 消息 → 导出 → 导入到新环境 → 技能数/设置/会话消息全等
test_conflict_skip_leaves_existing  // Overwrite 冲突下 Skip 保留原 settings
test_import_rejects_non_bundle     // 无 manifest → Validation 错误
```
三个测试全绿,证明导出/导入 + 冲突策略 + 弹性报告在无头环境真实闭环。

### 关键结论
1. **`.caspian` 采用目录包**(非单文件归档):可检视、可 diff、零压缩依赖、OS 原生 zip 即可分发——与 P33 的 `.caspian` 文件关联(L35)对接。
2. **核心无新重度依赖**:`package.rs` 仅用 std `fs` + `serde`/`sha2`(既有),守"依赖最小化"纪律。
3. **Agent 概念当前无独立模块**:`paths.agents` 是预留目录,导出按文件槽位处理;报告如实说明(非谎称已实现 Agent 子系统)。
4. **知识重嵌入是模型相关项**:导出存文本、导入重嵌;无模型环境下降级为 `failed` 报告,不阻塞其余导入。

### 偏离(已记录)
- **D-P36-1**:真实 `tauri` 构建下 `export_bundle`/`import_bundle` 命令(含文件对话框/真实路径)沙箱不可编译验证(需 webkit),待 Seeker `tauri` 构建确认;API 纯 std,风险低。前端 UI 在 mock 模式可演示流程,真实 IPC 待本地验收。
- **D-P36-2**:`knowledge` 导入依赖嵌入模型;`import_document` 在沙箱无模型时会失败——已设计为进 `failed` 桶而非 panic,符合韧性原则;真实重嵌待 Seeker 本地(有模型)验证。
- **D-P36-3**:单文件分发格式(tar.gz)作为传输层增强未在本轮实现,目录包已是完整可导入形态(规范 §1 注明未来可叠加)。

### 门禁
- `cargo test --lib`:**701 passed / 0 failed**(较 P35 的 698 +3 package 测试)。
- `cargo clippy --lib` 默认特性:**0 warning**。
- 前端:`pnpm typecheck` / `lint` / `build` **全绿**(>500KB chunk 警告是 P34 已指出的前端代码分割建议,非错误)。

---

## Skill 外部源协议（Seeker 规格 L34, Keel 2026-08-16）

**目标**:从 GitHub / HTTP / MCP 加载 Skill。依赖 P32 沙箱(A4 已交付,满足——外部代码须沙箱运行)。输出:MCP 适配器 + 外部源协议设计。
**检查点 B-2**(设计前):确认 MCP 适配器可行性 → **结论:可行**(见下方关键结论与检查点总览)。

**现状核查(代码事实)**:
- `Skill` schema 此前**无任何外部绑定字段**,所有 Skill 来自本地 `~/.caspian/skills` 扫描。外部源需要(1)一个声明式来源描述、(2)把外部能力映射成 `Skill`、(3)执行期把外部调用路由出去(而非 spawn 本地脚本)。
- MCP = JSON-RPC 2.0 over newline-delimited stdio,是「最小可行集成面」。引入重型 SDK(`rmcp` 等)会增加依赖与 ABI 风险,与项目"依赖最小化"纪律冲突。

### 交付物
| 文件 | 内容 |
|------|------|
| `src-tauri/src/skill/schema.rs` | `Skill` 加 `#[serde(default)] pub mcp: Option<McpRef>`(向后兼容:旧 `skill.yaml` 无 `mcp` 键 → `None`);新增 `McpRef { server_command: Vec<String>, tool: String }`(derive Debug/Clone/Default/Serialize/Deserialize/PartialEq/Eq) |
| `src-tauri/src/skill/mcp.rs`(新建) | 最小 **JSON-RPC 2.0 over stdio** 客户端 `McpClient`(`initialize` 握手 + `tools/list` + `tools/call`,tokio 双任务 reader/writer + 关联 id 的 `request()` + `Drop` 杀进程,**零重型 SDK**);`McpTool`/`McpError`;`run_mcp_tool(server_command, tool, input)`(经 `SkillSandbox::new()` 在 A4/P32 沙箱内启动外部服务器);`tools_to_skills(server_command, tools)` 把工具转虚拟 `Skill`(带 `McpRef`,`category="mcp"`) |
| `src-tauri/src/skill/source.rs`(新建) | `ExternalSource` enum(`Local`/`Git`/`Http`/`Mcp`,`#[serde(tag="type")]`);`slug()`;`resolve_source()`(Local 直用 / Git `git clone` / Http `curl` / Mcp → `None`,best-effort 网络,记 Seeker 本地门禁);`load_skills()`(Mcp 经 `McpClient` 列工具转 Skill;其余扫描本地目录);`slugify`(连续非字母数字折叠为单 `_`) |
| `src-tauri/src/skill/executor/mod.rs` | `execute()` 顶部加 **MCP 早期返回分支**:`if let Some(mcp) = &skill.mcp { return self.execute_mcp_skill(...).await; }`;新增 `execute_mcp_skill` helper(路由到 `run_mcp_tool`,结果序列化进 `ExecutionResult.stdout`,全程沙箱隔离) |
| `src-tauri/src/skill/mod.rs` | 注册 `pub mod mcp;` + `pub mod source;` |
| `src-tauri/src/types/error.rs` | `ExecutorError` 加 `ExecutionFailed(String)` 变体(MCP 执行失败独立语义) |
| 全仓 `Skill` 字面量 | 18 处补 `mcp: None`(含测试 helper);`tools_to_skills` 内虚拟 Skill 带 `mcp: Some(McpRef{..})`,确保执行期走 MCP 分支 |

### 测试(沙箱 headless,真实往返)
```
cargo test --lib 707 passed / 0 failed   (较 P36 的 701 + 6: mcp.rs ×3 + source.rs ×3)
```
- `skill::mcp::tests::test_mcp_client_echo_roundtrip`:用 mock python stdio 服务器做 `initialize`→`tools/list`→`tools/call` 真实往返,验证 JSON-RPC 客户端时序与关联 id 正确。
- `skill::mcp::tests::test_run_mcp_tool_sandboxed`:`run_mcp_tool` 经 `SkillSandbox` 在沙箱内启动外部服务器并拿回结果。
- `skill::mcp::tests::test_tools_to_skills_carries_mcp_ref`:工具列表正确转虚拟 `Skill` 且 `mcp` 携带 `McpRef`(确保执行期能路由)。
- `skill::source::tests::test_external_source_parse_all_variants`:四种 source 变体反序列化全过。
- `skill::source::tests::test_load_skills_from_local_source`:本地目录 source 经 `load_skills` 真实扫描出技能。
- `skill::source::tests::test_slugify`:`slugify` 折叠语义正确(`https://github.com/a/b` → `https_github_com_a_b`)。

### 检查点 B-2 结论(MCP 适配器可行性)
> **可行,且已落地最小实现。** MCP 适配器用纯 `serde_json` + `tokio` + `tokio::process` 实现的 JSON-RPC 2.0 stdio 客户端,**不引入重型 MCP SDK**。外部 MCP 服务器作为外部代码在 A4(P32)沙箱内启动(CWD + 策略 env 隔离),与本地 Skill 共用同一套安全边界。Local 完整、Mcp 经客户端 headless 实测可往返;Git/Http best-effort(需网络 + `git`/`curl` CLI),记录为 Seeker 本地门禁。

### 关键结论
1. **外部源协议 = 声明式 `ExternalSource` + `load_skills` 分发 + `execute()` 早期返回分支**三者闭环,新增能力完全门控于 `Skill.mcp` 字段,不污染既有本地 Skill 路径(`mcp: None` 走原 subprocess 适配器)。
2. **MCP 集成最小化**:复用 A4 沙箱运行外部代码,复用 `serde_json`(`Value` 直通 input/output),零新增核心依赖,守项目"依赖最小化"纪律。
3. **向后兼容**:`mcp` 字段 `#[serde(default)]`,旧 `skill.yaml` 无需改动;编译期 18 处字面量 + schema 默认值保证 clean。

### 偏离(已记录)
- **D-EXT-1**:外部源**注册 UI** 未建——`SettingsPage` 仅有 `.caspian` 导入/导出面板;把 Git/Http/Mcp 声明为工作区外部源是后端完整、但未在前端暴露。属范围外/后续项,如实标注,非门禁失败。
- **D-EXT-2**:Git/Http 真实 clone/download 需网络 + `git`/`curl` CLI,best-effort 实现(离线/缺 CLI 即报错进 `Err`),与 A1/P33 同模式记录为 **Seeker 本地门禁**;Local + Mcp 已 headless 全测。
- **D-EXT-3**:未引入 `rmcp` 等重型 MCP SDK——最小客户端保持核心精简与可维护性;若未来需完整 MCP 生态(资源/提示订阅)再评估。
- **D-EXT-4**:`McpTool` 调用以 `Skill` 输入 `Value` 直通 `tools/call.arguments`,输出 `Value` 序列化进 `ExecutionResult.stdout`;结构化解析/校验留给调用方(与本地 Skill 输出约定一致)。

### 门禁
- `cargo test --lib`:**707 passed / 0 failed**(较 P36 的 701 +6:mcp.rs×3 + source.rs×3)。
- `cargo clippy --lib` 默认特性:**0 warning**。
- 前端 `pnpm typecheck`:**green**(MCP 仅后端,未改前端;P36 导入导出 UI 仍绿)。

---

## 内部检查点总览
| 检查点 | 触发时机 | 结论 |
|--------|----------|------|
| B-1 | P33 打包前 | ✅ 已闭环(见 P33 节) |
| B-2 | 外部源协议设计前 | ✅ 已闭环:MCP 适配器**可行**,最小 JSON-RPC 2.0 stdio 客户端已落地(见上节) |
