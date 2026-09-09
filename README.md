<div align="center">

# CC Switch

### 适配国产 Agent 扩展的 Claude Code / Codex / Gemini 多 CLI 管家（Windows 桌面端）

本仓库是 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 的 Fork，专注适配国产 Agent 扩展使用。

[![Platform](https://img.shields.io/badge/platform-Windows%20only-lightgrey.svg)](../../releases)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

</div>

## 本 Fork 的定位

[farion1231/cc-switch](https://github.com/farion1231/cc-switch) 是一款统一管理 Claude Code、Claude Desktop、Codex、Gemini CLI、Grok Build、OpenCode、OpenClaw、Hermes 等 AI CLI 工具的桌面应用。本 Fork 在其基础上，面向**国产 Agent 扩展**（OpenClaw / Hermes / OpenCode / DSH 等）的配置管理与日常使用，裁剪为 Windows 单平台、自建更新源的自用版本。

## 与原仓库的差异

| 项 | 原仓库 | 本 Fork |
|---|---|---|
| 发布平台 | Windows / macOS / Linux | **仅 Windows**（x64 + arm64） |
| 自动更新源 | `dl.ccswitch.io` + 上游 Release | 本 Fork Release 单源 |
| 签名密钥 | 上游密钥 | 独立生成（与上游不通用） |
| 版本号 | `<x.y.z>` | `<上游版本>-<N>`（如 `3.20.0-5`） |
| 功能扩展 | 上游功能集 | 上游功能集 + Fork 增强（见下节） |

除上表差异与下节增强外，其余功能、完整文档与上游一致；多平台构建见原仓库 [farion1231/cc-switch](https://github.com/farion1231/cc-switch)。

## 本 Fork 增强的功能

- **DSH（DeepSeek Harness）接入**：skills 与 MCP 统一管理。MCP 条目同时写入 `<dsh home>/profiles/web` 与 `profiles/desktop` 两个 profile 的 `cordis.patch.yml`（DSH 客户端两种形态各用其一；DSH 原生风格序列化，含 `!!js` 扩展表达式时拒绝改写，单文件被拒不影响其余）；读取时按 serverName 去重合并（web 优先）。skills 部署目录可在设置页切换。
- **Zcode 接入**：skills / prompts / MCP / sessions 管理。
- **悬浮球快速切换**：贴边隐藏，显示 Provider 与今日 Token 用量，点击快速切换供应商（DSH / ZCode 供应商由应用内自管，不出现在悬浮球切换列表）。
- **悬浮球弹窗尺寸可调**：悬浮球弹窗面板宽高可在设置页调整（240–480 × 320–800 px，默认 300×480），保存后即时生效。
- **代理 API 报文记录**：本地代理可落盘记录转发报文用于排查（默认关闭，正文落盘前自动精简）。
- **供应商用量自动刷新**：全局开关（设置 → 用量统计，默认关闭），开启后非当前启用的供应商也定时查询余额用量；实际刷新间隔钳制为最快 5 分钟一次（供应商设置的刷新频率大于 5 分钟则按设置执行）。刷新由后端后台任务兜底，主窗口关闭或切换页面时照常执行，悬浮球面板实时同步最新余额与查询时间。
- **用量 t/s 输出速度**：为本地代理直录的流式请求统计输出速度（t/s）；请求日志、详情面板、汇总卡片、Provider 统计（按供应商加权平均）与悬浮球「今日输出速度」均有展示，无可计算数据的行显示 —。
- **会话级模型路由**：本地代理运行时，请求模型名带路由前缀（默认 `G.<key>`）即可把该会话路由到指定供应商；配套 `/v1/models` 模型列表开关，开启后在 `/model` 选择器里可直接选分组与模型，详见[下文用法说明](#会话级模型路由)。
- **云同步启动延迟备份**：WebDAV / S3 备份可配置启动后延迟 N 分钟执行。

## 会话级模型路由

本地路由（本地代理接管）开启后，通过在请求模型名前加「路由前缀 + 路由 key」（默认形如 `G.ds`），可以把**当前会话**的请求路由到指定供应商（下称分组），实现一个 Claude Code / Claude Desktop 内按会话使用不同分组，互不影响默认分组。一句话心智模型：**前缀只认 key，key 只属于勾了「加入会话级路由」的分组；不带前缀就走默认分组。**

仅 **Claude（Claude Code CLI）与 Claude Desktop** 的接管链路支持路由；其他 CLI（Codex / Gemini 等）即使开启本地路由、流量走代理，也不解析前缀。

### 前置配置

1. **开启本地路由**：主窗口顶部「Claude 本地路由」开关打开（或 设置 → 路由 →「本地路由」面板）。前缀由本地代理在代理层解析，**直连流量不解析前缀**——关闭本地路由后，`G.ds` 这样的模型名会原样发给上游（通常报「模型不存在」）。
2. **为供应商开启路由**：编辑供应商（Claude / Claude Desktop 表单均支持），勾选「加入会话级路由」并填写**路由 key**（如 `ds`、`fast`）：
   - key 仅允许字母、数字与 `. _ -`，长度 1–32 位，同一应用内唯一（大小写不敏感）；
   - 保留字 `default` 不可用（恒为解绑语义，见下文）；
   - 不勾选路由的分组永远不被前缀命中；默认分组想被前缀点名，同样需要勾选并设置 key；
   - 被点名的分组建议在其 env 中配置默认模型（`ANTHROPIC_MODEL`），否则 `G.<key>`（不带模型）形式的请求会报错。
3. **路由前缀（可选）**：设置 → 路由 →「会话级路由前缀」，默认 `G.`。该值为**完整触发串**（前缀与分隔符整体填写，如 `G.`、`@`），必须以非字母数字字符结尾，否则会把普通模型名误判为路由请求；留空保存即恢复默认。前缀与 key 匹配均大小写不敏感。
4. **`/v1/models` 模型列表（可选）**：设置 → 路由 →「/v1/models 模型列表」。开启后本地代理的 `/v1/models` 响应追加路由分组与映射模型清单，**条目 id 自包含，可整体复制进 `--model` 直接路由**（如 `G.DS`、`G.DS:deepseek-v4-pro[1M]`），返回类型可选「仅分组 / 模型 / 分组+模型」。存在已开路由的分组时，列表首条恒为回落条目 `G.Default`——在 `/model` 选择器里选中它即可解除当前会话的粘性绑定，无需手打前缀。Claude Desktop 侧 `/claude-desktop/v1/models` 同受该开关控制。

### 用法

在 Claude Code 中通过 `--model`（或会话内 `/model`）带上前缀即可触发，语法为 `<前缀><key>[:<模型>]`：

| `--model` 值 | 行为 |
|---|---|
| `sonnet`（无前缀） | 走**默认分组**（即原「当前供应商」），行为与未开路由时完全一致 |
| `G.ds` | 路由到 key 为 `ds` 的分组，使用该分组的默认模型（`ANTHROPIC_MODEL`；未配置则报错） |
| `G.ds:claude-opus-4-8` | 路由到 `ds` 分组，`claude-opus-4-8` 按该分组的档位映射解析（sonnet/opus/haiku 档位替换照常生效） |
| `G.ds:deepseek-reasoner` | 路由到 `ds` 分组，显式模型名未命中档位时**原样透传**，不落默认模型兜底 |
| `G.Default` | **解绑**当前会话，回落默认分组（手打小写 `G.default` 同样有效） |
| `G.notexist` | 报错（fail-closed）：提示当前可用 key 列表，不静默回落默认分组 |

### 会话粘性绑定

- 首个带 `G.<key>` 的请求会把该会话（session）绑定到对应分组；此后同一会话内**不带前缀**的请求（subagent、classifier、后台 haiku 等自动发出的请求）自动跟随该分组，无需再带前缀。跟随时模型名照常走该分组的常规映射（档位映射 → subagent 保护 → `ANTHROPIC_MODEL` 兜底），不做显式透传。
- 发起新的 `G.<key2>` 请求即可中途换分组（覆盖旧绑定）；发 `G.Default` 解绑回落默认分组；删除分组或关闭其路由开关，绑定自动失效。
- 绑定保存在内存中（不落盘），命中即续期（默认 1 小时无活动过期，容量上限 1000 会话），应用重启后全部解绑回落默认分组。
- 路由命中的请求**锁定**目标分组：不偷换默认分组（界面「默认中」标记不变）、不参与故障转移、绕过熔断放行检查；显式模型名（`G.key:model`）跳过该分组的模型映射兜底变体；token 用量计入锁定分组名下（使用统计页可见）。

### 验证与排查

- 应用日志（`~/.cc-switch/logs/cc-switch.log`）搜 `[RoutePrefix]`，可见「路由 key → 分组」「粘性跟随分组」「解绑回落默认分组」等记录，可解释「为什么这个会话走了这个分组」。
- 已知限制：默认模型只配在「通用配置片段」里时，路由运行时读不到，需直接配到该供应商 env；分组的 body override 若覆盖 `model` 字段，则赢过路由改写。

## 下载与安装

从本 Fork 的 [Releases](../../releases/latest) 页面下载：

| 平台 | 安装包 |
|---|---|
| Windows x64 | `CC-Switch-v<版本>-Windows.msi`（安装版）/ `CC-Switch-v<版本>-Windows-Portable.zip`（绿色版） |
| Windows ARM64 | `CC-Switch-v<版本>-Windows-arm64.msi`（安装版）/ `CC-Switch-v<版本>-Windows-arm64-Portable.zip`（绿色版） |

- 系统要求：Windows 10 及以上。
- 安装后，应用内自动更新持续指向本 Fork，可就地升级后续版本。
- 从上游正式版切换到本 Fork 需手动安装一次：版本号语义上 `-N` 后缀（如 `3.20.0-5`）低于同号正式版 `3.20.0`，不会被当作自动升级推送。

## 如何跟随上游

本 Fork 长期跟踪上游 tag，采用三层分支：

- `main`：跟随上游版本快照；README 等少量文件保留 Fork 定制（同步上游时由脚本自动恢复）。
- `local/main`：累积 Fork 长期定制（更新源、签名公钥、构建范围、README 等）。
- `local/v<上游版本>-<N>`：发版分支，改版本号、打 tag、推送后由 [`release.yml`](.github/workflows/release.yml) 自动构建发布。

同步上游的一键脚本见 [`scripts/sync-upstream.sh`](scripts/sync-upstream.sh)。

## 致谢

本 Fork 基于原作者 [Jason Young](https://github.com/farion1231) 的开源工作，向上游项目致谢。

上游地址：<https://github.com/farion1231/cc-switch>

## License

MIT © Jason Young（沿用上游协议）
