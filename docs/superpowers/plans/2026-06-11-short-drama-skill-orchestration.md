# 短剧 Skill 编排 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用一份 `short-drama` SKILL.md 指导 agent 自主编排「剧本→分镜→（人物图）→分镜视频→ffmpeg 合成」，不建胖 internal tool；增量 2 用三行后端改动透出生成图的可引用 URL，打通"生成人物图→图生视频"。

**Architecture:** 薄 skill 驱动——skill 只是给 agent 的编排指导，真正的媒体生成仍由既有 `imagegen`/`videogen` internal tool 经权限路径执行，合成由 agent 用 Bash 跑 ffmpeg。增量 1 零后端改动且独立可交付；增量 2 在 `image_generation.rs` 把 runtime 已携带的 `remote_source_url` 透到工具输出 `referenceUrl`。

**Tech Stack:** Rust（puffer-core workflow tool）、resources/skills 资源加载、ffmpeg（Bash 外调）。

**Spec:** `docs/superpowers/specs/2026-06-11-short-drama-skill-orchestration-design.md`

---

## File Structure

- `resources/skills/short-drama/SKILL.md` — **新增**。短剧编排 skill：frontmatter + 五阶段编排指导 + 按需门控 + manifest 约定 + ffmpeg 模板 + 失败契约。（增量 1 唯一交付物。）
- `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs` — **增量 2 修改**。`ImageGenerationArtifactResult` 加 `remote_source_url` 字段、构建处透传、`image_generation_output` 输出 `referenceUrl`，并补一条测试断言。

无 `internal_tools.rs` 改动（短剧是编排 skill，非 CLI 内部工具；靠资源加载器自动发现）。

---

## 增量 1 — 纯 skill 编排循环（零后端改动）

### Task 1: 确认 skill 仅靠 SKILL.md 即可被发现

**Files:**
- Read: `crates/puffer-resources/src/loader.rs`（`load_skill_embedded`）
- Read: `resources/skills/reviewer/SKILL.md`（一个无 CLI 条目的编排 skill 样例）

- [ ] **Step 1: 核对加载机制**

Run: `grep -n "load_skill_embedded\|SKILL.md\|skill_by_name\|user-invocable\|disable-model-invocation" crates/puffer-resources/src/loader.rs`
Expected: 看到 skills 目录被逐个扫描 `SKILL.md` 并解析 frontmatter（`name/description/allowed-tools/user-invocable/disable-model-invocation`）。

- [ ] **Step 2: 核对一个无 CLI 条目的 skill 确实存在且被加载**

Run: `grep -rn "reviewer\|security\|autodream" crates/puffer-tools/src/internal_tools.rs; ls resources/skills/reviewer/SKILL.md`
Expected: `internal_tools.rs` 里**没有** reviewer/security/autodream 条目，但 `resources/skills/reviewer/SKILL.md` 存在 → 证明编排 skill 无需 CLI 注册。

- [ ] **Step 3: 记录结论**

无需改动 `internal_tools.rs`。若上述任一前提不成立（例如存在一个集中的 skill 白名单），停下来在本任务记录实际发现并据此调整后续任务。

### Task 2: 写 short-drama SKILL.md

**Files:**
- Create: `resources/skills/short-drama/SKILL.md`

- [ ] **Step 1: 创建 skill 文件（完整内容如下）**

```markdown
---
name: short-drama
description: Use when the user asks to create a short drama from a prompt — e.g. "生成短剧", "制作微短剧", "make a short drama", "turn this script into a short drama", "ショートドラマを生成", "숏드라마를 만들어". Orchestrates script, storyboard, optional character images, per-shot video clips, and ffmpeg composition through the existing media tools.
allowed-tools:
  - Bash
  - Read
  - Write
user-invocable: true
disable-model-invocation: false
---

You orchestrate a short drama by driving the existing media tools yourself. There is
no single short-drama tool. allowed-tools is guidance; media generation is enforced by
the internal tool permission path.

Trigger only on a request to CREATE/generate a short drama. Requests to analyze,
rewrite, summarize, or brainstorm a script do NOT trigger this skill unless the user
asks to produce the drama. Progress-only or promise-only replies are not completion:
after starting, either drive the pipeline or report the concrete blocker plainly.

## Pipeline (run in order; skip any stage whose inputs the prompt already supplies)

Pick a short kebab slug `<id>` from the drama title. Put project files under
`.puffer/media/drama/<id>/`. Generated image/video artifacts are written by the tools
to `.puffer/media/images|videos/` — you only reference them, never relocate them.

1. **Script.** If the prompt already contains a script (or names a script file), use it.
   Otherwise write one yourself and save it to `.puffer/media/drama/<id>/script.md`.

2. **Storyboard.** If the prompt already contains a shot breakdown, use it. Otherwise
   break the script into ordered shots (aim for a handful; one beat per shot). Give each
   shot a stable lowercase id (`shot-001`, `shot-002`, …) and record: subject, action,
   scene, lighting, camera, style, target duration (seconds), which characters appear,
   and any stability constraints. These fields become the video prompt — richer shots
   yield better clips. Save to `.puffer/media/drama/<id>/storyboard.md`.

3. **Character images (reference for video).** Scan the prompt for image references that
   are `https://` or `asset://` URLs.
   - If present, use those URLs directly as `--image-reference` in stage 4. Do NOT
     generate images.
   - If absent, run text-to-video in stage 4 (do not generate character images — a
     locally generated image cannot currently be passed to the video tool).

4. **Per-shot video.** For each shot in storyboard order, run one `videogen` command:
   - `videogen --prompt "<shot visual + action>"`
   - Add `--image-reference <url>` once per `https://`/`asset://` reference the shot uses;
     keep order stable and refer to them as image 1, image 2, … in the prompt.
   - Set an explicit long Bash timeout within the current Bash cap before running.
   - Record each result's video artifact path into the manifest (see below).

5. **Compose.** Stitch the successful shot clips in storyboard order with ffmpeg. First
   probe ffmpeg: `command -v ffmpeg`. If missing, stop and report — do not fake a file.
   Include only shots whose video succeeded; if none succeeded, skip composition and
   report. Build the concat list with single-quote escaping (each clip line is
   `file '<path>'`, with any `'` in the path written as `'\''`). Prefer stream-copy
   (clips from the same provider share codec/params); only if concat-copy fails with a
   codec/params mismatch, retry with a re-encode:

   ```bash
   : > .puffer/media/drama/<id>/concat.txt
   # append one line per SUCCEEDED clip, in order (escape single quotes):
   printf "file '%s'\n" "<clip path, ' -> '\\''>" >> .puffer/media/drama/<id>/concat.txt
   # primary: fast, no re-encode
   ffmpeg -f concat -safe 0 -i .puffer/media/drama/<id>/concat.txt \
     -c copy .puffer/media/drama/<id>/final.mp4
   # fallback only if the copy fails on mismatched streams:
   ffmpeg -f concat -safe 0 -i .puffer/media/drama/<id>/concat.txt \
     -c:v libx264 -pix_fmt yuv420p .puffer/media/drama/<id>/final.mp4
   ```

   If some shots failed but others composed, report it as a partial drama and list the
   missing shot ids.

## Manifest (your working ledger — keep it simple)

Maintain `.puffer/media/drama/<id>/manifest.json` as you go. It is a plain ordered list,
not a schema'd artifact:

```json
{
  "id": "<id>",
  "shots": [
    { "shotId": "shot-001", "status": "succeeded", "prompt": "...", "imageReferences": ["https://..."], "videoArtifactId": "...", "videoPath": ".puffer/media/videos/<aid>/..." }
  ],
  "final": ".puffer/media/drama/<id>/final.mp4"
}
```

## Failure contracts (never paper over)

- If a chosen video provider is Relaydance (prompt-only) and the user wants image
  references, report that the configured provider does not support image references.
- If ffmpeg is unavailable or composition fails, report it plainly and keep the
  per-shot clips; do not claim a composed drama was produced.
- Report final-video success only when `final.mp4` actually exists; a missing final
  video can still leave useful per-shot clips — say so rather than implying success.
- Do not hand-author placeholder media (SVG, stills, stub mp4) and present it as
  generated output.
```

- [ ] **Step 2: 构建确认 skill 被嵌入加载**

Run: `cargo build -p puffer-resources 2>&1 | tail -5`
Expected: 编译通过（`include_dir!` 把新目录纳入）。

- [ ] **Step 3: 断言 skill 可解析（若有现成测试入口）**

Run: `cargo test -p puffer-resources skill 2>&1 | tail -20`
Expected: 既有 skill 加载测试通过；若存在"枚举所有 skill"类测试，short-drama 出现在结果中。若无相关测试，跳过此步并在 Step 4 用手动验收替代。

- [ ] **Step 4: 提交**

```bash
git add -f resources/skills/short-drama/SKILL.md
git commit -m "feat(skill): add short-drama orchestration skill (increment 1, no backend change)"
```

### Task 3: 增量 1 手动验收

**Files:** 无（运行期验证）

- [ ] **Step 1: 文生视频路径**

在一个测试 workspace 让 agent 触发 short-drama，prompt 不带任何图 URL（例："做一个 15 秒
两镜头的搞笑短剧"）。
Expected: 生成 `script.md`/`storyboard.md`/`manifest.json`，逐镜 `videogen` 文生视频，
ffmpeg 合成出 `.puffer/media/drama/<id>/final.mp4`。

- [ ] **Step 2: prompt 自带图 URL 路径**

prompt 里给 1 个公开 `https://` 人物图 URL + 要求两镜头。
Expected: skill **不调 imagegen**，把该 URL 作 `--image-reference` 传给 videogen（前提是
选中的视频 provider 支持图参考，如 BytePlus）。

- [ ] **Step 3: ffmpeg 缺失契约**

临时让 PATH 不含 ffmpeg（或在无 ffmpeg 环境）跑一次。
Expected: agent 明确报"ffmpeg 不可用"，保留分镜片段，不产出假的 final.mp4。

---

## 增量 2 — 打通"生成人物图 → 图生视频"（方案 B 的桥）

### Task 4: 前置 spike（go/no-go 闸门）

**Files:** 无（一次性验证，不提交代码）

- [ ] **Step 1: 生一张图并取远程 URL**

Run: `imagegen --prompt "a young woman, studio portrait, plain background" --count 1`
然后读该 artifact 的 sidecar 取 `remoteSourceUrl`：
Run: `cat .puffer/media/artifact-sidecars/*.json | grep -o '"remoteSourceUrl":[^,}]*' | tail -1`
Expected: 拿到一个 `https://` URL（MiniMax 应有；若为空/null → 见 Step 3）。

- [ ] **Step 2: 用该 URL 直接图生视频**

Run: `videogen --prompt "she smiles and waves" --image-reference "<上一步的 https URL>"`
（确保选中支持图参考的视频 provider，如 BytePlus。）
Expected: 成功产出视频 artifact → **桥成立，继续 Task 5**。

- [ ] **Step 3: go/no-go 判定**

- URL 可被消费、视频生成成功 → GO，进入 Task 5。
- URL 为空、已过期、或跨 provider 拉取失败 → **NO-GO：停止，向用户如实汇报 spike 结论**，
  不引入上传/暂存 tool（方案 C 不在本计划内）。后续 Task 5/6 不执行。

### Task 5: 透出 `referenceUrl`（TDD）

**Files:**
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`（struct ≈52、构建处 ≈106-112、输出 ≈160-178）
- Test: 同文件 `#[cfg(test)]` 内（既有图生成测试旁，约 line 830-890）

- [ ] **Step 1: 写失败测试**

在该文件测试模块里，找到已断言 artifact 输出字段的测试（含 `artifact["artifactId"]`
的那个，约 line 877-887），在其断言区追加：

```rust
        // referenceUrl is surfaced from the runtime artifact's remote_source_url.
        assert!(
            artifact.get("referenceUrl").is_some(),
            "image artifact output must include a referenceUrl key (null allowed)"
        );
```

若该测试用的 mock provider 会返回远程 URL，则进一步断言其为字符串：
```rust
        assert!(artifact["referenceUrl"].is_string() || artifact["referenceUrl"].is_null());
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p puffer-core image_generation 2>&1 | tail -20`
Expected: FAIL —— `referenceUrl` 键不存在（当前输出无此字段）。

- [ ] **Step 3: 实现三处改动**

(a) `ImageGenerationArtifactResult` struct（≈line 52-58）加字段：

```rust
struct ImageGenerationArtifactResult {
    artifact_id: String,
    index: usize,
    path: Option<String>,
    mime_type: String,
    byte_count: u64,
    remote_source_url: Option<String>,
}
```
（`path` 的实际类型以文件现状为准，仅新增最后一行 `remote_source_url`。）

(b) 构建处（≈line 106-112）透传：

```rust
        .map(|artifact| ImageGenerationArtifactResult {
            artifact_id: artifact.artifact_id,
            index: artifact.index,
            path: artifact.path,
            mime_type: artifact.mime_type,
            byte_count: artifact.byte_count,
            remote_source_url: artifact.remote_source_url,
        })
```

(c) 输出 `image_generation_output`（≈line 164-170）加键：

```rust
        "artifacts": result.artifacts.iter().map(|artifact| json!({
            "artifactId": artifact.artifact_id,
            "index": artifact.index,
            "path": artifact.path,
            "referenceUrl": artifact.remote_source_url,
            "mimeType": artifact.mime_type,
            "size": artifact.byte_count
        })).collect::<Vec<_>>(),
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test -p puffer-core image_generation 2>&1 | tail -20`
Expected: PASS。

- [ ] **Step 5: 全包构建 + clippy**

Run: `cargo build -p puffer-core 2>&1 | tail -5 && cargo clippy -p puffer-core 2>&1 | tail -5`
Expected: 无 error。

- [ ] **Step 6: 提交**

```bash
git add crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs
git commit -m "feat(media): surface generated image referenceUrl for image-to-video"
```

### Task 6: skill 增加"生成人物图 → 图生视频"分支

**Files:**
- Modify: `resources/skills/short-drama/SKILL.md`（阶段 3）

- [ ] **Step 1: 改写阶段 3 的"图缺失"分支**

把阶段 3 中"If absent, run text-to-video"替换为：

```markdown
   - If absent and the user wants character-consistent shots, generate each character
     once with `imagegen --prompt "<character sheet>" --count 1`. Read the tool result's
     `referenceUrl` for that artifact:
       - If `referenceUrl` is a URL, use it as `--image-reference` in stage 4.
       - If `referenceUrl` is null, stop and report that the configured image provider
         does not produce a referenceable URL, so image-to-video is unavailable. Do NOT
         silently fall back to text-to-video.
   - If absent and consistency is not required, run text-to-video in stage 4.
```

- [ ] **Step 2: 提交**

```bash
git add -f resources/skills/short-drama/SKILL.md
git commit -m "feat(skill): short-drama generates character images and feeds referenceUrl to video"
```

- [ ] **Step 3: 端到端验收**

让 agent 跑一个"无图 + 要求人物一致"的短剧。
Expected: imagegen 生人物图 → 取 `referenceUrl` → videogen 图生视频 → ffmpeg 合成；
若 provider 不回 URL，按失败契约明确报错而非降级。

---

## Self-Review 结论

- **Spec 覆盖**：§3 五阶段门控 → Task 2/6；§4 三行桥 → Task 5；§5 失败契约 → Task 2(skill body)+Task 6；§6 两增量 → 增量 1（Task 1-3）/增量 2（Task 4-6）；§7 受影响文件 → File Structure；§8 开放问题（ffmpeg 探测/skill 发现/spike）→ Task 1、Task 2 Step1、Task 4。无遗漏。
- **无占位符**：SKILL.md、测试、三处改动均给出完整内容/命令/期望输出。
- **类型一致**：`referenceUrl`（输出 JSON 键）/`remote_source_url`（Rust 字段）全程一致；runtime 源字段 `ExactGeneratedArtifact.remote_source_url`（`runtime.rs:94`）与构建处透传名一致。
