//! 编译进 App 的插件都在这个目录里。
//!
//! # 写一个新插件
//!
//! 1. 在这个目录新建一个文件，比如 `corp.rs`，参考 `example.rs` 实现 [`Plugin`] trait。
//!    只需要实现你关心的方法，其余的都有默认实现：
//!    - 特殊鉴权：`uses_login` 返回 true，实现 `login`（可选 `refresh`、`apply_credential`、`is_auth_failure`）
//!    - 改请求：`on_request`（URL、请求头、请求体都能改）
//!    - 改响应：`on_response`（非流式）、`on_stream_event`（流式，逐个 SSE 事件）
//!    - 需要用户填的东西：`settings`（会出现在端点编辑页里，Secret 类型的存进钥匙串）
//! 2. 在下面加一行 `mod corp;`，并在 [`registry`] 里加一行 `Arc::new(corp::Corp)`。
//! 3. 重新构建 App（`npm run tauri build`），在端点设置里选择这个插件即可。
//!
//! 网关已经帮你处理好：协议转换、出口代理、故障切换、日志、凭证缓存与过期刷新、
//! 401 后自动重新登录并重试。插件只需要关心「这个上游和标准 API 哪里不一样」。

use crate::plugin::Plugin;
use std::sync::Arc;

mod example;

/// 所有可用的插件
pub fn registry() -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(example::ExampleSso),
        // 在这里注册你的插件，例如：
        // Arc::new(corp::Corp),
    ]
}
