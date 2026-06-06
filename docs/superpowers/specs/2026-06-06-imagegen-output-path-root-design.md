# 生图输出路径统一到 Image folder — 设计

- 日期: 2026-06-06
- 状态: 已批准设计,待实现
- 作用域: `ImageGeneration` 工具的输出路径解析语义

## 背景

桌面端 Image Settings 已展示当前会话的图片输出目录:

```text
<session cwd>/.puffer/workflows/images
```

但 `ImageGeneration` 工具当前把 `outputPath` 解释为 workspace-relative path。
因此当模型传入 `outputPath: "ceramic-cup.png"` 时,实际输出会落到:

```text
<session cwd>/ceramic-cup.png
```

这与设置页展示的 Image folder 语义冲突。用户看到的是唯一输出目录,但工具允许
显式 `outputPath` 写出该目录。

## 目标

让 Image Settings 中显示的 Image folder 成为所有生图用户可见输出的唯一根目录。

`ImageGeneration.outputPath` 不再表示 workspace-relative path,而表示 Image folder
内部的相对文件名或子路径。

示例:

```text
无 outputPath
=> <session cwd>/.puffer/workflows/images/generated-<timestamp>.<ext>

outputPath: "cup.png"
=> <session cwd>/.puffer/workflows/images/cup.png

outputPath: "drafts/cup.png"
=> <session cwd>/.puffer/workflows/images/drafts/cup.png
```

## 非目标

- 不新增可配置 output directory。
- 不改设置结构、设置保存 RPC、桌面端 open folder 命令或 opener 插件接线。
- 不保持旧的 workspace-relative `outputPath` 行为。
- 不为图片输出引入新的跨模块路径服务或资源发现 API。
- 不做显式 `outputPath` 的扩展名推断、扩展名纠正或唯一文件名版本化。

## 设计

### 路径模型

在 `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs` 中把图片输出根目录
固定为:

```text
.puffer/workflows/images
```

`resolve_output_path(cwd, outputPath, outputFormat)` 的职责调整为:

1. 计算 `image_root = cwd.join(".puffer/workflows/images")`。
2. 如果 `outputPath` 为空,生成默认文件名 `generated-<timestamp>.<ext>`。
3. 如果 `outputPath` 非空,把它作为 Image folder 内部的相对文件名或子路径。
4. 返回 `image_root.join(relative_name_or_subpath)`。

默认文件名应只包含文件名,不要再包含 `.puffer/workflows/images/` 前缀,避免根目录重复拼接。

实现应保持局部、可读:

- 使用一个本文件内常量表示 `.puffer/workflows/images`。
- 可以把默认函数从 `default_output_name` 改成只返回文件名的
  `default_output_filename`。
- 不新增跨 crate helper,不新增配置查询,不让桌面端 UI 通过 RPC 读取这个路径。

### 显式 outputPath 语义

显式 `outputPath` 只决定 Image folder 内部的目标路径,不改变输出根目录。

- `cup.png` 写到 Image folder 根下。
- `drafts/cup.png` 写到 Image folder 子目录下。
- `.puffer/workflows/images/cup.png` 不做特殊剥离;它会被视为 Image folder 内部的普通子路径,
  即 `<cwd>/.puffer/workflows/images/.puffer/workflows/images/cup.png`。
- 如果文件名扩展名和实际图片编码不一致,本次不修正。当前工具只负责路径归属,不负责命名策略。
- 如果显式目标文件已存在,沿用现有 `fs::copy` 行为覆盖目标文件,不新增版本化或冲突处理。

### 安全规则

继续使用现有安全相对路径规则,但校验对象变为 Image folder 内部路径:

- 拒绝绝对路径,例如 `/tmp/cup.png` 或 `/Users/name/cup.png`。
- 拒绝父级逃逸,例如 `../cup.png`。
- 允许普通文件名,例如 `cup.png`。
- 允许子目录,例如 `drafts/cup.png`。
- 保持现有 `.` 路径组件处理即可,不新增复杂规范化逻辑。
- 不依赖 `canonicalize`,因为目标目录和子目录可能尚不存在。

这样即使模型显式传 `outputPath`,也无法写出 Image Settings 展示的目录。

### Tool schema 文案

更新 `resources/tools/image_generation.yaml` 中 `outputPath` 的描述。

当前文案强调 workspace-relative path,会诱导模型把文件写到会话根目录。新文案应说明:

- `outputPath` 是 Image folder 内部的相对文件名或子路径。
- 不要传绝对路径。
- 不要传 workspace 根目录路径。
- 省略时使用 `generated-<timestamp>.<ext>` 写入 Image folder。

描述只影响模型选择,真正约束仍由 Rust 路径解析保证。

## 数据流

```text
session cwd
  -> Image Settings 展示 <cwd>/.puffer/workflows/images
  -> ImageGeneration resolve_output_path
  -> <cwd>/.puffer/workflows/images/<output filename or subpath>
  -> create_dir_all(parent)
  -> copy generated artifact into final user-visible path
```

生成服务返回的 artifact 仍先进入 media artifact store;`ImageGeneration` 再复制到
Image folder 内的最终用户可见路径。该流程不变,只改变最终复制目标路径的 root。

## 错误处理

- `outputPath` 为空:使用默认文件名。
- `outputPath` 为绝对路径或包含父级逃逸:返回现有风格错误
  `ImageGeneration outputPath must be a safe relative path`。
- 目录不存在:沿用现有 `create_dir_all(parent)` 创建。
- 复制失败:沿用现有 `write image output <path>` 上下文错误。

## 测试

更新并补充 `image_generation.rs` 的路径解析测试:

- 无 `outputPath` 时输出到 `<cwd>/.puffer/workflows/images/generated-...`。
- `outputPath: "cup.png"` 输出到 `<cwd>/.puffer/workflows/images/cup.png`。
- `outputPath: "drafts/cup.png"` 输出到
  `<cwd>/.puffer/workflows/images/drafts/cup.png`。
- `outputPath: ".puffer/workflows/images/cup.png"` 不被特殊兼容或剥离,输出到
  `<cwd>/.puffer/workflows/images/.puffer/workflows/images/cup.png`。
- 绝对路径仍被拒绝。
- `../cup.png` 仍被拒绝。
- 更新现有 `out/image.png` 相关期望路径,确认它变为 Image folder 内部子路径。

无需新增端到端测试:该变化是纯 Rust 路径解析语义,单元测试能直接覆盖核心行为。
桌面端已有 Image Settings 路径展示测试,本次不扩大前端测试范围。

## 影响文件

- `crates/puffer-core/runtime/claude_tools/workflow/image_generation.rs`
- `resources/tools/image_generation.yaml`

不影响 `apps/puffer-desktop/src-tauri/src/lib.rs` 的 `open_image_dir`;该命令已经打开同一个
固定目录。

## 长期收益

- UI 展示目录与运行时写入目录一致。
- 模型仍可指定文件名,但不能绕过 Image folder。
- 路径安全边界更窄,默认行为更可预测。
- 不引入新配置、新 RPC 或新抽象,保持实现面小。
