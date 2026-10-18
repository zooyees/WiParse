# FPGA 工程矩阵优化实施与验收

2026年10月18日。对应 `fpga-project-matrix-optimization-2026-10-18.md`，交付软件1.2.2 / 插件0.5.0 EXE预览；完整ZIP须用户确认后进行。

## 实施边界

软件工程独占通用workspace、Hub库、runner、API及Cargo。插件工位独占业务schema/UI、业务解析和迁移；系统架构只读复核接口、隔离与恢复边界。项目协议见 `../HUB_PROJECT_CONFIG_CONTRACT.md`。

矩阵主关系为当前工程配置，运行只按稳定ID叠加。当前配置或共享哈希变化使旧结论过期，历史数据不改写。未选中且未执行、无结果的行显示待测试，运行源中的SKIPPED仍保留在历史；选中后真实SKIPPED、ERROR、ABORTED等保持原义。已移除项不计入当前工程总数。

工程设置不存入插件station.json；共享实例位于当前test目录的`.hub/<plugin>/project-config.json`。声明共享协议时，同一project/test/plugin各仓库共用project-scope锁、预检、journal及runbinding，避免换目录绕过CAS或活动运行保护。journal保留原repository与auditFile，恢复仍校验授权根和安装目录保护。

迁移发现原20项有三组绑定，不取首项合并。经架构确认，集中建立COM日志、Dongle日志、电源等角色，各项只引用实际需要角色，端口/通道/设备身份在工程设置统一维护。预览输出原差异及映射，显式提交时校验全部旧项及共享版本，原子写入备份、升级项与共享实例；失败回滚，重复迁移幂等。源recipes20项没有自动修改。

结果读取在后台执行，JSON限512KiB，路径限制当前test目录；声明结果协议且必须匹配scope、item、run、numeric attempt、可用UUID、raw配置哈希和共享元数据。缺身份显示历史不可核验；快速切换通过ID/run/attempt检查丢弃迟返回。

## 已完成回归

持久证据目录：`evidence/release-1.2.2-2026-10-18`，路径以仓库根为基准。

|范围|结果|证据|
|---|---|---|
|Rust完整工作区|426通过；核心1项依赖样本忽略；GUI2项已单独执行通过|rust-tests.log、ui-frames.log、rule-pointer.log|
|Node完整组合|251通过，0失败|node-tests.log|
|插件专项|160通过，含112故障与18工程共享专项|插件evidence/plugin-regression-0.5.0-2026-10-18.json/.log|
|MCP|版本1.2.2，6工具、20Hub及4报告路由|mcp-tests.log|
|工程隔离与事务|7项专项通过，含双仓库CAS/恢复/活动run阻断、错scope/attempt/hash|project-host.log|
|矩阵/草稿/容量/迟返回|0/5/20/500、草稿生命周期及指针选择独立回归通过|rust-tests.log|
|20项正向/负向|20 PASS / 20 FAIL，各62个有效证据链接|fpga-20-pass.log、fpga-20-fail.log|
|20项仅选2项|2 PASS，18未选源状态保留，当前工程仍20项|fpga-subset.log|
|实际UI schema渲染|52帧通过；宽窄屏、中英文及明暗主题选定场景|ui-frames.log及截图目录|

架构审查曾发现跨仓库共享锁绕过、结果来源可缺失、迁移容量绕过三项问题，均修正并复审通过。未将源码复审等同于重新执行测试。

插件最终冻结83文件（含20项迁移源），PLUGIN_FREEZE SHA-256：`497f2e8917075cbd01b2b38ae6d6546033328c6359846bc2116a50bce6694344`。宿主冻结107份运行/插件文件、125份Rust构建输入，source-freeze SHA-256：`0e767517855bedbfd783492ebc459d76eb215f21d0a2cd3c8c27eb8dc116d6d2`。最终EXE构建/实际运行/停止和预览目录复验另记录到证据摘要。

## 保留边界

模拟运输明确标识，不更改限值制造通过。真实硬件、长期老化、实际500项连续执行、精确T0以及完整工业认证仍未验收；industrial_approved=false。旧历史不伪造迁移后PASS。EXE预览不等同于完整ZIP验收。

## 最终EXE与预览目录复验

发行构建完成，GUI版本1.2.2，SHA-256 `42481492ad67338298a54e89d000952b5dcee0a42559ba445076539db209244d`；保留匹配PDB，LTO=false/codegen-units=16，与构建档案一致。

实际EXE在隔离工程20项模拟全部PASS；MD、PNG、CSV、串口日志、JSON五类证据均能打开，相对路径没有改写，工作目录为外部临时目录。项间暂停确认、继续启动第二项、Stop取消当前项均通过。随后从最终预览目录的EXE重新运行同样20项，自动定位资源根、便携路径与五类报告跳转均通过。

预览目录107份受冻结约束的运行/插件文件与源码逐字节匹配，EXE与构建产物一致。证据见gui-runtime.log、gui-controls.log、preview-runtime.log及verification-summary.json。交付 `dist/wiparse-1.2.2-preview/bin/WiParse.exe`；没有生成本轮完整ZIP，须用户确认效果后才进入完整打包。
