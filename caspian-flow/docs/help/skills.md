# 内置技能

CaspianFlow 内置 **17 个系统级技能**，覆盖文件、网络、系统、数据、自我管理五类。它们随应用分发、首次运行自动安装，无需手动下载。

智能体会根据任务自动选择技能；你也可以在任何技能页查看、启用或禁用某个技能。

## 核心技能（5 个）

| 技能 | 能力 | 权限 |
|------|------|------|
| `read_file` | 读取文本文件内容 | 文件系统（读） |
| `write_file` | 写入/覆盖文本文件 | 文件系统（读+写） |
| `shell_command` | 执行 shell 命令 | shell |
| `http_request` | 发起 HTTP 请求 | 网络 |
| `summarize_text` | 文本摘要 | 无 |

## 系统技能（12 个）

| 技能 | 能力 | 权限 |
|------|------|------|
| `file-reader` | 读文件 → content / line_count / size / encoding | 文件系统（读） |
| `file-writer` | 写或追加文本到文件 | 文件系统（读+写） |
| `file-search` | 递归正则 grep 文件内容 | 文件系统（读） |
| `web-fetcher` | 用标准库抓取 URL → status / headers / body | 网络 |
| `shell-runner` | 执行命令 → stdout / stderr / exit_code | shell |
| `system-info` | 返回 OS / Python / CPU / 内存 的 JSON | 无 |
| `code-interpreter` | 用 `exec` 运行 Python 片段 | 无 |
| `json-parser` | 校验 / 美化 / 查询 JSON | 无 |
| `note-taker` | 追加带时间戳的笔记到 `notes.md` | 文件系统（写） |
| `memory-manager` | 读取 / 修改 `MEMORY.md` 记忆文件 | 文件系统（读+写） |
| `skill-manager` | 扫描技能目录，列出已安装技能 | 文件系统（读） |
| `workflow-runner` | 列出 `workflows/` 下的工作流 | 文件系统（读） |

## 权限边界说明

每个技能都有**显式权限声明**，智能体只能做技能被允许的事：

- **文件系统权限**限定可读/可写的路径前缀（如 `~/.caspian`、工作区）。
- **网络权限**的技能在离线时会被自动降级（见下），避免无声失败。
- **shell 权限**的技能执行命令时受沙箱约束（P32）。

## 离线降级

当检测到网络不可达时，所有标注 `network` 权限的技能会被自动禁用，本地技能照常工作。这意味着：

- 离线状态下，`web-fetcher` / `http_request` 不会运行，但 `file-reader` / `code-interpreter` 等本地技能不受影响。
- 这是优雅降级，不是错误——重新联网后技能会自动恢复可用。

## 管理技能

- 打开「技能」页查看全部已安装技能及其权限。
- 可按标签（file / network / system / data / self）筛选。
- 可临时禁用某个技能；禁用后智能体会改用其他可用技能或提示你。
