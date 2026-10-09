export type RoutePrefixValidation =
  | { ok: true }
  | {
      ok: false;
      reason: "empty" | "length" | "charset" | "colon" | "boundary";
    };

/** 分组 key 的最大长度（与后端 stack::KEY_MAX_LEN 一致）。 */
export const STACK_KEY_MAX_LEN = 24;

/**
 * 分组 key 归一化预览（与后端 stack::slug 同规则，fork 五项优化）：ASCII 字母数字
 * （大小写保留）之外的字符——含 `.`（id 的 key↔模型分隔符）——换成 `-`，连续 `-`
 * 合并、首尾去掉、超长截断。仅用于输入框实时展示「将保存为 xxx」；保留字/查重的
 * 权威校验仍在后端 validate_member_key。
 */
export function normalizeStackKey(raw: string): string {
  let out = "";
  for (const c of raw) {
    const code = c.charCodeAt(0);
    const isAsciiAlnum =
      (code >= 0x41 && code <= 0x5a) || // A-Z
      (code >= 0x61 && code <= 0x7a) || // a-z
      (code >= 0x30 && code <= 0x39); // 0-9
    if (isAsciiAlnum) {
      out += c;
    } else if (out !== "" && !out.endsWith("-")) {
      out += "-";
    }
    if (out.length >= STACK_KEY_MAX_LEN) break;
  }
  return out.replace(/-+$/, "");
}

/**
 * 默认聚合模型 id 前缀（与后端 route_prefix::DEFAULT_ROUTE_PREFIX 一致）。
 * fork: 会话路由改造为聚合模式后默认对齐上游 "ccs-"（doc/20261009-设计文档）。
 */
export const DEFAULT_ROUTE_PREFIX = "ccs-";

/**
 * 展示层前缀归一化（与后端 normalize_route_prefix 的 fail-safe 一致）：
 * trim 后为空或校验不过（如直接改库绕过保存校验）时回退默认前缀，
 * 保证 UI 展示的前缀与代理实际生效的前缀相同。
 */
export function resolveDisplayRoutePrefix(raw: string | undefined): string {
  const trimmed = raw?.trim() ?? "";
  if (!trimmed) return DEFAULT_ROUTE_PREFIX;
  return validateRoutePrefixValue(trimmed).ok ? trimmed : DEFAULT_ROUTE_PREFIX;
}

/**
 * 路由触发前缀校验（与后端 route_prefix::validate_route_prefix 规则一致）：
 * 非空、1–8 个字符、可打印 ASCII、不含冒号、以非字母数字字符结尾
 * （自带边界符——裸前缀会把 glm-4.7 等字母开头模型名误判为路由请求）。
 */
export function validateRoutePrefixValue(value: string): RoutePrefixValidation {
  const trimmed = value.trim();
  if (trimmed.length === 0) return { ok: false, reason: "empty" };
  if (trimmed.length < 1 || trimmed.length > 8)
    return { ok: false, reason: "length" };
  // 可打印 ASCII：0x21–0x7E（排除空白与控制字符）
  if (
    ![...trimmed].every(
      (c) => c.charCodeAt(0) >= 0x21 && c.charCodeAt(0) <= 0x7e,
    )
  ) {
    return { ok: false, reason: "charset" };
  }
  if (trimmed.includes(":")) return { ok: false, reason: "colon" };
  if (/[A-Za-z0-9]$/.test(trimmed)) return { ok: false, reason: "boundary" };
  return { ok: true };
}

const MODEL_FAMILIES = [
  "claude",
  "gpt",
  "gemini",
  "glm",
  "deepseek",
  "qwen",
  "grok",
  "kimi",
  "doubao",
  "minimax",
  "sonnet",
  "opus",
  "haiku",
  "fable",
];

/**
 * 误匹配提示（不强校验，设计 §3.9）：触发串去掉结尾边界符后
 * 若与常见模型名族前缀重合（如 "claude." 撞 "claude.xx" 形态），提示用户。
 */
export function routePrefixLikelyConflicts(value: string): boolean {
  const stem = value
    .trim()
    .replace(/[^A-Za-z0-9]+$/, "")
    .toLowerCase();
  if (!stem) return false;
  return MODEL_FAMILIES.some(
    (family) =>
      stem.startsWith(family) ||
      // 反向匹配要求 stem ≥3 位：单/双字母（如 "G." → "g"）过于宽泛，会误报
      (stem.length >= 3 && family.startsWith(stem)),
  );
}
