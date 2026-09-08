export type RoutePrefixValidation =
  | { ok: true }
  | {
      ok: false;
      reason: "empty" | "length" | "charset" | "colon" | "boundary";
    };

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
