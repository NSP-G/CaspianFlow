# CaspianFlow (V1)

> AI 驱动的工作流与技能编排桌面应用 —— Rust + Tauri v2 + React 19。

本仓库为 **V1 全量源码快照**（P01–P41 完整交付）。Jason 可直接据此编译、打包并生成三平台安装包。

## 目录速览

| 路径 | 内容 |
|------|------|
| `src/` | 前端（React 19 + TS + Vite + Tailwind v4 + Zustand v5） |
| `src-tauri/` | Rust 后端（Tauri v2 命令、Skill 引擎、自愈合、SQLite） |
| `src-tauri/src/skill/builtin/` | 17 个内置 Skill（5 核心 + 12 系统） |
| `docs/help/` | 用户手册（6 篇 Markdown，含 14 条 FAQ） |
| `.github/workflows/` | CI（`ci.yml`）与 Release（`release.yml`，三平台矩阵） |
| `BIG_ITEM_A/B/C_REPORT.md` | 大件 A/B/C 交付报告 |
| `DIRECTION_SYNC.md` | 全程方向同步记录（L1–L44） |

## 环境要求

- **Node.js** ≥ 22（已用 22.13.0 验证）
- **pnpm**（前端包管理，`pnpm-lock.yaml` 已锁定）
- **Rust** 稳定工具链（含 `cargo`、`rustc`）
- **Tauri v2 系统依赖**：
  - Linux：webkit2gtk-4.1、libsoup3、libjavascriptcoregtk、patchelf、librsvg
  - macOS：Xcode Command Line Tools
  - Windows：WebView2 运行时 + Visual Studio Build Tools（C++ 桌面）
- **Tauri CLI**：`cargo install tauri-cli`（或 `pnpm tauri` 经 `package.json` 脚本）

## 构建（三平台）

```bash
# 1. 解压并进入仓库
unzip caspianflow-v1-full-src.zip
cd caspian-flow

# 2. 前端依赖与构建（pnpm-lock.yaml 已锁定，可复现）
pnpm install
pnpm build

# 3. 后端 + Tauri 打包（--features tauri 开启 GUI/打包特性）
cargo tauri build --features tauri

# 安装包输出位置
# - Linux  : src-tauri/target/release/bundle/appimage/
# - macOS  : src-tauri/target/release/bundle/dmg/
# - Windows: src-tauri/target/release/bundle/msi/
```

## 门禁（沙箱内已验证）

- `cargo test` **736 passed / 0 failed**（lib 725 + 集成 6 + doctest 5）
- `cargo clippy --lib --tests` **0 warnings**
- `npx tsc --noEmit` **0 errors**

## 诚实标注（需本地/CI 环境确认的项，属环境限制非代码缺口）

- GUI 崩溃上报需 webkit 构建环境
- 完整 `vite build` 在本沙箱因内存不足中断（已用最小 lib 构建验证 glob 内联）
- `cargo tarpaulin` 覆盖率数字需本地 `llvm-tools-preview`
- GitHub Actions 真实运行需 runner

更新器 `pubkey` 与签名/公证密钥为一次性人工步骤，详见 `.github/workflows/release.yml` 头部说明。
