# 插件适配说明（给负责适配的人 / AI 编码助手）

这份文档告诉你：怎样把一个公司内部的模型服务接进 kiki-proxy。照着做，**只需要新增两个文件、改一行注册代码**，不需要改框架的任何代码。

---

## 0. 先理解这件事

kiki-proxy 是一个本地 AI 网关。Claude Code、Codex 等客户端都连它，它按模型名把请求转发到不同的「端点」。

一个**插件**就是一种端点类型（比如「公司内部服务」）。框架已经负责：

- 把任何客户端的请求（Anthropic / OpenAI / Gemini 格式）**转换成插件声明的格式**，响应再转换回去。流式输出、工具调用、图片都处理好了。
- 凭证（token）存到 macOS 钥匙串、App 启动时自动登录、到期前后台续期、上游返回鉴权失败时重新登录并重试一次。
- 出口代理、故障切换、日志、用量统计、分步测试。

**插件只负责两件事：**

1. **登录状态**：怎么登录拿到 token、怎么续期、token 放在请求的哪里。
2. **内部格式和标准格式的差异**：比如多一个请求头、多 / 少一个字段、URL 不同、响应多包了一层。完全一样就不用写。

> 典型情况：内部服务是 OpenAI Chat Completions 格式（`/v1/chat/completions`）+ 网页登录 + token 定时续期。
> 示例插件 `example.rs` 就是这个场景，**从复制它开始**。

---

## 1. 你要交付的东西

| 文件 | 内容 |
| --- | --- |
| `src-tauri/core/src/plugins/<名字>.rs` | 插件实现（复制 `example.rs` 改） |
| `src-tauri/core/src/plugins/mod.rs` | 加 `mod <名字>;`，并在 `registry()` 里加 `Arc::new(<名字>::<结构体>)` |
| `src-tauri/core/tests/plugin_<名字>.rs` | 插件测试（复制 `tests/plugin_example.rs` 改），必须通过 |

**不要修改**这两个文件以外的代码（`protocol/`、`gateway.rs`、`plugin.rs` 等都是框架）。如果你觉得框架缺了什么能力，停下来说明缺什么、为什么，不要自己改框架。

---

## 2. 动手前，先把这些信息搞清楚

信息来源：现有的命令行代理代码（比如 `cc-proxy`）、内部服务的接口文档，或者直接问用户。**每一项都要有确定的答案，不要猜。**

### 接口格式
- [ ] 内部服务是哪种格式？OpenAI Chat（`/v1/chat/completions`）/ OpenAI Responses / Anthropic / Gemini？→ 决定 `protocol()` 返回什么
- [ ] 接口地址是什么？完整 URL 举例 → 决定 `default_base_url()`。注意：框架会在地址后面拼 `/chat/completions`；地址以 `/v1` 这样的版本号结尾时不再加 `/v1`
- [ ] 和标准格式相比，**请求**有什么不同？（额外的请求头、额外 / 不支持的字段、模型名写法、URL 路径）
- [ ] **响应**有什么不同？（是否包了一层 `{"data": ...}`、错误格式、流式里有没有非标准事件）

### 登录
- [ ] 怎么登录？网页登录（打开浏览器，回调到本机）/ 账号密码 / 验证码 / 调用本机命令 / 固定 key？
- [ ] 网页登录：登录页地址是什么？回调参数叫什么（通常是 `redirect_uri`）？回调地址有没有固定端口的要求？回调时 URL 上带的是什么（`code`？`token`？）
- [ ] 换 token：接口地址、请求方法、请求体字段、响应字段（access_token / refresh_token / expires_in 叫什么？）
- [ ] token 放在请求的哪里？`Authorization: Bearer xxx`（默认）/ 自定义请求头 / cookie / URL 参数？
- [ ] token 多久过期？怎么续期？有没有 refresh_token？
- [ ] token 失效时内部服务怎么返回？HTTP 401（默认处理）/ 403 / 200 + 业务错误码？

### 模型
- [ ] 有哪些模型可用？模型名是什么？（用户之后在界面里配置，测试里写一个就行）

---

## 3. 从命令行代理迁移（如果有现成的代码）

| 命令行代理里的东西 | 在插件里 |
| --- | --- |
| 本地 HTTP 服务、监听端口 | **删掉**，框架就是这个服务 |
| Anthropic ↔ OpenAI 格式转换 | **删掉**，`protocol()` 返回 `OpenaiChat` 即可 |
| 启动时打开网页登录 | 写进 `login()`。框架会在 App 启动时调用（`login_on_start` 默认开启） |
| 接收登录回调的本地服务 | 用 `CallbackServer`（见下文） |
| 存 token 到文件 | **删掉**，`login()` 返回 `Credential` 就行，框架存钥匙串 |
| 定时器续期 | 写进 `refresh()`，**不要自己起定时器**，框架会在到期前 `refresh_before_secs()` 秒调用它 |
| 给请求加 token | 默认就是 `Authorization: Bearer`；不同的话重写 `apply_credential()` |
| 内部格式的特殊处理 | 写进 `on_request()` / `on_response()` / `on_stream_event()` |

---

## 4. Plugin 接口（`crate::plugin::Plugin`）

除了 `id`、`name` 都有默认实现，**只重写需要的**。

```rust
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    // —— 基本信息 ——
    fn id(&self) -> &'static str;                // 唯一 id，写进用户配置文件，定了就不要改。小写加下划线
    fn name(&self) -> &'static str;              // 界面「类型」里显示的名字，如 "公司内部服务"
    fn description(&self) -> &'static str;       // 一句话说明
    fn settings(&self) -> Vec<SettingField>;     // 端点编辑页里让用户填的设置项
    fn protocol(&self) -> Protocol;              // 内部服务的格式，默认 Protocol::OpenaiChat
    fn default_base_url(&self) -> &'static str;  // 新建端点时预填的地址

    // —— 登录状态 ——
    fn uses_login(&self) -> bool;                // 返回 true 才会用下面这些。默认 false
    fn login_on_start(&self) -> bool;            // App 启动时没凭证就登录。默认 = uses_login()
    fn refresh_before_secs(&self) -> i64;        // 到期前多少秒续期。默认 300
    async fn login(&self, ctx: &PluginCtx) -> Result<Credential>;
    async fn refresh(&self, ctx: &PluginCtx, old: &Credential) -> Result<Credential>; // 默认直接调 login
    fn apply_credential(&self, cred: &Credential, req: &mut UpstreamRequest) -> Result<()>; // 默认 Authorization: Bearer
    fn is_auth_failure(&self, status: u16, body: &[u8]) -> bool; // 默认 status == 401

    // —— 格式差异 ——
    async fn on_request(&self, ctx: &PluginCtx, req: &mut UpstreamRequest) -> Result<()>;
    async fn on_response(&self, ctx: &PluginCtx, resp: &mut UpstreamResponse) -> Result<()>;
    fn on_stream_event(&self, ctx: &PluginCtx, ev: SseEvent) -> Vec<SseEvent>;
}
```

### 调用时机

```
App 启动        → 没有有效凭证且 login_on_start() → login()
每分钟检查一次   → 凭证在 refresh_before_secs() 秒内到期 → refresh()
每个请求：
  框架把客户端请求转换成 protocol() 格式的 JSON
  → 取凭证（没有 → login()；已过期且有 refresh_token → refresh()）
  → apply_credential()
  → on_request()                 ← req.body 已经是转换好的 JSON，req.body["model"] 是上游模型名
  → 发送（走端点的出口代理）
  → is_auth_failure()？是 → 作废凭证 → login() → 同一个请求再发一次（只重试一次）
  → 非流式：on_response()（在框架解析之前）
     流式：  每个 SSE 事件 on_stream_event()（在框架解析之前）
  → 框架转换回客户端的格式
```

### 可以用的东西

```rust
// 设置项（显示在端点编辑页；secret 类型存钥匙串）
SettingField::url("login_url", "登录页地址").required().help("说明文字")
SettingField::text("tenant", "租户").default_value("default")
SettingField::secret("app_secret", "App Secret")

// PluginCtx
ctx.setting("key") -> Option<&str>   // 读设置（空字符串当作没填）
ctx.require("key")? -> &str          // 必填设置，没填返回清楚的错误
ctx.http                             // reqwest::Client，已经走了端点的出口代理。调登录 / token 接口必须用它
ctx.open_url(url).await?             // 用默认浏览器打开
ctx.prompt(form).await? -> BTreeMap<String, String>  // 弹表单等用户填写，用户取消会返回错误
ctx.notify("标题", "内容")            // 在 App 里提示
ctx.load("key") / ctx.save("key", "value")?          // 插件自己的持久化数据（钥匙串，按端点隔离）
ctx.endpoint_id / ctx.endpoint_name / ctx.base_url / ctx.model

// 网页登录回调
let cb = CallbackServer::bind(0).await?;          // 0 = 随机端口；要求固定端口就传端口号
let redirect = cb.redirect_uri("/callback");      // http://127.0.0.1:<端口>/callback
ctx.open_url(&format!("{login_url}?redirect_uri={}", urlencode(&redirect))).await?;  // 建议用 url::Url 拼参数
let hit = cb.wait(Duration::from_secs(300)).await?;  // 等浏览器跳回来，浏览器会显示「登录成功」
let code = hit.query.get("code");

// 表单（账号密码 / 验证码）
PromptForm::new("登录内部服务").message("说明").submit("登录")
    .field(PromptField::text("username", "账号").default_value(&last))
    .field(PromptField::password("password", "密码"))
    .field(PromptField::code("otp", "验证码").placeholder("6 位数字"))
    .image(captcha_data_url)                       // 可选：图形验证码

// 凭证
Credential::bearer(token).expires_in(3600)        // expires_in：秒；不调用表示不过期
cred.refresh_token = Some(rt);
cred.extra.insert("cookie".into(), c);            // 其他需要带着的东西

// 请求 / 响应
req.url, req.headers, req.body (serde_json::Value), req.raw_body (Option<Vec<u8>>，设置后忽略 body)
req.set_header("name", "value")?; req.remove_header("name");
resp.status, resp.headers, resp.body (Vec<u8>，可整个替换)
SseEvent { event: Option<String>, data: String }; ev.json() -> Option<Value>; SseEvent::data("...")

// 从各种错误响应里取出一句人话
crate::protocol::error_message(&bytes) -> String
```

---

## 5. 常见差异的写法

```rust
// token 放在自定义请求头
fn apply_credential(&self, cred: &Credential, req: &mut UpstreamRequest) -> Result<()> {
    req.set_header("x-auth-token", &cred.token)
}

// token 放在 URL 参数
fn apply_credential(&self, cred: &Credential, req: &mut UpstreamRequest) -> Result<()> {
    let mut u = url::Url::parse(&req.url)?;
    u.query_pairs_mut().append_pair("access_token", &cred.token);
    req.url = u.to_string();
    Ok(())
}

// 额外的请求头 / 字段，去掉不支持的字段
async fn on_request(&self, ctx: &PluginCtx, req: &mut UpstreamRequest) -> Result<()> {
    req.set_header("x-tenant", ctx.require("tenant")?)?;
    req.body["user"] = json!("kiki-proxy");
    if let Some(o) = req.body.as_object_mut() {
        o.remove("stream_options");
        o.remove("parallel_tool_calls");
    }
    Ok(())
}

// 接口路径不是 /chat/completions
async fn on_request(&self, _ctx: &PluginCtx, req: &mut UpstreamRequest) -> Result<()> {
    req.url = req.url.replace("/chat/completions", "/llm/chat");
    Ok(())
}

// token 失效时返回 200 + 业务错误码
fn is_auth_failure(&self, status: u16, body: &[u8]) -> bool {
    status == 401
        || serde_json::from_slice::<Value>(body).ok().and_then(|v| v["code"].as_i64()) == Some(40101)
}

// 响应包了一层 {"code":0,"data":{...标准 OpenAI 响应...}}
async fn on_response(&self, _ctx: &PluginCtx, resp: &mut UpstreamResponse) -> Result<()> {
    if let Ok(v) = serde_json::from_slice::<Value>(&resp.body) {
        if let Some(inner) = v.get("data").filter(|d| d.is_object()) {
            resp.body = serde_json::to_vec(inner)?;
        }
    }
    Ok(())
}

// 流式里有自定义的心跳事件，或者每个事件也包了一层
fn on_stream_event(&self, _ctx: &PluginCtx, ev: SseEvent) -> Vec<SseEvent> {
    if ev.event.as_deref() == Some("heartbeat") {
        return vec![];
    }
    match ev.json().and_then(|v| v.get("data").cloned()) {
        Some(inner) => vec![SseEvent::data(inner.to_string())],
        None => vec![ev],
    }
}

// 调本机命令拿 token（公司 CLI）
async fn login(&self, _ctx: &PluginCtx) -> Result<Credential> {
    let out = tokio::process::Command::new("corp-cli").args(["token", "--json"]).output().await?;
    anyhow::ensure!(out.status.success(), "corp-cli 失败：{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout)?;
    Ok(Credential::bearer(v["token"].as_str().unwrap_or_default()).expires_in(3600))
}
```

---

## 6. 测试（必须做）

1. 复制 `src-tauri/core/tests/plugin_example.rs` 为 `tests/plugin_<名字>.rs`。
2. 改模拟服务，让它的行为和**真实内部服务一致**（字段名、状态码、token 失效时的表现、流式格式）。有真实报文的话照抄。
3. 改 `setup()` 里的端点配置：`plugin` 改成你的插件 id，`plugin_settings` 填你的设置项。
4. 改断言：检查上游收到的请求（鉴权头、特殊字段、URL）是否符合内部服务的要求。
5. 至少覆盖：首次登录后请求成功（非流式 + 流式）、token 过期后续期、token 失效（401 或业务错误码）后自动重新登录。
6. 运行：

```bash
cd src-tauri && cargo test -p kiki-core --test plugin_<名字>
```

7. 确认没有破坏其他东西，并且整个 App 能编译：

```bash
cd src-tauri && cargo test -p kiki-core && cargo clippy -p kiki-core
```

```bash
npm run tauri build
```

8. 用户在 App 里验证：添加端点 → 类型选你的插件 → 填设置 → 添加模型 → 「保存并测试」，分步结果应该全部通过（网络、登录、每个模型）。

---

## 7. 规则

- 只新增 / 修改第 1 节列出的三个文件。
- 不要在日志、错误信息、`println!` 里输出 token、密码等敏感内容。
- 密码、secret 之类的设置项用 `SettingField::secret`，不要用 `text`。
- 网络请求一律用 `ctx.http`（走端点的出口代理），不要自己 `reqwest::Client::new()`。
- 不要 `unwrap()` / `expect()` 外部输入（接口响应、设置项），用 `?` 和 `anyhow!` 返回清楚的中文错误，用户会在界面上看到这些错误。
- 不要阻塞：用 `tokio::process`、`tokio::time`，不要用 `std::thread::sleep`、`std::process::Command`。
- 不要自己起定时器续期，也不要自己存 token。
- `id()` 一旦发布就不要改，否则用户已有的端点会找不到插件。

## 8. 完成标准

- [ ] 第 2 节的问题都有确定答案，并且写在插件文件顶部的注释里
- [ ] `cargo test -p kiki-core --test plugin_<名字>` 通过
- [ ] `cargo test -p kiki-core` 全部通过，`cargo clippy -p kiki-core` 没有新警告
- [ ] `mod.rs` 已注册
- [ ] 告诉用户：在 App 里怎么配置（要填哪些设置项、地址是什么），以及哪些地方是根据猜测写的、需要用真实环境确认
