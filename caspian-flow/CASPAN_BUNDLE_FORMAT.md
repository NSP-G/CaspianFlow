# `.caspian` 包格式规范（P36）

> 状态：`CONF`（实现已交付，`package.rs` + Tauri 命令 + 设置页 UI 已落地）
> 承接：大件B · P36（Seeker 规格 L34）；与 P33 `tauri.conf.json` 的 `.caspian` 文件关联（L35）对接

CaspianFlow 用 `.caspian` 包在设备 / 用户 / 团队之间搬运**本地优先**状态：技能、配置、会话（记忆）、知识（长期记忆）、Agent 定义。目标是「可检视、可 diff、零压缩依赖、导入不静默丢弃」。

---

## 1. 形态：目录包（不是单文件归档）

一个 `.caspian` 包是**一个目录**，约定命名为 `<名称>.caspian/`。选目录而非 tar.gz/zip 的理由：

- **可检视**：人眼可直接读 `manifest.json` 与每个技能目录，便于审计、review、diff。
- **零压缩依赖**：无需引入 `tar`/`flate2`，核心 `package.rs` 仅用标准库 `std::fs`。
- **便于传输**：用户/CI 用 OS 原生 `zip` 即可打包成单文件分发；导入器接受 `<名称>.caspian/` 目录本身。

> 未来若需要单文件分发格式，可在不破坏目录语义的前提下在外部包一层 tar.gz（仅传输层，不影响本规范）。

## 2. 目录布局

```
<名称>.caspian/
├── manifest.json          # 必选：包清单（见 §3）
├── skills/                # 可选：技能目录，逐技能原样拷贝（含 skill.yaml + 资源）
│   └── <skill_name>/
│       ├── skill.yaml
│       └── …（entry 脚本、示例等）
├── agents/                # 可选：Agent 定义（预留槽位，当前为文件级拷贝）
│   └── <agent_name>
├── config/
│   └── settings.yaml      # 可选：设置快照
├── sessions.json          # 可选：所有会话 + 其消息（JSON 数组）
└── knowledge.json         # 可选：所有文档 + 分块文本（JSON 数组）
```

`skills/`、`agents/`、`config/` 为文件树拷贝；`sessions.json`、`knowledge.json` 为序列化的 JSON 侧车文件。

## 3. `manifest.json` 结构

```jsonc
{
  "format": "caspian-bundle",     // 魔数字符串，校验包类型
  "version": 1,                   // 包 schema 版本（见 §5 兼容矩阵）
  "created_at": "1723800000",     // Unix 秒，朴素时间戳
  "app_version": "0.1.0",         // 产出该包的应用版本（仅供参考）
  "items": [                      // 逐项清单，供导入器校验与报告
    {
      "kind": "skill",            // skill | agent | config | sessions | knowledge
      "name": "demo_skill",
      "path": "skills/demo_skill",// 相对包根的路径
      "checksum": "a1b2…"         // sha256（目录为其内容确定性哈希）
    }
  ]
}
```

校验规则（导入器强制）：
1. 缺 `manifest.json` → 拒绝（`not a .caspian bundle`）。
2. `format != "caspian-bundle"` → 拒绝（`unknown bundle format`）。
3. `version > BUNDLE_VERSION`（当前 1）→ 拒绝（`unsupported bundle version`）。

## 4. 各模块的序列化策略

| 模块 | 导出 | 导入 |
|------|------|------|
| **skills** | 整个技能目录原样拷贝 | 按冲突策略拷贝到 `paths.skills/<name>` |
| **agents** | `paths.agents` 内容原样拷贝（预留槽位） | 按冲突策略拷贝到 `paths.agents/<name>` |
| **config** | 拷贝 `settings.yaml` 快照 | 按冲突策略拷贝覆盖（注意：覆盖后需重载设置） |
| **sessions（会话/记忆）** | `SessionStore` 读出每个会话 + 全部消息 → `sessions.json` | `create_session` 重建会话，`append_message` 回填消息（重映射 `session_id`） |
| **knowledge（知识/长期记忆）** | 每个文档经 `chunks_of_document` 取回分块文本 → `knowledge.json` | `import_document` 重新入库（**重新生成嵌入向量**，依赖当前配置的嵌入模型） |

> **知识嵌入说明**：导出的 `knowledge.json` 存的是分块**文本**，不是向量。导入时由 `SqliteKnowledgeStore::import_document` 重新切分并嵌入，因此导入结果依赖导入端配置的嵌入模型。若导入端无模型可用，`import_document` 失败，该文档进入报告的 `failed` 桶（弹性：不中断其余导入）。

## 5. 版本与兼容

| `version` | 内容 |
|-----------|------|
| 1 | 初始格式：manifest + skills/agents/config 文件树 + sessions.json/knowledge.json |

向后兼容：导入器拒绝 `version > BUNDLE_VERSION` 的包（避免静默误读）；`version <= BUNDLE_VERSION` 一律接受。破坏性变更时**递增 `BUNDLE_VERSION`** 并在本表登记。

## 6. 冲突策略（导入端）

导入器对「目标已存在」的项应用 `ConflictPolicy`：

- **`skip`（默认）**：保留现有项，记入报告 `skipped`。
- **`overwrite`**：删除现有项后写入包中版本。
- **`rename`**：以 `<stem>_<N>` 找空名后写入，避免覆盖。

所有结果落入 `ImportReport { imported, skipped, failed }`——**任何损坏/失败项都进 `failed` 桶并明文报告，绝不静默丢弃**（P30 WS1 §3 韧性原则）。

## 7. 校验和

- `skills/<name>` 目录：对其下所有文件按路径字典序拼接（路径 + 字节）做 sha256，保证内容确定性可校验。
- `sessions.json` / `knowledge.json`：对整个序列化 JSON 做 sha256。
- 校验和当前用于清单自描述与审计；导入器**未强制比对**（包内完整性由文件系统保证），后续如需「传输损坏检测」可在此扩展为强制校验。

## 8. 安全边界

- 导入即把外部文件写入本地状态目录。**外部 `.caspian` 包视同外部输入**：技能导入后将在 P32 沙箱中执行（大件A · A4 已交付隔离 + 权限层），包本身不绕过该沙箱。
- `config` 覆盖可能在下次启动时改变应用行为；UI 在覆盖前明确标注策略，用户知情。
- 本格式不携带执行权限位 / 不执行包内任何脚本；所有内容经显式拷贝与显式重嵌入进入系统。
