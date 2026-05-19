# pdfv 用户指南

[English](user-guide.md)

状态：对应 Phase 18。

这份指南面向日常使用：怎么验证 PDF、怎么选择报告格式、怎么做特征提取和策略检查、怎么处理带密码的文件，以及怎样安全地尝试元数据修复。

## 安装

在仓库根目录执行：

```bash
cargo install --path apps/cli
```

确认命令可用：

```bash
pdfv --version
```

## 验证文件

验证单个文件，输出一段便于阅读的摘要：

```bash
pdfv validate document.pdf --format text
```

一次验证多个文件，并把紧凑 JSON 写入文件：

```bash
pdfv validate a.pdf b.pdf --format json --output report.json
```

递归扫描目录，同时限制并行任务数：

```bash
pdfv validate ./documents --recursive --jobs 4 --format json --output report.json
```

递归扫描默认只收集 `.pdf` 文件。如果你的输入没有 PDF 扩展名，可以主动放开这个限制：

```bash
pdfv validate ./incoming --recursive --non-pdf-extension --format json
```

## 选择 Profile

让 `pdfv` 根据 XMP 元数据自动判断 flavour；如果判断不出来，就用 PDF/A-1b 兜底：

```bash
pdfv validate document.pdf --flavour auto --default-flavour pdfa-1b
```

也可以直接指定一个内置 flavour：

```bash
pdfv validate document.pdf --flavour pdfa-2b --format json
```

使用自定义 profile XML：

```bash
pdfv validate document.pdf --profile profile.xml --format json
```

查看内置 profile 以及规则覆盖情况：

```bash
pdfv profiles list
```

## veraPDF 就绪状态

`pdfv` 当前已经在 M9 G8 就绪快照中导入、lower 并绑定 vendored PDF/A、
PDF/UA 和 WTPDF profile 规则。第一个公开的 veraPDF-grade 声明仍然暂停，
直到私有 T3 真实世界语料门禁通过。

支持的 profile 范围、语料证据、已知 drift 和不在范围内的能力，见
[veraPDF Readiness](../verapdf-readiness.md)。

## 报告格式

通过 `--format` 选择输出格式：

| 格式 | 适合场景 |
| --- | --- |
| `text` | 本地查看，读起来最直接 |
| `json` | 脚本、CI、系统集成 |
| `json-pretty` | 人工审阅 JSON |
| `xml` | 需要 veraPDF 风格机器可读报告的场景 |
| `mrr` | `xml` 的兼容别名，已不推荐新脚本使用 |
| `raw` | processor 风格的原始 XML 报告 |
| `html` | 静态 HTML 报告，方便人工浏览 |

如果 CI 日志里不应暴露路径，可以加上 `--redact-paths`：

```bash
pdfv validate ./documents --recursive --format json --redact-paths
```

## 退出码

| 代码 | 含义 |
| --- | --- |
| 0 | 所有处理过的文件都通过验证 |
| 1 | 验证完成，但至少有一个文件不合规 |
| 2 | 至少有一个文件无法解析 |
| 3 | 至少有一个加密文件无法验证 |
| 4 | 有必需规则尚未支持，本次验证不完整 |
| 64 | 命令行参数或配置文件无效 |
| 70 | 内部处理失败 |

批量验证时，最终退出码会取最严重的结果。

## YAML 配置

命令行参数优先级高于 YAML 配置。常见配置如下：

```yaml
validation:
  flavour: auto
  defaultFlavour: pdfa-1b
  maxFailedAssertionsPerRule: 100
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

运行时指定配置文件：

```bash
pdfv validate --config pdfv.yaml ./documents --recursive
```

## 带密码的 PDF

`pdfv` 不支持把密码直接写成命令行参数。请从标准输入、文件或环境变量读取密码：

```bash
printf '%s' "$PDF_PASSWORD" | pdfv validate encrypted.pdf --password-stdin
```

```bash
pdfv validate encrypted.pdf --password-file password.txt
```

```bash
PDF_PASSWORD='secret' pdfv validate encrypted.pdf --password-env PDF_PASSWORD
```

密码不会写入报告，也不会出现在 `Debug` 输出里。

## 特征和策略报告

提取所有已支持的特征族：

```bash
pdfv validate document.pdf --extract --format json
```

只提取指定特征族：

```bash
pdfv validate document.pdf --extract catalog,page,font --format json
```

基于特征报告执行 YAML 策略：

```bash
pdfv validate document.pdf --policy-file policy.yaml --format xml
```

这里的策略文件使用 `pdfv` 自己的受限 YAML 格式。任意 Schematron 和 XSLT 执行不在当前范围内。

## 元数据修复

元数据修复走保守路线：不会原地改输入文件，所有输出都写到你指定的目录。

```bash
mkdir -p repaired
pdfv repair-metadata document.pdf --output-dir repaired --prefix fixed- --format json
```

当前行为可以概括为：

- 对有效且 flavour 唯一的输入，可能只复制文件，并返回 `noAction` 报告；
- 解析失败、加密输入、输出路径不安全或暂不支持的情况，都会给出结构化拒绝原因；
- 修复失败时会清理失败输出；如果目标文件已经存在，则不会覆盖它。

## 从 veraPDF 迁移

为了方便迁移旧脚本，`pdfv` 接受这些别名：

- `--defaultflavour` 等同于 `--default-flavour`
- `--recurse` 等同于 `--recursive`
- `--nonpdfext` 等同于 `--non-pdf-extension`

几个需要注意的差异：

- 没有 `--password <text>`，避免密码进入 shell 历史、进程列表或 CI 日志；
- 元数据修复使用单独的 `repair-metadata` 命令，不挂在 `validate` 下面；
- 策略检查使用受限 YAML，不执行任意 XSLT/Schematron；
- GUI、安装器、ZIP 输入、server mode、进度显示、报告内嵌日志暂不支持。
