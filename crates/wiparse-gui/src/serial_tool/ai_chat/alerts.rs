use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub(super) struct Limit {
    pub key: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

#[derive(Clone)]
pub(super) struct Rules {
    pub contains: Vec<String>,
    pub limits: Vec<Limit>,
}
impl Default for Rules {
    fn default() -> Self {
        Self {
            contains: vec![
                "ERROR".into(),
                "FAULT".into(),
                "PANIC".into(),
                "异常".into(),
                "故障".into(),
            ],
            limits: Vec::new(),
        }
    }
}

pub(super) fn validate(
    contains: &Option<Vec<String>>,
    limits: &Option<Vec<Limit>>,
) -> Result<(), String> {
    if contains
        .as_ref()
        .is_some_and(|v| v.len() > 16 || v.iter().any(|s| s.trim().is_empty() || s.len() > 80))
        || limits.as_ref().is_some_and(|v| {
            v.len() > 16
                || v.iter().any(|r| {
                    r.key.trim().is_empty()
                        || r.key.len() > 64
                        || r.min.is_none() && r.max.is_none()
                        || r.min.is_some_and(|v| !v.is_finite())
                        || r.max.is_some_and(|v| !v.is_finite())
                        || r.min.zip(r.max).is_some_and(|(min, max)| min > max)
                })
        })
    {
        return Err(
            "AI.RULE_INVALID: maximum 16 text rules and 16 finite ordered numeric limits".into(),
        );
    }
    Ok(())
}

fn value(line: &str, key: &str) -> Option<f64> {
    line.match_indices(key)
        .filter(|(pos, _)| {
            *pos == 0
                || !line.as_bytes()[pos - 1].is_ascii_alphanumeric()
                    && line.as_bytes()[pos - 1] != b'_'
        })
        .find_map(|(pos, _)| {
            let rest = line[pos + key.len()..]
                .strip_prefix('"')
                .unwrap_or(&line[pos + key.len()..])
                .trim_start();
            let rest = rest
                .strip_prefix('=')
                .or_else(|| rest.strip_prefix(':'))
                .unwrap_or(rest)
                .trim_start();
            let number: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
                .collect();
            number.parse::<f64>().ok().filter(|v| v.is_finite())
        })
}

impl Rules {
    pub fn check(&self, line: &str) -> Option<(String, String)> {
        for rule in &self.limits {
            if let Some(value) = value(line, &rule.key) {
                if rule.min.is_some_and(|min| value < min)
                    || rule.max.is_some_and(|max| value > max)
                {
                    return Some((
                        format!("limit:{}", rule.key),
                        format!(
                            "{} = {} · [{}, {}]",
                            rule.key,
                            value,
                            rule.min.map_or("−∞".into(), |v| v.to_string()),
                            rule.max.map_or("+∞".into(), |v| v.to_string())
                        ),
                    ));
                }
            }
        }
        let lower = line.to_ascii_lowercase();
        self.contains
            .iter()
            .find(|key| lower.contains(&key.to_ascii_lowercase()))
            .map(|key| (format!("text:{key}"), format!("匹配 / Matched: {key}")))
    }
}

impl SerialAiChat {
    pub(super) fn observe_rules(&mut self, source: u64, lines: &[String]) {
        if !self
            .watch
            .as_ref()
            .is_some_and(|w| w.binding.source_id == source)
        {
            return;
        }
        let now = Instant::now();
        let mut shown = 0;
        for line in lines {
            let sample = bounded(line, 600);
            let Some((key, reason)) = self.rules.check(&sample) else {
                continue;
            };
            self.alert_count = self.alert_count.saturating_add(1);
            let recent = self
                .alert_last
                .get(&key)
                .is_some_and(|last| now.duration_since(*last) < Duration::from_secs(2));
            if !recent && shown < 4 {
                self.alert_last.insert(key, now);
                shown += 1;
                self.entry("alert", format!("{reason}\n\n{sample}"));
                if let Some(entry) = self.entries.last_mut() {
                    entry.automatic = true;
                    entry.error = true;
                }
                self.notice = Some((
                    format!("{reason} · {sample}"),
                    now + Duration::from_secs(10),
                ));
            }
            if let Some(watch) = &mut self.watch {
                if watch.stats.alert_examples.len() == 12 {
                    watch.stats.alert_examples.pop_front();
                }
                watch.stats.alert_examples.push_back(sample.clone());
                watch.alert_waiting = true;
                if watch.failures == 0 {
                    watch.next = watch.next.min(
                        (now + Duration::from_secs(1))
                            .max(watch.last_request + Duration::from_secs(MIN_INTERVAL_SECONDS)),
                    );
                }
            }
        }
    }
}

impl SerialToolPanel {
    pub(crate) fn ai_alert_ui(&mut self, ctx: &egui::Context, lang: Lang) -> bool {
        let Some((message, until)) = self.ai_chat.notice.clone() else {
            return false;
        };
        if Instant::now() >= until {
            self.ai_chat.notice = None;
            return false;
        }
        let mut open = false;
        egui::Area::new(egui::Id::new("serial-ai-alert-toast"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16., -16.))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let t = if ctx.style().visuals.dark_mode {
                    Tokens::dark()
                } else {
                    Tokens::light()
                };
                Frame::NONE
                    .fill(t.surface_bg)
                    .stroke(Stroke::new(1.0, t.warning))
                    .corner_radius(8)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.set_width(320.0_f32.min(ctx.screen_rect().width() - 40.));
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(label(
                                    lang,
                                    "串口监控预警",
                                    "Serial monitoring alert",
                                ))
                                .strong()
                                .color(t.warning),
                            );
                            if ui.small_button("×").clicked() {
                                self.ai_chat.notice = None;
                            }
                        });
                        ui.add(egui::Label::new(bounded(&message, 180)).wrap());
                        if ui
                            .button(label(
                                lang,
                                "查看对话与证据",
                                "View conversation and evidence",
                            ))
                            .clicked()
                        {
                            open = true;
                            self.ai_chat.open = true;
                            self.ai_chat.mode = Mode::Analyse;
                            self.ai_chat.notice = None;
                        }
                    });
            });
        ctx.request_repaint_after(until.saturating_duration_since(Instant::now()));
        open
    }
}
