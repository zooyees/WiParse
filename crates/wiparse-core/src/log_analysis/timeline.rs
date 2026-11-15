//! Bounded time series. All field and boundary rules are inferred by the caller from evidence.
use super::*;
use crate::extract::{Record, Selector};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub pattern: String,
    pub unit: String,
    #[serde(default = "one")]
    pub scale: f64,
}
fn one() -> f64 {
    1.0
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Rule {
    pub timestamp_pattern: String,
    pub timestamp_unit: String,
    pub start_time: Option<String>,
    pub start_pattern: Option<String>,
    pub end_pattern: Option<String>,
    pub fields: Vec<Field>,
    /// Names of fields already converted to V and A, paired on the same line only.
    pub power_pair: Option<[String; 2]>,
    pub max_points: usize,
    pub retain_dataset: bool,
}
impl Default for Rule {
    fn default() -> Self {
        Self {
            timestamp_pattern: String::new(),
            timestamp_unit: "clock".into(),
            start_time: None,
            start_pattern: None,
            end_pattern: None,
            fields: vec![],
            power_pair: None,
            max_points: 48,
            retain_dataset: false,
        }
    }
}
impl Rule {
    pub fn validate(&self) -> Result<(), String> {
        let max = crate::extract::MAX_SERIES;
        if !(1..=max).contains(&self.fields.len()) {
            return Err(format!("LOG.TIMELINE_FIELD_COUNT: requested {} fields; supported 1..={max}", self.fields.len()));
        }
        if !matches!(self.timestamp_unit.as_str(), "clock" | "s" | "ms" | "us") {
            return Err(format!("LOG.INVALID_TIMESTAMP_UNIT: {}; expected clock, s, ms or us", self.timestamp_unit));
        }
        if !(8..=128).contains(&self.max_points) {
            return Err(format!("LOG.INVALID_TIMELINE_POINTS: requested {}; supported 8..=128 overview points", self.max_points));
        }
        Selector::timestamp(&self.timestamp_pattern, false)
            .map_err(|e|format!("LOG.INVALID_TIMESTAMP_SELECTOR: {e}"))?;
        let mut names = HashSet::new();
        for f in &self.fields {
            if f.name.is_empty()
                || f.name.len() > 32
                || f.name.chars().any(char::is_control)
                || f.unit.len() > 32
                || f.unit.chars().any(char::is_control)
                || !f.scale.is_finite()
                || f.scale == 0.0
                || !names.insert(&f.name)
            {
                return Err(format!("LOG.INVALID_TIMELINE_FIELD: {:?}; use a unique name (1–32 bytes), unit up to 32 bytes and finite nonzero scale", f.name));
            }
            Selector::compile(&f.pattern, false, true).map_err(|e|format!("LOG.INVALID_FIELD_SELECTOR [{}]: {e}", f.name))?;
        }
        for p in [&self.start_pattern, &self.end_pattern]
            .into_iter()
            .flatten()
        {
            compile(p, true)?;
        }
        if let Some(t) = &self.start_time {
            parse_time(t, &self.timestamp_unit).ok_or("LOG.INVALID_START_TIME")?;
        }
        if let Some(pair) = &self.power_pair {
            let units: Vec<_> = pair
                .iter()
                .map(|name| {
                    self.fields
                        .iter()
                        .find(|f| &f.name == name)
                        .map(|f| f.unit.as_str())
                })
                .collect();
            if pair[0] == pair[1]
                || units != vec![Some("V"), Some("A")]
                || names.contains(&"power_W".to_owned())
            {
                return Err("LOG.POWER_REQUIRES_FIELDS_IN_V_AND_A".into());
            }
        }
        Ok(())
    }
}
pub fn parse_time(t: &str, unit: &str) -> Option<f64> {
    let t=t.trim();
    if unit == "clock" {
        let parts = t.split(':').collect::<Vec<_>>();
        if parts.len() != 3 {
            return None;
        }
        let h = parts[0].parse::<u32>().ok()?;
        let m = parts[1].parse::<u32>().ok()?;
        let seconds=parts[2];
        let s=if seconds.contains(','){seconds.replace(',',".").parse::<f64>()}else{seconds.parse::<f64>()}.ok()?;
        (h < 24 && m < 60 && s.is_finite() && (0.0..60.0).contains(&s))
            .then_some(h as f64 * 3600.0 + m as f64 * 60.0 + s)
    } else {
        let v = t.parse::<f64>().ok()?;
        let scale = match unit {
            "ms" => 1e-3,
            "us" => 1e-6,
            _ => 1.0,
        };
        (v.is_finite() && v >= 0.0).then_some(v * scale)
    }
}
#[derive(Default)]
struct Stat {
    count: u64,
    sum: f64,
    min: Option<f64>,
    max: Option<f64>,
}
impl Stat {
    fn push(&mut self, v: f64) {
        self.count += 1;
        self.sum = self.sum * (1.0 - 1.0 / self.count as f64) + v / self.count as f64;
        self.min = Some(self.min.map_or(v, |a| a.min(v)));
        self.max = Some(self.max.map_or(v, |a| a.max(v)));
    }
    fn json(&self) -> Value {
        json!({"count":self.count,"min":self.min,"max":self.max,"mean":(self.count>0).then(||self.sum)})
    }
}
struct Point {
    t: f64,
    row: u64,
    values: Vec<Option<f64>>,
    weights: Vec<u64>,
    count: u64,
}
impl Point {
    fn merge(&self, b: &Self) -> Self {
        let weights = self
            .weights
            .iter()
            .zip(&b.weights)
            .map(|(a, b)| a + b)
            .collect::<Vec<_>>();
        let values = self
            .values
            .iter()
            .zip(&b.values)
            .enumerate()
            .map(|(i, (a, value_b))| {
                let n = weights[i];
                (n > 0).then(|| {
                    a.unwrap_or(0.0) * (self.weights[i] as f64 / n as f64)
                        + value_b.unwrap_or(0.0) * (b.weights[i] as f64 / n as f64)
                })
            })
            .collect();
        Self {
            t: self.t * (self.count as f64 / (self.count + b.count) as f64)
                + b.t * (b.count as f64 / (self.count + b.count) as f64),
            count: self.count + b.count,
            row: self.row,
            values,
            weights,
        }
    }
}
struct Series {
    id: String,
    points: Vec<Point>,
    stats: Vec<Stat>,
    first: Option<(f64, u64)>,
    last: Option<(f64, u64)>,
    last_clock: Option<f64>,
    day: f64,
    ended: bool,
    start_confirmed: bool,
    end_confirmed: bool,
    missing_time: u64,
    invalid_time: u64,
    valid_time: u64,
    backwards: u64,
    matched_rows: u64,
}
#[derive(Clone)]
pub struct Dataset {
    pub fields: Vec<Field>,
    pub rows: Vec<(String, f64, u64, Vec<Option<f64>>)>,
    pub omitted: u64,
    pub provenance: Value,
}
pub(super) struct Extractor {
    dataset: Dataset,
    rule: Rule,
    time: Selector,
    fields: Vec<Selector>,
    start: Option<Regex>,
    end: Option<Regex>,
    series: Vec<Series>,
    omitted_sources: u64,
}
impl Extractor {
    pub fn new(rule: Rule) -> Result<Self, String> {
        rule.validate()?;
        let mut fields=rule.fields.clone();
        if rule.power_pair.is_some(){fields.push(Field{name:"power_W".into(),pattern:String::new(),unit:"W".into(),scale:1.});}
        Ok(Self {
            dataset:Dataset{fields,rows:vec![],omitted:0,provenance:Value::Null},
            time: Selector::timestamp(&rule.timestamp_pattern, false)?,
            fields: rule
                .fields
                .iter()
                .map(|f| Selector::compile(&f.pattern, false, true))
                .collect::<Result<_, _>>()?,
            start: rule
                .start_pattern
                .as_ref()
                .map(|s| compile(s, true))
                .transpose()?,
            end: rule
                .end_pattern
                .as_ref()
                .map(|s| compile(s, true))
                .transpose()?,
            rule,
            series: vec![],
            omitted_sources: 0,
        })
    }
    pub fn observe(&mut self, id: &str, row: u64, text: &str) {
        if self.series.last().is_none_or(|s| s.id != id) {
            if self.series.len() >= 32 {
                self.omitted_sources += 1;
                return;
            }
            self.series.push(Series {
                id: id.into(),
                points: vec![],
                stats: (0..self.rule.fields.len() + usize::from(self.rule.power_pair.is_some()))
                    .map(|_| Stat::default())
                    .collect(),
                first: None,
                last: None,
                last_clock: None,
                day: 0.0,
                ended: false,
                start_confirmed: false,
                end_confirmed: false,
                missing_time: 0,
                invalid_time: 0,valid_time: 0,
                backwards: 0,
                matched_rows: 0,
            });
        }
        let s = self.series.last_mut().unwrap();
        if s.ended {
            return;
        }
        let record = Record::new(text);
        let token=self.time.first(&record);
        let Some(clock)=token.as_deref().and_then(|v|parse_time(v,&self.rule.timestamp_unit)) else{
            s.missing_time+=1;
            if token.is_some(){s.invalid_time+=1;}
            return;
        };
        s.valid_time+=1;
        if let Some(last) = s.last_clock {
            if clock < last {
                if self.rule.timestamp_unit == "clock" && last - clock > 43200.0 {
                    s.day += 86400.0;
                } else {
                    s.backwards += 1;
                    return;
                }
            }
        }
        s.last_clock = Some(clock);
        let t = clock + s.day;
        if s.first.is_none() {
            if self
                .rule
                .start_time
                .as_ref()
                .and_then(|v| parse_time(v, &self.rule.timestamp_unit))
                .is_some_and(|min| t < min)
            {
                return;
            }
            if self.start.as_ref().is_some_and(|p| !p.is_match(text)) {
                return;
            }
            s.start_confirmed = self.start.is_some();
            s.first = Some((t, row));
        }
        s.last = Some((t, row));
        let mut values = self
            .fields
            .iter()
            .zip(&self.rule.fields)
            .map(|(p, f)| {
                p.first(&record)
                    .and_then(|v| crate::extract::number(&v))
                    .map(|v| v * f.scale)
                    .filter(|v| v.is_finite())
            })
            .collect::<Vec<_>>();
        if let Some(pair) = &self.rule.power_pair {
            let a = self
                .rule
                .fields
                .iter()
                .position(|f| f.name == pair[0])
                .unwrap();
            let b = self
                .rule
                .fields
                .iter()
                .position(|f| f.name == pair[1])
                .unwrap();
            values.push(
                values[a]
                    .zip(values[b])
                    .map(|(v, i)| v * i)
                    .filter(|p| p.is_finite()),
            );
        }
        if values.iter().any(Option::is_some) {
            s.matched_rows += 1;
            for (v, stat) in values.iter().zip(&mut s.stats) {
                if let Some(v) = v {
                    stat.push(*v);
                }
            }
            if self.rule.retain_dataset {
                if self.dataset.rows.len()<250_000{self.dataset.rows.push((s.id.clone(),t,row,values.clone()));}else{self.dataset.omitted+=1;}
            }
            let weights = values.iter().map(|v| u64::from(v.is_some())).collect();
            s.points.push(Point {
                t,
                row,
                values,
                weights,
                count: 1,
            });
            if s.points.len() > self.rule.max_points {
                compact(&mut s.points);
            }
        }
        if self.end.as_ref().is_some_and(|p| p.is_match(text)) {
            s.ended = true;
            s.end_confirmed = true;
        }
    }
    pub fn dataset(&self)->Result<Dataset,String>{
        if !self.rule.retain_dataset{return Err("DATA.RETENTION_NOT_REQUESTED: repeat timeline with retain_dataset:true".into());}
        if self.dataset.omitted>0 || self.omitted_sources>0{return Err("DATA.DATASET_LIMIT: narrow source/time range; no truncated import".into());}
        if self.dataset.rows.is_empty(){
            return Err(if self.series.iter().all(|s|s.valid_time==0){"DATA.NO_VALID_TIMESTAMPS: inspect the source timestamp selector and unit; no timed samples to plot"}
                else{"DATA.NO_MATCHING_TIMED_VALUES: timestamps exist; check the requested window and numeric field selectors"}.into());
        }
        let mut dataset=self.dataset.clone();dataset.provenance=self.report();dataset.provenance.as_object_mut().unwrap().remove("sources");
        dataset.provenance["sources"]=json!(self.series.iter().map(|s|json!({"source_id":s.id,"missing_timestamp_rows":s.missing_time,"backward_rows":s.backwards,"start_confirmed":s.start_confirmed,"end_confirmed":s.end_confirmed})).collect::<Vec<_>>());
        Ok(dataset)
    }
    pub fn report(&self) -> Value {
        let mut fields = self
            .rule
            .fields
            .iter()
            .map(|f| json!({"name":f.name,"unit":f.unit,"scale":f.scale}))
            .collect::<Vec<_>>();
        if self.rule.power_pair.is_some() {
            fields.push(
                json!({"name":"power_W","unit":"W","calculation":"same timestamped line V*A only"}),
            );
        }
        let mut result = json!({"cycle_requested":self.start.is_some() || self.end.is_some() || self.rule.start_time.is_some(),"fields":fields,"sources_omitted_rows":self.omitted_sources,"sources":self.series.iter().map(|s|json!({"clock_domain":"source_local","external_alignment_verified":false,"timestamp_unit":self.rule.timestamp_unit,"source_id":s.id,"first_time_s":s.first.map(|v|v.0),"last_time_s":s.last.map(|v|v.0),"first_line":s.first.map(|v|v.1),"last_line":s.last.map(|v|v.1),"observed_duration_s":s.first.zip(s.last).map(|(a,b)|b.0-a.0),"start_marker_confirmed":s.start_confirmed,"end_marker_confirmed":s.end_confirmed,"missing_or_invalid_timestamp_rows":s.missing_time,"timestamp_selector_unmatched_rows":s.missing_time-s.invalid_time,"invalid_timestamp_value_rows":s.invalid_time,"parsed_timestamp_rows":s.valid_time,"clock_midnight_rollovers_assumed":(s.day/86400.0) as u64,"backward_timestamp_rows_skipped":s.backwards,"matched_rows":s.matched_rows,"statistics":s.stats.iter().map(Stat::json).collect::<Vec<_>>(),"points":s.points.iter().map(|p|json!([p.t,p.row,p.values])).collect::<Vec<_>>()})).collect::<Vec<_>>(),"chart":"bounded sample-weighted bucket means; statistics use every valid sample. Missing values are not interpolated. Points: [seconds, first bucket row, field means]. Source files are separate. Duration is observed span, not verified charging time unless boundary evidence confirms it."});
        result["local_dataset"]=json!({"retained":self.rule.retain_dataset,"rows":self.dataset.rows.len(),"omitted":self.dataset.omitted,"raw_uploaded":false});
        // Keep the local report compact without hiding total counts/statistics.
        while result.to_string().len() > 3600 {
            let Some(source) = result["sources"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .max_by_key(|s| s["points"].as_array().map_or(0, Vec::len))
            else {
                break;
            };
            let points = source["points"].as_array_mut().unwrap();
            if points.len() <= 2 {
                if result["sources"].as_array().unwrap().len() > 1 {
                    result["sources"].as_array_mut().unwrap().pop();
                    let count = result["report_sources_omitted_for_size"]
                        .as_u64()
                        .unwrap_or(0)
                        + 1;
                    result["report_sources_omitted_for_size"] = json!(count);
                    continue;
                }
                break;
            }
            let last = points.len() - 1;
            let mut i = 0;
            points.retain(|_| {
                let keep = i == 0 || i == last || i % 2 == 0;
                i += 1;
                keep
            });
            result["plot_points_omitted_for_size"] = json!(true);
        }
        result
    }
}
fn compact(points: &mut Vec<Point>) {
    let mut original = std::mem::take(points);
    let first = original.remove(0);
    let last = original.pop().unwrap();
    let mut reduced = vec![first];
    let mut middle = original.into_iter();
    while let Some(a) = middle.next() {
        if let Some(b) = middle.next() {
            reduced.push(a.merge(&b));
        } else {
            reduced.push(a);
        }
    }
    reduced.push(last);
    *points = reduced;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamp_diagnostics_distinguish_selection_parse_and_numeric_fields(){
        let mut rule=rule();rule.timestamp_pattern=r"clock=(\S+)".into();rule.start_time=None;rule.retain_dataset=true;
        let mut x=Extractor::new(rule).unwrap();
        x.observe("a",1,"no clock");x.observe("a",2,"clock=25:00:00 v=5");
        assert!(x.dataset().err().unwrap().contains("NO_VALID_TIMESTAMPS"));
        x.observe("a",3,"clock=20:05:09,865 unrelated");
        assert!(x.dataset().err().unwrap().contains("NO_MATCHING_TIMED_VALUES"));
        x.observe("a",4,"clock=20:05:10,865 v=5000 i=100");
        let report=x.report();let s=&report["sources"][0];
        assert_eq!(s["parsed_timestamp_rows"],2);assert_eq!(s["timestamp_selector_unmatched_rows"],1);
        assert_eq!(s["invalid_timestamp_value_rows"],1);assert_eq!(s["matched_rows"],1);
        assert_eq!(s["observed_duration_s"],1.0);assert_eq!(x.dataset().unwrap().rows.len(),1);
    }
    #[test]
    fn capacity_and_selector_errors_identify_the_actual_argument() {
        let mut r=rule();
        r.power_pair=None;
        let template=r.fields[0].clone();
        r.fields=(0..crate::extract::MAX_SERIES).map(|n| Field{name:format!("v{n}"),..template.clone()}).collect();
        r.validate().unwrap();
        r.fields.push(Field{name:"extra".into(),..template});
        assert!(r.validate().unwrap_err().contains("requested 9"));
        r.fields.pop();
        r.fields[7].pattern=r"PH \d+ T \d+".into();
        let error=r.validate().unwrap_err();
        assert!(error.contains("v7") && error.contains("捕获组"));
        r.timestamp_pattern="(".into();
        assert!(r.validate().unwrap_err().contains("TIMESTAMP_SELECTOR"));
    }
    fn rule() -> Rule {
        Rule {
            timestamp_pattern: r"(\d{2}:\d{2}:\d{2}\.\d+)".into(),
            start_time: Some("20:05:09.865".into()),
            end_pattern: Some("END".into()),
            fields: vec![
                Field {
                    name: "vin".into(),
                    pattern: r"v=(\d+)".into(),
                    unit: "V".into(),
                    scale: 0.001,
                },
                Field {
                    name: "iin".into(),
                    pattern: r"i=(\d+)".into(),
                    unit: "A".into(),
                    scale: 0.001,
                },
            ],
            power_pair: Some(["vin".into(), "iin".into()]),
            max_points: 8,
            ..Default::default()
        }
    }
    #[test]
    fn cycle_uses_actual_timestamps_units_and_only_same_line_power() {
        let mut x = Extractor::new(rule()).unwrap();
        for (i, line) in [
            "20:05:08.865 v=1000 i=1000",
            "20:05:09.865 v=5000 i=2000",
            "20:05:10.865 v=6000",
            "20:05:11.865 i=3000",
            "20:05:12.865 v=5000 i=1000 END",
            "20:05:13.865 v=9000 i=4000",
        ]
        .iter()
        .enumerate()
        {
            x.observe("a", i as u64 + 1, line);
        }
        let result = x.report();
        let s = &result["sources"][0];
        assert_eq!(s["observed_duration_s"], 3.0);
        assert_eq!(s["first_line"], 2);
        assert_eq!(s["end_marker_confirmed"], true);
        assert_eq!(s["statistics"][2]["count"], 2);
        assert_eq!(s["statistics"][2]["max"], 10.0);
        assert_eq!(s["statistics"][2]["mean"], 7.5);
        assert!(s["points"][1][2][2].is_null());
    }
    #[test]
    fn bounded_series_discloses_gaps_and_keeps_full_scan_statistics() {
        let mut r = rule();
        r.timestamp_pattern = r"t=(\d+)".into();
        r.timestamp_unit = "ms".into();
        r.start_time = None;
        r.end_pattern = None;
        let mut x = Extractor::new(r).unwrap();
        x.observe("a", 1, "missing timestamp v=1");
        for i in 0..5000 {
            x.observe("a", i + 2, &format!("t={i} v=5000 i=2000"));
        }
        x.observe("a", 5002, "t=1 v=5000 i=2000");
        x.observe("b", 1, "t=1 v=3000 i=1000");
        let v = x.report();
        let s = &v["sources"][0];
        assert_eq!(s["statistics"][2]["count"], 5000);
        assert_eq!(s["missing_or_invalid_timestamp_rows"], 1);
        assert_eq!(s["backward_timestamp_rows_skipped"], 1);
        assert_eq!(s["end_marker_confirmed"], false);
        assert!(s["points"].as_array().unwrap().len() <= 8);
        assert_eq!(s["points"][0][0], 0.0);
        assert_eq!(s["points"].as_array().unwrap().last().unwrap()[0], 4.999);
        assert_eq!(v["sources"][1]["observed_duration_s"], 0.0);
        assert!(v.to_string().len() < 3600);
    }
    #[test]
    fn clock_midnight_and_invalid_capture_or_units_are_explicit() {
        let mut r = rule();
        r.start_time = None;
        let mut x = Extractor::new(r.clone()).unwrap();
        x.observe("a", 1, "23:59:59.500 v=5000 i=1000");
        x.observe("a", 2, "00:00:00.500 v=5000 i=1000 END");
        assert_eq!(x.report()["sources"][0]["observed_duration_s"], 1.0);
        r.fields[0].unit = "mV".into();
        assert!(r.validate().is_err());
        r.fields[0].unit = "V".into();
        r.timestamp_pattern = "no-capture".into();
        let mut invalid_value = Extractor::new(r.clone()).unwrap();
        invalid_value.observe("a", 1, "no-capture v=5000 i=1000");
        assert_eq!(invalid_value.report()["sources"][0]["invalid_timestamp_value_rows"], 1);
        r.timestamp_pattern = "(".into();
        assert!(r.validate().is_err());
    }
}
