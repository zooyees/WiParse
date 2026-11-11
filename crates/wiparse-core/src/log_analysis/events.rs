//! General state-transition extraction. Patterns come from observed logs, not product-specific constants.
use super::*;
use std::collections::HashMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rule {
    pub active_pattern: String,
    pub inactive_pattern: String,
    pub reason_pattern: Option<String>,
    pub timestamp_pattern: Option<String>,
    pub entity_pattern: Option<String>,
    pub context_lines: usize,
    pub offset: u64,
    pub limit: usize,
}
impl Default for Rule {
    fn default() -> Self {
        Self {
            active_pattern: String::new(),
            inactive_pattern: String::new(),
            reason_pattern: None,
            timestamp_pattern: None,
            entity_pattern: None,
            context_lines: 2,
            offset: 0,
            limit: 12,
        }
    }
}
impl Rule {
    pub fn validate(&self) -> Result<(), String> {
        compile(&self.active_pattern, false)?;
        compile(&self.inactive_pattern, false)?;
        if self.active_pattern == self.inactive_pattern
            || self.context_lines > 8
            || self.limit == 0
            || self.limit > 32
            || self.offset > 1_000_000
        {
            return Err("LOG.INVALID_EVENT_RULE".into());
        }
        for p in [
            &self.reason_pattern,
            &self.timestamp_pattern,
            &self.entity_pattern,
        ]
        .into_iter()
        .flatten()
        {
            let r = compile(p, false)?;
            if r.captures_len() < 2 {
                return Err("LOG.EVENT_CAPTURE_GROUP_1_REQUIRED".into());
            }
        }
        Ok(())
    }
}
#[derive(Default)]
struct State {
    active: Option<bool>,
    active_line: Option<u64>,
    recent: VecDeque<Value>,
}
pub(super) struct Extractor {
    rule: Rule,
    on: Regex,
    off: Regex,
    reason: Option<Regex>,
    timestamp: Option<Regex>,
    entity: Option<Regex>,
    states: HashMap<String, State>,
    rows: Vec<Value>,
    count: u64,
    unpaired: u64,
    repeated: u64,
    ambiguous: u64,
    skipped: u64,
}
impl Extractor {
    pub fn new(rule: Rule) -> Result<Self, String> {
        rule.validate()?;
        Ok(Self {
            on: compile(&rule.active_pattern, false)?,
            off: compile(&rule.inactive_pattern, false)?,
            reason: rule
                .reason_pattern
                .as_deref()
                .map(|p| compile(p, false))
                .transpose()?,
            timestamp: rule
                .timestamp_pattern
                .as_deref()
                .map(|p| compile(p, false))
                .transpose()?,
            entity: rule
                .entity_pattern
                .as_deref()
                .map(|p| compile(p, false))
                .transpose()?,
            rule,
            states: HashMap::new(),
            rows: vec![],
            count: 0,
            unpaired: 0,
            repeated: 0,
            ambiguous: 0,
            skipped: 0,
        })
    }
    pub fn reset_source(&mut self) {
        self.states.clear();
    }
    pub fn observe(&mut self, source: &str, row: u64, text: &str) {
        let entity = if let Some(r) = &self.entity {
            let Some(v) = r
                .captures(text)
                .and_then(|c| c.get(1).map(|v| v.as_str().to_owned()))
            else {
                return;
            };
            if v.len() > 128 {
                self.skipped += 1;
                return;
            }
            v
        } else {
            "default".into()
        };
        if !self.states.contains_key(&entity) && self.states.len() >= 64 {
            self.skipped += 1;
            return;
        }
        let state = self.states.entry(entity.clone()).or_default();
        let reason = self
            .reason
            .as_ref()
            .and_then(|r| r.captures(text))
            .and_then(|c| c.get(1).map(|v| clip(v.as_str(), 180)));
        if let Some(reason) = &reason {
            let evidence = json!({"line":row,"text":reason});
            state.recent.push_back(evidence.clone());
            for event in self.rows.iter_mut().rev() {
                if event["source_id"] == source
                    && event["entity"] == entity
                    && event["line"].as_u64().is_some_and(|line| {
                        row >= line && row - line <= self.rule.context_lines as u64
                    })
                {
                    if event["associated_observations"]
                        .as_array()
                        .map_or(0, Vec::len)
                        < 8
                    {
                        event["associated_observations"]
                            .as_array_mut()
                            .unwrap()
                            .push(evidence.clone());
                    }
                }
            }
        }
        while state.recent.front().is_some_and(|v| {
            v["line"]
                .as_u64()
                .is_some_and(|line| row.saturating_sub(line) > self.rule.context_lines as u64)
        }) {
            state.recent.pop_front();
        }
        let on = self.on.is_match(text);
        let off = self.off.is_match(text);
        if on && off {
            self.ambiguous += 1;
            return;
        }
        if on {
            state.active = Some(true);
            state.active_line = Some(row);
        }
        if off {
            match state.active {
                Some(true) => {
                    self.count += 1;
                    if self.count > self.rule.offset && self.rows.len() < self.rule.limit {
                        self.rows.push(json!({"event_id":format!("event-{}",self.count),"source_id":source,"entity":entity,"active_line":state.active_line,"line":row,"timestamp_text":self.timestamp.as_ref().and_then(|r|r.captures(text)).and_then(|c|c.get(1).map(|m|clip(m.as_str(),80))),"evidence":crate::engineering_agent::match_excerpt(text,self.off.find(text).map_or("",|v|v.as_str()),240),"associated_observations":state.recent.iter().cloned().collect::<Vec<_>>() }));
                    }
                }
                Some(false) => self.repeated += 1,
                None => self.unpaired += 1,
            }
            state.active = Some(false);
            state.active_line = None;
        }
    }
    pub fn report(&self) -> Value {
        json!({"confirmed_active_to_inactive_transitions":self.count,"unpaired_initial_inactive":self.unpaired,"repeated_inactive_lines_not_counted":self.repeated,"ambiguous_lines":self.ambiguous,"entity_budget_skipped":self.skipped,"rule":self.rule,"rule_quality_complete":self.ambiguous==0&&self.skipped==0,"events":self.rows,"next_offset":(self.rule.offset+(self.rule.limit as u64)<self.count).then_some(self.rule.offset+self.rule.limit as u64),"semantics":"Exact counts for the supplied patterns and entity grouping within each source. Initial inactive is not a confirmed disconnect; repeated inactive is not a new event. Nearby reasons are correlations, not proven causes. Timestamp text is not converted or clock-synchronized. Model must explain and verify how these patterns map to product behavior."})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    #[test]
    fn generic_state_rules_count_events_not_alarm_lines_and_keep_entities_separate() {
        let mut r = Rule {
            active_pattern: "mode=RUN".into(),
            inactive_pattern: "mode=OFF".into(),
            entity_pattern: Some("unit=(\\w+)".into()),
            reason_pattern: Some("reason=(\\w+)".into()),
            ..Default::default()
        };
        let mut x = Extractor::new(r.clone()).unwrap();
        for (i, t) in [
            "unit=A mode=OFF",
            "unit=A mode=RUN",
            "unit=B mode=RUN",
            "unit=A reason=UV",
            "unit=A mode=OFF",
            "unit=A mode=OFF",
            "unit=B mode=OFF",
        ]
        .iter()
        .enumerate()
        {
            x.observe("s", i as u64 + 1, t);
        }
        let v = x.report();
        assert_eq!(v["confirmed_active_to_inactive_transitions"], 2);
        assert_eq!(v["unpaired_initial_inactive"], 1);
        assert_eq!(v["repeated_inactive_lines_not_counted"], 1);
        assert_eq!(v["events"][0]["associated_observations"][0]["text"], "UV");
        r.inactive_pattern = r.active_pattern.clone();
        assert!(r.validate().is_err());
    }
    #[test]
    fn state_never_crosses_file_boundary() {
        let mut x = Extractor::new(Rule {
            active_pattern: "START".into(),
            inactive_pattern: "END".into(),
            ..Default::default()
        })
        .unwrap();
        x.observe("a", 1, "START");
        x.reset_source();
        x.observe("b", 1, "END");
        assert_eq!(x.report()["confirmed_active_to_inactive_transitions"], 0);
    }
    #[test]
    fn arbitrary_format_pagination_keeps_exact_counts_and_ambiguous_states_visible() {
        let mut x = Extractor::new(Rule {
            active_pattern: r#"\"state\":\s*7"#.into(),
            inactive_pattern: r#"\"state\":\s*3"#.into(),
            offset: 1,
            limit: 1,
            ..Default::default()
        })
        .unwrap();
        for (i, t) in [
            r#"{"state":3}"#,
            r#"{"state":7}"#,
            r#"{"state":3}"#,
            r#"{"state":7}"#,
            r#"{"state":3}"#,
            r#"{"state":7}"#,
            r#"{"state":3}"#,
            r#"{"state":7, "state":3}"#,
        ]
        .iter()
        .enumerate()
        {
            x.observe("json-source", i as u64 + 1, t);
        }
        let v = x.report();
        assert_eq!(v["confirmed_active_to_inactive_transitions"], 3);
        assert_eq!(v["events"][0]["event_id"], "event-2");
        assert_eq!(v["events"][0]["line"], 5);
        assert_eq!(v["next_offset"], 2);
        assert_eq!(v["ambiguous_lines"], 1);
        assert_eq!(v["rule_quality_complete"], false);
    }
}
