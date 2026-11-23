// 浏览器预览用的假数据（没有 Tauri 时）。只用于调试界面。

import type { Config, LogRecord, Stats, UsageRow, DailyRow, ClientInfo } from "./api";

const listeners = new Map<string, Set<(p: unknown) => void>>();

export function mockListen(event: string, cb: (p: unknown) => void) {
  if (!listeners.has(event)) listeners.set(event, new Set());
  // 每次订阅包一层，同一个回调订阅两次（StrictMode）也能各自取消
  const wrapped = (p: unknown) => cb(p);
  listeners.get(event)!.add(wrapped);
  return () => {
    listeners.get(event)?.delete(wrapped);
  };
}

function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((cb) => cb(payload));
}

let config: Config = {
  server: { port: 41521, accessKey: "" },
  routing: { defaultModel: "internal/sonnet", claudeAlias: true, modelOrder: [] },
  failover: { onConnectError: true, onTimeout: true, on5xx: true, on429: true, extraStatus: [] },
  logging: { bodyRetentionHours: 24, statsRetentionDays: 90, maxBodyKb: 8192 },
  ui: { theme: "auto", probeUrl: "https://www.gstatic.com/generate_204" },
  pricing: { url: "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json", proxy: "system" },
  proxies: [
    { id: "corp", name: "公司代理", kind: "http", url: "http://proxy.corp.example:8080", username: "" },
    { id: "clash", name: "Clash", kind: "socks5", url: "127.0.0.1:7890", username: "" },
  ],
  endpoints: [
    {
      id: "internal",
      name: "内部模型服务",
      enabled: true,
      protocol: "anthropic",
      baseUrl: "http://127.0.0.1:8787",
      auth: "none",
      proxy: "direct",
      process: { enabled: true, command: "~/bin/cc-proxy --port 8787", workdir: "", env: "", network: "" },
      headers: {},
      timeoutSecs: 300,
      compat: { sendReasoningContent: false, useMaxCompletionTokens: false, sendReasoningEffort: false, dropSamplingParams: false },
      models: [
        { name: "sonnet", upstream: "claude-sonnet-4-5", displayName: "Sonnet（内部）", enabled: true, contextWindow: 1000000, vision: true, price: { input: 3, output: 15, cacheRead: 0.3, cacheWrite: 3.75 }, fallbacks: [] },
        { name: "opus", upstream: "claude-opus-4-5", displayName: "Opus（内部）", enabled: true, contextWindow: 200000, vision: true, price: { input: 5, output: 25, cacheRead: 0.5, cacheWrite: 6.25 }, fallbacks: [] },
      ],
      fallbacks: ["anthropic/claude-sonnet-4-5", "ds/deepseek-chat"],
      note: "",
    },
    {
      id: "anthropic",
      name: "Anthropic 官方",
      enabled: true,
      protocol: "anthropic",
      baseUrl: "https://api.anthropic.com",
      auth: "auto",
      proxy: "clash",
      process: { enabled: false, command: "", workdir: "", env: "", network: "" },
      headers: {},
      timeoutSecs: 300,
      compat: { sendReasoningContent: false, useMaxCompletionTokens: false, sendReasoningEffort: false, dropSamplingParams: false },
      models: [
        { name: "claude-sonnet-4-5", upstream: "", displayName: "", enabled: true, contextWindow: 200000, vision: true, price: { input: 3, output: 15, cacheRead: 0.3, cacheWrite: 3.75 }, fallbacks: [] },
        { name: "claude-haiku-4-5", upstream: "", displayName: "", enabled: true, contextWindow: 200000, vision: true, price: { input: 1, output: 5, cacheRead: 0.1, cacheWrite: 1.25 }, fallbacks: [] },
      ],
      fallbacks: [],
      note: "",
    },
    {
      id: "ds",
      name: "DeepSeek",
      enabled: true,
      protocol: "openai_chat",
      baseUrl: "https://api.deepseek.com",
      auth: "auto",
      proxy: "direct",
      process: { enabled: false, command: "", workdir: "", env: "", network: "" },
      headers: {},
      timeoutSecs: 300,
      compat: { sendReasoningContent: true, useMaxCompletionTokens: false, sendReasoningEffort: false, dropSamplingParams: false },
      models: [
        { name: "deepseek-chat", upstream: "", displayName: "DeepSeek V3", enabled: true, contextWindow: 131072, vision: false, price: { input: 0.28, output: 0.42, cacheRead: 0.028, cacheWrite: 0 }, fallbacks: [] },
        { name: "deepseek-reasoner", upstream: "", displayName: "DeepSeek R", enabled: true, contextWindow: 131072, vision: false, price: { input: 0.28, output: 0.42, cacheRead: 0.028, cacheWrite: 0 }, fallbacks: [] },
      ],
      fallbacks: [],
      note: "",
    },
    {
      id: "gm",
      name: "Gemini",
      enabled: false,
      protocol: "gemini",
      baseUrl: "https://generativelanguage.googleapis.com",
      auth: "auto",
      proxy: "clash",
      process: { enabled: false, command: "", workdir: "", env: "", network: "" },
      headers: {},
      timeoutSecs: 300,
      compat: { sendReasoningContent: false, useMaxCompletionTokens: false, sendReasoningEffort: false, dropSamplingParams: false },
      models: [{ name: "gemini-2.5-pro", upstream: "", displayName: "", enabled: true, contextWindow: 1048576, vision: true, price: { input: 1.25, output: 10, cacheRead: 0.31, cacheWrite: 0 }, fallbacks: [] }],
      fallbacks: [],
      note: "",
    },
  ],
  rules: [
    { id: "r1", name: "后台小任务走 DeepSeek", enabled: true, pattern: "claude-*haiku*", client: "anthropic", target: "ds/deepseek-chat", proxy: "" },
    { id: "r2", name: "Codex 默认模型", enabled: true, pattern: "gpt-5*", client: "openai_responses", target: "internal/sonnet", proxy: "" },
  ],
  desktop: { routes: [] },
};

let running = true;
const secrets = new Set<string>(["endpoint/anthropic/api_key", "endpoint/ds/api_key"]);

const MODELS = ["internal/sonnet", "internal/opus", "ds/deepseek-chat", "anthropic/claude-sonnet-4-5"];
const now = Date.now();

function fakeLog(i: number): LogRecord {
  const m = MODELS[i % MODELS.length];
  const ep = config.endpoints.find((e) => m.startsWith(e.id + "/"))!;
  const err = i % 11 === 3;
  const failover = i % 13 === 5;
  const input = 1200 + ((i * 7919) % 30000);
  const output = 80 + ((i * 104729) % 2400);
  const client = i % 4 === 0 ? "OpenAI Responses" : i % 5 === 0 ? "OpenAI Chat" : "Anthropic";
  return {
    id: `log${i}`,
    ts: now - i * 97_000,
    clientProtocol: client,
    path: client === "Anthropic" ? "/v1/messages" : client === "OpenAI Chat" ? "/v1/chat/completions" : "/v1/responses",
    requestedModel: i % 6 === 1 ? "claude-haiku-4-5-20251001" : m,
    gatewayModel: failover ? "ds/deepseek-chat" : m,
    endpointId: failover ? "ds" : ep.id,
    endpointName: failover ? "DeepSeek" : ep.name,
    upstreamModel: ep.models[0].upstream || ep.models[0].name,
    upstreamProtocol: failover ? "OpenAI Chat" : ep.protocol === "anthropic" ? "Anthropic" : "OpenAI Chat",
    proxy: ep.proxy,
    rule: i % 6 === 1 ? "后台小任务走 DeepSeek" : failover ? "故障切换" : "",
    status: err ? 529 : 200,
    error: err ? "[内部模型服务] Overloaded" : "",
    durationMs: 900 + ((i * 3571) % 24000),
    ttfbMs: 300 + ((i * 1999) % 1800),
    stream: i % 3 !== 2,
    usage: { input, output, cacheRead: i % 2 ? input * 3 : 0, cacheWrite: 0 },
    cost: err ? 0 : (input * 3 + output * 15) / 1e6,
    attempts: failover
      ? [{ gatewayModel: m, endpoint: ep.name, status: 503, error: "[内部模型服务] upstream connect error", durationMs: 1200 }]
      : [],
  };
}

const logs: LogRecord[] = Array.from({ length: 80 }, (_, i) => fakeLog(i));

function usageRows(key: "gatewayModel" | "endpointId"): UsageRow[] {
  const map = new Map<string, UsageRow>();
  for (const l of logs) {
    const k = l[key];
    const r =
      map.get(k) ??
      ({ key: k, label: l.endpointName, requests: 0, errors: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, avgMs: 0 } as UsageRow);
    r.requests++;
    if (l.status !== 200) r.errors++;
    r.input += l.usage.input;
    r.output += l.usage.output;
    r.cacheRead += l.usage.cacheRead;
    r.cost += l.cost;
    r.avgMs += l.durationMs;
    map.set(k, r);
  }
  return [...map.values()].map((r) => ({ ...r, avgMs: r.avgMs / r.requests })).sort((a, b) => b.requests - a.requests);
}

function stats(days: number): Stats {
  const byModel = usageRows("gatewayModel");
  const total = byModel.reduce(
    (t, r) => ({
      ...t,
      requests: t.requests + r.requests,
      errors: t.errors + r.errors,
      input: t.input + r.input,
      output: t.output + r.output,
      cacheRead: t.cacheRead + r.cacheRead,
      cost: t.cost + r.cost,
      avgMs: 4200,
    }),
    { key: "", label: "", requests: 0, errors: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, avgMs: 0 } as UsageRow,
  );
  const daily: DailyRow[] = Array.from({ length: days }, (_, i) => {
    const d = new Date(now - (days - 1 - i) * 86400000);
    const req = Math.round(40 + 60 * Math.abs(Math.sin(i * 1.7)) + (i % 7 < 5 ? 50 : 0));
    return {
      day: d.toISOString().slice(0, 10),
      requests: req,
      errors: Math.round(req * 0.04),
      input: req * 21000,
      output: req * 900,
      cost: req * 0.045,
    };
  });
  return { total, byModel, byEndpoint: usageRows("endpointId"), daily };
}


const clients: ClientInfo[] = [
  { id: "claude", name: "Claude Code", path: "~/.claude/settings.json", connected: true, hasBackup: true, model: "internal/sonnet", maxContextTokens: 0 },
  { id: "codex", name: "Codex CLI", path: "~/.codex/config.toml", connected: false, hasBackup: false, model: "", maxContextTokens: 0 },
  { id: "gemini", name: "Gemini CLI", path: "~/.gemini/.env", connected: false, hasBackup: false, model: "", maxContextTokens: 0 },
];

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// 模拟实时请求
setInterval(() => {
  if (!running) return;
  const l = { ...fakeLog(Math.floor(Math.random() * 50)), id: `live${Date.now()}`, ts: Date.now() };
  logs.unshift(l);
  emit("log", l);
}, 7000);

export async function mockInvoke<T>(cmd: string, args: Record<string, any> = {}): Promise<T> {
  await sleep(60);
  const r = (v: unknown) => v as T;
  switch (cmd) {
    case "get_config":
      return r(structuredClone(config));
    case "save_config":
      config = structuredClone(args.config);
      return r(null);
    case "get_status":
    case "start_server":
    case "stop_server":
      if (cmd === "start_server") running = true;
      if (cmd === "stop_server") running = false;
      return r({ running, port: config.server.port, error: null, startedAt: now - 3 * 3600_000 });
    case "export_config":
      return r(`# kiki-proxy 配置\n[server]\nport = ${config.server.port}\naccessKey = ""\n`);
    case "import_config":
      return r(config);
    case "reveal_config":
      return r(null);
    case "list_models": {
      const order = config.routing.modelOrder ?? [];
      const pos = (id: string) => (order.includes(id) ? order.indexOf(id) : 1e9);
      return r(
        config.endpoints
          .filter((e) => e.enabled)
          .flatMap((e) =>
            e.models.map((m) => ({ id: `${e.id}/${m.name}`, displayName: m.displayName || `${e.id}/${m.name}`, endpoint: e.name, vision: m.vision, contextWindow: m.contextWindow })),
          )
          .map((m, i) => ({ m, i }))
          .sort((a, b) => pos(a.m.id) - pos(b.m.id) || a.i - b.i)
          .map((x) => x.m),
      );
    }
    case "explain_route": {
      const name = String(args.model).replace(/^claude@/, "");
      const rule = config.rules.find((x) => x.enabled && new RegExp("^" + x.pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".") + "$", "i").test(name));
      const target = rule?.target || (name.includes("/") ? name : config.routing.defaultModel);
      const [prefix, model] = target.split("/");
      const ep = config.endpoints.find((e) => e.id === prefix);
      if (!ep) throw `kiki-proxy 找不到模型「${args.model}」`;
      const m = ep.models.find((x) => x.name === model);
      const hops = [{ gatewayModel: target, endpointId: ep.id, endpointName: ep.name, protocol: ep.protocol, upstreamModel: m?.upstream || model, proxy: rule?.proxy || ep.proxy, rule: rule?.name ?? (name.includes("/") ? "" : "默认模型") }];
      for (const f of (m?.fallbacks.length ? m.fallbacks : ep.fallbacks)) {
        const [p2, m2] = f.split("/");
        const e2 = config.endpoints.find((e) => e.id === p2);
        if (e2) hops.push({ gatewayModel: f, endpointId: e2.id, endpointName: e2.name, protocol: e2.protocol, upstreamModel: m2, proxy: e2.proxy, rule: "故障切换" });
      }
      return r(hops);
    }
    case "secret_flags":
      return r(Object.fromEntries((args.keys as string[]).map((k) => [k, secrets.has(k)])));
    case "set_secret":
      if (args.value) secrets.add(args.key);
      else secrets.delete(args.key);
      return r(null);
    case "process_status":
      if (args.id !== "internal") return r(null);
      return r({
        state: processState,
        pid: processState === "running" ? 48213 : null,
        held: processState === "stopped",
        paused: !running,
        startedAt: now - 2 * 3600_000,
        restarts: 1,
        lastExit: processState === "stopped" ? "已手动停止" : "程序退出，退出码 1",
        command: "~/bin/cc-proxy --port 8787",
        logs: [
          "[kiki-proxy 01:12:40] 已载入终端环境变量 58 个；代理 / 证书相关：HTTPS_PROXY、NO_PROXY、SSL_CERT_FILE",
          "[kiki-proxy 01:12:40] 启动：~/bin/cc-proxy --port 8787",
          "cc-proxy v0.4.2",
          "需要登录，正在打开浏览器…",
          "登录成功，token 有效期 8 小时",
          "listening on http://127.0.0.1:8787",
          "POST /v1/messages model=claude-sonnet-4-5 200 1.8s",
          "POST /v1/messages model=claude-sonnet-4-5 200 3.2s",
          "token 将在 5 分钟后过期，正在续期…",
          "续期成功",
        ],
      });
    case "restart_process":
    case "start_process":
      processState = "starting";
      setTimeout(() => (processState = "running"), 1500);
      return r(null);
    case "stop_process":
      processState = "stopped";
      return r(null);
    case "export_desktop_config":
      return r("/Users/kirin/Downloads/kiki-proxy-claude-desktop.json");
    case "apply_desktop_config":
      return r("/Users/kirin/Library/Application Support/Claude-3p/configLibrary/6b696b69-7072-4f78-8079-6b696b690001.json");
    case "restore_desktop_config":
      return r(null);
    case "inspect_claude":
      return r({
        files: [
          {
            path: "/Users/kirin/.claude/settings.json",
            modified: Date.now() - 4000,
            entries: [
              ["ANTHROPIC_BASE_URL", "http://127.0.0.1:41521"],
              ["ANTHROPIC_MODEL", "internal/sonnet[1m]"],
              ["ANTHROPIC_DEFAULT_OPUS_MODEL", "gpt-6.2-sol[1M]"],
              ["ANTHROPIC_DEFAULT_OPUS_MODEL_NAME", "gpt-6.2-sol"],
            ],
          },
        ],
        issues: [
          {
            file: "/Users/kirin/.claude/settings.json",
            key: "ANTHROPIC_DEFAULT_OPUS_MODEL",
            value: "gpt-6.2-sol[1M]",
            problem: "不是网关里的模型名，会被默认模型转到 internal/sonnet",
          },
        ],
      });
    case "desktop_connected":
      return r(false);
    case "test_endpoint":
      await sleep(700);
      return r(
        args.id === "gm"
          ? { ok: false, status: 400, latencyMs: 412, message: "API key not valid. Please pass a valid API key." }
          : { ok: true, status: 200, latencyMs: 600 + Math.round(Math.random() * 900), message: "Hello" },
      );
    case "test_step": {
      const ep = config.endpoints.find((e) => e.id === args.id)!;
      if (args.step === "network") {
        await sleep(400);
        return r(
          ep.id === "gm"
            ? { kind: "network", label: "网络", status: "fail", detail: "经Clash访问 generativelanguage.googleapis.com 失败：连接失败：Connection refused", ms: 3 }
            : { kind: "network", label: "网络", status: "ok", detail: `经${ep.proxy === "direct" ? "直连" : "Clash"}访问 ${new URL(ep.baseUrl).host} 正常`, ms: 42 },
        );
      }
      if (args.step === "auth") {
        await sleep(700);
        return r(
          ep.process.enabled
            ? { kind: "auth", label: "鉴权", status: "skip", detail: "没有配置 API Key（服务不需要的话可以忽略）", ms: 0 }
            : { kind: "auth", label: "鉴权", status: "ok", detail: "已配置 API Key，下一步实际调用模型时验证", ms: 0 },
        );
      }
      const i = ep.models.findIndex((m) => m.name === args.model);
      await sleep(800 + i * 1500);
      return r(
        ep.id === "internal" && i === 1
          ? { kind: "model", label: args.model, status: "fail", detail: "HTTP 403：model not allowed for this user（鉴权失败：检查 API Key 或登录状态、是否有这个模型的权限）", ms: 310 }
          : { kind: "model", label: args.model, status: "ok", detail: "回复了「Hello!」", ms: 800 + i * 1500 },
      );
    }
    case "test_endpoint_report": {
      await sleep(900);
      const ep = config.endpoints.find((e) => e.id === args.id)!;
      if (ep.id === "gm")
        return r({ ok: false, steps: [{ kind: "network", label: "网络", status: "fail", detail: "经Clash访问 generativelanguage.googleapis.com 失败：连接失败：Connection refused", ms: 3 }] });
      const steps: any[] = [{ kind: "network", label: "网络", status: "ok", detail: `经${ep.proxy === "direct" ? "直连" : "Clash"}访问 ${new URL(ep.baseUrl).host} 正常`, ms: 42 }];
      steps.push(
        ep.process.enabled
          ? { kind: "auth", label: "鉴权", status: "skip", detail: "没有配置 API Key（服务不需要的话可以忽略）", ms: 0 }
          : { kind: "auth", label: "鉴权", status: "ok", detail: "已配置 API Key，下一步实际调用模型时验证", ms: 0 },
      );
      ep.models.filter((m) => m.enabled).forEach((m, i) =>
        steps.push(
          ep.id === "internal" && i === 1
            ? { kind: "model", label: m.name, status: "fail", detail: "HTTP 403：model not allowed for this user（鉴权失败：检查 API Key 或登录状态、是否有这个模型的权限）", ms: 310 }
            : { kind: "model", label: m.name, status: "ok", detail: "回复了「Hello!」", ms: 600 + i * 230 },
        ),
      );
      return r({ ok: steps.every((x) => x.status !== "fail"), steps });
    }
    case "test_proxy":
      await sleep(400);
      return r({ ok: true, status: 204, latencyMs: 120 + Math.round(Math.random() * 300), message: "HTTP 204" });
    case "list_logs": {
      const q = args.query ?? {};
      let out = logs;
      if (q.q) out = out.filter((l) => l.requestedModel.includes(q.q) || l.gatewayModel.includes(q.q) || l.error.includes(q.q));
      if (q.endpoint) out = out.filter((l) => l.endpointId === q.endpoint);
      if (q.status === "ok") out = out.filter((l) => l.status === 200);
      if (q.status === "error") out = out.filter((l) => l.status !== 200);
      if (q.before) out = out.filter((l) => l.ts < q.before);
      return r(out.slice(0, q.limit ?? 100));
    }
    case "get_log": {
      const record = logs.find((l) => l.id === args.id);
      if (!record) return r(null);
      const req = {
        model: record.requestedModel,
        max_tokens: 32000,
        stream: record.stream,
        system: [{ type: "text", text: "You are Claude Code, Anthropic's official CLI for Claude." }],
        messages: [
          { role: "user", content: [{ type: "text", text: "帮我看看这个函数为什么慢" }, { type: "image", source: { type: "base64", media_type: "image/png", data: "iVBORw0KGgoAAAANSUhEUgAA…" } }] },
        ],
        tools: [{ name: "Read", description: "Reads a file", input_schema: { type: "object" } }],
      };
      const up = { ...req, model: record.upstreamModel, biz: "ai-coding" };
      return r({
        record,
        bodies: {
          clientRequest: JSON.stringify(req),
          clientHeaders: "content-type: application/json\nanthropic-version: 2023-06-01\nanthropic-beta: interleaved-thinking-2025-05-14\nauthorization: ••••roxy",
          upstreamUrl: `https://llm.corp.example/api/v1/messages`,
          upstreamRequest: JSON.stringify(up),
          upstreamHeaders: "content-type: application/json\naccept: text/event-stream\nanthropic-version: 2023-06-01\nx-corp-token: ••••a9f2",
          upstreamStatus: record.status,
          upstreamResponse: record.stream
            ? 'event: message_start\ndata: {"type":"message_start","message":{"id":"msg_01","model":"claude-sonnet-4-5"}}\n\nevent: content_block_delta\ndata: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"这个函数慢是因为"}}\n\n'
            : '{"id":"msg_01","type":"message","content":[{"type":"text","text":"这个函数慢是因为……"}]}',
          clientResponse: "（与上游响应相同，透传）",
        },
      });
    }
    case "clear_logs":
      logs.length = 0;
      return r(null);
    case "clear_failed_logs": {
      const before = logs.length;
      const keep = logs.filter((l) => l.status >= 200 && l.status < 300 && !l.error);
      logs.length = 0;
      logs.push(...keep);
      return r(before - keep.length);
    }
    case "get_stats":
      return r(stats(args.days));
    case "list_clients":
      return r(clients);
    case "apply_client":
      if (args.client === "claude") {
        const id = args.options.model || "internal/sonnet";
        clients[0].model = id;
        return r({
          file: "~/.claude/settings.json",
          changes: [
            "删除 apiKeyHelper（原来是 ~/bin/get-key.sh；它会顶掉网关用的 token）",
            "修改 env.ANTHROPIC_BASE_URL：https://old.example.com → http://127.0.0.1:41521",
            "删除 env.ANTHROPIC_API_KEY（原来是 ••••；和 ANTHROPIC_AUTH_TOKEN 冲突）",
            `修改 env.ANTHROPIC_MODEL：gpt-6.2-sol[1M] → ${id}[1m]`,
            `修改 env.ANTHROPIC_DEFAULT_OPUS_MODEL：gpt-6.2-sol[1M] → ${id}[1m]`,
            "修改 env.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME：gpt-6.2-sol → Sonnet（内部）",
            `修改 env.ANTHROPIC_DEFAULT_SONNET_MODEL：gpt-6.2-sol[1M] → ${id}[1m]`,
            "修改 env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME：gpt-6.2-sol → Sonnet（内部）",
            "修改 env.ANTHROPIC_DEFAULT_HAIKU_MODEL：gpt-6.2-sol → ds/deepseek-chat",
            "删除 env.CLAUDE_CODE_SUBAGENT_MODEL（原来是 gpt-6.2-sol；会让子代理固定用这个模型）",
          ],
        });
      }
      clients.find((c) => c.id === args.client)!.connected = true;
      clients.find((c) => c.id === args.client)!.hasBackup = true;
      clients.find((c) => c.id === args.client)!.model = args.options.model;
      return r({ file: "", changes: [] });
    case "restore_client":
      clients.find((c) => c.id === args.client)!.connected = false;
      clients.find((c) => c.id === args.client)!.hasBackup = false;
      return r(null);
    case "pricing_status":
      return r({ count: pricingLoaded ? 7039 : 0, updatedAt: pricingLoaded ? now - 2 * 86400_000 : 0, url: config.pricing.url });
    case "refresh_pricing":
      await sleep(900);
      pricingLoaded = true;
      return r({ count: 7039, updatedAt: Date.now(), url: config.pricing.url });
    case "lookup_models":
      await sleep(pricingLoaded ? 150 : 900);
      pricingLoaded = true;
      return r((args.models as string[]).map((m) => PRICES[m.replace(/-\d{8}$/, "")] ?? null));
    case "list_upstream_models":
      await sleep(500);
      return r(
        args.id === "ds"
          ? ["deepseek-chat", "deepseek-reasoner"]
          : ["claude-haiku-4-5-20251001", "claude-opus-4-1-20250805", "claude-opus-4-5-20251101", "claude-sonnet-4-20250514", "claude-sonnet-4-5-20250929"],
      );
    case "app_info":
      return r({ version: "0.1.0", gitHash: "浏览器预览", buildTime: Math.floor(Date.now() / 1000) });
    case "get_autostart":
      return r(true);
    case "set_autostart":
      return r(null);
  }
  throw new Error(`mock: 未实现的命令 ${cmd}`);
}


let pricingLoaded = true;
const P = (input: number, output: number, cacheRead: number, cacheWrite: number, contextWindow: number, vision: boolean, matched: string, provider: string) => ({
  input, output, cacheRead, cacheWrite, contextWindow, vision, matched, provider, maxOutput: 64000,
});
const PRICES: Record<string, ReturnType<typeof P>> = {
  "claude-sonnet-4-5": P(3, 15, 0.3, 3.75, 200000, true, "claude-sonnet-4-5", "anthropic"),
  "claude-sonnet-4": P(3, 15, 0.3, 3.75, 200000, true, "claude-sonnet-4-20250514", "anthropic"),
  "claude-opus-4-5": P(5, 25, 0.5, 6.25, 200000, true, "claude-opus-4-5", "anthropic"),
  "claude-opus-4-1": P(15, 75, 1.5, 18.75, 200000, true, "claude-opus-4-1", "anthropic"),
  "claude-haiku-4-5": P(1, 5, 0.1, 1.25, 200000, true, "claude-haiku-4-5", "anthropic"),
  "deepseek-chat": P(0.28, 0.42, 0.028, 0, 131072, false, "deepseek/deepseek-chat", "deepseek"),
  "deepseek-reasoner": P(0.28, 0.42, 0.028, 0, 131072, false, "deepseek/deepseek-reasoner", "deepseek"),
  "gemini-2.5-pro": P(1.25, 10, 0.125, 0, 1048576, true, "gemini/gemini-2.5-pro", "gemini"),
};

let processState: "running" | "starting" | "stopped" = "running";
