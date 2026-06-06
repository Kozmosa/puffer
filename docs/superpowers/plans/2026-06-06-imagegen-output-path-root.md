# ImageGeneration 输出路径统一到 Image folder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or equivalent task-by-task execution. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `ImageGeneration` 不管是否传入 `outputPath`,最终用户可见图片都写到
`<session cwd>/.puffer/workflows/images` 内。显式 `outputPath` 只表示该目录内部的
相对文件名或子路径。

**Architecture:** 纯运行时路径解析收敛。只改 `ImageGeneration` 工具的输出路径解析和工具
schema 文案。不新增配置字段、不新增 RPC、不改桌面端 open folder 命令、不加跨 crate 路径
服务。路径安全仍靠现有 safe relative path 校验,但 join base 改为 Image folder root。

**Tech Stack:** Rust + YAML resources.

**Spec:** `docs/superpowers/specs/2026-06-06-imagegen-output-path-root-design.md`

---

### Task 1: TDD 覆盖 Image folder rooted outputPath 语义

**Files:**
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`

- [ ] **Step 1: 定位现有路径测试**

Run:

```bash
rg -n "resolve_output_path|default_output_path|builds_request_with_prompt_file_and_output|rejects_unsafe_output_paths|execute_uses_descriptor_adapter" crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs crates/puffer-core/runtime/claude_tools/workflow/image_generation/tests
```

确认现有测试中哪些断言仍以 workspace root 为 `outputPath` base。

- [ ] **Step 2: 先写失败测试**

在 `image_generation.rs` 测试模块中更新/新增路径断言:

- `resolve_output_path(dir.path(), None, "webp")` 必须位于
  `dir/.puffer/workflows/images/`,文件名以 `generated-` 开头且扩展名为 `webp`。
- `resolve_output_path(dir.path(), Some("cup.png"), "png")` 必须等于
  `dir/.puffer/workflows/images/cup.png`。
- `resolve_output_path(dir.path(), Some("drafts/cup.png"), "png")` 必须等于
  `dir/.puffer/workflows/images/drafts/cup.png`。
- `resolve_output_path(dir.path(), Some(".puffer/workflows/images/cup.png"), "png")`
  必须等于 `dir/.puffer/workflows/images/.puffer/workflows/images/cup.png`
  (明确不做旧前缀特殊兼容)。
- 现有 `builds_request_with_prompt_file_and_output` 中 `out/image.png` 的期望改为
  `dir/.puffer/workflows/images/out/image.png`。
- 现有 descriptor adapter 执行测试若断言 `requested/ship.png`,同步改为
  `dir/.puffer/workflows/images/requested/ship.png`。

- [ ] **Step 3: 跑测试确认失败**

Run:

```bash
cargo test -p puffer-core image_generation
```

Expected: 至少显式 `outputPath` rooted 路径测试失败,因为当前实现仍是 `cwd.join(outputPath)`。

---

### Task 2: 最小实现路径 root 收敛

**Files:**
- Modify: `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`

- [ ] **Step 1: 添加本文件内常量**

在 `MAX_PROMPT_CHARS` 附近添加:

```rust
const IMAGE_OUTPUT_DIR_RELATIVE: &str = ".puffer/workflows/images";
```

不要新增跨 crate helper 或配置读取。

- [ ] **Step 2: 修改 `resolve_output_path`**

把当前逻辑从:

```rust
Ok(cwd.join(relative))
```

改为:

```rust
let image_root = cwd.join(IMAGE_OUTPUT_DIR_RELATIVE);
Ok(image_root.join(relative))
```

校验顺序保持为:先得到 explicit/default relative,再 `safe_relative_path(&relative)`,
再 join 到 image root。

- [ ] **Step 3: 修改默认文件名函数**

把默认函数从返回 `.puffer/workflows/images/generated-...` 改成只返回
`generated-<timestamp>.<ext>`。

可重命名为 `default_output_filename(output_format)` 以避免误读;同步调用点和测试。

- [ ] **Step 4: 跑 puffer-core 目标测试**

Run:

```bash
cargo test -p puffer-core image_generation
```

Expected: PASS。

---

### Task 3: 更新工具 schema 文案,减少模型误用

**Files:**
- Modify: `resources/tools/image_generation.yaml`

- [ ] **Step 1: 修改 `outputPath` 描述**

把当前:

```yaml
description: Workspace-relative image output path. Defaults to .puffer/workflows/images/.
```

改为明确语义:

```yaml
description: Optional image-folder-relative output filename or subpath. Relative to .puffer/workflows/images; omit to use generated-<timestamp>.<ext>. Do not pass absolute paths or workspace-root paths.
```

不要改 schema 结构、required 列表、权限策略或 handler。

- [ ] **Step 2: 验证资源仍可加载**

Run:

```bash
cargo test -p puffer-resources
```

Expected: PASS。

---

### Task 4: 终验与提交

**Files:**
- Modified files only:
  - `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`
  - `resources/tools/image_generation.yaml`

- [ ] **Step 1: 跑目标回归**

Run:

```bash
cargo test -p puffer-core image_generation
cargo test -p puffer-resources
git diff --check
```

Expected: 全部通过。

- [ ] **Step 2: 复查 diff 范围**

Run:

```bash
git diff -- crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs resources/tools/image_generation.yaml
git status --short
```

确认没有前端、Tauri opener、配置或无关文件改动。

- [ ] **Step 3: 提交**

```bash
git add crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs resources/tools/image_generation.yaml
git commit -m "fix(imagegen): root output paths in image folder"
```

