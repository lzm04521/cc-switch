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
- **代理 API 报文记录**：本地代理可落盘记录转发报文用于排查（默认关闭，正文落盘前自动精简）。
- **供应商用量自动刷新**：全局开关（设置 → 用量统计，默认关闭），开启后非当前启用的供应商也定时查询余额用量；实际刷新间隔钳制为最快 5 分钟一次（供应商设置的刷新频率大于 5 分钟则按设置执行）。
- **云同步启动延迟备份**：WebDAV / S3 备份可配置启动后延迟 N 分钟执行。

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
