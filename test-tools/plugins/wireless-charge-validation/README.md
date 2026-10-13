# 无线充电自动检验插件

版本 0.1.1，兼容 WiParse 1.1.30 和 Node 18 以上。依据总契约 S1 与插件规格 PL-01 至 PL-12。本版交付离线编排和规则回放，实物刺激流程尚未接通；不属于 G2 至 G4 工业验收系统。

## 运行

在 WiParse 根目录执行；Testing Hub 中选择“无线充电自动检验”也可使用同一生命周期。

```powershell
# 检查配置/矩阵；不分配 run，不连接仪表
node test-tools/runner.mjs --plugin wireless-charge-validation --lifecycle preflight
# 无刺激演练，所有未测检查为 INCONCLUSIVE
node test-tools/runner.mjs --plugin wireless-charge-validation --data-root .\validation-data --project demo
# 另一个终端先启动服务，服务与 GUI 的数据根须一致
.\bin\WiParse-Service.exe --data-root .\validation-data\automation-data --port 7879
# 经真实 SW check.evaluate 评估模拟量测数据库
node test-tools/runner.mjs --plugin wireless-charge-validation --data-root .\validation-data --project demo -- --mode replay --suite regression --fixture gold --service_root .\validation-data\automation-data
# 请求当前用例退出；不分配新 run
node test-tools/runner.mjs --plugin wireless-charge-validation --lifecycle stop --data-root .\validation-data --project demo
```

`gold/bad/missing/quality` 分别为正常、失败、缺测、质量受损数据库。全部是明确标注的模拟证据。`validation_verdict` 表示回放规则结论；`product_verdict` 始终为 INCONCLUSIVE，不能用模拟库认证产品。`ok` 只表示执行成功：执行成功且回放 FAIL 是合法结果。`checks.json` 保留每个已确认失败，不因后续异常或取消抹掉；首次异常通过 `first_error_id` 关联。

## 配置与边界

产品 profile、limits、states，station、三类 catalog、scenarios 和五类 suite 独立版本化并冻结 SHA256。只允许表单定义的 product/suite/mode/fixture/DUT/服务目录/报告名等参数；limits/approved/安全配置等 overlay 拒绝。参考 limits 的 approved 仅表示模拟库批准，simulation_only 阻止实物使用，不代表正式产品限值审批。没有签名审批系统；受信任工程师维护配置文件，不能开放给不可信插件作者。

适配器 ADP01 至 ADP09、充电 CHG01 至 CHG09 等用例模板保留整体目标。VIN/VBRG 数值、频率和四个 VGS 方向死区通过 SW 标量判定；本版读取模拟 MeasurementResult，不自行解析波形。TX/输入状态检查转换、PT 物理功率一致性和 EPT 原因；电池 CC/CV 规则要求可观测证据。通信规则分别处理输入层和 Qi 层，需要独立参考分母、完整覆盖和帧计数；不输出 BER。完整多协议消息语义、原始波形自动导入、真实测量窗识别和电池状态图联动仍待开发。

ADP/CHG 和安全/追溯等 `integrity` 标量是模拟契约占位，不能替代实际刺激、量测、测量系统分析或审批。模板中的步骤、时间预算和 pre/post 窗尚未批准，只用于描述集成需求。smoke 保留高风险保护和历史故障项；其他四个 suite 均列出全矩阵。它们当前均执行一次离线回放，不代表已实现 soak 长稳或生产节拍测试。

live 模式固定拒绝 `INTEGRATION.LIVE_BACKEND_NOT_VALIDATED`。预检额外列出缺少的 capture.arm/status、dongle.capture.start、stimulus.apply、station.safe_stop、设备验证、正式限值、保护顺序、DUT身份和 G4 批准。不会使用旧 GUI API 绕过独占服务，不打开 USB/VISA/串口，不发 I2C SCAN 或 DAP halt。不自动更换连接或重试未知输出命令。

## 结果与故障处理

结果位于 `projects/<project>/tests/<报告名>/runs/<stamp>/artifacts/reports/`：summary.md、checks.json、coverage.json、errors.jsonl、configuration.json、replay-input.json 和 manifest.json。每个文件关联 SHA256，配置冻结包含产品、规则、限值、工位、目录和用例；每个检查保存 SYS/PL 与量测、窗口、限值哈希。coverage 列出没有选择的全矩阵用例，空集合不 PASS。该 manifest 验证内容完整性，不提供数字签名或工业真实性认证。

同一用例使用独占 `wireless.active.json`，stop 只对该 attempt 生效。异常退出保留锁时，先确认原 Node 进程退出、保存证据，再人工移除该锁；不自动判断并抢占。当前无功率输出，stop 为下一读取边界退出，HTTP 单次请求上限为 case 剩余 deadline（默认 5 秒），不能视为硬实时保护。每次复测采用新 run stamp 和 attempt，首个失败存储在原目录。

## 验证和后续集成

```powershell
node --test test-tools/test/*.test.mjs test-tools/lib/*.test.mjs
node test-tools/test/wireless-charge-service.smoke.mjs bin/WiParse-Service.exe
```

模拟库生成器 `tests/generate-reference.py` 仅重建参考产品和模拟数据，禁止对正式产品执行。服务联调不访问硬件。工业启用前需补齐 Host 安全状态/设备共享/武装反馈、Dongle 时序捕获与健康、批准产品数据及物理工位，并完成契约 G2 至 G4；详细责任追踪见仓库 `docs/specs/wireless-test/05-plugin-implementation-review.md`。
