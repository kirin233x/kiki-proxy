import type { Protocol } from "./api";

/** 每种协议一个颜色（只用于标签，与「状态色」区分开） */
export const protocolColor: Record<Protocol, string> = {
  anthropic: "#d97757",
  openai_chat: "#10a37f",
  openai_responses: "#0ea5e9",
  gemini: "#4f7cff",
};

export function fmtNum(n: number): string {
  if (n >= 1e9) return (n / 1e9).toFixed(1) + "B";
  if (n >= 1e6) return (n / 1e6).toFixed(1) + "M";
  if (n >= 1e4) return (n / 1e3).toFixed(1) + "K";
  return n.toLocaleString();
}

export function fmtCost(n: number): string {
  if (!n) return "$0";
  if (n < 0.01) return "<$0.01";
  if (n < 100) return "$" + n.toFixed(2);
  return "$" + Math.round(n).toLocaleString();
}

export function fmtMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60000)}m${Math.round((ms % 60000) / 1000)}s`;
}

export function fmtTime(ts: number): string {
  const d = new Date(ts);
  const now = new Date();
  const hm = d.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
  if (d.toDateString() === now.toDateString()) return hm;
  return `${d.getMonth() + 1}/${d.getDate()} ${hm}`;
}

export function fmtAgo(ts: number): string {
  const s = Math.round((Date.now() - ts) / 1000);
  if (s < 60) return "刚刚";
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`;
  return `${Math.floor(s / 86400)} 天前`;
}

export function statusTone(status: number): "ok" | "warn" | "err" {
  if (status >= 200 && status < 300) return "ok";
  if (status === 429 || status === 404) return "warn";
  return "err";
}

/** 尝试把字符串格式化成好读的 JSON；SSE 流按事件拆行保留原样 */
export function pretty(s: string): string {
  if (!s) return "";
  const t = s.trim();
  if (t.startsWith("{") || t.startsWith("[")) {
    try {
      return JSON.stringify(JSON.parse(t), (k, v) => (typeof v === "string" && v.length > 600 && /data|base64/i.test(k) ? `${v.slice(0, 80)}…（${v.length} 字符）` : v), 2);
    } catch {
      return s;
    }
  }
  return s;
}

export function slugify(s: string): string {
  return s
    .toLowerCase()
    .replace(/[^a-z0-9-_]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 24);
}
