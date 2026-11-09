# 新界面成套更新契约

设置 → 运行包更新：检查索引、下载校验或导入本地完整 ZIP，核对版本后确认关闭安装。仅用户主动点击会访问更新服务。下载失败和校验失败不替换运行文件；程序或宿主仍运行时更新助手不强制终止它们。工程、日志、密钥和个人配置不属于受管运行文件。

预览构建由 `scripts/stage-next-preview.ps1` 写入 `runtime-manifest.json`。增加调试符号等运行文件后，交付流程需再次执行 `scripts/write-next-runtime-manifest.ps1 -Directory <目录> -Version <版本>`，随后计算最终交付清单。确认需要完整 ZIP 后再压缩目录，本次不自动创建 ZIP。

运行清单格式：`product=wiparse-next`、`schema=1`、数值 `build`、字符串 `version`、`files=[{path,bytes,sha256}]`。路径为正斜杠相对路径。必须包含四个 EXE、MCP 和插件 package.json；允许 bin、mcp、test-tools、recipes、Icon 及 config.default.json/launch.cmd。用户 config.json、工程、日志和其他未受管文件保留。旧单 EXE 包没有整包清单，明确拒绝。

远程索引示例（索引和 ZIP 必须同源；HTTPS，或本机 HTTP 测试）：

```json
{"product":"wiparse-next","schema":1,"version":"版本名称","build":1794140000,"archive_url":"https://updates.example/wiparse-next.zip","bytes":123456789,"sha256":"完整 ZIP 的 64 位十六进制 SHA256"}
```

更新时先验证受管安装文件与原清单一致，再备份全部旧文件，最后开始替换。更新日志标记 applying 后若异常，执行整套回退；启动检测未完成日志时禁止启动可能混版的宿主，用户从设置恢复。不得在正在运行的其他窗口所属宿主上进行替换。更新目录保留在运行根的 `.updates`，便于恢复和诊断；不自动递归删除用户目录。

自动测试通过 `scripts/test-next-runtime-update.ps1` 在 target 下建立假 EXE 目录，不运行程序，不连接仪器。正式在线服务、真实多机更新和断电设备验证仍需发布环境验收，自动测试不等于这些验收。

R46 串口实时日志新增 serial.log.changed 事件及类型过滤订阅。请成套更新 GUI 和 WiParse-Host.exe；前端发现旧宿主不支持该能力时会明确提示，不静默退回日志轮询。
