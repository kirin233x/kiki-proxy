//! 编译进 App 的插件都在这个目录里。
//!
//! # 插件是什么
//!
//! 一个插件就是一种「端点类型」，比如「公司内部服务」。在 App 里新建端点时，它和 Anthropic、
//! OpenAI 并列出现在「类型」里，选了以后就是一个普通端点：照常配模型、出口代理、故障切换。
//!
//! 插件只关心两件事：
//! - **登录状态**：`login`（网页登录 / 账号密码 / 调本机命令……）、`refresh`（续期）。
//!   网关负责把凭证存进钥匙串、App 启动时自动登录、到期前后台续期、遇到 401 重新登录并重试。
//! - **内部格式和标准格式不一样的地方**：`protocol()` 声明内部服务说哪种标准格式（默认 OpenAI Chat），
//!   网关会把任何客户端的请求转换成这种格式；剩下的小差异在 `on_request` / `on_response` /
//!   `on_stream_event` 里改。
//!
//! # 新增一个插件
//!
//! 1. 复制 `example.rs` 为 `corp.rs`，改成你们的真实逻辑。
//! 2. 在下面加 `mod corp;`，并在 [`registry`] 里加 `Arc::new(corp::Corp)`。
//! 3. 重新构建 App（`npm run tauri build`），新建端点时类型选你的插件。

use crate::plugin::Plugin;
use std::sync::Arc;

mod example;

/// 所有可用的插件
pub fn registry() -> Vec<Arc<dyn Plugin>> {
    vec![
        Arc::new(example::CorpExample),
        // 在这里注册你的插件，例如：
        // Arc::new(corp::Corp),
    ]
}
