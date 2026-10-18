# Hub 工程共享配置协议 v1

宿主唯一维护通用存储、事务、API、GUI 与冻结。插件维护业务 schema/UI、迁移计划及冻结值解析。新增能力要求 WiParse >=1.2.2。

## 声明与存储

插件声明 `ui.project_config = {"protocol":"wiparse.project-config/1","schema":"schemas/project.schema.json","ui_schema":"schemas/project.ui.json"}`。schema/UI 位于插件目录，只读。实例位于 `{test_dir}/.hub/{plugin_id}/project-config.json`，按 data_root/project/test/plugin 隔离，不使用 station.json 保存用户设置。

实例与 `ctx.projectConfig` 使用同一格式：

```json
{"schema_version":"wiparse.project-config/1","project_id":"default","test_id":"FpgaFirmwareTest","plugin_id":"fpga-firmware-test","revision":1,"schema_hash":"<host schema fingerprint>","sha256":"<canonical envelope excluding sha256>","value":{"name":"示例工程","description":"","dut":{},"instruments":{}}}
```

`value` 的字段完全由插件 schema 定义。缺少实例时 project.get 返回 saved:false、revision:0、sha256:""、默认 value 及错误；默认值不得视作已保存。预检/运行要求保存且校验通过。

## API 与验证

`project.get` 返回实例、saved、schema、documents、ui、errors。`project.save` 参数 `{value,expected_revision,expected_sha256}`；宿主事务原子提交、审计、使预检失效。旧版本写入冲突拒绝；活动运行期间拒绝修改。`project.validate` 校验 schema/路径及业务。

插件可导出 `configureProject(ctx)`，ctx.overlay.project_config 为候选完整实例；返回 `{lifecycle:"configure",ok,errors:[],warnings:[]}`。configure 用例仍调用 `configure(ctx)`，并额外传入 `ctx.projectConfig` 与 `ctx.overlay.project_config`。两类 configure 无设备权限。

## 冻结、恢复与结果

manifest 的 `project_config` 为 `{revision,sha256,schema_hash}`，参与 manifest_sha256。内存冻结对象另有 `project_config_snapshot` 为完整实例。预检与运行插件均通过 **ctx.projectConfig** 读取冻结完整实例，禁止回读可变工程文件。run/config-manifest.json 仅保存元数据，run/project-config.json 保存完整实例；恢复需校验二者及当前内容，不能仅校验用例。报告和矩阵必须携带 project_config 元数据；矩阵顶层允许 project_config，条目 result_ref 指向结果 JSON。

矩阵当前结论必须同时匹配稳定 ID、config_sha256、project_config.sha256；缺失共享哈希的旧结果保留为历史，当前状态为 TBT。运行状态不能用旧结果补成 PASS。

## 迁移交接

插件负责纯计算导出 `planProjectMigration({cases,projectConfig})`，返回 `{ok,conflicts:[],project_value,items:[{id,value}]}`，不得写文件或调用设备。宿主 `project.migration.preview` 返回候选、冲突与计划 sha256；`project.migration.commit` 必须提供预览 sha256，重新计算并核对全部旧用例/共享版本，在同一事务备份、写入共享实例及升级用例。冲突拒绝提交、重复执行不重复迁移。插件只按明确协议移除 dut/instruments，保留名称、ID、单项超时等，异值列出每项字段差异。

业务 schema 的最小样例由插件提供；宿主不内置 FPGA 字段与默认仪器。宿主将提供迁移预览/显式提交按钮，原始配置不自动迁移。

## 工程锁与结果适配

声明共享配置时，editorRoot 固定为 project/test/plugin/project-scope；同范围更换仓库仍共用 mutex、journal、预检及运行锁。journal 绑定原 repository 和 auditFile，恢复路径须仍在当前授权根，且不能位于插件安装目录。不同工程/test/plugin 隔离。迁移备份在共享实例同目录 backups；预览仅返回 items 的 ID/名称摘要，计划哈希仍绑定完整候选。500 项及字节预算对迁移同样适用。

插件声明 ui.result_details={protocol:"wiparse.result-details/1",schema_version:"<plugin result schema>"}。结果 JSON 必须包含声明版本、scope{project_id,test_id,plugin_id}、item_id、run_id、numeric attempt、config_sha256，与矩阵匹配；矩阵有 attempt_id 时结果 UUID 必须一致；project_config 三个元数据字段必须与矩阵相同。后台仅读取当前 testDir 内有界 JSON，旧数据缺身份显示 RESULT.UNVERIFIABLE_HISTORY，保留历史原文件查看入口，不认作当前核验结果。FPGA 新数据使用 wiparse.fpga-result/2。
