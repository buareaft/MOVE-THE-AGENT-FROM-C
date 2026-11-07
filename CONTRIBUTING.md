# 开发与构建

仅支持 Windows x64。开发环境需要 Node.js、Rust MSVC 工具链、Visual Studio C++ Build Tools 和 WebView2。字体文件已内置，常规构建不需要 Python。

```powershell
npm ci
npm run desktop
```

`desktop` 使用项目生成样本；只调整界面时可运行 `npm run dev`，浏览器会明确标记模拟数据。

```powershell
npm run build
npm run check:core
$env:PLAYWRIGHT_BROWSERS_PATH = "$PWD\.tools\playwright"
npx playwright install chromium
npm run test:ui
```

构建便携包：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1
```

输出位于 `artifacts/`。便携包保留同目录标记文件、CLI 和第三方许可；不要把开发缓存或测试数据一起打包。

## 文件操作边界

- 开发主机仅在仓库 `.lab/agent-space` 下进行生成样本扫描和迁移测试，不访问开发者真实 Agent 数据、凭据或配置。
- 不在开发主机启动生产版扫描；直接运行 CLI 时务必显式传入 `--fixtures`。`npm run cli` 已默认传入。
- 真实 Windows 文件系统、VHDX 和跨盘验收应使用独立测试电脑及生成目录。不得用生产数据测试中断、磁盘满或权限失败。
- Rust 核心保持独立于 Tauri，SQLite 访问放在 Rust，桌面和 CLI 共用实现。
- 未知链接拒绝；只跟随本程序记录且验证过的迁移目录联接。
- 保持共享、关联、独占、未归属统计和文件身份去重；不从 cwd 推断生成文件所有权。
- 迁移必须有具体方案、进程检查、校验、保留副本和可恢复记录，不添加任意目录删除功能。

`.lab/`、`.tools/`、本机连接信息、测试机身份与原始验收数据均不属于公开源码。
