# WiParse 1.1.28 自动测试基础服务

2026-10-11。开发验证版，`industrial_approved=false`。本页描述已实现接口，不能代替 `specs/wireless-test/01-wiparse-software-spec.md` 的完整验收。职责和未完成项见 [实施评审](specs/wireless-test/04-software-implementation-review.md)。

## 启动与目录

发行包根目录运行 `start-automation-service.ps1`，或：

```powershell
.\bin\WiParse-Service.exe --data-root .\automation-data --port 7879
```

独立进程不依赖 GUI 生命周期。本机仅监听 `127.0.0.1`，每次启动生成访问 token，保存于数据根 `service-token.json`；不要分发或上传该文件。API 请求必须携带 `Authorization: Bearer <token>`。GUI 转发接口默认读取当前工作目录 `automation-data`；自定义目录时在启动 GUI 前设置 `WIPARSE_SERVICE_ROOT` 为同一绝对路径。

输入文件必须已经存在于数据根的 `projects/` 内，推荐 `projects/<project>/tests/<test>/runs/<attempt>/raw/`。规范化实际路径后检查，项目外文件和链接逃逸拒绝。任务台账为 `jobs.json`，结果/帧/规范化事件为 `jobs/<job_id>.*`，保留源与结果 SHA256。当前为项目级路径限制，尚未实现插件权限隔离和 run 级绑定；不能用此版开放不受信任插件。

服务根独占锁 `service.lock` 防止两个不同端口服务共写台账。正常停止用认证 `POST /v1/stop`，有活动任务时拒绝，先取消并查询结束。异常退出后锁会残留：确认原进程退出且无采集后，保存台账和证据，再手动删除这个锁文件；不得在活跃服务运行时删除。重启将 queued/running 的旧任务标为 unknown/SERVICE.INTERRUPTED，不自动重放命令。旧 lease/token/session_epoch 失效。

## 调用和完成查询

`POST /v1/invoke`：`{"method":"...","params":{...}}`；返回 `{"ok":true,"data":...}` 或 `{"ok":false,"error":"...","error_record":...}`。接受任务只返回 job_id，不代表任务完成。`job.get` 是完成状态的权威查询。`job.cancel` 设置 cancel_requested，终态为 cancelled；终态已确认结果保留，不能撤销已经发生的设备动作。

Node 客户端：

```javascript
import { AutomationClient } from './test-tools/lib/automation-client.mjs';
const client = new AutomationClient('./automation-data');
const accepted = await client.invoke('wave.analyze', {
  command_id: 'unique-attempt-step-command',
  path: 'projects/p/tests/t/runs/1/raw/wave.csv',
  sha256: '<actual-file-sha256>', channel: 0,
  analysis: 'electrical',
  options: { window: [0, 0.001], threshold_v: 1.5, burst_gap_s: 0.00005 },
  quality: { issues: [] }
});
const job = await client.wait(accepted.job_id);
// Read result only after checking job.state and verifying job.result.sha256.
```

同 command_id、method、params 返回同一任务；参数不同拒绝。不要超时后换 command_id 盲重发。4 个活动任务上限，其中波形分析/协议解码同一时间只运行一个；台账最多 1024 项、单任务参数 64 KiB。归档需停服务，保存完整数据根和哈希，再迁移至新数据根；尚无在线清理 API。

## 已实现方法

| 方法 | 参数/输出与限制 |
|---|---|
| system.capabilities / GUI automation.capabilities | 返回实现/不支持列表、8 种协议、开发档标记 |
| device.capabilities | 本机 serialport 资源与 USB 身份；verified=false，不能推断仪表校准或 Dongle IO 能力 |
| device.lease.acquire/release | resource、ttl_s 1–3600；返回 lease_token/session_epoch；活动采集不能释放或抢占 |
| job.get/cancel | job_id；状态 queued/running/succeeded/failed/cancelled/unknown，附取消请求、结果引用/错误 |
| wave.analyze | command_id、path/hash、channel；electrical/deadtime；options.window 为秒，门限/回差为 V |
| protocol.decode | command_id、path/hash、kind、channels、threshold_v/baud/scp_ui_us、window、byte_budget；离线有界解码，结果及帧 JSONL 落盘 |
| stream.validate | command_id、path/hash；规范化事件 JSONL 输入；逐记录校验、标记质量并输出事件文件/统计 |
| uart.capture | command_id、resource、lease_token、session_epoch、run_dir、baud、duration_ms；原始 COM 字节保存；非 Dongle IO 捕获 |
| time.mapping.fit | clock/reference_clock/boot/session IDs、tick_hz、sync_method、3–4096个{tick字符串,reference_s}、sync_uncertainty_s、max_residual_s、quality |
| time.mapping.convert | mapping、tick字符串、boot_id、capture_session_id；拒绝时钟断裂、坏质量、跨session或有效范围外外推 |
| check.evaluate | measurement、limits；返回标量 CheckResult；需由 PL 管理批准限值、状态窗/持续规则、总体覆盖 |

异步任务 `deadline_ms` 为 1–3600000，默认 120000。UART 超时读每 50ms 检查取消/lease；此周期不是 Windows 最坏时限保证。波形加载/解码内部暂不支持随时抢占，取消需等当前算法返回；不得用它作为实时保护或确定性停机。

### 波形与协议

结构化电压 min/max/pp/mean；指定门限后返回频率 min/max/median、周期 min/max、占空比 min/max。burst_gap_s 排除超过给定间隔的周期，PING/PT状态窗由 PL 给定。窗口内无完整周期返回 null + ANALYSIS.NO_CYCLES。支持 s/ms/us/µs/ns 和 V/mV/uV/µV，异常长度、非有限值、非递增时间与未知单位拒绝。

`deadtime` 需要不同 H/L 通道、同一时间基的实际原始 VGS，以及 provenance：capture_id、raw、synchronized、deskew_verified、high_deskew_s、low_deskew_s、uncertainty_s、max_pair_gap_s。输出 H→L/L→H 最小死区，包括负值交叠。无可信同步/deskew拒绝；不确定度小于采样间隔标质量异常。元数据是调用方证据声明，当前尚未接通示波器自动读回验证，不能仅设布尔值宣称实物合格。

文件上限 64 MiB、加载后总样本数上限 400万；超预算失败，不缩点后宣称全量。协议默认 byte_budget=65536，可设 512–1048576，frame 上限 262144；GUI仍使用512字节/2048帧显示预算。截断/无帧/decoder error 明示，源质量传播，不把 observed decoder frames 当真实PER分母。支持 UART/I2C/SPI/I2S/DDSSS/USB PD/SCP/UFCS；未知参数和错误字段类型拒绝，不静默回退；当前 API 只开放基础设置，传统 Qi ASK/FSK全量路径及增量跨包 streaming 未实现。

### 事件、时间和判定

事件唯一源为 `test-tools/schemas/wireless-test/automation-v1.schema.json#/$defs/event`。seq/device_tick 使用十进制 u64字符串，host_monotonic_ns/UTC允许缺失（规范化输出为null）；兼容开发输入省略 schema_version，输出自动加版本。连续性按 source/stream 分开，session/boot变化、缺口/重复、逆向tick、tick_hz变化均有质量标记。**它不是 Dongle 二进制 ICD**，不主动发 I2C SCAN、DAP halt 或输出刺激。

时钟映射仅使用明确同步观测点，拟合漂移，返回最大残差和不确定度，不使用主机收包时间冒充设备事件时间。超残差门限映射标质量并拒绝转换。只允许同步点覆盖范围内换算，不外推长期漂移。硬件同步通道及适用误差预算仍须 DG/测试负责人验证。

`check.evaluate` 拒绝未批准/无界限/错误单位/空样本/缺失值/坏质量/不确定度未知。区间完全越界 FAIL，与边界相交 INCONCLUSIVE，完全在范围内 PASS。approved/hash 是认证 PL 调用方的声明，本版不提供签名批准或哈希对限值文件的验证。频率/电压校准不确定度目前未自动建模，require_uncertainty=true 时仍会阻止这些指标PASS；不能为通过而省略真实不确定度。总体聚合函数在执行错误时保留已确认FAIL，未完成/空集合无PASS。

### 采集完整性

UART支持1200–2000000 baud、最长1小时、512MiB预算。成功与捕获中止均尽量刷新原始文件，并写 `<job_id>.capture.json` 包含实际字节/hash/complete/error；断电或磁盘失效可能无法写完，重启任务仍unknown。硬件时间戳和线错计数不可用被明确标记。服务不迁移旧GUI串口/示波器所有权，OS串口独占拒绝双开；在完成SW-01迁移前不允许新服务与旧GUI同时管理同设备。

## 验证与故障处置

```powershell
cargo test -p wiparse-core -p wiparse-service -p wiparse-gui --lib --bins
cargo build -p wiparse-service
node test-tools/test/automation-service.smoke.mjs target/debug/wiparse-service.exe
```

smoke只创建临时文件和虚拟资源lease，不连接设备。覆盖授权、fencing、64位时间、幂等、离线频率、质量/null门禁、路径边界、持久结果及正常停止。

| 错误族 | 处置 |
|---|---|
| LEASE.EXPIRED_OR_FENCED / DEVICE.BUSY | 查询旧任务/会话，确认采集停止后重新绑定；不抢占，不复用旧token |
| SERVICE.INTERRUPTED / unknown | 先保全台账/原始文件，检查设备真实状态和保护反馈，再由编排决定新attempt |
| DATA.HASH_MISMATCH / SOURCE_CHANGED | 停止使用产物，核对源文件/采集ID；不忽略hash继续判定 |
| STREAM.* / TIME.* | 定位规范化事件质量标记，重新同步/捕获；受损区间不得PASS |
| PROTOCOL.TRUNCATED / ANALYSIS.SAMPLE_BUDGET | 保留证据，分批准窗口离线分析或扩容后回归；不改名为完整统计 |
| DATA.JOB_COMMIT_FAILED / STORAGE_FAULT_LATCHED / 磁盘错误 | 台账写失败自动锁存拒绝新任务（health可查），保存现有文件，确认存储健康；未提交结果不视为成功 |

Windows服务安装、自动重启、GUI监控页、设备全迁移、保护协调、物理压力/72h测试尚未交付。软件包包含普通Windows进程，不是已安装Windows Service，也不是工业批准测试系统。
