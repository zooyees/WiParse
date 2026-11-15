use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;

fn field_name(name: &str) -> &str {
    name.strip_prefix("key:").unwrap_or(name)
}

/// Compare names against the curves the GUI actually installed, including empty input fields.
pub fn include_expected_plot_fields<'a>(
    receipt: &mut Value,
    expected: impl Iterator<Item = &'a str>,
) {
    let Some(curves) = receipt["curves"].as_array() else {
        return;
    };
    let mut unextracted = receipt["unextracted_fields"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut missing = receipt["missing_fields"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for name in expected {
        let matches = |value: &Value| {
            value
                .as_str()
                .is_some_and(|v| field_name(v) == field_name(name))
        };
        if !curves.iter().any(|curve| matches(&curve["field"])) && !missing.iter().any(matches) {
            missing.push(json!(name));
            unextracted.push(json!(name));
        }
    }
    receipt["missing_fields"] = Value::Array(missing);
    receipt["unextracted_fields"] = Value::Array(unextracted);
}

// Capture explicit lists of identifiers in plotting instructions. Free-form names/aliases can
// be passed by the agent in expected_fields without guessing physical sensor equivalence.
pub(super) fn requested_fields(question: &str) -> Vec<String> {
    static LIST: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?x)[A-Za-z_][A-Za-z0-9_.-]* (?: \s* (?: [,，、/] | 和 | 以及 | 与 | \band\b ) \s* [A-Za-z_][A-Za-z0-9_.-]* )+"
    ).unwrap()
    });
    static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_.-]*").unwrap());
    let mut fields = Vec::new();
    for clause in question.split(['。', '；', ';', '\n']) {
        let lower = clause.to_lowercase();
        if !["画", "绘", "曲线", "展示", "显示", "plot", "chart"]
            .iter()
            .any(|w| lower.contains(w))
            || ["不要", "排除", "不含", "except", "exclude"]
                .iter()
                .any(|w| lower.contains(w))
        {
            continue;
        }
        for list in LIST.find_iter(clause) {
            let names: Vec<_> = ID
                .find_iter(list.as_str())
                .map(|m| m.as_str())
                .filter(|&s| s != "and")
                .collect();
            // Ambiguous prose and units like V/A must not introduce obligations.
            if !names.iter().any(|n| {
                n.len() > 1
                    && n.bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            }) {
                continue;
            }
            for name in names {
                if fields.len() < 32 && !fields.iter().any(|n| n == name) {
                    fields.push(name.to_owned());
                }
            }
        }
    }
    fields
}

#[derive(Clone)]
pub(super) struct Presentation {
    pub id: String,
    pub receipt: Value,
}
impl Presentation {
    pub fn partial(&self) -> bool {
        self.receipt["missing_fields"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    }
    pub fn description(&self) -> String {
        let fields: Vec<_> = self.receipt["curves"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| c["field"].as_str())
            .collect();
        let missing: Vec<_> = self.receipt["missing_fields"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let mut result = format!(
            "图表回执 {}：实际显示 {} 条曲线（{}）",
            self.id,
            fields.len(),
            fields.join("、")
        );
        if !missing.is_empty() {
            result.push_str(&format!("；未显示：{}", missing.join("、")));
        }
        result
    }

    pub fn validate(&self, text: &str) -> Result<(), String> {
        static CLAIM: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
            r"(?ix)(?:已(?:经)?(?:成功)?(?:显示|展示|绘制|绘出|画出|绘制了)[了共]?\s*([0-9]+|[一二两三四五六七八九十])\s*条(?:数据)?曲线|([0-9]+|[一二两三四五六七八九十])\s*条(?:曲线)?(?:均|都|全部)?已(?:显示|展示|绘制|画出)|(?:plotted|displayed)\s+([0-9]+)\s+(?:curves|series))"
        ).unwrap()
        });
        static ALL: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
            r"(?i)(?:所有|全部|所需|请求的)(?:字段|曲线)(?:均|都)?已(?:显示|绘制|展示)|(?:已全部|已完整)(?:绘制|显示|展示)(?:所有|全部|所需|请求的)?(?:字段|曲线)|all (?:requested )?(?:fields|curves|series) (?:have been |are |were )(?:plotted|displayed)"
        ).unwrap()
        });
        static FIELD_CLAIM: LazyLock<Regex> = LazyLock::new(|| Regex::new(
            r"(?:^|[^A-Za-z0-9_])([A-Za-z_][A-Za-z0-9_.-]*)\s*(?:曲线)?(?:均|也|都)?已(?:显示|展示|绘制)|已(?:显示|展示|绘制)\s+([A-Za-z_][A-Za-z0-9_.-]*)"
        ).unwrap());
        let count = self.receipt["curves"].as_array().map_or(0, Vec::len);
        for clause in text.split(['。', '；', ';', '\n']) {
            if [
                "之前",
                "先前",
                "原先",
                "上一",
                "有误",
                "误称",
                "更正",
                "假设",
                "previously",
                "incorrect",
            ]
            .iter()
            .any(|v| clause.contains(v))
            {
                continue;
            }
            if [
                "并非",
                "绝非",
                "并不是",
                "不能说",
                "不能确认",
                "无法确认",
                "not all",
                "not displayed",
                "not plotted",
            ]
            .iter()
            .any(|v| clause.contains(v))
            {
                continue;
            }
            if self.partial() && ALL.is_match(clause) {
                return Err(format!("AGENT.PLOT_FIELDS_MISSING: {}", self.description()));
            }
            for cap in FIELD_CLAIM.captures_iter(clause) {
                let field = cap.iter().skip(1).flatten().next().unwrap().as_str();
                if self.receipt["missing_fields"].as_array().into_iter().flatten()
                    .filter_map(Value::as_str).any(|missing| field_name(missing)==field) {
                    return Err(format!("AGENT.PLOT_FIELDS_MISSING: {}", self.description()));
                }
            }
            for cap in CLAIM.captures_iter(clause) {
                let n = cap.iter().skip(1).flatten().next().unwrap().as_str();
                let number = n.parse().ok().or_else(|| match n {
                    "两" => Some(2),
                    _ => "一二三四五六七八九十"
                        .chars()
                        .position(|c| n.starts_with(c))
                        .map(|i| i + 1),
                });
                if number != Some(count) {
                    return Err(format!("AGENT.PLOT_COUNT_CONFLICT: {}", self.description()));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::Workflow;
    use super::*;

    #[test]
    fn no_sample_and_unextracted_are_distinct_and_field_selectors_are_reconciled() {
        let mut receipt = json!({"curves":[{"field":"key:VIN"}],"missing_fields":["T"]});
        include_expected_plot_fields(&mut receipt, ["VIN", "T", "VBRG"].into_iter());
        assert_eq!(receipt["missing_fields"], json!(["T", "VBRG"]));
        assert_eq!(receipt["unextracted_fields"], json!(["VBRG"]));
        include_expected_plot_fields(&mut receipt, ["VIN", "VBRG"].into_iter());
        assert_eq!(receipt["unextracted_fields"], json!(["VBRG"]));
    }
    #[test]
    fn reported_curves_are_bound_to_actual_fields_and_current_task() {
        let mut w = Workflow::default();
        w.bind_request("在数据分析页显示 VIN、VBRG、IBRG 和 T 的曲线");
        let mut data = json!({"presented":true,"records":200,"series":3,
            "curves":[{"field":"VIN","sample_count":200},{"field":"VBRG","sample_count":190},{"field":"IBRG","sample_count":180}]});
        w.record("S1", "data.present", &json!({"ok":true,"data":data}));
        assert!(w.partially_completed());
        assert!(w
            .validate_reply("四条曲线均已显示。")
            .unwrap_err()
            .contains("PLOT_REEXTRACT"));
        assert!(w
            .validate_reply("四条曲线均已显示。")
            .unwrap_err()
            .contains("PLOT_COUNT_CONFLICT"));
        assert!(w.validate_reply("日志已显示12条告警。").is_ok());
        assert!(w.validate_reply("温度 T 曲线已显示。").is_err());
        assert!(w.validate_reply("所有曲线均已显示").is_err());
        assert!(w
            .validate_reply("已显示 3 条曲线，T 没有有效样本。")
            .is_ok());
        let mut text = String::new();
        w.append_verified_results(&mut text);
        assert!(text.contains("未显示：T"));
        data["curves"]
            .as_array_mut()
            .unwrap()
            .push(json!({"field":"T","sample_count":1}));
        data["series"] = json!(4);
        w.record("S2", "data.present", &json!({"ok":true,"data":data}));
        assert!(!w.partially_completed());
        assert!(w.validate_reply("已显示四条曲线").is_ok());
        w.record(
            "S3",
            "data.present",
            &json!({"ok":false,"error":"DATA.EXPIRED"}),
        );
        assert!(w.partially_completed());
        let mut text = String::new();
        w.append_verified_results(&mut text);
        assert!(text.contains("更新失败 S3") && text.contains("实际显示 4 条"));
        w.new_task();
        assert!(w.validate_reply("已显示 5 条曲线").is_ok());
    }
}
