//! Trusted built-in engineering skill descriptions.
//!
//! Selection provides guidance only. Host tool authorization remains authoritative.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Skill {
    pub id: &'static str,
    pub version: u32,
    pub title: &'static str,
    pub triggers: &'static [&'static str],
    pub tools: &'static [&'static str],
    pub guidance: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct SkillContract {
    pub id: &'static str,
    pub inputs: &'static str,
    pub output: &'static str,
    pub stop: &'static str,
    pub effect: &'static str,
    pub steps: &'static [&'static str],
    pub excludes: &'static str,
    pub compatibility: &'static str,
}

pub const CONTRACTS: &[SkillContract] = &[
    SkillContract { id: "serial_log", inputs: "已选日志与范围", output: "覆盖率、行号及日志证据", stop: "来源变化、取消或读取失败", effect: "read", steps:&["确认来源与权限","本地扫描及续读","引用覆盖和原始行"], excludes:"用户排除日志或禁止读取",compatibility:"仅宿主内置工具；恢复必须核验来源与版本" },
    SkillContract { id: "waveform", inputs: "已加载波形、通道与测量区间", output: "单位、采样质量与测量证据", stop: "数据缺失、来源变化或取消", effect: "read", steps:&["确认波形和单位","按实际区间测量","报告分辨率与限制"],excludes:"用户排除波形或禁止测量",compatibility:"仅使用已加载且来源可核验的波形" },
    SkillContract { id: "qi_protocol", inputs: "已解码通信帧与波形", output: "帧范围、错误与来源", stop: "缺少帧数据、来源变化或取消", effect: "read",steps:&["确认解码配置与来源","按偏移续读帧","核对范围和完整性"],excludes:"用户排除 Qi 或协议帧",compatibility:"需实际 DDSSS 解码帧，不把其他协议当作 Qi" },
    SkillContract { id: "test_result", inputs: "当前运行编号、限值和报告", output: "运行证据、缺失项与原始判定", stop: "运行或报告不可用", effect: "read",steps:&["确认报告与运行","按游标续读","核对实际限值和判定"],excludes:"用户排除报告或结果",compatibility:"历史报告不自动关联当前运行" },
    SkillContract { id: "charging_protection", inputs: "充电日志、波形及实际时钟", output: "关联事件、覆盖和不确定性", stop: "来源或时间基准不可核验", effect: "read",steps:&["扫描充电事件","检查关联波形","核对时间基准与不确定性"],excludes:"用户排除充电保护分析",compatibility:"未同步的时钟不能直接关联或断言因果" },
    SkillContract { id: "scope_capture", inputs: "已连接示波器、具体采集目标", output: "截图或源波形采集回执与设备状态", stop: "未授权、忙碌、取消或结果未知", effect: "device",steps:&["确认设备和采集能力","确认具体动作","读取回执与状态"],excludes:"用户禁止设备操作",compatibility:"仅限宿主现有设备接口及会话；源波形由宿主安全归档" },
    SkillContract { id: "test_execution", inputs: "已选项目、测试配置与预检", output: "运行编号、状态和结果", stop: "预检失败、未授权、取消或结果未知", effect: "test",steps:&["检查项目配置和设备","执行并验证预检","确认启动并等待同一运行"],excludes:"用户禁止测试执行",compatibility:"需支持绑定的 Testing Hub 插件与当前预检" },
];

fn valid_contract_text(value: &str) -> bool { !value.trim().is_empty() && value.len() <= 4096 }

pub fn validate_registry() -> Result<(), String> {
    use std::collections::HashSet;
    let mut ids = HashSet::new();
    if BUILTINS.is_empty() || BUILTINS.len() > 32 || BUILTINS.len() != CONTRACTS.len() {
        return Err("SKILL.REGISTRY_SIZE".into());
    }
    for skill in BUILTINS {
        if skill.id.is_empty() || skill.id.len() > 64 || !skill.id.bytes().all(|b|b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'_')
            || skill.version == 0 || skill.title.trim().is_empty() || skill.title.len()>256
            || skill.guidance.trim().is_empty() || skill.guidance.len()>4096 || skill.triggers.is_empty()
            || skill.triggers.len()>32 || skill.triggers.iter().any(|term|term.trim().is_empty() || term.len()>128)
            || skill.triggers.iter().collect::<HashSet<_>>().len()!=skill.triggers.len()
            || skill.tools.is_empty() || skill.tools.len()>64
            || !ids.insert(skill.id) {
            return Err(format!("SKILL.INVALID: {}", skill.id));
        }
        let contracts: Vec<_> = CONTRACTS.iter().filter(|item| item.id == skill.id).collect();
        if contracts.len() != 1 || [contracts[0].inputs, contracts[0].output, contracts[0].stop,
            contracts[0].excludes, contracts[0].compatibility].iter().any(|field| !valid_contract_text(field))
            || contracts[0].steps.is_empty() || contracts[0].steps.len()>8
            || contracts[0].steps.iter().any(|step|step.trim().is_empty() || step.len()>256)
            || !matches!(contracts[0].effect, "read" | "device" | "test") {
            return Err(format!("SKILL.INVALID_CONTRACT: {}", skill.id));
        }
        let mut methods = HashSet::new();
        for method in skill.tools {
            if !methods.insert(*method) || (crate::engineering_agent::tool(method).is_none()
                && !matches!(*method, "log.files" | "log.analyze" | "test.status" | "instrument.overview" | "engineering.wave.measure" | "engineering.wave.frames")) {
                return Err(format!("SKILL.UNKNOWN_TOOL: {method}"));
            }
        }
    }
    Ok(())
}

pub fn contract(id: &str) -> Option<&'static SkillContract> {
    CONTRACTS.iter().find(|item| item.id == id)
}

pub fn english_title(id: &str) -> &'static str {
    match id {
        "serial_log" => "Serial log analysis",
        "waveform" => "Waveform analysis",
        "qi_protocol" => "Qi protocol analysis",
        "test_result" => "Test result analysis",
        "charging_protection" => "Charging protection and restart",
        "scope_capture" => "Oscilloscope capture",
        "test_execution" => "Test execution",
        _ => "Unknown skill",
    }
}

pub fn english_contract(id: &str) -> (&'static str, &'static str, &'static str) {
    match id {
        "serial_log" => ("Selected log and range", "Coverage, line numbers and evidence", "Source change, cancellation or read failure"),
        "waveform" => ("Loaded waveform, channel and interval", "Units, sample quality and measurement evidence", "Missing data, source change or cancellation"),
        "qi_protocol" => ("Decoded frames and waveform", "Frame scope, errors and provenance", "Missing frames, source change or cancellation"),
        "test_result" => ("Run, limits and report", "Run evidence, gaps and original verdict", "Run or report unavailable"),
        "charging_protection" => ("Charging logs, waveforms and actual clocks", "Associated events, coverage and uncertainty", "Source or clock cannot be verified"),
        "scope_capture" => ("Connected scope and explicit target", "Capture receipt and device state", "No grant, busy, cancellation or unknown outcome"),
        "test_execution" => ("Selected project, configuration and preflight", "Run ID, status and results", "Preflight failure, no grant, cancellation or unknown outcome"),
        _ => ("", "", ""),
    }
}

pub const BUILTINS: &[Skill] = &[
    Skill { id: "serial_log", version: 1, title: "串口日志分析", triggers: &["串口日志", "串口数据", "日志", "serial log", "serial data"], tools: &["log.files", "log.analyze"], guidance: "核查来源和覆盖范围，引用日志行及完整性。" },
    Skill { id: "waveform", version: 2, title: "波形分析", triggers: &["波形", "waveform"], tools: &["wave.channels", "wave.measure", "wave.timing", "wave.frames", "engineering.wave.measure", "engineering.wave.frames"], guidance: "核查通道、单位和测量区间，引用原始波形测量。PWM 或死区时间应使用实际阈值、同步通道和采样分辨率；限值由用户指定。" },
    Skill { id: "qi_protocol", version: 2, title: "Qi 协议分析", triggers: &["qi协议", "qi 协议", "qi protocol"], tools: &["wave.channels", "wave.frames", "wave.decode", "engineering.wave.frames"], guidance: "使用实际解码帧及时间范围，区分通信异常与缺失数据。分段回执 partial=true 时按 next_offset 续读；宿主在当前任务中保留精简覆盖记录，可引用当前分段由宿主核验同一来源、过滤条件和总数的连续覆盖。说明任何 contains 或 errors_only 筛选仅覆盖匹配帧，不代表全部帧。原文淘汰后不得声称仍可查看原始帧。" },
    Skill { id: "test_result", version: 3, title: "测试结果分析", triggers: &["测试报告", "测试结果", "test report"], tools: &["report.read", "test.status"], guidance: "核对运行编号、实际限值、结果和证据，保留原始判定。报告分段时使用 next_cursor 续读至 reached_eof；宿主在当前任务中保留精简覆盖记录，可引用当前分段由宿主核验同源连续覆盖。不能将部分正文当作完整报告。与测试执行组合时，先等待本轮运行完成，读取该运行清单明确列出的报告；无宿主运行关联的历史报告只能作为部分证据，不推定本轮结果。" },
    Skill { id: "charging_protection", version: 3, title: "充电保护与复充", triggers: &["复充", "断充", "充电中断", "充电保护", "recharge", "charging protection", "charging interruption"], tools: &["log.files", "log.analyze", "wave.channels", "wave.measure", "wave.frames", "engineering.wave.measure", "engineering.wave.frames"], guidance: "区分本轮和历史证据，不将未同步的时间直接关联；只有真实的公共边界事件及误差约束能支持有限的跨时钟估计，不能据此断言因果。" },
    Skill { id: "scope_capture", version: 2, title: "示波器辅助采集", triggers: &["示波器采集", "示波器源", "示波器波形源", "读取示波器波形", "scope capture", "scope waveform source"], tools: &["instrument.list", "instrument.overview", "instrument.capture", "instrument.waveform_source", "instrument.readback", "wave.channels", "wave.measure", "engineering.wave.measure"], guidance: "确认已连接设备与采集目标，使用宿主的设备控制授权和回执，未知结果不重试。" },
    Skill { id: "test_execution", version: 3, title: "测试执行辅助", triggers: &["执行测试", "run test"], tools: &["testing.inspect", "testing.status", "testing.cases", "testing.preflight", "testing.run", "testing.pause", "testing.resume", "testing.stop", "testing.wait", "report.inspect", "report.read"], guidance: "先检查所选测试和预检结果，执行时遵循宿主测试锁和具体操作确认。只能暂停、继续或停止本任务的运行；暂停和继续是请求，需读取实际矩阵状态。启动回执不是运行完成；对本次运行编号调用 testing.wait，进程失败与产品判定必须分别说明。" },
];

/// Candidates are suggestions and do not grant access or select a device.
pub fn candidates(question: &str) -> Vec<&'static Skill> {
    let text = question.to_lowercase();
    BUILTINS.iter().filter(|skill| skill.triggers.iter().any(|term| text.contains(term))
        && !skill_excluded(question,skill.id)).collect()
}

/// Do not automatically route or collect a domain the user explicitly excluded.
/// Clause boundaries prevent a negation of one domain from suppressing another.
pub fn skill_excluded(question:&str,id:&str)->bool {
    let Some(skill)=BUILTINS.iter().find(|skill|skill.id==id) else{return false;};
    let text=question.to_lowercase();
    let aliases:&[&str]=match id {
        "serial_log"=>&["日志","串口","log"],
        "waveform"=>&["波形","waveform"],
        "qi_protocol"=>&["qi","协议帧"],
        "test_result"=>&["报告","测试结果","report"],
        "charging_protection"=>&["断充","复充","保护","recharge"],
        "scope_capture"=>&["示波器采集","示波器截图","示波器源","scope capture","scope waveform source"],
        "test_execution"=>&["执行测试","启动测试","运行测试","测试执行","run test","start test"],
        _=>&[],
    };
    // A contrast starts a new intent even without punctuation: in
    // “不要分析日志但分析波形” the negation must not exclude waveform.
    // Preserve other conjunctions so “不要日志和波形” still excludes both.
    let clauses = text.replace(" but ", "，").replace(" however ", "，");
    clauses.split(['，',',','。','；',';','\n','但','却']).any(|clause| {
        skill.triggers.iter().chain(aliases.iter()).flat_map(|term|clause.match_indices(*term).map(|(at, term)| (at, term))).any(|(at, term)| {
            let before=&clause[..at];
            ["不要","无需","不需要","不用","不分析","不读取","不读","不看","不执行","不启动","不运行","不采集","别分析","别执行","别启动","别运行","别采集","排除","do not ","don't ","without ","exclude "]
                .iter().any(|negation|before.contains(negation))
                || (matches!(id, "test_execution" | "scope_capture")
                    && (before.ends_with('不') || before.ends_with('别'))
                    && ["执行", "启动", "运行", "采集"].iter().any(|verb| term.starts_with(verb)))
        })
    })
}

pub fn measurement_excluded(question:&str)->bool {
    let text=question.to_lowercase();
    ["不要测量","不用测量","无需测量","不测量","do not measure","don't measure","without measuring"]
        .iter().any(|term|text.contains(term))
}

pub fn explanatory_only(question: &str) -> bool {
    let lower = question.trim().to_lowercase();
    let explicit_only = ["仅解释", "只解释", "only explain"].iter().any(|term| lower.starts_with(term));
    let question_prefix = ["什么是", "如何使用", "你能做什么", "what is"].iter().any(|term| lower.starts_with(term));
    // A conceptual question can be followed by an independent request for
    // evidence or an action. Do not classify the entire turn as explanation
    // only merely because its first clause asks for an explanation.
    let follow_up_action = ["并检查", "并分析", "并读取", "并搜索", "并执行", "并启动", "同时检查", "同时分析", "然后检查", "然后分析", "顺便检查", "顺便分析", "，检查", "，分析", ", check ", ", analyze ", " and check ", " and analyze ", " and inspect ", " and read ", " and run "]
        .iter().any(|term| lower.contains(term));
    explicit_only || question_prefix && !follow_up_action
        || ["不要调用工具", "不调用工具", "不要读取任何数据", "不读取任何数据", "no tools", "without tools"]
            .iter().any(|term| lower.contains(term))
        || (["不要读","不读取","do not read","don't read"].iter().any(|term|lower.contains(term))
            && !["日志","串口","log","波形","waveform","报告","report","qi"].iter().any(|term|lower.contains(term)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteStatus { Fallback, Candidate, Multiple }

#[derive(Debug, Clone)]
pub struct RouteCandidate {
    pub skill: &'static Skill,
    pub matched_terms: Vec<&'static str>,
    pub available_tools: Vec<&'static str>,
    pub missing: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RouteSources {
    pub log: Option<bool>,
    pub waveform: Option<bool>,
    pub report: Option<bool>,
}

pub fn english_missing(reason: &str) -> &str {
    match reason {
        "当前页面或权限缺少完成该技能所需的工具" => "Required tools are unavailable on this page or under current permissions",
        "当前来源尚未核实" => "The current data source has not been verified",
        "当前工程尚未核实" => "The current project has not been verified",
        "测试配置与预检状态需由宿主核验" => "The host must verify the test configuration and preflight",
        "支持所需采集功能的示波器会话尚未核实" => "An oscilloscope session with the requested capture capability has not been verified",
        "设备或测试目标需通过宿主检查" => "The host must verify the device or test target",
        _ => reason,
    }
}

/// Each entry represents one capability; aliases inside an entry are alternatives.
/// Inventory access by itself must never make an analysis skill appear executable.
fn required_capabilities(id: &str) -> &'static [&'static [&'static str]] {
    match id {
        "serial_log" => &[&["log.analyze"]],
        "waveform" => &[&["wave.measure", "engineering.wave.measure", "wave.timing"]],
        "qi_protocol" => &[&["wave.frames", "engineering.wave.frames"]],
        "test_result" => &[&["report.read"]],
        "charging_protection" => &[&["log.analyze"], &["wave.measure", "engineering.wave.measure"]],
        "scope_capture" => &[&["instrument.capture", "instrument.waveform_source"]],
        "test_execution" => &[&["testing.preflight"], &["testing.run"]],
        _ => &[],
    }
}

/// Describe candidates against the host's current permission-filtered tool list.
/// A missing tool is a limitation, never an authorization to change context.
pub fn contextual_candidates(question: &str, page: &str, source_available: Option<bool>,
    project_available: Option<bool>, device_available: Option<bool>,
    available_methods: &[&str]) -> Vec<RouteCandidate> {
    contextual_candidates_scoped(question, page, RouteSources { log:source_available,
        waveform:source_available, report:source_available }, project_available, device_available, available_methods)
}

pub fn contextual_candidates_scoped(question: &str, page: &str, sources: RouteSources,
    project_available: Option<bool>, device_available: Option<bool>,
    available_methods: &[&str]) -> Vec<RouteCandidate> {
    let lower = question.to_lowercase();
    candidates(question).into_iter().map(|skill| {
        let matched_terms = skill.triggers.iter().copied().filter(|term| lower.contains(term)).collect();
        assess_skill(skill, matched_terms, page, sources, project_available, device_available, available_methods)
    }).collect()
}

fn assess_skill(skill: &'static Skill, matched_terms: Vec<&'static str>, page: &str, sources: RouteSources,
    project_available: Option<bool>, device_available: Option<bool>, available_methods: &[&str]) -> RouteCandidate {
        let available_tools = skill.tools.iter().copied().filter(|method| available_methods.contains(method)).collect::<Vec<_>>();
        let mut missing = Vec::new();
        if required_capabilities(skill.id).iter().any(|alternatives|
            !alternatives.iter().any(|method| available_methods.contains(method))) {
            missing.push("当前页面或权限缺少完成该技能所需的工具");
        }
        let source_ready = match skill.id {
            "serial_log" => sources.log == Some(true),
            "waveform" | "qi_protocol" => sources.waveform == Some(true),
            "test_result" => sources.report == Some(true),
            "charging_protection" => sources.log == Some(true) && sources.waveform == Some(true),
            _ => true,
        };
        if !source_ready {
            missing.push("当前来源尚未核实");
        }
        if matches!(skill.id, "test_result" | "test_execution") && project_available != Some(true) { missing.push("当前工程尚未核实"); }
        if skill.id == "test_execution" { missing.push("测试配置与预检状态需由宿主核验"); }
        if skill.id == "scope_capture" && device_available != Some(true) { missing.push("支持所需采集功能的示波器会话尚未核实"); }
        if matches!(skill.id, "scope_capture" | "test_execution") && page != "instruments" && page != "test_tool" {
            missing.push("设备或测试目标需通过宿主检查");
        }
        RouteCandidate { skill, matched_terms, available_tools, missing }
}

/// Assess explicit selections using the same capability/source rules as routing.
/// Trigger text here is only used to enumerate definitions, not to grant access.
pub fn assess_selected(ids:&[String],page:&str,sources:RouteSources,project:Option<bool>,
    device:Option<bool>,methods:&[&str])->Vec<RouteCandidate> {
    let Ok(skills)=select(ids) else {return Vec::new();};
    skills.into_iter().map(|skill|assess_skill(skill, Vec::new(), page, sources, project, device, methods)).collect()
}

/// Route matches are suggestions. Explicit selections are authoritative and
/// neither path grants access to a tool or data source.
pub fn route(question: &str, ids: &[String]) -> Result<(RouteStatus, Vec<&'static Skill>), String> {
    validate_registry()?;
    let result = if ids.is_empty() { candidates(question) } else { select(ids)? };
    let status = match result.len() {
        0 => RouteStatus::Fallback,
        1 => RouteStatus::Candidate,
        _ => RouteStatus::Multiple,
    };
    Ok((status, result))
}

/// Validate a requested explicit selection against the built-in registry.
pub fn select(ids: &[String]) -> Result<Vec<&'static Skill>, String> {
    validate_registry()?;
    if ids.len() > BUILTINS.len() {
        return Err("SKILL.SELECTION_LIMIT".into());
    }
    let mut selected = Vec::new();
    for id in ids {
        let skill = BUILTINS.iter().find(|skill| skill.id == id)
            .ok_or_else(|| format!("SKILL.UNKNOWN: {id}"))?;
        if selected.iter().any(|existing: &&Skill| existing.id == skill.id) {
            return Err(format!("SKILL.DUPLICATE: {id}"));
        }
        selected.push(skill);
    }
    Ok(selected)
}

/// Management calls coordinate the task; all other calls must belong to the selection.
pub fn permits(ids: &[String], method: &str) -> bool {
    if ids.is_empty() {
        return true;
    }
    if matches!(method, "agent.plan" | "agent.evidence" | "agent.help" | "agent.recipe") {
        return select(ids).is_ok();
    }
    if method == "agent.align" {
        return select(ids).is_ok() && (ids.iter().any(|id| id == "charging_protection")
            || ids.iter().any(|id| id == "serial_log") && ids.iter().any(|id| id == "qi_protocol"));
    }
    select(ids).is_ok_and(|skills| skills.iter().any(|skill| skill.tools.contains(&method)))
}

pub fn references(ids: &[String]) -> Vec<serde_json::Value> {
    select(ids).unwrap_or_default().iter()
        .map(|skill| serde_json::json!({"id": skill.id, "version": skill.version}))
        .collect()
}

/// Inventory and configuration are inputs, not completion evidence.
pub fn is_completion_method(skill: &str, method: &str) -> bool {
    match skill {
        "serial_log" => method == "log.analyze",
        "waveform" => matches!(method, "wave.measure" | "engineering.wave.measure" | "wave.timing"),
        "qi_protocol" => matches!(method, "wave.frames" | "engineering.wave.frames"),
        "test_result" => method == "report.read",
        "charging_protection" => matches!(method, "log.analyze" | "wave.measure" | "engineering.wave.measure"),
        "scope_capture" => matches!(method, "instrument.capture" | "instrument.waveform_source"),
        "test_execution" => method == "testing.wait",
        _ => false,
    }
}

/// Serialize trusted guidance as task context. The host retains all existing checks.
pub fn guidance(question: &str) -> String {
    guidance_for(question, &[])
}

pub fn guidance_for(question: &str, ids: &[String]) -> String {
    let (status, selected) = route(question, ids).unwrap_or((RouteStatus::Fallback, Vec::new()));
    if selected.is_empty() {
        return String::new();
    }
    let mut text = String::from("\nDomain guidance (suggestions only; existing host permissions and source checks apply):");
    if ids.is_empty() && status == RouteStatus::Multiple {
        text.push_str(" Multiple domains match; combine only when the user's request supports that interpretation. Ask for clarification when the intended scope is materially ambiguous.");
    }
    for skill in selected {
        text.push_str(&format!("\n{} v{}: {}", skill.id, skill.version, skill.guidance));
        if let Some(spec)=contract(skill.id) {
            text.push_str(&format!(" Steps: {}. Stop: {}. Exclusions: {}. Compatibility: {}.",
                spec.steps.join(" → "),spec.stop,spec.excludes,spec.compatibility));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    #[test]
    fn contract_text_rejects_blank_and_oversized_fields() {
        assert!(!super::valid_contract_text(" \t\n"));
        assert!(!super::valid_contract_text(&"x".repeat(4097)));
        assert!(super::valid_contract_text("实际证据"));
    }
    use super::*;

    #[test]
    fn test_execution_skill_includes_owned_run_controls() {
        let ids=vec!["test_execution".to_owned()];
        assert!(permits(&ids,"testing.pause"));
        assert!(permits(&ids,"testing.resume"));
        assert!(permits(&ids,"testing.stop"));
    }

    #[test]
    fn registry_rejects_unknown_and_duplicate_selection() {
        assert!(validate_registry().is_ok());
        assert!(select(&["missing".into()]).is_err());
        assert!(select(&["waveform".into(),"waveform".into()]).is_err());
        assert!(!permits(&["waveform".into()],"testing.run"));
        assert!(permits(&["waveform".into()],"wave.measure"));
    }

    #[test]
    fn scope_source_is_an_authorized_capture_capability_with_observed_completion() {
        let selected = vec!["scope_capture".to_owned()];
        assert!(permits(&selected, "instrument.waveform_source"));
        assert!(is_completion_method("scope_capture", "instrument.waveform_source"));
        assert!(candidates("读取示波器波形源文件").iter().any(|skill| skill.id == "scope_capture"));
    }

    #[test]
    fn unknown_source_never_qualifies_for_automatic_routing() {
        let choices=contextual_candidates("分析波形","waveform",None,Some(true),None,&["wave.measure"]);
        assert_eq!(choices.len(),1);
        assert!(!choices[0].missing.is_empty());
    }

    #[test]
    fn domain_exclusions_do_not_suppress_unrelated_reads() {
        let question="不要分析波形，只分析串口日志";
        assert!(skill_excluded(question,"waveform"));
        assert!(!skill_excluded(question,"serial_log"));
        assert!(!explanatory_only(question));
        assert_eq!(candidates(question).iter().map(|skill|skill.id).collect::<Vec<_>>(),vec!["serial_log"]);
        assert!(measurement_excluded("不要测量，只分析日志"));
        assert!(!explanatory_only("不要测量，只分析日志"));
        assert!(explanatory_only("不要调用工具"));
    }

    #[test]
    fn conceptual_prefix_does_not_discard_a_followup_analysis_request() {
        for question in ["什么是过压，并分析串口日志", "如何使用示波器，然后检查波形", "what is overvoltage and check the log"] {
            assert!(!explanatory_only(question), "{question}");
        }
        assert!(explanatory_only("什么是过压"));
        assert!(explanatory_only("只解释过压，不调用工具"));
    }

    #[test]
    fn contrast_limits_negation_without_weakening_joint_exclusions() {
        for question in ["不要分析日志但分析波形", "do not read log but analyze waveform"] {
            assert!(skill_excluded(question, "serial_log"));
            assert!(!skill_excluded(question, "waveform"));
        }
        assert!(skill_excluded("不要日志和波形", "serial_log"));
        assert!(skill_excluded("不要日志和波形", "waveform"));
    }

    #[test]
    fn later_explicit_exclusion_is_not_hidden_by_an_earlier_mention() {
        assert!(skill_excluded("日志未准备好时不要读取日志但分析波形", "serial_log"));
        assert!(!skill_excluded("日志未准备好时不要读取日志但分析波形", "waveform"));
        assert!(skill_excluded("if the log is unavailable do not read the log but analyze waveform", "serial_log"));
        assert!(!skill_excluded("if the log is unavailable do not read the log but analyze waveform", "waveform"));
    }

    #[test]
    fn excluded_control_skills_do_not_suppress_report_analysis() {
        for question in ["不执行测试，只分析测试报告", "别执行测试但分析测试报告", "do not run test but analyze test report"] {
            assert!(skill_excluded(question, "test_execution"));
            assert!(!skill_excluded(question, "test_result"));
            assert!(!candidates(question).iter().any(|skill| skill.id == "test_execution"));
        }
        assert!(skill_excluded("不要示波器采集，分析波形", "scope_capture"));
        assert!(!skill_excluded("不要示波器采集，分析波形", "waveform"));
    }

    #[test]
    fn explicit_assessment_preserves_every_selection_and_order() {
        let ids:Vec<String>=BUILTINS.iter().rev().map(|skill|skill.id.to_owned()).collect();
        let assessed=assess_selected(&ids,"serial",RouteSources::default(),None,None,&[]);
        assert_eq!(assessed.iter().map(|candidate|candidate.skill.id).collect::<Vec<_>>(),ids.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(assessed.iter().all(|candidate|candidate.matched_terms.is_empty()));
        assert!(assessed.iter().all(|candidate|!candidate.missing.is_empty()));
    }

    #[test]
    fn routes_common_serial_and_charging_phrases() {
        for (question, expected) in [
            ("分析串口数据", "serial_log"),
            ("analyze serial data", "serial_log"),
            ("分析充电中断", "charging_protection"),
            ("analyze charging protection", "charging_protection"),
        ] {
            assert!(candidates(question).iter().any(|skill| skill.id == expected), "{question}");
        }
    }

    #[test]
    fn router_handles_domains_combinations_missing_context_and_explicit_priority() {
        for (request, expected) in [
            ("分析串口日志", "serial_log"), ("分析波形", "waveform"),
            ("分析 Qi 协议", "qi_protocol"), ("检查测试报告", "test_result"),
            ("分析充电保护", "charging_protection"),
        ] {
            let (status, choices) = route(request, &[]).unwrap();
            assert_eq!(status, RouteStatus::Candidate, "{request}");
            assert_eq!(choices[0].id, expected);
        }
        let (status, choices) = route("检查日志和波形", &[]).unwrap();
        assert_eq!(status, RouteStatus::Multiple);
        assert!(choices.iter().any(|skill| skill.id == "serial_log"));
        assert!(choices.iter().any(|skill| skill.id == "waveform"));
        let (status, choices) = route("检查日志", &["waveform".into()]).unwrap();
        assert_eq!(status, RouteStatus::Candidate);
        assert_eq!(choices[0].id, "waveform");
        assert_eq!(route("请介绍功能", &[]).unwrap().0, RouteStatus::Fallback);
        let missing = contextual_candidates_scoped("分析波形", "serial", RouteSources::default(),
            Some(true), None, &["wave.channels"]);
        assert!(!missing[0].missing.is_empty());
        assert!(!permits(&["serial_log".into()], "instrument.capture"));
        assert!(!permits(&["unknown".into()], "agent.plan"));
    }
    #[test]
    fn cross_domain_candidates_remain_suggestions_without_inventing_a_selection() {
        let (status, choices) = route("检查串口日志和波形，判断是否需要进一步测试", &[]).unwrap();
        assert_eq!(status, RouteStatus::Multiple);
        assert!(choices.iter().any(|skill| skill.id == "serial_log"));
        assert!(choices.iter().any(|skill| skill.id == "waveform"));
        assert!(!choices.iter().any(|skill| skill.id == "test_execution"));
        assert_eq!(route("设备最近不稳定，帮我看看", &[]).unwrap().0, RouteStatus::Fallback);
    }
}
