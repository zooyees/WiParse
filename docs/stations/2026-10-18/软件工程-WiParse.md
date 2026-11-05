# 软件工程工位：WiParse 改进方案

| 项 | 值 |
|----|----|
| 日期 | 2026-10-18 |
| 基线 | WiParse **1.2.2** |
| 读者 | 软件工程。只改宿主与 runner |
| 对照 | [插件开发工位方案](./插件开发-FPGA插件.md) |
| 依据 | 集成测试框架与 FPGA 插件结构评估（1.2.2 / 插件 0.5.0） |

插件开发工位不再改 `crates/`、`test-tools/runner.mjs`、`test-tools/hub-workspace.mjs`、`test-tools/lib/hub/`、`backend/`。本工位不改 `test-tools/plugins/fpga-firmware-test/**` 与 `recipes/fpga-firmware-test/**`。

两边并行。字段名以本文「冻结接口」为准，不等对方先提交。

---

## 1. 本工位要达成的行为

1. 插件清单声明 `host_adapter: "wiparse.fpga-host/1"` 时，runner 把实现了该协议的对象放进 `ctx.fpgaAdapter`。未声明的插件行为与今天相同。
2. 硬件预检不再因为「宿主没注入适配器」失败。仪器未连接、租约失败、串口被占用，由适配器抛出对应错误，而不是 `HOST.ADAPTER_REQUIRED`。
3. 配置工作区空态、新建提示、能力目录示例不再出现 FPGA、复位、波形、`fpga_smoke`、`fpga-firmware-test`。
4. 工作区在没有已保存工程时，若插件声明了 `ui.project_config.seed`，用该文件做工程草稿。路径必须落在 data_root 或插件允许的配置根内。

Hub 仍不解析 FPGA 用例、规则、固件哈希或判定。这些留在插件。

---

## 2. 冻结接口（双方共用）

### 2.1 清单字段

`plugin.json` 增加可选字符串：

```json
"host_adapter": "wiparse.fpga-host/1"
```

`ui.project_config` 增加可选模板，展开 `{config_dir}` 后必须落在配置仓库的父目录（配置根）内，不能再向上：

```json
"seed": "{config_dir}/../project.json"
```

`readManifest` 必须把 `host_adapter` 和现有 `ui` 一并放进 runner 的 plugin 对象。不要按 `plugin.id` 分支。

### 2.2 `ctx.fpgaAdapter`

仅当 `plugin.host_adapter === "wiparse.fpga-host/1"` 时赋值。对象满足插件 `lib/host-adapter.mjs` 已有要求，本工位不增删方法：

| 方法 | 宿主映射 | 失败时 |
|------|----------|--------|
| `version` | 固定 `wiparse.fpga-host/1` | — |
| `capabilities` | 含 `resource_lease`、`serial_handoff_raw_cursor`、`timed_capture`、`safe_cleanup`、`storage_check` | 缺一则不要注入半截对象 |
| `devices()` | `instrument.list` 加本机串口。每项带稳定 `serial` 或 `resource` 或 `port` 或 `dongle_serial`，以及 `kind`、`model` | 空列表是合法结果 |
| `preflight({config,resources,identity,station})` | 核对资源仍在 `devices()` 中；工位锁未被其他进程持有 | 抛错，信息含缺失角色 |
| `storage({minimum_free_bytes})` | 检查 data_root 所在卷剩余空间 | 不足则抛错 |
| `acquire({station_id,run_id,resources})` | 跨进程锁；日志角色从串口工具交接，记下起始字节游标 | 得不到锁返回抛错，不要返回 `false` 假装成功 |
| `release(lease)` | 释放锁，串口还回实时终端 | 必须可在进程退出后由下次启动清掉死锁（锁内记录 pid） |
| `sourceOutput(resource,channel,enabled)` | `instrument.command` `SourceOutput`，确认应答 | 设备不支持则抛错 |
| `voltage(resource,channel)` | `instrument.measure`，返回数值 | 无读数则抛错 |
| `probeReset(resource,{hardware:true})` | `instrument.command` `ProbeReset` | 未释放复位则抛错 |
| `collectLog({resource,start_mono_ms,end_mono_ms,dir,stopped})` | 从租约游标读取该时间窗原始字节 | 返回 `raw_base64`、`lines`、`start_cursor`、`end_cursor`、`dropped_lines`、`window_covered`、`complete`。超时或停止时 `complete:false`，不要丢游标 |
| `capture({resource,config,dir})` | `instrument.capture`，文件写入 `dir/shots` | 返回 `{screenshot:{path,sha256}}` |
| `waveformSource({resource,config,dir})` | `instrument.waveform_source`，文件写入 `dir/waves` | 返回 `{artifact:{path,sha256}}`。现有接口没有可靠 `metrics` 时不要编造；插件会得到 INCONCLUSIVE |
| `cleanup({safe_power_state})` | 按插件传入的安全状态关闭直流输出 | 返回可审计结果 |
| `cancelAndConfirm({operation_id,method,reason})` | 停止该操作对应的仪器任务并确认空闲 | 只有确认空闲才返回 `{quiescent:true}` |
| `measure(resource,channel)` | `instrument.measure` | 插件运行路径会调用 |

`ai` 本轮不实现。插件在未配置 provider 时会跳过；不要返回伪建议。

所有仪器调用走已经存在的 GUI HTTP（`WIPARSE_URL`）或 core 命令。超时使用插件传入的预算。不要用 `Promise.race` 留下仍在跑的仪器任务。

实现文件：`test-tools/lib/host/fpga-host-adapter.mjs`，由 `runner.mjs` 引用。GUI 工作区不 import 该文件。

### 2.3 工程种子

`workspace.open` 时，若当前测试还没有已保存的 `project_config`，且 `ui.project_config.seed` 指向的文件在路径 jail 内，把该 JSON 当作未保存草稿显示在工程页。用户仍要点保存后，预检才看得到 `ctx.projectConfig`。不要静默写入。

---

## 3. 改哪些文件

| 文件 | 改动 |
|------|------|
| `test-tools/runner.mjs` | `readManifest` 带上 `host_adapter`。创建 `ctx` 时按协议注入适配器 |
| `test-tools/lib/host/fpga-host-adapter.mjs` | 新文件。第 2.2 节全部方法 |
| `test-tools/schemas/plugin.schema.json` | 允许可选 `host_adapter` 字符串、`ui.project_config.seed` |
| `test-tools/lib/hub/manifest.mjs` | 校验 seed 路径落在允许根内 |
| `crates/wiparse-gui/src/test_tool/workspace.rs` | 空态与 ID 提示改为通用文案；打开时加载 seed 草稿 |
| `crates/wiparse-gui/src/backend/capabilities.rs` | `ui.test_tool.run.control` 示例插件改为 `hub-workspace-demo` |

不改：配色、`params[]` 旧插件面板、规则判定、用例 schema、`recipes/`。

空态文案：

- 标题保持「创建第一个测试配置」。
- 说明改为「按该插件的配置表填写，保存后执行预检。」
- 输入提示改为「配置 ID」。

---

## 4. 明确不做

- 不解析 `fpga-test-case` schema，不写 FPGA 判定。
- 不把 `execution_mode` 默认改成 simulation。默认仍由插件清单决定。
- 不实现精确时间轴控件。`rule_builder` / 字段级 `timeline` 保持现有摘要列表加详情。
- 不实现 `ai`。
- 不修改插件目录和 20 个用例文件。
- 不把无线充电检验改成矩阵插件。

---

## 5. 验收

1. `hub-workspace-demo` 无 `host_adapter` 时，runner 的 `ctx` 没有 `fpgaAdapter`，面板与今天相同。
2. 声明了该协议的插件在硬件预检时，错误码不是 `HOST.ADAPTER_REQUIRED`。无仪器时错误来自 `devices` / `preflight`。
3. 两路 `acquire` 同一 `station_id`，第二路失败，第一路 `release` 后可以再获取。
4. `collectLog` 返回的 `raw_base64` 与游标区间字节一致。
5. `capture` / `waveformSource` 的文件在插件给的 `dir` 下，`sha256` 与文件一致。
6. `cancelAndConfirm` 在任务未停时不返回 `quiescent:true`。
7. 新建配置空态中英文都没有「复位」「波形」「fpga」。
8. `GET /v1/capabilities` 的 `run.control` 示例插件 id 为 `hub-workspace-demo`。
9. 仅有 seed 文件、用户未保存时，预检仍要求先保存工程。保存后 `ctx.projectConfig` 与该文件内容一致。

与插件工位的联合项见插件方案第 5 节后半。本工位先独立完成 1–9，不阻塞在用例迁移上。
