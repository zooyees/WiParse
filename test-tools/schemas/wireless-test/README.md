# 共同接口契约（开发版）

当前实现源：`automation-v1.schema.json`，JSON Schema 2020-12。使用具体类型时引用 `urn:wiparse:automation:1#/$defs/measurement`、`event`、`limits`、`check`、`job` 或 `timeMapping`。根对象只提供 `$defs`，不是一个可直接验证任何模型的联合类型。

共同职责见 `docs/specs/wireless-test/00-system-contract.md`；实际接口/限制见 `docs/AUTOMATION-SERVICE.md`。该版本覆盖已落地的SW基础模型，G0尚未联合冻结；不得据此自行推断Dongle二进制格式。RunContext、完整CaptureRecord、DeviceBinding、ErrorRecord和产品case关联等仍待冻结与实施。扩展应增加兼容测试并记录影响SW/PL/DG，不能只改固件或插件单边字段。

`seq`/`device_tick`/`host_monotonic_ns`使用十进制字符串。schema限制形状，运行时额外验证u64范围、顺序和有效映射范围。缺时钟不能补零；host/UTC可为null。开发输入暂兼容未带schema_version的事件，规范化输出加版本。

limits.approved及limits_hash目前是认证调用方声明，不是签名批准；生产前须完成冻结限值文件hash校验及批准链。`quality.issues`非空的数据不得按正常门限PASS。job.state是执行状态，不是产品判定。

PL附加记录为 `plugin-v1.schema.json`，使用 `urn:wiparse:wireless-plugin:1#/$defs/caseSpec`、`checkSpec` 或 `run`；该文件不替换公共automation模型，也不等于总契约全部RunContext/ErrorRecord已冻结。插件在配置加载时另做角色/限值/矩阵/强制项/路径和单调deadline检查。现有SW/DG字段和结果语义保持兼容，实物集成仍等待联合G0/G2评审。
