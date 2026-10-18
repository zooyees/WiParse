# Testing Hub 通用表单声明

本协议是 `wiparse.config-ui/1` 的兼容扩展，由宿主实现，插件只声明业务语义。字段均可选；不识别的必需控件必须阻断配置工作区，不能伪装成功。

```json
{
  "fields": {
    "name": {"order": 10, "width": "normal", "label_zh": "名称"},
    "timeout_s": {"order": 20, "width": "short"},
    "id": {"order": 90, "advanced": true},
    "rules.*.pattern": {"order": 10, "width": "long"},
    "rules.*.type": {"enum_labels_zh": {"log_contains": "日志包含文本"}}
  },
  "widgets": {"rules": {"type": "rule_builder", "summary_fields": ["pattern", "minimum", "maximum", "unit"]}},
  "required_widgets": ["rule_builder"]
}
```

`fields` 和 `widgets` 的键是点分路径；`*` 只匹配数组索引，精确路径优先。`order` 按同一对象内升序排列，未声明项排后，名称作为稳定次序。`width` 为 short（220）、normal（320）、long（可用宽度），单位是逻辑像素；空间允许时相邻短字段两列，窄屏堆叠。标签在控件上方，单位只呈现一次。

`label`/`label_zh`、`description`/`description_zh` 覆盖 schema 的标题和说明。`enum_labels`/`enum_labels_zh` 是原始枚举值到显示标签的映射，不改变存储值。`advanced` 将已有字段移入集中高级区；缺失字段统一从添加菜单显式创建，不因绘制而写入默认值。创建时才应用 schema 默认值。高级区提供显式移除非必需字段；必需字段不能移除。

`hidden` 只隐藏展示，不能删除值、修正值或跳过校验。schema `const` 默认不重复显示；错误的已有常量保持原值并显示错误。`readOnly` 禁止编辑。类型切换是显式操作，保留公共字段，初始化新分支所需字段，移除不适用的旧分支字段。

`rule_builder` 和 `timeline` 数组组件呈现摘要列表及当前所选项详情。`summary_fields` 是相对于数组项的点分路径数组，补充摘要；仅展示标量，长度受限。摘要优先显示 name/label，否则显示本地化类型标题。时间轴预览由原有预览区提供，不能以此编辑器宣称新增精确时间轴功能。

`reference_selector` 以 source_path 指向配置根的数组或对象，数组取 value_field（默认id），对象取键，label_field（默认name）用于显示。选择或手工输入仅更新引用值；未解析引用给出提示，由预检校验。

支持的必需控件：file/path/file_picker/path_picker、tags/tag_input、discriminated_union、timeline、rule_builder、reference_selector、serial_port/serial_selector/serial_port_selector、instrument_selector、read_only_summary/json_read_only。缺少必需控件拒绝打开；未知非必需控件可降级为类型化表单，并在界面显示兼容提示。禁止可执行表达式或远程 UI 资源。

`global_actions: ["ai_settings"]` 在校验区提供全局 AI 设置入口；密钥仍只归全局配置管理。

文件字段遵循 schema 的 x-path-base（config_dir 或数据根）。file widget 可声明 hash_field 点分路径。浏览或点击文件校验按钮是显式操作：后台 worker 在批准目录边界内流式计算 SHA256、检查读取期间文件是否变化，成功后仅当当前配置和原字段值仍匹配才应用相对路径与哈希。检查不自动保存，不替代预检和运行复核；不把打开页面当作文件校验。目录选择不计算文件哈希。
