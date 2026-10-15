# 本地代理「Body 重写规则」方案文档

- 日期：2026-10-14
- 状态：已确认（2026-10-14 经 curl 实测验证，实施中）
- 关联问题：新版 Claude Code（claude-cli/2.1.293）发送 `messages[]` 内 `role:"system"` 消息，commandcode 等严格校验型中转返回 400

## 1. 背景与问题

用户通过本地代理（接管模式）使用 commandcode 的 `claude-haiku-5-5`。日志
`~/.cc-switch/logs/api_logs/20261014_114903_732_3608c802.json` 显示：

- Claude Code 2.1.293 启用了 `mid-conversation-system-2026-04-07` beta（见 `anthropic-beta` 请求头），
  开始在 `messages[]` **数组内部**发送 `{"role":"system", ...}` 消息（本例为 `messages[1]`，
  内容带 `cache_control` 的 34047 字符系统上下文）。
- commandcode 的 Anthropic 兼容接口对 `messages[].role` 做枚举校验，直接拒绝：

  ```json
  {"type":"error","error":{"type":"invalid_request_error","message":"Invalid input at messages.1.role"}}
  ```

- 顶层 `system` 字段是 Anthropic 官方协议的一部分，已被正常接受；**问题只出在数组内的 system 角色**。

## 2. 根因分析

1. Anthropic 官方 Messages API 的 `messages[].role` 只允许 `user` / `assistant`；
   `mid-conversation-system` 是官方 beta 特性，官方端点支持，第三方中转多数尚未跟进。
2. 现有【本地代理请求覆盖 → Body 覆盖】是**静态 JSON 深合并**（`merge_json_override`）：
   对象递归合并、**数组整体替换**。`messages` 数组每轮请求的长度与内容都不同，
   静态 patch 无法表达“把每个 `messages[*].role == "system"` 改为 `"user"`”这类**条件、动态**的改写。
3. Header 覆盖无法解决：`anthropic-beta` 请求头只是客户端对上游的“能力声明”，
   消息体由客户端本地拼装，改头不影响 body 里的 `role:"system"`。

因此需要一个能作用于**动态数组内容**的改写机制，且必须**按 provider 显式配置**——
不能全局默认启用，否则会破坏官方 Anthropic 等支持该 beta 的上游。

## 3. 目标与非目标

### 目标

1. 在【本地代理请求覆盖】区块新增第三项「**Body 重写规则**」，支持按路径模式对
   数组/嵌套结构中的字符串值做**条件替换**，解决 `messages[*].role` 的 `system → user` 场景。
2. 语法通用：同类“枚举值不被中转接受”的未来问题（如 `thinking.type: "adaptive"`）
   填规则即可解决，无需改代码发版。
3. 与现有 Header/Body 覆盖一致的实现模式：TS 前置校验 + Rust warn-and-ignore 兜底；
   per-provider 存储；旧配置数据完全向后兼容。
4. 未配置规则的 provider 行为零变化（含官方端点）。

### 非目标

- 不做删除字段规则（列为 v2 候选，见 §9）。
- 不做完整 JSONPath（索引、`**`、过滤器等）。
- 不做响应侧（response body）改写。
- 不做“合并连续同角色消息”内置逻辑（列为后备方案，见 §8 风险 1）。
- 不修改 Claude Code 客户端行为。

## 4. 方案设计

### 4.1 规则语法与语义

「Body 重写规则」是一个 JSON 对象：**键 = 路径模式，值 = 值映射表**。

```json
{
  "messages[*].role": { "system": "user" }
}
```

- **路径模式**：以 `.` 分段；每段为字面对象键，或 `[*]` 表示“当前节点若是数组，
  则对每个元素继续匹配剩余路径”。支持嵌套通配，如 `messages[*].content[*].type`。
- **值映射表**：必须是对象，键/值均为字符串。语义：路径命中的每个**字符串**叶子，
  若其值等于映射表的某个键，则替换为对应的映射值；否则原样保留。
  非字符串叶子（数字、布尔、对象、数组）v1 不匹配、不动。
- 路径不存在的请求（如该 provider 的请求没有此字段）→ 静默跳过，属正常情况。
- **保护字段**：顶层路径恰为 `stream` 的规则被忽略并记 warn
  （与 Body 覆盖禁止 `stream` 的保护一致，避免破坏代理的流式响应处理）。
- 规则间相互独立（均为叶子级替换），应用顺序不影响结果。

### 4.2 数据模型与存储

存储位置不变：`provider.meta.localProxyRequestOverrides`，新增可选字段，无需数据迁移。

TS（`src/types.ts`）：

```ts
export interface LocalProxyRequestOverrides {
  headers?: Record<string, string>;
  body?: Record<string, unknown>;
  bodyRewrites?: Record<string, Record<string, string>>; // 新增
}
```

Rust（`src-tauri/src/provider.rs`）：

```rust
pub struct LocalProxyRequestOverrides {
    // ... 现有 headers / body ...
    #[serde(default, rename = "bodyRewrites", skip_serializing_if = "Option::is_none")]
    pub body_rewrites: Option<serde_json::Value>,
}
```

- `#[serde(default)]` 保证旧数据（无该字段）反序列化为 `None`；
  `skip_serializing_if` 保证未配置时 meta 不多出字段。
- 序列化为 camelCase（与现有 `localProxyRequestOverrides` 一致）。

### 4.3 生效位置（代理转发管线）

在 `forwarder.rs` 的 `forward_with_retry_inner` 中，紧跟现有静态 Body 覆盖之后应用：

```rust
let mut filtered_body = prepare_upstream_request_body(request_body);
if !is_copilot {
    if let Some(overrides) = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.local_proxy_request_overrides.as_ref())
    {
        if apply_local_proxy_body_overrides(&mut filtered_body, overrides) {
            filtered_body = prepare_upstream_request_body(filtered_body);
        }
        if apply_local_proxy_body_rewrites(&mut filtered_body, overrides) {   // 新增
            filtered_body = prepare_upstream_request_body(filtered_body);
        }
    }
}
```

要点：

- 作用于**协议转换后的最终上游 body**（与静态覆盖同一时点），用户按目标上游格式写规则。
- 每次转发尝试（含重试/failover）都从原始 `request_body` 重新构造，规则自然重复生效；
  failover 到别的 provider 时读取的是**该 provider 自己**的配置，符合 per-provider 语义。
- 改写后重跑 `prepare_upstream_request_body`（过滤 `_` 前缀私有字段 + key 排序规范化），
  与现有静态覆盖的行为保持一致。
- 与静态覆盖一样，copilot provider 跳过（沿用现有 `is_copilot` 守卫）。
- 命中时记 `log::debug!("[LocalProxyOverrides] body rewrites applied, hits: N")`，
  **不打印任何请求内容**（prompt 属敏感数据，与现有日志脱敏口径一致）。

新增函数（同文件，模式对齐 `apply_local_proxy_body_overrides`）：

```rust
fn apply_local_proxy_body_rewrites(body: &mut Value, overrides: &LocalProxyRequestOverrides) -> bool
```

内部按规则遍历：`rewrite_path(node, segments, mapping)` 递归——对象键下钻、
数组遇 `[*]` 逐元素下钻、走到叶子且为字符串且命中映射表则替换。

### 4.4 UI（供应商表单）

`LocalProxyRequestOverridesField.tsx` 在「Header 覆盖 / Body 覆盖」旁新增第三个
Textarea「Body 重写规则」：

- placeholder：

  ```json
  {
    "messages[*].role": { "system": "user" }
  }
  ```

- props 新增 `rewritesJson` / `onRewritesJsonChange`；
  `ProviderForm.tsx` 新增对应 state（从 `meta.localProxyRequestOverrides.bodyRewrites` 初始化），
  提交时并入 `buildLocalProxyRequestOverrides` 的产物；
  `ClaudeFormFields.tsx`、`CodexFormFields.tsx` 透传 props。
- 注意 `GrokBuildProviderForm.tsx` 是独立表单：持有自己的覆盖 JSON state，
  且是 `buildLocalProxyRequestOverrides` 的另一个调用点（签名由 2 参变 3 参时需同步），
  需一并新增 state、调用与 props，而非仅透传。
- 布局在实现时定（如第三项占整行），不作为本方案的约束。

### 4.5 校验策略

TS（`src/lib/requestOverrides.ts`，新增 `parseBodyRewritesJson`）：

- 必须是 JSON 对象；键（路径）非空、段语法合法（非空段，`[*]` 为唯一通配形式）；
- 顶层路径 `stream` 直接报错（与 Body 覆盖一致）；
- 每个规则的值必须是对象，且映射表键/值均为字符串，否则报错；
- `buildLocalProxyRequestOverrides(headersJson, bodyJson, rewritesJson)` 组装 `bodyRewrites`。

Rust 兜底（运行时，与静态覆盖同模式）：结构不符的规则记 `log::warn!` 并忽略，
不拒绝请求——覆盖配置错误不应导致代理不可用。

### 4.6 i18n

`src/i18n/locales/{zh, zh-TW, ja, en}.json` 各新增：

- `providerForm.localProxyBodyRewrites`（标签，如「Body 重写规则」/ "Body rewrite rules"）
- `providerForm.localProxyBodyRewritesHint`（一句话说明：按路径匹配条件替换字符串值）
- `providerForm.localProxyBodyRewritesInvalidDetail`（"Body 重写规则格式错误：{{error}}"）

## 5. 用户配置示例（commandcode）

编辑 commandcode 供应商 → 高级/本地代理请求覆盖 → Body 重写规则：

```json
{
  "messages[*].role": { "system": "user" }
}
```

若后续 commandcode 对其它枚举值报同类错误（如 `thinking.type: "adaptive"` 不被支持），
追加一条规则即可，例如：

```json
{
  "messages[*].role": { "system": "user" },
  "thinking.type": { "adaptive": "enabled" }
}
```

## 6. 边界情况

| 场景 | 行为 |
| --- | --- |
| 路径在请求中不存在 | 静默跳过 |
| 叶子值不等于任何映射键（如 `role:"assistant"`） | 不动 |
| 叶子非字符串（数字/布尔/对象/数组） | 不动（v1） |
| 规则值不是对象 / 映射含非字符串 | TS 校验拦截；Rust warn + 忽略 |
| 顶层 `stream` 规则 | 拒绝/忽略，warn |
| copilot provider | 整套覆盖跳过（现状不变） |
| 未配置任何规则的 provider | 零额外开销，零行为变化 |
| failover 切换 provider | 使用目标 provider 自己的规则 |

## 7. 测试计划

Rust（`src-tauri/src/proxy/forwarder.rs` 测试模块）：

- 用日志中的真实形状构造 body（`user` + `system` 两条消息），断言规则应用后两条均为
  `user`、其余字段（content、cache_control 等）不动；
- 嵌套通配 `messages[*].content[*].type`；
- 不匹配值保留、非字符串叶子不动、路径不存在跳过；
- 顶层 `stream` 规则被忽略；
- 非法规则 warn + 忽略；
- `apply_local_proxy_body_rewrites` 返回值正确反映是否发生改写。

Rust（`src-tauri/src/provider.rs`）：

- `provider_meta_roundtrips_local_proxy_request_overrides` 扩展 `body_rewrites`
  的序列化/反序列化与未配置时的字段省略。

TS（`tests/lib/requestOverrides.test.ts`）：

- `parseBodyRewritesJson` 合法/非法用例；
- `buildLocalProxyRequestOverrides` 正确组装 `bodyRewrites`。

手工验收：

- commandcode 供应商配置 §5 规则后，Claude Code 2.1.293 + haiku 请求返回 200 且正常流式输出；
- api_logs 中转发 body 确认 `messages` 内不再出现 `role:"system"`。

## 8. 风险与后备方案

1. **连续同角色消息**：本例改写后 `messages` 将出现相邻两条 `user`。
   - **已实测通过（2026-10-14，curl 直连 commandcode `/provider/v1/messages`，`claude-haiku-5-5`）**：
     - 仅含 `role:"system"` 的请求 → 400 `Invalid input at messages.1.role`（稳定复现）；
     - 相邻两条 `user` → 正常回复；
     - 相邻两条 `user` 且第二条带消息级 `output_config` 与块级 `cache_control` → 正常回复。
   - 结论：v1 无需“合并连续同角色消息”，也无需删除消息级 `output_config`；
     该后备路径已被实测排除，仅在上游行为变化时再启用。
2. **路径写错静默不生效**：规则是声明式匹配，打错路径无报错。缓解：debug 日志输出
   命中数；文档与 placeholder 给出示例。
3. **commandcode 后续拒绝其它新参数**：本例 body 还含 `output_config`、
   `context_management`、`thinking.type: "adaptive"` 等新字段，当前仅 `role` 被拒。
   旁证：本次报错路径为 `messages.1.role`（该消息对象最后一个键），而更早序列化的
   顶层 `context_management`、消息内 `output_config` 均未报错，说明其 schema 对多余键
   是透传容忍的，`role` 是唯一被枚举校验拦截的字段——改写后通过概率较高。
   枚举类问题可用新规则覆盖；若被拒的是“字段本身”而非“取值”，需要删除规则（§9）。
4. **性能**：仅在配置了规则的 provider 上生效，每请求一次 O(body) 路径遍历，
   与现有静态覆盖同量级，可忽略。

稳健性说明：值映射改写是**幂等**的（`system → user` 重放结果不变），签名修复重试等
以已改写 body 再次进入转发管线的场景不会二次破坏；`forward_with_retry_inner` 的
`for provider` 循环内每次尝试均从原始 body 重新构造并读取**当前供应商自己的**规则，
failover 语义天然正确（均已对现码核实）。

## 9. 未来扩展（不阻塞本方案）

- **删除规则**：`{"$remove": ["messages[*].output_config"]}` 形式，用于中转拒绝整个字段的场景。
- 映射值放宽为任意 JSON 标量；数字/布尔叶子的映射匹配。
- 路径支持数字索引（`messages[0].role`）。

## 10. 改动文件清单

| 文件 | 改动 |
| --- | --- |
| `src-tauri/src/provider.rs` | `LocalProxyRequestOverrides` 增加 `body_rewrites` 字段 + roundtrip 测试 |
| `src-tauri/src/proxy/forwarder.rs` | `apply_local_proxy_body_rewrites` + 调用接线 + 单测 |
| `src/types.ts` | `LocalProxyRequestOverrides` 增加 `bodyRewrites` |
| `src/lib/requestOverrides.ts` | `parseBodyRewritesJson` + `buildLocalProxyRequestOverrides` 扩展 |
| `tests/lib/requestOverrides.test.ts` | 校验测试 |
| `src/components/providers/forms/LocalProxyRequestOverridesField.tsx` | 第三个 Textarea |
| `src/components/providers/forms/ProviderForm.tsx` | state + 提交接线 |
| `src/components/providers/forms/ClaudeFormFields.tsx` | props 透传 |
| `src/components/providers/forms/CodexFormFields.tsx` | props 透传 |
| `src/components/providers/forms/GrokBuildProviderForm.tsx` | 独立覆盖 state + `buildLocalProxyRequestOverrides` 调用点 + props（非仅透传） |
| `src/i18n/locales/{zh, zh-TW, ja, en}.json` | 新增 3 个 key |

## 11. 验收标准

1. commandcode 配置 §5 规则后，新版 Claude Code 请求不再报
   `Invalid input at messages.1.role`，可正常对话；
2. 未配置规则的 provider（含官方 Anthropic、Copilot）行为零变化；
3. 旧配置数据（meta 无 `bodyRewrites`）加载、保存无损；
4. Rust 单测、前端 vitest 全部通过。
