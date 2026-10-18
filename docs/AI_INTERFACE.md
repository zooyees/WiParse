# WiParse 全局 AI 接口

实现日期：2026-10-18。纳入 WiParse 1.2.0 发行包。

## 使用

主窗口右上角 **AI** 或设置菜单 **AI** 打开“AI 设置与助手”。选择服务预设，填写模型名称和 API Key，点击“测试连接”，确认后启用并保存。配置在所有工具间共享。

| 服务 | 预设地址 | 预设模型 | 接口 |
|---|---|---|---|
| DeepSeek | `https://api.deepseek.com` | `deepseek-flash`，可编辑 | Chat Completions |
| 千问北京 | `https://dashscope.aliyuncs.com/compatible-mode/v1` | `qwen-plus`，可编辑 | Chat Completions，流式兼容 |
| 千问国际 | `https://dashscope-intl.aliyuncs.com/compatible-mode/v1` | `qwen-plus`，可编辑 | Chat Completions，流式兼容 |
| OpenAI | `https://api.openai.com/v1` | 用户填写 | Responses |
| 自定义兼容服务 | 用户填写基础地址或完整端点 | 用户填写 | Chat Completions / Responses |

模型可用性以账户为准，不绑定固定版本；千问服务器地域必须与 API Key 地域一致，也可填写百炼业务空间专属域名。千问使用 OpenAI 兼容端点，本实现不调用 DashScope 原生 generation 端点。当前能力为文本输入与文本分析，不包括图像、语音、工具执行或多轮推理历史。

“深度思考”按厂商发送不同参数：DeepSeek 为 `thinking.type`；千问为 `enable_thinking`。预设默认关闭思考；部分模型不支持关闭，须按具体模型要求调整。千问预设启用流式兼容，服务端流式回复完成后统一展示最终文本；不显示 reasoning_content。Responses 当前采用非流式请求。

API Key 使用 Windows 当前用户 DPAPI 加密后保存到 `ai.protected_api_key`，不是明文；跨电脑或用户迁移时重新输入。也可填写环境变量名称（如 `DEEPSEEK_API_KEY` / `DASHSCOPE_API_KEY`），填写后优先读取该变量。非 Windows 平台使用环境变量，不保存本地密钥。切换服务预设或编辑服务器地址会清除旧服务器的凭据草稿；保存后才更改全局服务。

AI 助手可输入问题，并主动加入页面摘要，编辑确认后发送：串口为当前日志页最近 50 行，每行最多 400 字符；波形为最多 16 通道的缓存测量及最多 20 条协议帧摘录；测试报告为目录/选中项摘要，不会自动上传报告正文或全部附件。摘要只在点击时收集；打开窗口、切换工具或启动程序不会自动发送数据。

## 内部调用

共享实现位于 `wiparse_core::ai`。GUI 安装唯一的 `AiService`，各工具通过 `crate::ai::service(ui.ctx())` 获取同一服务克隆。克隆共享配置与并发预算，不应为每次操作创建新的服务。

```rust
use wiparse_core::ai::AiRequest;

let ai = crate::ai::service(ui.ctx()).ok_or("AI service unavailable")?;
let task = ai.submit(AiRequest {
    purpose: "解释通信异常，列出证据与待验证项".into(),
    text: selected_context,
})?;
// 在后续帧中用 task.result.try_recv() 轮询，禁止在 UI 线程 recv() 阻塞。
// task.cancel() 取消等待；已发出的 HTTP 请求可能继续处理，直到完成或超时。
```

核心服务默认禁用，最大并发 2；连接测试也占用同一预算。请求输入上限 256 KiB，服务响应上限 2 MiB，超时可配置 5–300 秒。没有自动重试或重定向，防止重复计费或向重定向地址传递密钥。错误不回显服务器正文、请求文本或 Authorization。HTTP 状态、网络超时、响应损坏、输出截断、密钥不可用分别可识别。

AI 回复只呈现为建议。它不拥有仪器输出、串口写入、测试执行或判定修改入口；不替代 `00-system-contract.md` 的采集质量与产品判定规则。

## 接口依据与验证范围

- [DeepSeek 官方首次调用](https://api-docs.deepseek.com/)
- [DeepSeek Chat Completions](https://api-docs.deepseek.com/api/create-chat-completion/)
- [千问 OpenAI 兼容说明](https://help.aliyun.com/zh/model-studio/compatibility-of-openai-with-dashscope)
- [千问 Chat 参数与思考模式](https://help.aliyun.com/zh/model-studio/qwen-api-via-openai-chat-completions/)
- [OpenAI Chat Completions](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)

本地 HTTP 模拟覆盖请求路径、Bearer 鉴权、厂商参数、中文与流式正文、usage、Responses、401/429/503、重定向、输出截断、响应过大、配置迁移和密钥加解密。真实 DeepSeek / 百炼账户未提供，因此不宣称真实云端鉴权或模型调用验收通过。


## 助手体验修正（源码更新，尚未重新打包）

- **连接测试与问答分开**：助手页也可点击“测试当前连接”，测试只发送简短探测，不携带对话或页面摘要。成功由客户端根据有效响应判断，不根据模型是否说“联网”判断。当前填写的配置可以先测试，必须保存并启用后才用于对话。
- **回复可读**：解析并缓存 Markdown，显示标题、强调、列表、引用、表格及代码块；代码可单独复制，整段回复可复制，亦可切换原文。长表格及代码横向滚动，正文自动换行。不自动获取模型提供的图片或读取其本地文件引用，网页链接仅在点击后打开。
- **连续追问**：上下文按 user/assistant 角色传输，不只发送最后一句。内存最多保留12轮、512 KiB（单轮超限仍保留最新轮），请求历史取最近最多10轮且不超过128 KiB；完整请求仍受256 KiB限制。新对话清除当前历史；切换服务器、模型、接口或凭据后不把旧对话传给新配置。历史不自动保存到磁盘。
- **上下文透明**：页面摘要须由用户选择并可编辑；发送成功受理后从输入草稿清除，已发送的摘要保留在对应轮次中。当前轮次草稿不会在请求失败时丢失。
- **状态与恢复**：显示连接成功、生成耗时、输入/输出 tokens；原始 usage JSON 收入折叠详情。认证失败、权限、路径/模型、限流/额度、网络超时提供不同提示。没有后台自动重试；用户可点击重试。取消不清除已完成对话。
- **保留已有内容**：达到输出 token 上限时保留已生成正文并标明“部分回复”，支持继续追问；无结束标志的损坏/中断流仍明确报错。超长正文限制预览，复制仍保留完整回复。
- **输入操作**：Ctrl+Enter 发送，Enter 换行；对话区显示在输入区之前。服务设置未保存或未启用时明确提示，避免分析请求静默使用旧服务器或旧密钥。

回归使用本地模拟服务覆盖两种接口的连续角色、短连接探测、部分输出保留、GUI两轮追问/401失败/再次连接测试、宽窄屏和明暗主题的 Markdown 显示。未使用用户的真实 API Key，云端服务实际调用仍需在用户环境验证。

实际 egui 渲染示例（本地模拟数据）：[窄屏深色回复](ui-ai-assistant-2026-10-18/reply-narrow-dark.png)、[宽屏深色回复](ui-ai-assistant-2026-10-18/reply-wide-dark.png)、[浅色回复](ui-ai-assistant-2026-10-18/reply-wide-light.png)。验证结果见 `evidence/ai-assistant-2026-10-18/verification-summary.json`。
