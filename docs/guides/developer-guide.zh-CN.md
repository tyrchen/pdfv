# pdfv 开发者指南

[English](developer-guide.md)

状态：对应 Phase 18。

这份指南写给要改代码的人，覆盖本地环境、仓库结构、生成任务、质量门禁，以及提交前需要注意的工程约束。

## 仓库结构

```text
apps/cli/        CLI 入口，负责把命令行参数转成 pdfv-core 调用
crates/core/     解析、模型、验证、修复、报告，以及公开库 API
docs/            用户文档和研究记录
docs/guides/     用户指南和开发者指南
specs/           产品、设计、安全、测试和实现计划
tests/fixtures/  共享 PDF 测试样本
vendors/         vendored veraPDF 参考实现
fuzz/            fuzzing 工作区
```

请把 CLI 保持得足够薄。解析 PDF、执行规则、生成报告、提取特征、跑策略、做修复，这些能力都应该放在 `pdfv-core` 里。

## 工具链

仓库用 `rust-toolchain.toml` 固定 Rust 版本。构建和测试走固定的 stable 工具链；格式化用 nightly：

```bash
rustup toolchain install stable
rustup toolchain install nightly
cargo --version
cargo +nightly fmt --version
```

还需要安装两个检查工具：

```bash
cargo install cargo-audit
cargo install cargo-deny
```

## 构建和测试

完整构建整个 workspace：

```bash
cargo build --workspace --all-targets
```

运行所有测试，并顺带确认 benchmark harness 能编过：

```bash
cargo test --workspace --all-targets
```

只跑某个 CLI 集成测试：

```bash
cargo test -p pdfv --test validate test_should_validate_pdf_and_emit_text_report
```

只跑某个 core 测试：

```bash
cargo test -p pdfv-core parser::tests::test_should_parse_header_and_catalog_from_m0_fixture
```

## 必跑检查

交付前请跑完这一组命令：

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

`cargo deny check` 目前会打印一些已知警告，比如允许列表里暂时没遇到的许可证，以及 lockfile 里重复的 `wit-bindgen` 版本。只要命令成功退出即可；不要为了消掉这些已知警告顺手改依赖树。

## Makefile

已有目标能用就直接用：

```bash
make build
make test
make generate-profiles
make check-agent-sync
```

如果要加新的自动化，请放进 Makefile。这样别人能发现，也能复用。

## 生成 Profile

内置 profile 的生成结果在 `crates/core/src/generated_profiles.rs`。

更新 vendored profile 输入后，重新生成：

```bash
make generate-profiles
```

然后至少跑这两个测试：

```bash
cargo test -p pdfv-core profile::tests::test_should_load_and_validate_every_generated_builtin_profile
cargo test -p pdfv --test validate test_should_validate_with_every_phase_13_builtin_profile
```

## 写文档

用户文档放在 `docs/`，指南放在 `docs/guides/`。新增或改名时记得：

- 更新 `docs/index.md`；
- README 和指南级文档要同时维护中文版本；
- 命令示例要和 `pdfv validate --help` 以及集成测试保持一致；
- 设计背景尽量链接到 specs/research，不要在指南里大段重复。

## 代码要求

项目的硬性规则在 `AGENTS.md`。日常最容易踩到的是这些：

- 不写 `unsafe`；
- 生产代码不用 `unwrap()` 和 `expect()`；
- 不留下 `todo!()`、`unimplemented!()` 或半成品占位；
- 公开 item 要写文档；
- 库里用结构化错误，CLI 里用带上下文的 `anyhow`；
- 不可信输入一进边界就验证，并受资源限制约束；
- 生成数据、profile 行为和报告输出都要可复现。

## 改 CLI 时

CLI 只负责适配，不负责核心逻辑。改 CLI 行为时请注意：

- 解析和验证逻辑仍放在 `pdfv-core`；
- 补上或更新 `apps/cli/tests/validate.rs`；
- 退出码行为要稳定，并在文档中说清楚；
- 不添加字面量密码参数；
- 不支持或有歧义的参数组合，要在 clap/config 边界直接拒绝。

## 改报告时

报告写入器是库 API，改动要保持这些约束：

- JSON 字段使用 camelCase，serde 形状要稳定；
- XML、Raw、HTML 都必须正确转义文本和属性；
- 报告里不能出现密码、密钥、解密后的字节或原始 PDF 内容；
- 新增输出内容时，要补 golden 风格测试。

## 加依赖时

加依赖前先想清楚是否真的需要。确实要加时：

- 优先放到 workspace dependencies；
- 能用维护良好的纯 Rust crate，就不要引入 FFI；
- 按项目现有策略固定版本；
- 跑 `cargo audit` 和 `cargo deny check`；
- 如果依赖影响用户可见行为或安全边界，同步更新文档和 spec。

## 提交

提交要聚焦。按阶段推进的工作，提交信息里写清 phase，并说明对应的 spec 和退出条件。小范围文档或 bug 修复，标题直接说明改了什么，并在说明里列出跑过的验证命令。
