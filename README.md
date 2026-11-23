# kiki-proxy

一个跑在 Mac 菜单栏里的**本地 AI 网关**。所有客户端（Claude Code、Codex、Gemini CLI、Cursor……）都只连 `http://127.0.0.1:41521`，网关按模型名把请求分流到不同的端点，自动做协议转换、走不同的出口代理，出错时按你的配置切换到备用模型。

界面沿用 [Ankiki](https://github.com/kirin233x/ankiki) 的「雾紫」风格，基于 Tauri 2（Rust + 系统 WebView）。

## 能做什么

- **一个入口，多路分流**：模型名写成「端点前缀/模型名」，比如 `internal/sonnet`、`ds/deepseek-chat`，网关就知道发到哪里。Claude Code 的 `/model` 里能直接看到所有模型。
- **4 种协议互相转换**：Anthropic Messages、OpenAI Chat Completions、OpenAI Responses、Gemini。Claude Code 可以用 DeepSeek，Codex 可以用 Claude，Gemini CLI 可以用内部服务。流式输出、工具调用、思考过程、图片都会正确转换。
- **每个端点走不同的线路**：直连、跟随系统代理、HTTP 代理、SOCKS5 代理，可以按端点设置，也可以按规则覆盖。
- **可配置的故障切换**：每个端点（或每个模型）可以设置「出错时依次改用哪些模型」；触发条件（连接失败、超时、5xx、429、其他状态码）也可以配置。只在上游还没开始返回内容时切换，不会把输出了一半的回答重来一遍。
- **本地程序**：公司内部服务如果已经有命令行代理（负责登录、续期、格式转换，比如 cc-proxy），在端点里填上它的启动命令即可。kiki-proxy 会在启动时把它拉起来、挂了自动重启、退出时一起结束，并在界面上显示它的运行状态和输出日志；网关把这个端点的请求转给它监听的地址。
- **请求日志**：每个请求的客户端原始请求、发给上游的请求、上游原始响应、返回客户端的响应都能看到（密钥自动打码），可以一键复制成 curl 重放。完整内容默认保留 24 小时。
- **用量统计**：按模型、按端点统计请求数、token、费用、耗时、失败率。
- **一键获取模型和价格**：从上游的 `/models` 接口拉取模型列表，勾选添加；价格、上下文窗口、是否支持图片从 [LiteLLM 公开价格表](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json) 自动填写（内部模型查不到的手动填）。
- **上下文窗口**：每个模型可以标注上下文大小。≥ 1M 的模型在 Claude Code 的 `/model` 里带 `[1m]` 后缀；其他模型通过 `CLAUDE_CODE_MAX_CONTEXT_TOKENS` 告诉 Claude Code 何时压缩上下文（Claude Code 对不认识的模型名默认按 1M 处理）。上游返回「上下文超长」时，网关会改写成 Claude Code 认识的错误，让它自动压缩后重试。
- **一键接入客户端**：自动修改 Claude Code / Codex / Gemini CLI 的配置文件（修改前自动备份，可一键恢复）；Claude Code 桌面版可以生成配置文件，在桌面版里导入即可。

## 路由规则

一个请求的模型名会按下面的顺序决定去哪里：

1. **规则**：按顺序匹配（支持 `*`、`?` 通配符），可以改模型、换出口。适合处理客户端写死的模型名，比如 Claude Code 后台任务用的 `claude-haiku-*`。
2. **端点前缀**：`internal/sonnet` → 端点 `internal` 的 `sonnet` 模型。没在端点里列出的模型名也可以直接透传，比如 `ds/deepseek-reasoner`。
3. **默认模型**：以上都不匹配时使用。
4. **故障切换**：上面选出的目标失败时，按端点（或模型）配置的备选列表依次尝试。

「路由与切换」页面里有一个路由测试工具，输入模型名就能看到它会被发到哪里。

## 在 Mac 上编译

准备环境（只需要一次）：

```bash
xcode-select --install
```

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

```bash
brew install node
```

然后在项目目录下：

```bash
npm install
```

```bash
npm run tauri build
```

生成的 App 在 `src-tauri/target/release/bundle/macos/kiki-proxy.app`，拖进「应用程序」即可。

开发调试（改代码自动刷新）：

```bash
npm run tauri dev
```

只调界面（浏览器里用假数据，不需要 Rust）：

```bash
npm run dev
```

跑网关引擎的测试（16 种协议组合、故障切换、本地程序托管等）：

```bash
npm run test:core
```

## 接入公司内部服务（命令行代理）

如果你已经有一个命令行代理（启动后负责网页登录、token 续期、把 Anthropic 格式转换成内部格式，并在本机监听一个端口）：

1. 添加端点，选预设「本地命令行服务」。
2. 协议选这个代理**对外提供**的格式（比如它接收 Anthropic 格式，就选 Anthropic），地址填它监听的地址，比如 `http://127.0.0.1:8787`。
3. 打开「由 kiki-proxy 启动这个端点背后的程序」，填启动命令，和在终端里敲的一样（通过登录 shell 执行，能用到终端里的 PATH）。
4. 添加内部模型，保存。之后在 Claude Code 的 `/model` 里选「端点前缀/模型名」就会走这个代理。

程序的运行状态、输出日志、重启按钮都在端点编辑页里；端点列表和概览页会显示它是否在运行。

## Claude Code 桌面版

桌面版不读 `~/.claude/settings.json`，需要用它自己的「第三方推理」配置。在「客户端接入」页选好要放进桌面版的模型，点「生成配置文件」，然后在桌面版里：Help → Troubleshooting → Enable Developer Mode；Developer → Configure Third-Party Inference… → Import configuration…，选择生成的文件，Apply Changes 后 Relaunch。也可以按页面上列出的值手动填写。

## 项目结构

```
src-tauri/core/          网关引擎（纯 Rust，不依赖 Tauri，可以单独测试）
  src/ir.rs              协议无关的中间表示
  src/protocol/          4 种协议的请求/响应/流式适配器
  src/router.rs          模型名 → 端点的路由、故障切换链
  src/gateway.rs         本地 HTTP 服务与请求处理主流程
  src/upstream.rs        出口代理、上游地址拼接
  src/process.rs         本地程序托管（启动、日志、自动重启）
  src/pricing.rs         模型价格表（LiteLLM）
  src/store.rs           请求日志与用量统计（SQLite）
  src/clients.rs         一键配置 Claude Code / Codex / Gemini CLI，生成桌面版配置文件
  tests/                 端到端测试、真实报文测试
src-tauri/src/lib.rs     Tauri 外壳：钥匙串、菜单栏、前端命令
src/                     界面（React）
design/                  图标与视觉风格说明
```

## 数据存在哪里

- 配置：`~/Library/Application Support/com.kirin233x.kiki-proxy/config.toml`（可读的 TOML，可以在设置里导出导入）
- API Key、代理密码：macOS 钥匙串（服务名 `kiki-proxy`），不会写进配置文件
- 请求日志：同目录下的 `logs.db`

所有数据都只在本机，网关只监听 `127.0.0.1`。

## 已知限制

- Claude Code 桌面版不读 `settings.json`，需要在它的 Developer → Configure Third-Party Inference 里手动填网关地址（客户端接入页有步骤）。
- 跨协议转换时，各家独有的功能（Anthropic 的服务端工具和 prompt 缓存标记、OpenAI 的内置工具、加密的推理内容等）没有对应物，会被丢弃；同协议之间是原样透传，不受影响。
- Claude Code 的 `/model` 只显示名字含 claude 的模型，所以其他模型在列表里会显示成 `claude@前缀/模型`，网关会自动去掉这个前缀。
