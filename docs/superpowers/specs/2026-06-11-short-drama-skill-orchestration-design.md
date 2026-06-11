# 短剧 Skill 编排设计（薄 skill + 最小桥）

- 日期：2026-06-11
- 状态：设计已确认，待写实现 plan
- 范围：仅 agent/skill 编排（后端 + skill），**不含**桌面 workbench UI

## 1. 背景与问题

短剧生成此前的设想是建一个"胖" internal tool，由 Rust 端一把梭编排
剧本→分镜→人物图→分镜视频→合成。本设计推翻该方向，改为
**薄 skill 驱动 agent 自主编排**——与项目里现有
`image-generation` / `video-generation` skill 模式一致：skill 是薄壳指导，
真正的媒体生成由既有 internal tool 经权限路径强制执行。

### 现状事实（代码实证）

- 现有 internal tool 只有 `image_generation`（`imagegen`）与 `video_generation`
  （`videogen`），各配一个同名 skill。**短剧 tool / skill 均不存在。**
- `videogen` 的 `imageReferences` **硬拒绝本地图**
  （`crates/puffer-core/runtime/claude_tools/workflow/video_generation.rs:145-162`）：
  只接受 `asset://` 或 `https://`，其它一律 `bail!`。
- `imagegen` 给 agent 的输出**只有本地 `path`，无任何可引用 URL**
  （`image_generation.rs:160-178`，字段仅 `artifactId/index/path/mimeType/size`）。
- 但底层 sidecar 元数据里**已存** provider 返回的 hosted 图地址
  `remoteSourceUrl`（`artifacts.rs:228-232` 有读取 helper；MiniMax 在
  `minimax_image.rs:278-282` 写入）。
- 产物落盘路径由工具硬编码（`artifacts.rs:557-575`）：
  图 `→ .puffer/media/images/`，视频 `→ .puffer/media/videos/<artifactId>/`，
  sidecar `→ .puffer/media/artifact-sidecars/<artifactId>.json`。

### 核心卡点

"人物图 → 生视频"链路当前**完全断开**：生图只交本地 path、生视频只收 URL、
中间无上传/取 URL 的桥。合成（多镜头拼接）则**零实现**。

## 2. 目标 / 非目标

**目标**

- 一份 `short-drama` skill，指导 agent 按需编排五阶段流水线。
- 打通"人物图 → 图生视频"，保住跨镜头人物一致性。
- 合成由 skill 指导 agent 用 Bash 跑 ffmpeg 完成。
- 短剧项目文件统一落盘到 `.puffer/media/drama/<id>/`。

**非目标（范围红线）**

- ❌ 不建短剧胖 internal tool（正是要避免的）。
- ❌ 不做桌面 workbench UI。
- ❌ 不做配乐 / TTS / 字幕烧录等高级合成（ffmpeg 仅做片段拼接 + 基础转场）。
- ❌ 不改 artifact 落盘路径、不改 provider、不碰权限路径。
- ❌ 不做上传/暂存 tool（曾作为"方案 C"，本设计**明确不采用**）。

## 3. 编排模型：五阶段 + 按需门控

`short-drama` skill 是一份编排剧本。agent 读它，对每阶段先判断
"资源是否已具备"，缺什么补什么，全程只调既有原子 tool + Bash。

| 阶段 | 门控判断（已具备就跳过） | 缺失时动作 |
|---|---|---|
| 1. 剧本 | prompt 已含剧本 / 指向剧本文件？ | agent 原生写剧本 → 存 `script.md` |
| 2. 分镜 | prompt 已含分镜表？ | agent 原生拆分镜（每镜：画面/对白/时长/所需人物）→ 存 `storyboard.md` |
| 3. 人物图 | **prompt 已含 `https://` / `asset://` 图地址？** | 缺则 `imagegen` 生成 → 取 `referenceUrl`（见 §4） |
| 4. 分镜视频 | —（始终生成） | 逐镜 `videogen`，把人物图 URL 作 `--image-reference` |
| 5. 合成 | —（始终合成） | skill 指导 agent 用 Bash 跑 ffmpeg 拼接 → `final.mp4` |

**关键门控规则**：

- 阶段 3 先扫 prompt 中是否已带 `https://` 或 `asset://` 图地址——
  **有则直接用作 `--image-reference`，不调 imagegen**；无则才生图。
- 阶段 1/2 同理：prompt 已给剧本/分镜则不重复创作。

由此同一 skill 既能"一句话生成整部短剧"，也能
"我已有人物图 + 分镜，只帮我生视频 + 合成"。

**资源整理中枢**：agent 在 `.puffer/media/drama/<id>/manifest.json` 维护
镜头清单（镜头序号 → prompt → 引用图 URL → 产出 video artifactId/path），
既是跨阶段资源台账，也是 ffmpeg 拼接的顺序依据。

## 4. 最小桥：透出 `referenceUrl`

唯一后端改动——`image_generation_output`（`image_generation.rs:160-178`）
的 artifact JSON 增加一个字段：

```diff
  "artifactId": artifact.artifact_id,
  "index": artifact.index,
  "path": artifact.path,
+ "referenceUrl": artifact.remote_source_url,   // 来自 sidecar 的 remoteSourceUrl，可能为 null
  "mimeType": artifact.mime_type,
  "size": artifact.byte_count
```

值复用 `artifacts.rs:228-232` 已有的 `remoteSourceUrl` 读取 helper；
需把该值从 provider 结果透到 `ImageGenerationResult` 的 artifact 结构再到输出。
**改动面仅此一处链路**，不碰 provider、落盘、权限路径。

agent 阶段 4 直接拿 `referenceUrl` 当 `videogen --image-reference`。

### 落盘约定

- 生成的图/视频 artifact：**工具硬编码**到 `.puffer/media/images|videos/`，
  skill 不重定向，只在 manifest 里**引用**。
- 短剧项目文件（`script.md` / `storyboard.md` / `manifest.json` / `final.mp4`）：
  由 skill 指定，统一存 `.puffer/media/drama/<id>/`。

## 5. 失败契约（写进 skill，杜绝静默降级）

1. 阶段 3 需要人物图，但 imagegen 返回的 `referenceUrl` 为 `null`
   （provider 不产出 hosted URL）→ agent **明确报错**
   "当前图像 provider 不产出可引用 URL，无法图生视频"，
   **不得**偷偷退回文生视频。
2. 选中视频 provider 为 Relaydance（prompt-only，不收图参考）→
   直接告知"该 provider 不支持图参考"（沿用现有 video skill 规则）。
3. ffmpeg 不存在 / 合成失败 → 如实报错，保留已生成的分镜片段，
   不假装合成成功。

## 6. 增量切分

**增量 1 — 纯 skill 编排循环（零后端改动）**

新增 `resources/skills/short-drama/SKILL.md` + 注册 skill/CLI 入口。
跑通：剧本→分镜→imagegen(预览)→videogen(**文生视频**)→ffmpeg 合成→
`.puffer/media/drama/<id>/final.mp4`，含 manifest 与全部按需门控。
**目的：先验证 agent 自主编排本身可行**，不依赖桥。

**增量 2 — 打通图生视频（方案 B 的桥）**

- **前置 spike（go/no-go 闸门，~1h）**：手动用 MiniMax 生一张图 → 取
  `referenceUrl` → 直接喂给 `videogen --image-reference` 走一次 BytePlus
  图生视频。验证 (a) URL 未即时过期；(b) 跨 provider 可被拉取。
- **通过** → 加 `referenceUrl` 字段，skill 阶段 3 改为传图参考做图生视频，
  保人物一致性。
- **不通过** → **停下来如实汇报**，不在本设计内引入上传/暂存 tool；
  是否另启替代方案为单独决策。

## 7. 受影响文件（预估）

- 新增 `resources/skills/short-drama/SKILL.md`（编排指导 + 门控 + 失败契约）。
- skill / CLI 别名注册点（参照 image/video 的 `internal_tools.rs` 注册方式）。
- 增量 2：`image_generation.rs` 输出加 `referenceUrl` 字段；
  `ImageGenerationResult` artifact 结构透传 `remote_source_url`。

## 8. 开放问题

- ffmpeg 在目标运行环境是否默认可用？skill 需先探测并在缺失时按契约 #3 报错。
- 增量 2 spike 的结论将决定桥是否成立（本设计已约定失败即汇报，不降级、不建 C）。
