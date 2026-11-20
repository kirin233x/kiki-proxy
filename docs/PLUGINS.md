# 写一个插件

## 插件是什么

一个插件就是一种**端点类型**，比如「公司内部服务」。在 App 里新建端点时，它和 Anthropic、OpenAI、Gemini 并列出现在「类型」里。选了以后它就是一个普通端点：照常配模型、出口代理、故障切换，日志和统计也照常记录。

```
Claude Code / Codex / Gemini CLI ……
        │
  kiki-proxy：客户端接入 · 模型映射 · 路由 · 故障切换 · 出口代理 · 日志
        │  把请求转换成插件声明的格式（比如 OpenAI Chat）
        ▼
  你的插件：登录状态 + 内部格式的小差异
        │
        ▼
  公司内部服务
```

插件只需要关心两件事：

1. **登录状态**：怎么登录（网页登录、账号密码、调本机命令……）、怎么续期。
2. **内部格式和标准格式不一样的地方**：多一个字段、换一个请求头、URL 不同等。完全一样的话一行都不用写。

下面这些框架已经帮你做好了：

- 任何客户端的请求转换成插件声明的格式（`protocol()`，默认 OpenAI Chat），响应再转换回去；流式输出、工具调用、图片都支持
- App 启动时没有有效凭证就自动登录（和命令行工具启动时打开网页登录一样）
- 凭证存进钥匙串；到期前在后台自动续期；遇到 401 作废旧凭证、重新登录、再重试一次
- 端点编辑页显示登录状态，可以手动登录 / 退出；「测试」会分步检查网络 → 登录 → 每个模型

## 步骤

1. 复制 `src-tauri/core/src/plugins/example.rs` 为 `corp.rs`，改成你们的真实逻辑。示例就是「OpenAI Chat 格式 + 网页登录 + 自动续期」，和常见的内部服务一样。
2. 在 `src-tauri/core/src/plugins/mod.rs` 里注册：

   ```rust
   mod corp;

   pub fn registry() -> Vec<Arc<dyn Plugin>> {
       vec![
           Arc::new(example::CorpExample),
           Arc::new(corp::Corp),
       ]
   }
   ```

3. 重新构建：`npm run tauri build`（开发时用 `npm run tauri dev`）。
4. 在 App 里添加端点，「类型」选你的插件，填好插件声明的设置项和地址，添加模型。

## 接口一览

```rust
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn id(&self) -> &'static str;                 // 写进配置文件的 id，定了就别改
    fn name(&self) -> &'static str;               // 「类型」里显示的名字
    fn description(&self) -> &'static str;
    fn settings(&self) -> Vec<SettingField>;      // 端点编辑页里让用户填的东西
    fn protocol(&self) -> Protocol;               // 内部服务说的标准格式，默认 OpenaiChat
    fn default_base_url(&self) -> &'static str;   // 新建端点时预填的地址

    // —— 登录状态（uses_login 返回 true 才会用到）——
    fn uses_login(&self) -> bool;
    fn login_on_start(&self) -> bool;             // App 启动时没有凭证就登录，默认 = uses_login
    async fn login(&self, ctx: &PluginCtx) -> Result<Credential>;
    async fn refresh(&self, ctx: &PluginCtx, old: &Credential) -> Result<Credential>;  // 默认直接 login
    fn refresh_before_secs(&self) -> i64;         // 到期前多少秒后台续期，默认 300
    fn apply_credential(&self, cred: &Credential, req: &mut UpstreamRequest) -> Result<()>;  // 默认 Bearer
    fn is_auth_failure(&self, status: u16, body: &[u8]) -> bool;  // 默认 status == 401

    // —— 内部格式的小差异 ——
    async fn on_request(&self, ctx: &PluginCtx, req: &mut UpstreamRequest) -> Result<()>;
    async fn on_response(&self, ctx: &PluginCtx, resp: &mut UpstreamResponse) -> Result<()>;
    fn on_stream_event(&self, ctx: &PluginCtx, ev: SseEvent) -> Vec<SseEvent>;
}
```

除了 `id` 和 `name`，其他方法都有默认实现，只写你需要的。

## 一次请求经过插件的顺序

```
客户端请求（任意格式）
  → 框架转换成 protocol() 声明的格式（比如 OpenAI Chat 的 JSON）
  → [凭证] 取缓存 / 没有则 login → apply_credential
  → on_request（可以改 URL、请求头、请求体）
  → 通过端点的出口代理发送
  → is_auth_failure？是 → 作废凭证，重新 login，再发一次
  → 非流式：on_response（改原始响应体）→ 框架转换回客户端格式
     流式：  on_stream_event（逐个 SSE 事件）→ 框架转换回客户端格式

后台：每分钟检查一次，凭证快到期（refresh_before_secs）就调 refresh
```

## PluginCtx 能做什么

| 方法 / 字段 | 作用 |
| --- | --- |
| `ctx.setting("key")` / `ctx.require("key")` | 读插件设置（Secret 类型的会自动从钥匙串读出来） |
| `ctx.http` | 走端点出口代理的 `reqwest::Client`，调登录 / token 接口用它 |
| `ctx.open_url(url)` | 用默认浏览器打开链接 |
| `CallbackServer::bind(port)` | 在本机起一个临时服务接收网页登录的回调，`wait()` 拿到 URL 上的参数 |
| `ctx.prompt(PromptForm)` | 弹出表单（账号、密码、验证码，可以带一张图片），等用户填完 |
| `ctx.notify(title, body)` | 在 App 里提示一条消息 |
| `ctx.load(key)` / `ctx.save(key, value)` | 插件自己的持久化数据（存在钥匙串里，按端点隔离） |
| `ctx.base_url` / `ctx.model` / `ctx.client_protocol` | 当前端点地址、模型、客户端协议 |

## 从命令行工具迁移

如果你已经有一个类似 `cc-proxy` 的命令行代理（本地起服务 → 网页登录 → 存 token → 定时续期 → Anthropic 转内部格式）：

- 本地服务、Anthropic → OpenAI 的转换：**删掉**，框架已经做了。
- 网页登录：搬到 `login` 里，用 `CallbackServer` 接回调、`ctx.open_url` 打开登录页。
- 续期：搬到 `refresh` 里。不用自己起定时器，框架会在到期前调用它。
- 存 token：删掉，`login` / `refresh` 返回 `Credential` 就行，框架负责存。
- 内部格式和标准 OpenAI 的差异：写进 `on_request` / `on_response`。

## 调试

- 端点编辑页的「保存并测试」会分步显示卡在哪一步：网络、登录、每个模型。
- 请求日志里能看到插件改写后的「上游请求」和改写前的「上游响应」，以及每次切换、重新登录的记录。
- 可以仿照 `src-tauri/core/tests/e2e.rs` 里的 `TestAuth`，给插件写测试：起一个模拟上游，用 `Gateway::in_memory(..., vec![Arc::new(你的插件)])` 走完整流程。
