# 内置字体

1.2.1 将字体文件随前端资源编译进 Windows 程序，不安装到系统、不调用字体 CDN。

- 英文、数字、路径：Geist Mono 可变字体。
- 中文：Noto Sans SC 可变字体。
- 界面、表格、输入框、容量数值和弹窗统一使用这套字体；100–900 字重由字体自身提供，不合成粗体。

字体来自上游官方仓库，固定提交及 SHA-256 保存在 `src/assets/fonts/sources.json`。Geist Mono 使用原始 WOFF2；Noto Sans SC 从完整 TTF 转换为 WOFF2，保留字符映射，不按界面文案裁剪，避免项目名、路径和会话名称中的汉字缺失。字体不包含的字符仍可能使用系统后备字体。

两个字体使用 SIL Open Font License 1.1；原始版权及许可全文保存在 `src/assets/fonts/*-OFL.txt`，便携包包含 `THIRD-PARTY-NOTICES.txt`。

转换工具：项目内 Python 环境，fonttools 4.68.0 / brotli 1.2.1。`scripts/convert-font.py` 校验原始文件摘要及转换前后的字符映射。常规构建直接使用已提交的 WOFF2，不依赖 Python。

字体测试禁止外部 HTTP 请求，并检查浏览器实际渲染的英文/数字和中文字体均为内置字体；不以 CSS 声明的字体名代替实际加载结果。

1.2.1 验证：26 项 Rust 测试、9 项界面与合成接口测试通过，包括实际字体渲染检查。截图包含模拟数据。字体许可随便携包附带，本轮未在虚拟机桌面重做原生字体验收。
