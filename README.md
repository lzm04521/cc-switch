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
| 版本号 | `<x.y.z>` | `<上游版本>-<N>`（如 `4.0.12-2`） |
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
- **会话级模型路由（聚合模式）**：构建在 v4 聚合（Stack）模式之上，多家供应商合成统一模型目录，模型 id 形如 `ccs-<分组key>.<模型名>`（前缀可配置）；会话内选用带某家 key 的条目即把该会话固定到那家供应商，带粘性绑定与 `ccs-default` 解绑，未知 key 报错不静默回落。Claude Code 与 Codex 两条链路可用，详见[下文用法说明](#会话级模型路由)。
- **本地代理 Body 重写规则**：供应商表单【本地代理请求覆盖】里除 Body 覆盖外，还可按「路径模式 + 值映射」配置重写规则，对请求体做条件动态改写（如 `{"messages[*].role": {"system": "user"}}` 把 `role:"system"` 逐条改为 `"user"`），用于应付严格校验型中转不收会话中 `system` 消息的场景（新版 Claude Code 的 `mid-conversation-system` beta）。按供应商显式配置、不全局默认启用；Claude、Codex、Grok Build 供应商表单均可配置。
- **聚合表单「设为默认模型」**：聚合（Stack）布局下编辑供应商时，模型列表行首 ★ 一键把该行设为这家供应商的默认模型（挪到列表首位；第一行即默认模型，会写成该卡的 `ANTHROPIC_MODEL`）。上游 v4.0.9 删除了该交互，本 Fork 恢复（Claude / Codex 两侧）。
- **供应商简称**：供应商可配置简称；聚合模型条目的显示名用简称括注供应商（如 `glm-5.3(ZhiPuPro)`），未设置时回退供应商全名。
- **云同步启动延迟备份**：WebDAV / S3 备份可配置启动后延迟 N 分钟执行。

## 会话级模型路由

自 v4.0.4-2 起，本 Fork 的会话级模型路由构建在 v4 的聚合（Stack）模式之上：把多家供应商加入聚合名单后，本地代理把各家的模型合成为一个统一目录，模型 id 形如 `<前缀><分组key>.<模型名>`（默认前缀 `ccs-`）。在会话里选用带某家 key 的条目，即可把**该会话**的请求固定路由到那家供应商（下称成员），实现同一个 Claude Code / Codex 内按会话使用不同供应商，互不影响默认成员。一句话心智模型：**key 只属于聚合名单里的成员；不带前缀就走默认成员。**

仅 **Claude Code 与 Codex** 链路支持聚合与前缀解析；Claude Desktop 链路已随聚合模式改造退役。旧版本的 `G.<key>:<模型>` 冒号寻址与 `G.<key>` 条目不再支持，升级后请在模型列表里重新选择。

### 前置配置

1. **进入聚合模式**：供应商页「直连 / 路由 / 聚合」三格中切到「聚合」，把参与的供应商加入聚合名单并确认生效。每家成员一个**分组 key**：
   - 仅字母、数字、横线（大小写保留），`.` 等其他符号自动转为横线，最长 24 位；保留字 `default`（解绑语义）不可用；
   - key 查重忽略大小写、撞已占用 key 会被拒绝；已删除供应商让出的 key 只挡自动分配，手动改名不受限（但复用后，仍带旧 id 的会话会转投这家）；
   - key 可在聚合名单里随时修改；改名后旧模型 id 失效，`/model` 里需重选。
2. **聚合模式设置**（设置 → 路由 →「聚合模式设置」面板）：
   - **聚合模型 id 前缀**：默认 `ccs-`。该值为完整触发串（前缀与分隔符整体填写，如 `ccs-`、`G.`），必须以非字母数字字符结尾，否则会把普通模型名误判为路由请求；留空保存即恢复默认。前缀与 key 匹配均大小写不敏感。
   - **聚合模型列表内容**：仅完整条目 / 仅分组条目 / 全部（默认），作用于 `/v1/models` 系端点与 Codex 模型目录。完整条目（每成员每模型一条）在前、分组短形式条目在后，另有 `ccs-default` 解绑条目。
   - **粘性会话**：默认开，行为见下文。
3. **默认成员**：聚合生效时的那家供应商即默认成员，不带前缀的请求都走它；聚合中照常「切换供应商」即可换默认成员，模型目录里默认成员的条目排在最前。

### 用法

Claude Code 通过 `--model`（或会话内 `/model`）选择条目，Codex 在模型选择器里选择（聚合条目同样发布进目录），语法为 `<前缀><key>`（短形式）与 `<前缀><key>.<模型>`（完整）：

| 模型值 | 行为 |
|---|---|
| `sonnet`（普通模型名，无前缀） | 走**默认成员**；会话已粘性绑定时跟随绑定成员 |
| `ccs-zhipu`（短形式） | 路由到 key 为 `zhipu` 的成员，使用该成员的默认模型（其模型列表第一行），并把会话绑定到该成员 |
| `ccs-zhipu.glm-5.3` | 路由到 `zhipu` 成员，显式指定模型；Claude 侧 1M 上下文窗口模型的 id 带 `[1M]` 后缀 |
| `ccs-default` | **解绑**当前会话，本请求与后续请求回落默认成员（大小写不敏感） |
| `ccs-notexist` | 400 报错（fail-closed）：提示 key 未登记或成员已移除，不静默回落 |

未开启聚合（直连 / 路由模式）时，带前缀条目的请求会被本地代理以明确错误拒绝（「聚合的模型只能在聚合模式下使用」）；此时切换到聚合模式即可。聚合名单在路由模式下保留但不发布条目，回到聚合模式即恢复。

### 会话粘性绑定

- 首个带 `<前缀><key>` 的请求把该会话（session）绑定到对应成员；此后同一会话内**不带前缀**的请求（subagent、classifier、后台 haiku 等自动发出的请求）自动跟随该成员，模型名照常走该成员的常规模型映射。
- 发起新的 `<前缀><key2>` 请求即可中途换绑；发 `ccs-default` 解绑回落默认成员；把成员移出名单或删除供应商，绑定自动失效（回落默认成员并记日志）。
- 绑定保存在内存中（不落盘），命中即续期（默认 1 小时无活动过期，容量上限 1000 会话），应用重启后全部解绑回落默认成员。
- Codex 侧粘性绑定依赖请求携带会话标识（`session_id` 头等）；无会话标识则不建立绑定，各请求按模型名独立解析。
- 路由命中的请求**锁定**目标成员：不偷换默认成员（界面「默认中」标记不变）、不参与故障转移、绕过熔断放行检查；token 用量计入锁定成员名下（使用统计页可见）。
- 粘性跟随可在 设置 → 路由 →「粘性会话」关闭：关闭后不带前缀的请求一律走默认成员；绑定与解绑动作本身不受开关影响，重开即恢复。

### 验证与排查

- 应用日志（`~/.cc-switch/logs/cc-switch.log`）搜 `[SessionRouting]`，可见「粘性跟随成员」「请求解绑，回落默认成员」「绑定的成员已失效，回落默认成员」等记录，可解释「为什么这个会话走了这家供应商」。
- 聚合名单与 key 登记情况（含已删除供应商让出的 key）记录在 `~/.cc-switch/live-state.json` 的 `stack` 段，遇到「key 被占用」类问题先查这里。

## 下载与安装

从本 Fork 的 [Releases](../../releases/latest) 页面下载：

| 平台 | 安装版 | 绿色版 |
|---|---|---|
| Windows x64 | `CC-Switch-v<版本>-Windows.exe`（NSIS）或 `CC-Switch-v<版本>-Windows.msi` | `CC-Switch-v<版本>-Windows-Portable.zip` |
| Windows ARM64 | `CC-Switch-v<版本>-Windows-arm64.msi` | `CC-Switch-v<版本>-Windows-arm64-Portable.zip` |

- 系统要求：Windows 10 及以上。
- x64 的 `.exe`（NSIS）与 `.msi` 均为安装版，二选一即可（`.exe` 自 v3.20.2-4 起提供）；ARM64 仅提供 `.msi` 安装版。
- 应用内自动更新统一通过 `.msi` 分发，持续指向本 Fork，可就地升级后续版本。
- 从上游正式版切换到本 Fork 需手动安装一次：版本号语义上 `-N` 后缀（如 `3.20.0-5`）低于同号正式版 `3.20.0`，不会被当作自动升级推送。

## 如何跟随上游

本 Fork 长期跟踪上游 tag，采用三层分支：

- `main`：跟随上游版本快照；README 等少量文件保留 Fork 定制（同步上游时由脚本自动恢复）。
- `local/main`：累积 Fork 长期定制（更新源、签名公钥、构建范围、README 等）。
- `local/v<大版本>.0-1`：发版分支，改版本号、打 tag、推送后由 [`release.yml`](.github/workflows/release.yml) 自动构建发布。同一大版本线共用一条分支连续 merge（当前 v4.x 线为 `local/v4.0.0-1`）。

仓库**默认分支始终指向最新大版本线的发版分支**（当前为 `local/v4.0.0-1`）；上游出 5.x 等新大版本时另起 `local/v5.0.0-1` 新线，并把默认分支一并切过去。

同步上游的一键脚本见 [`scripts/sync-upstream.sh`](scripts/sync-upstream.sh)。

## 致谢

本 Fork 基于原作者 [Jason Young](https://github.com/farion1231) 的开源工作，向上游项目致谢。

上游地址：<https://github.com/farion1231/cc-switch>

## License

MIT © Jason Young（沿用上游协议）
