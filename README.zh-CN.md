# pdfv

[English](README.md) | 简体中文

`pdfv` 是一个用 Rust 写的 PDF 合规性验证器。项目先把核心能力做成库，再在上层提供命令行工具，适合本地检查、仓库扫描和 CI 流水线。

现在的 `pdfv` 可以用有边界的解析器和规则引擎验证 PDF/A 相关 profile，也能加载从 veraPDF 导入的规则；报告输出稳定可复现，特征提取和策略检查都有明确的资源上限，面对不可信 PDF 时不会放任输入一路膨胀。

## 快速开始

在仓库根目录安装命令行工具：

```bash
cargo install --path apps/cli
```

验证一个文件：

```bash
pdfv validate tests/fixtures/minimal-valid.pdf --format text
```

递归验证目录，并把 JSON 报告写到文件：

```bash
pdfv validate ./documents --recursive --jobs 4 --format json --output report.json
```

查看内置 profile：

```bash
pdfv profiles list
```

把元数据修复结果写到单独目录：

```bash
pdfv repair-metadata input.pdf --output-dir repaired --prefix fixed- --format json
```

## 文档

- [用户指南](docs/guides/user-guide.zh-CN.md)
- [开发者指南](docs/guides/developer-guide.zh-CN.md)
- [JSON 和配置示例](docs/json-examples.md)
- [veraPDF CLI 兼容性](docs/verapdf-cli-compatibility.md)
- [文档索引](docs/index.md)

英文版：

- [User Guide](docs/guides/user-guide.md)
- [Developer Guide](docs/guides/developer-guide.md)

## 命令行能力

`pdfv validate` 常用参数如下：

- 报告格式：`json`、`json-pretty`、`text`、`xml`、`mrr`、`raw`、`html`。
- profile 选择：`--flavour`、`--default-flavour`、`--profile`。
- 批量处理：`--recursive`、`--non-pdf-extension`、`--jobs`。
- 报告控制：`--output`、`--redact-paths`、`--record-passes`、`--max-failures`。
- 密码来源：`--password-stdin`、`--password-file`、`--password-env`。
- 特征与策略：`--extract`、`--policy-file`。

退出码面向脚本和 CI 保持稳定：

| 代码 | 含义 |
| --- | --- |
| 0 | 所有处理过的文件都通过验证 |
| 1 | 验证完成，但至少有一个文件不合规 |
| 2 | 至少有一个文件无法解析 |
| 3 | 至少有一个加密文件无法验证 |
| 4 | 有必需规则尚未支持，本次验证不完整 |
| 64 | 命令行参数或配置文件无效 |
| 70 | 内部处理失败 |

## 配置

运行时默认值可以写在 YAML 里：

```yaml
validation:
  flavour: auto
  defaultFlavour: pdfa-1b
  recordPassedAssertions: false
resources:
  maxFileBytes: 268435456
  maxObjects: 1000000
  maxObjectDepth: 128
  maxArrayLen: 65536
  maxDictEntries: 16384
  maxNameBytes: 127
  maxStringBytes: 1048576
  maxStreamDeclaredBytes: 134217728
  maxStreamDecodeBytes: 268435456
  maxParseFacts: 100000
output:
  format: json
  path: report.json
  redactPaths: true
```

使用配置文件：

```bash
pdfv validate --config pdfv.yaml ./documents --recursive
```

## 作为库使用

```rust
use pdfv_core::{ReportFormat, Validator};

let validator = Validator::default();
let report = validator.validate_path("tests/fixtures/minimal-valid.pdf")?;
ReportFormat::JsonPretty.write_report(&report, std::io::stdout())?;
# Ok::<(), pdfv_core::PdfvError>(())
```

## 开发

提交前请跑完这些检查：

```bash
cargo build --workspace --all-targets
cargo test --workspace --all-targets
cargo +nightly fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic -W clippy::unwrap_used -W clippy::expect_used -W clippy::indexing_slicing -W clippy::panic
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo audit
cargo deny check
```

具体开发流程见[开发者指南](docs/guides/developer-guide.zh-CN.md)。

## 许可证

本项目使用 Mozilla Public License 2.0。

vendored veraPDF 的许可证说明见 [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md)。

完整许可证文本见 [LICENSE](LICENSE.md)。
