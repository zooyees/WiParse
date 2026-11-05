# 插件开发工位：FPGA 固件测试插件改进方案

| 项 | 值 |
|----|----|
| 日期 | 2026-10-18 |
| 基线 | 插件 **0.5.0**，宿主 WiParse **1.2.2** |
| 读者 | 插件开发。只改本插件与 `recipes/fpga-firmware-test/` |
| 对照 | [软件工程工位方案](./软件工程-WiParse.md) |
| 依据 | 集成测试框架与 FPGA 插件结构评估 |

本工位不改 `crates/`、`test-tools/runner.mjs`、`test-tools/hub-workspace.mjs`、`test-tools/lib/hub/`、`backend/`。宿主适配器由软件工程实现并注入。

两边并行。字段名以软件工程方案第 2 节为准。

---

## 1. 本工位要达成的行为

1. 清单声明宿主协议，使 runner 知道要注入 `wiparse.fpga-host/1`。插件仍不自己构造硬件适配器。
2. `recipes/fpga-firmware-test/cases/` 中 20 个示例变为 `wiparse.fpga-test-case/2`。DUT 与仪器角色只出现在同目录旁的工程文件里。
3. 打开该配方后，工程页能载入种子草稿；用户保存并选择模拟后，预检和运行不再要求手写迁移。
4. schema/1 仍可通过 `planProjectMigration` 预览。直接运行 schema/1 继续失败。
5. 默认 `execution_mode` 保持 `hardware`。模拟仍须显式选择，缺适配器时仍是 `HOST.ADAPTER_REQUIRED`（宿主未注入之前）。宿主注入之后，该错误只表示协议未接通，不再是正常路径。

---

## 2. 冻结接口（本工位写入的部分）

`plugin.json` 增加：

```json
"host_adapter": "wiparse.fpga-host/1"
```

`ui.project_config` 增加：

```json
"seed": "{config_dir}/../project.json"
```

`project.json` 为 `wiparse.fpga-project/1`，仪器角色覆盖现有 20 项里出现过的全部角色：调试探针、直流电源复位、示波器、`serial_port` 或 `wiparse_dongle_uart`。同一 Dongle 序列号保持单项约束，与 `resource-resolver.mjs` 一致。不要把某一项的角色覆盖掉其他项需要的角色；冲突时拆成工程内多角色，而不是取第一项。

每个 schema/2 用例删除内嵌 `dut` 与 `instruments`，保留 id、名称、启停、顺序、标签、超时、固件、复位方式、日志窗、波形时点、规则。`id` 与现有文件名保持不变。

`host-adapter.mjs` 的方法列表本轮不增不减。不要新增 `ai` 为必需能力。

---

## 3. 改哪些文件

| 文件 | 改动 |
|------|------|
| `test-tools/plugins/fpga-firmware-test/plugin.json` | `host_adapter`、`project_config.seed`。版本改为 `0.5.1` |
| `recipes/fpga-firmware-test/project.json` | 新建。共享 DUT 与仪器角色 |
| `recipes/fpga-firmware-test/cases/*.json` | 20 个文件迁到 schema/2 |
| `recipes/fpga-firmware-test/README.md` | 写明：cases 是可运行示例；schema/1 仅作外部旧数据迁移源 |
| `test-tools/plugins/fpga-firmware-test/test/` | 增加：种子工程 + schema/2 用例在 `execution_mode=simulation` 下预检通过；抽一项运行得到模拟证据且 `simulation_only=true` |
| `planning/README.md` | 保持「提案不是运行时源」。不要把 proposal JSON 再生成进 `schemas/` |

迁移用现有 `planProjectMigration` 生成，再人工核对角色没有被第一项覆盖。生成结果提交进仓库，不要求操作工在界面上再迁一次这 20 项。

`lib/firmware.mjs` 继续拒绝 `download !== false`。`industrial_approved` 继续固定 false。`simulation-adapter.mjs` 继续只在 `execution_mode=simulation` 时返回对象。

不把 `index.mjs` 拆文件列入本轮，除非为接种子工程必须改预检读取路径。预检仍只认 `ctx.projectConfig`，不直接读 seed 文件。seed 由宿主放进草稿，用户保存后才进入 `ctx`。

---

## 4. 明确不做

- 不实现 `ctx.fpgaAdapter` 的硬件传输，不在插件里调用 `instrument.command`。
- 不修改 Hub 空态、能力目录、工作区 Rust。
- 不把日志种类扩到以太网或 RTT。
- 不把 `custom_adapter` 规则放行。
- 不把默认运行模式改成 simulation。
- 不把 `planning/*.proposal.json` 当成第二份 schema 源。
- 不宣称硬件验收。`PLUGIN_FREEZE` 的 `hardware_accepted` 保持 false，直到软件工程适配器与实机联合项通过后再改。

---

## 5. 验收

本工位可在宿主种子逻辑未合并时，用测试直接调用 `configure` / `preflight`，并传入与 `project.json` 相同的 `ctx.projectConfig` 和 `simulationAdapter`：

1. 20 个用例 `schema_version` 均为 `wiparse.fpga-test-case/2`，文件内没有 `dut`、`instruments`。
2. `project.json` 通过 `schemas/project.schema.json`。
3. 模拟预检 `ok !== false`，且不会请求硬件适配器。
4. 模拟运行至少 `fpga_01_pll_clock` 一项，证据带 `simulation_only=true`，报告含「模拟演示」。
5. 一份 schema/1 夹具调用 `planProjectMigration` 仍得到差异预览；同一份夹具直接 `preflight` 失败。
6. `download: true` 的用例预检失败。
7. `plugin.json` 含 `host_adapter` 与 `seed`，版本 `0.5.1`。

### 与软件工程的联合验收（双方都合并之后）

8. 从 Hub 打开本插件，工程页出现种子草稿；未保存时不能运行。
9. 保存工程，运行模式选模拟，勾选 `fpga_01_pll_clock`，预检后运行，矩阵该行有结论，报告为模拟。
10. 运行模式为硬件且宿主已注入适配器时，错误不是 `HOST.ADAPTER_REQUIRED`。
11. `hub-workspace-demo` 的空态没有 FPGA 用语。

第 8–11 条由软件工程组织一次联合运行。插件工位不单独打包 WiParse EXE。
