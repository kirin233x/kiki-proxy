# 给 AI 编码助手的说明

kiki-proxy 是一个 macOS 菜单栏里的本地 AI 网关（Tauri 2 + Rust + React）。项目结构见 [README.md](README.md)。

- `src-tauri/core`：网关引擎（协议转换、路由、故障切换、本地程序托管、日志），不依赖 Tauri，改完跑测试。
- `src-tauri/src/lib.rs`：Tauri 外壳和给前端调用的命令。
- `src/`：界面（React），在浏览器里 `npm run dev` 会使用 `src/mock.ts` 的假数据。改了前后端共用的配置结构时，记得同步 `src/api.ts`、`src/mock.ts`，并更新 `src-tauri/core/tests/frontend_config.json`。

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
