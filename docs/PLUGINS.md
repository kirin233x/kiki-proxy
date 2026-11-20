# 写一个插件

插件用来处理「和标准 API 不一样的地方」：特殊的鉴权和登录、私有的请求字段、包了一层的响应格式等。下面这些网关已经帮你做好了，插件里不用管：

- 协议转换（客户端是 Claude Code、上游是 OpenAI 格式也没关系）
- 出口代理（`ctx.http` 已经走了端点配置的代理）
- 故障切换、请求日志、用量统计
- 凭证缓存（内存 + 钥匙串）、过期前刷新、多个请求同时过期时只登录一次
- 上游返回鉴权失败时，作废旧凭证、重新登录、再重试一次

## 步骤

1. 复制 `src-tauri/core/src/plugins/example.rs` 为 `src-tauri/core/src/plugins/corp.rs`，改成你的逻辑。
2. 在 `src-tauri/core/src/plugins/mod.rs` 里注册：

   ```rust
   mod corp;

   pub fn registry() -> Vec<Arc<dyn Plugin>> {
       vec![
           Arc::new(example::ExampleSso),
           Arc::new(corp::Corp),
       ]
   }
   ```

3. 重新构建：`npm run tauri build`（开发时用 `npm run tauri dev`，改了 Rust 代码会自动重新编译）。
4. 在 App 里编辑端点，「插件」一栏选择你的插件，填好插件声明的设置项。

## 请求经过插件的顺序

```
客户端请求
  → 网关转换成上游协议的 JSON
  → [凭证] 取缓存 / 过期则 refresh / 没有则 login → apply_credential
  → on_request（可以改 URL、请求头、请求体）
  → 通过出口代理发送
  → is_auth_failure？是 → 作废凭证，重新 login，再发一次
  → 非流式：on_response（改原始响应体）→ 网关解析、转换回客户端协议
     流式：  on_stream_event（逐个 SSE 事件）→ 网关解析、转换回客户端协议
```

## 接口一览

```rust
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn id(&self) -> &'static str;                 // 写进配置文件的 id，定了就别改
    fn name(&self) -> &'static str;               // 界面上的名字
    fn description(&self) -> &'static str;
    fn settings(&self) -> Vec<SettingField>;      // 端点编辑页里让用户填的东西

    // —— 鉴权（uses_login 返回 true 才会用到）——
    fn uses_login(&self) -> bool;
    async fn login(&self, ctx: &PluginCtx) -> Result<Credential>;
    async fn refresh(&self, ctx: &PluginCtx, old: &Credential) -> Result<Credential>;  // 默认直接 login
    fn apply_credential(&self, cred: &Credential, req: &mut UpstreamRequest) -> Result<()>;  // 默认 Bearer
    fn is_auth_failure(&self, status: u16, body: &[u8]) -> bool;  // 默认 status == 401

    // —— 改写 ——
    async fn on_request(&self, ctx: &PluginCtx, req: &mut UpstreamRequest) -> Result<()>;
    async fn on_response(&self, ctx: &PluginCtx, resp: &mut UpstreamResponse) -> Result<()>;
    fn on_stream_event(&self, ctx: &PluginCtx, ev: SseEvent) -> Vec<SseEvent>;
}
```

除了 `id` 和 `name`，其他方法都有默认实现，只写你需要的。

## PluginCtx 能做什么

| 方法 / 字段 | 作用 |
| --- | --- |
| `ctx.setting("key")` / `ctx.require("key")` | 读插件设置（Secret 类型的会自动从钥匙串读出来） |
| `ctx.http` | 走端点出口代理的 `reqwest::Client`，调登录接口用它 |
| `ctx.open_url(url)` | 用默认浏览器打开链接 |
| `ctx.prompt(PromptForm)` | 弹出表单（账号、密码、验证码，可以带一张图片，比如图形验证码），等用户填完 |
| `ctx.notify(title, body)` | 在 App 里提示一条消息 |
| `ctx.load(key)` / `ctx.save(key, value)` | 插件自己的持久化数据（存在钥匙串里，按端点隔离），比如记住上次的用户名 |
| `CallbackServer::bind(port)` | 在本机起一个临时服务接收 SSO 回调，`wait()` 拿到 URL 上的参数 |
| `ctx.endpoint_id` / `ctx.base_url` / `ctx.model` | 当前端点和模型 |
| `ctx.client_protocol` / `ctx.upstream_protocol` | 客户端协议和上游协议 |

## 常见写法

**静态 token，但要加签名头：**

```rust
async fn on_request(&self, ctx: &PluginCtx, req: &mut UpstreamRequest) -> Result<()> {
    let ts = crate::ir::now_secs().to_string();
    let sign = my_sign(ctx.require("secret")?, &ts); // 签名算法自己实现（需要的话在 core/Cargo.toml 加 hmac / sha2）
    req.set_header("x-timestamp", &ts)?;
    req.set_header("x-signature", &sign)
}
```

**浏览器 SSO：** 见 `example.rs` 里的 `login_with_browser`。

**调用本机命令拿临时凭证：**

```rust
async fn login(&self, _ctx: &PluginCtx) -> Result<Credential> {
    let out = tokio::process::Command::new("corp-cli").args(["token", "--json"]).output().await?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    Ok(Credential::bearer(v["token"].as_str().unwrap_or_default()).expires_in(3600))
}
```

**上游返回 200 但 body 里是鉴权错误：** 重写 `is_auth_failure`，见 `example.rs`。

**改模型名或请求路径：** 在 `on_request` 里直接改 `req.body["model"]` 或 `req.url`。

## 调试

- 请求日志里能看到插件改写后的「上游请求」和改写前的「上游响应」，以及每次切换、重新登录的记录。
- 可以仿照 `src-tauri/core/tests/e2e.rs` 里的 `TestAuth`，给插件写单元测试：起一个模拟上游，用 `Gateway::in_memory(..., vec![Arc::new(你的插件)])` 走完整流程。
