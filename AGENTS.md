# 给 AI 编码助手的说明

kiki-proxy 是一个 macOS 菜单栏里的本地 AI 网关（Tauri 2 + Rust + React）。项目结构见 [README.md](README.md)。

## 如果你的任务是适配一个内部服务（写插件）

**先完整阅读 [src-tauri/core/src/plugins/README.md](src-tauri/core/src/plugins/README.md)**，按里面的步骤做。要点：

- 只新增 `src-tauri/core/src/plugins/<名字>.rs`、`src-tauri/core/tests/plugin_<名字>.rs`，并在 `plugins/mod.rs` 注册一行。
- 不要修改框架代码（协议转换、网关、路由等）。
- 从复制 `plugins/example.rs` 和 `tests/plugin_example.rs` 开始。

## 常用命令

```bash
cd src-tauri && cargo test -p kiki-core          # 网关引擎的全部测试
cd src-tauri && cargo clippy -p kiki-core        # 代码检查
npx tsc --noEmit                                 # 前端类型检查
npm run tauri dev                                # 开发模式运行 App
npm run tauri build                              # 构建 App
```

## 约定

- 代码注释、界面文字、错误信息都用中文。
- 不要在日志里输出 token、API Key 等敏感信息。
