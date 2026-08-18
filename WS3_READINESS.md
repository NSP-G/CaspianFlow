# WS3 就绪备忘 · 主题库接入（`.caspian-theme`）

- **状态**：`READY`（缝已锁、规格待拍板；**未写任何加载代码**）
- **作者**：Keel
- **时间**：2026-08-15
- **承接**：P30（L25/L26/L27）WS3 延后决议；本备忘是 WS3 进入「开工设计报告 → 前置核对 → 分层实现」前的就绪捕获
- **硬约束**：在 `.caspian-theme` 包格式/加载器规格到位前，**不写任何 `.caspian-theme` 加载代码**（P30 延后的全部理由即此）

---

## 0. 为什么这是「就绪备忘」而非「开工设计报告」

P30 设计报告 §8 与交付报告 §8 已记录：WS3 的阻塞项是**外部规格**——`.caspian-theme` 包格式与加载器在代码里**完全不存在**（全仓零引用、零加载器），属 DIRECTION_SYNC §3 的设计意图而非代码事实。Keel 读代码无法凭空构造 manifest 字段集、解包落点、与 `@theme` token 的映射规则。

因此本阶段只做**规格无关**的两件事：
1. 锁定 WS3 将复用的**已有集成缝**（代码事实，已验证）。
2. 列出**必须 Seeker/Lantern 拍板的规格问题**（阻塞项，无此则不开工）。

---

## 1. 现有主题架构（WS3 的改造基底，已核代码事实）

| 层 | 事实 | 引用 |
|---|---|---|
| **配置** | `Settings.app.theme: String`，默认值 `default_theme()`=`"dark"`；仅接受 `dark`/`light`/`system`，否则 `validation` 警告并回退 `dark` | `config/settings.rs:42-43,55`；`config/validation.rs:47-51` |
| **热加载** | `ConfigWatcher`（`config/watcher.rs`）watch `settings.yaml`，debounce 500ms，`on_reload(callback)` 推 `ArcSwap<Settings>` 原子替换；`app.theme` 改动能经此热加载到 live 配置 | `config/watcher.rs:144` `on_reload`；`:198-205` 测试覆盖 theme 热更新 |
| **前端 token** | `src/index.css` 的 `@theme` 块定义 `--color-accent:#4a6b8a` + neutral 灰阶（50–950）+ `@theme inline` 把 `--color-background/foreground/muted/border/...` 映射到语义色；`app.theme` 决定 light/dark 取值 | `src/index.css:12-51` |
| **写入面** | `config_commands.rs` 有 `set_theme`/相关写盘（测试 `:86-91` 覆盖 `app.theme` 写回+重载） | `commands/config_commands.rs:86` |

**结论**：当前「主题」= `app.theme` 三态枚举 + 一份硬编码 `@theme` CSS。WS3 要做的不是改这个机制，而是让**外部 `.caspian-theme` 包**能注入/覆盖这套 token（或替换其中一部分）。

---

## 2. 已就位的集成缝（WS3 可直接复用，零新范式）

| 缝 | 位置 | WS3 用法 |
|---|---|---|
| **目录 watcher** | `src-tauri/src/hot_reload.rs`：`DirWatcher::watch(path: &Path, cb: DirChangeCallback) -> AppResult<Self>`，`DirChangeCallback = Arc<dyn Fn() + Send + Sync>`；路径不存在则 disabled 不 panic；存在则 500ms debounce + `RecursiveMode::Recursive` + 主线程收事件触发 `cb` | WS3 只需 `DirWatcher::watch(&paths.themes, on_theme_change)`——盯 `.caspian-theme/` 目录增删改，回调里 reload 主题包 + `emit theme_changed` |
| **Sync 范式** | `DirWatcher._debouncer: Mutex<Option<Box<dyn Any + Send>>>` 仅 RAII 恢复 `Sync`（同 `ConfigWatcher`，P24 F7 修过） | 直接复用，不重造、不回退兼容性 |
| **事件→UI 通道** | `tauri_app.rs` 已有 `app.emit("skills_reloaded", …)` / `("workflows_changed", ())` 范式；前端 `useCaspian.subscribeSkillsReloaded/subscribeWorkflowsChanged` 已封装 `listen` + mock 回退 | WS3 加 `emit "theme_changed"` + `useCaspian.subscribeThemeChanged`，前端接 `index.css` 变量注入 |
| **路径注册** | `CaspianPaths`（`config/paths.rs`）已有 `skills`/`workflows` 字段；`ensure_dirs` 已建这两个目录 | WS3 加 `CaspianPaths.themes` + `ensure_dirs` 一并建目录（沿用 P27/P28 同款字段+建目录范式） |

> 机械层面 WS3 **几乎零新基础设施**——这是 P30 把 `DirWatcher` 做对的直接红利。WS3 真正的工作量全在「包格式解析 + token 映射规则」，而这两件事依赖外部规格。

---

## 3. 必须 Seeker / Lantern 拍板的规格问题（WS3 硬前置）

> 以下任一条不闭合，WS3 不开工（避免造出与未来规格冲突的加载器）。

1. **`.caspian-theme` manifest 字段集**：包内 `theme.yaml`（或约定名）含哪些字段？至少需明确：包名/版本、`accent`/`neutral` 调色板是否走独立字段、是否含 `font`/`radius`/`shadow` 等扩展维度（注意 P25 §二 硬约束：半径 ≤4px、禁阴影——扩展维度须与此兼容）。
2. **与 `index.css` `@theme` token 的映射方式**：是 (a) 主题包提供一份完整 `@theme` 覆盖 CSS 由前端注入 `<style>`；还是 (b) 主题包只给语义色键值（`--color-accent` 等），由加载器写进 `document.documentElement.style.setProperty`；还是 (c) 走 Rust 侧读包 → 注入到某个 CSS 变量源？**P25 §二 的 token 体系是单一事实来源，WS3 不能在它之外另立一套**，须定清覆盖的优先级与边界。
3. **解包/落点/校验**：`.caspian-theme` 是目录(zip？tar？纯目录？)？落点是否在 `CaspianPaths.themes/<name>/`？有无 `checksum`/`signature` 校验（DIRECTION_SYNC §3 提过 `checksum`/`bin`/`lib`/`src`，但主题包大概率只需 `checksum`）？损坏包的处理（沿用 §3「缺失/损坏不崩溃」→ 回退默认主题）。
4. **与现 `app.theme` 三态的关系**：`.caspian-theme` 是替换 `dark`/`light` 两套内置，还是作为第三类「自定义主题」与 `system` 并存？`validation.rs` 的 `dark/light/system` 枚举是否要扩？
5. **热加载粒度**：目录级 `DirWatcher` 是否足够？还是单包内需 watch `theme.yaml` 单文件变更（当前 `DirWatcher` 用 `RecursiveMode::Recursive`，已覆盖子文件，够用——但需规格确认单包即一目录）。

---

## 4. 一旦规格到位，WS3 的预计落地形态（仅示意，待规格收敛）

- **Rust**：`config/paths.rs` 加 `themes` 字段 + `ensure_dirs` 建目录；`hot_reload` 或新 `theme/loader.rs` 解析 `.caspian-theme` 包（按拍板后的 manifest）；`tauri_app.rs` 起 `DirWatcher::watch(&paths.themes, on_theme_change)`，`on_theme_change` 重扫+`emit "theme_changed"`；命令 `list_themes`/`apply_theme`/`get_active_theme`（feature 门控）。
- **前端**：`useCaspian` 扩 `listThemes`/`applyTheme`/`subscribeThemeChanged` + mock 回退；`index.css` 的 `@theme` 引入 WS3 覆盖层（按拍板映射方式）；设置页加主题选择器（如 WS3 扩了 `app.theme` 枚举）。
- **门禁**：沿用 P25–P30（Rust 默认 `cargo test --lib` + clippy 0；前端 build/typecheck/lint 三绿；Tauri 命令本地 `tauri dev` 验收）。

---

## 5. 当前待办（阻塞在外部）

| # | 事项 | 阻塞方 | 状态 |
|---|---|---|---|
| W1 | `.caspian-theme` manifest 字段集 | Seeker / Lantern | `PEND` |
| W2 | 与 `@theme` token 的映射方式（a/b/c） | Seeker / Lantern | `PEND` |
| W3 | 解包格式/落点/校验规则 | Seeker / Lantern | `PEND` |
| W4 | 与 `app.theme` 三态的关系 | Seeker | `PEND` |
| W5 | 热加载粒度确认 | Seeker（可默认 Recursive 单目录） | `PEND`（可默认） |

> Keel 已就绪：缝已锁、门禁纪律已知、实现范式（DirWatcher + emit + useCaspian 订阅）已验证可复用。**规格一到，立写 `P31_DESIGN_DOC`（或 WS3 开工设计报告）→ 前置核对 → 分层实现 → 真跑验证**。

---

*本备忘不替代开工设计报告；它是 WS3 在规格到位前的就绪态捕获。在 W1–W4 闭合前，Keel 不会写入任何 `.caspian-theme` 加载代码。*
