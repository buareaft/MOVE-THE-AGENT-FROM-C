# DeepSeek Harness 适配

Move Your Agent 1.5.0。源码依据为官方仓库提交 `fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159`，审阅于 2026-11-07；对应上游桌面包版本 0.2.2-rc.1，Session 格式版本 4。

## 支持范围

- 默认根目录 `%USERPROFILE%\.dsh`，优先使用当前程序环境中的非空 `DSH_HOME`；支持当前用户 `~` 展开。根目录须为本地固定磁盘上的绝对路径，未知目录链接跳过。
- 根目录文件大小、磁盘占用、文件身份去重、实际卷统计。共享文件、会话独占、关联项目、未归属继续分别统计。
- 标准 `sessions/<project-key>/<encoded-id>/session[.vN].jsonl[.zstd]`，识别 v0–v4 的会话头。仅读取第一行；压缩头有输入、输出及窗口限制，支持扫描取消。
- 同一目录选择最高已提交代的头做元数据归属，将历史代文件的物理字节合计到一个会话。路径中的项目 key 不用于推断项目路径，项目关联只用已验证的头部 cwd；项目文件不归为对话生成物。
- 会话标识允许短字符串和 Unicode；核对 UTF-16 转义后的目录名，不要求 UUID。不同存储目录的同名 ID 使用独立索引标识。
- 未来版本、损坏头、身份不匹配、同版本多编码歧义及非标准日志不猜测归属，保留文件统计并提示。不会回退旧代假装识别最新会话。
- 固定迁移适配器 `deepseek-sessions`、`deepseek-cache`，共享现有校验、目录联接、保留副本、单独清理与最新数据恢复流程。可选择普通目录或已挂载的 VHDX 目标。

## 使用

点击“扫描本机”，在总览和对话分析中查看 DeepSeek Harness；到迁移记录预览会话或缓存目录。退出桌面程序及相关 CLI 后确认迁移。原路径的目录联接让后续文件写入目标，Move Your Agent 无需常驻。VHDX 使用前须挂载。

为避免漏掉使用共享目录的后台进程，检测到 `node.exe`、已知 Harness 桌面或主机进程时拒绝切换。应用不会自动结束这些进程，也不会读取命令行中的令牌。

`DSH_HOME` 必须存在于启动 Move Your Agent 的进程环境中。若仅在另一终端临时设置，已打开的程序不会自动继承。自定义插件把 sessions 放到其他位置时，本版不执行或推导配置中的 JavaScript。

## 暂未覆盖

不解析消息正文或缓存内的对话标题，不把共享投影缓存、附件或插件产物按 cwd 分摊给会话。不解析完整事件流或判定会话内容完整性。共享 `storages`、配置、凭据及 profiles 不整体迁移；Electron 在 Harness 根目录之外的数据、模型端远程数据不自动发现。

## 验证

新增生成样本覆盖普通/压缩头、历史代汇总、未知未来版本、歧义编码、目录身份校验、取消、项目关联及 SQL 索引空间口径。UI 使用明确的合成接口结果验证 DeepSeek 名称、对话详情和迁移适配器参数。指定测试机的真实文件系统验证只使用生成的 `DSH_HOME`，不访问实际 DeepSeek 对话或调用模型服务。

上游依据：[目录解析](https://github.com/deepseek-ai/deepseek-harness/blob/fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159/packages/util/home-paths/src/index.ts)、[标准存储组合](https://github.com/deepseek-ai/deepseek-harness/blob/fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159/packages/bundle/base/cordis.patch.yml)、[格式实现](https://github.com/deepseek-ai/deepseek-harness/blob/fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159/packages/session/session-persistence-jsonl/src/format.ts)。
