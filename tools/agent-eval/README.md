# WiParse 串口助手实测

从仓库根目录运行。依赖 Node.js 22.22 或更高版本、Python 3 和当前工程的 Rust 工具链。Promptfoo 只用于开发评测。

```powershell
npm ci --prefix tools/agent-eval
python tools/agent-eval/run.py --out target/agent-eval-new
```

每个模型和场景各启动一次独立工程，连续追问在同一个真实串口助手会话执行。助手通过工程实际 AI 客户端调用模型，只提供仓库内的合成日志。设备控制默认关闭。DeepSeek 从当前 WiParse 配置读取凭据，只接受官方端点，可通过 `--config` 选择配置。千问读取环境变量 `DASHSCOPE_API_KEY`，可用 `WIPARSE_EVAL_QWEN_REGION` 选择 cn 或 intl，用 `WIPARSE_EVAL_QWEN_MODEL` 指定模型。缺少凭据会明确记录。两者使用相同的提示、日志和宿主工具。

复验单个场景时可使用 `--providers deepseek --cases followup`，避免重复其他付费模型调用；`--grade-only` 重评分时应使用与已有轨迹一致的 `--providers` 和 `--cases`。

`--collect-only` 只采集新轨迹。已有输出目录不会被覆盖。`--grade-only --out <已有目录>` 使用同一份轨迹运行 Promptfoo，不会再次调用模型。若仅补测千问，指定 `--providers qwen` 和新目录。输出包含各轮回答、实际调用、任务编号、计划、证据编号、耗时和服务端返回的累计 Token 用量。收费与缓存折扣以服务商账单为准。

判定关注换算与禁用工具要求、去重后的状态切换和相邻代号、带证据的任务计划、后续是否沿用原任务和证据。三个场景用于定位回归，单次运行不能代表模型的总体能力。
