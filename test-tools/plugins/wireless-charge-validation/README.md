# 无线充电自动检验插件

插件支持配置检查、模拟回放及实采记录的只读核对。**当前 live 实机执行不可用**：外部台站执行层尚未实现并证明本轮 TX 日志、RX 就绪与 FT4222 I²C 操作之间的强制门禁，预检和采集入口都会返回 `INTEGRATION.TX_RX_FT4222_GATE_NOT_IMPLEMENTED`。仅有能力声明或批准配置不能解除该阻断。内置服务的能力清单见 system.capabilities。

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

### 实机工位接入（尚未启用）

以下是后续接入所需的条件和拟定流程，并非当前可用的实机功能。工位服务需要通过认证接口声明 live_contract=wiparse.fenced-wireless/1、工位 ID、审核过的 station.json SHA-256、看门狗核验状态和实际方法清单；还必须实现并验证逐次 RX 就绪门禁，不能仅凭能力声明放行。配置中的 station.live.resource 对应唯一夹具。产品限值、保护及停止顺序、角色绑定和采样时间预算分别批准。默认配置保留 UNCOMMISSIONED。

计划中的每个用例独立获取租约，调用 capture.arm、dongle.capture.start、capture.status、stimulus.apply；当前代码在发出这些设备请求之前即阻断。响应应对应本次工位、用例、运行、尝试、DUT 序列号和固件。加电前须回读示波器布防、Dongle 就绪、初态安全和保护健康。输出命令使用唯一 ID，迟到或丢失回执不重发。采集应返回同一 capture_id 的观测、原文、源坐标和摘要。完成、超时和取消时须安全退出，并在收到独立安全反馈后释放租约；工位固件负责看门狗、掉线保护及断电后的恢复。这些行为须在实际执行层实现并通过验证后才可声明支持。

当前仓库附带的设备服务未声明该实机契约。内置站点和合成测试不能取得产品验收资格。连接用户工位后仍需验证实物接线、独立反馈、掉线安全和同步误差。

## 结果与故障处理

结果位于 `projects/<project>/tests/<报告名>/runs/<stamp>/artifacts/reports/`：summary.md、checks.json、coverage.json、errors.jsonl、configuration.json、replay-input.json 和 manifest.json。每个文件关联 SHA256，配置冻结包含产品、规则、限值、工位、目录和用例；每个检查保存 SYS/PL 与量测、窗口、限值哈希。coverage 列出没有选择的全矩阵用例，空集合不 PASS。该 manifest 验证内容完整性，不提供数字签名或工业真实性认证。

同一用例使用独占 `wireless.active.json`，stop 只对该 attempt 生效。异常退出保留锁时，先确认原 Node 进程退出、保存证据，再人工移除该锁；不自动判断并抢占。当前无功率输出，stop 为下一读取边界退出，HTTP 单次请求上限为 case 剩余 deadline（默认 5 秒），不能视为硬实时保护。每次复测采用新 run stamp 和 attempt，首个失败存储在原目录。

## 验证和后续集成

```powershell
node --test test-tools/test/*.test.mjs test-tools/lib/*.test.mjs
node test-tools/test/wireless-charge-service.smoke.mjs bin/WiParse-Service.exe
```

模拟库生成器 `tests/generate-reference.py` 仅重建参考产品和模拟数据，禁止对正式产品执行。服务联调不访问硬件。工业启用前需补齐 Host 安全状态/设备共享/武装反馈、Dongle 时序捕获与健康、批准产品数据及物理工位，并完成契约 G2 至 G4；详细责任追踪见仓库 `docs/specs/wireless-test/05-plugin-implementation-review.md`。

## 导入工位采集记录

在集成测试中选择“工位记录（只读）”，填写记录 ID。在当前项目的测试目录内创建 `captures/<ID>.json`。ID 可使用字母、数字、下划线和连字符。单个文件最大 8 MiB。以下是格式示例，数值只演示字段含义。

```json
{
  "schema_version": "wiparse.station-observation/1",
  "simulation_only": false,
  "station_id": "bench-A",
  "dut_serial": "sample-001",
  "firmware": "dev-1",
  "clock_domain": "scope-1",
  "cases": { "ADP01": { "ADP01.01": { "evidence_refs": ["e1"] } } },
  "evidence": {
    "e1": { "source": "capture-001", "coordinate": "sample_0based", "position": 120, "raw": "1.03" }
  }
}
```

支持从 1 计的行号、从 0 计的样本编号和字节偏移，分别使用 `line_1based`、`sample_0based` 和 `byte_0based`。工位负责导出真实片段、物理单位与时钟信息。检查 ID 取自产品场景定义。导入会记录原文件摘要，并在报告目录保存证据副本与复核索引。已找到原文的检查标记为待复核；缺少对应原文的检查标记为缺测。该模式全程只读，产品结论保留为 INCONCLUSIVE。

## 对比存档

运行 `node lib/compare-runs.mjs <前一报告目录> <后一报告目录>` 得到逐检查项 JSON 对照。程序核对存档 SHA-256 和运行身份。配置、模式、信号通道、单位及采样窗匹配，且两侧均有有效标量判定时才计算差值。配置变化、缺失项目与不可比较项保留在结果中。
