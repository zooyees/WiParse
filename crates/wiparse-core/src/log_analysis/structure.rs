//! Bounded local schema discovery. Observations, never interpretations of private codes.
use regex::Regex;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
struct Field {
    count: u64,
    numeric: u64,
    min: Option<f64>,
    max: Option<f64>,
    units: BTreeMap<String, u64>,
    values: BTreeMap<String, u64>,
    values_omitted: u64,
    first: Option<(String, u64)>,
    last: Option<(String, u64)>,
}
pub struct Discovery {
    pairs: Regex,
    clock: Regex,
    fields: BTreeMap<String, Field>,
    omitted: u64,
    clock_lines: u64,
    sample: String,
    sample_source: String,
    sample_lines: usize,
}
impl Discovery {
    pub fn new() -> Self {
        Self {
            pairs: Regex::new(r#"["']?([\p{L}_][\p{L}\p{N}_.-]{0,39})["']?\s*[:=]\s*["']?([+-]?(?:0[xX][0-9A-Fa-f]+|0[bB][01]+|(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)|[\p{L}_][\p{L}\p{N}_.-]{0,39})([a-zA-Zµ°%/]{0,12})"#).unwrap(),
            clock: Regex::new(r"\b\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?\b").unwrap(),
            fields: BTreeMap::new(), omitted: 0, clock_lines: 0, sample:String::new(),sample_source:String::new(),sample_lines:0,
        }
    }
    pub fn observe(&mut self, source: &str, line: u64, text: &str) {
        if self.sample_lines==0 {self.sample_source=source.to_owned();}
        if self.sample_lines<64 && source==self.sample_source && self.sample.len()+text.len()<16384 {
            self.sample.push_str(text);self.sample.push('\n');self.sample_lines+=1;
        }
        if self.clock.is_match(text) { self.clock_lines += 1; }
        for c in self.pairs.captures_iter(text).take(128) {
            let key = &c[1];
            if !self.fields.contains_key(key) && self.fields.len() >= 64 { self.omitted += 1; continue; }
            let f = self.fields.entry(key.into()).or_default();
            f.count += 1;
            let position = (source.to_owned(), line);
            if f.first.is_none() { f.first = Some(position.clone()); }
            f.last = Some(position);
            if let Some(number) = crate::extract::number(&c[2]) {
                if !number.is_finite() { continue; }
                f.numeric += 1;
                f.min = Some(f.min.map_or(number, |v|v.min(number)));
                f.max = Some(f.max.map_or(number, |v|v.max(number)));
                let unit = &c[3];
                if f.units.contains_key(unit) || f.units.len()<8 { *f.units.entry(unit.into()).or_default() += 1; }
                else { f.values_omitted += 1; }
            } else {
                let value = format!("{}{}", &c[2], &c[3]);
                if f.values.contains_key(&value) || f.values.len() < 8 { *f.values.entry(value).or_default() += 1; }
                else { f.values_omitted += 1; }
            }
        }
    }
    pub fn report(&self) -> Value {
        let mut fields: Vec<_> = self.fields.iter().collect();
        fields.sort_by(|a,b|b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
        json!({"kind":"observed_key_value_schema","selector_candidates":crate::extract::discover(&self.sample).into_iter().take(8).collect::<Vec<_>>(),"candidate_source":self.sample_source,"fields_tracked":fields.len(),"untracked_field_occurrences":self.omitted,"fields_list_truncated":fields.len()>8,
            "clock_lines":self.clock_lines,"timestamp_recipe":if self.clock_lines>0{json!({"timestamp_pattern":"(\\d{2}:\\d{2}:\\d{2}(?:\\.\\d+)?)","timestamp_unit":"clock","warning":"clock-only; date/timezone unknown, day rollover must be checked"})}else{Value::Null},
            "fields":fields.into_iter().take(8).map(|(name,f)|{
                let pattern=format!(r#"["']?{}["']?\s*[:=]\s*["']?([+-]?(?:0[xX][0-9A-Fa-f]+|0[bB][01]+|(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?))"#,regex::escape(name));
                json!({"name":name,"occurrences":f.count,"numeric":f.numeric,"min":f.min,"max":f.max,"units":f.units,"states":f.values,"untracked_values":f.values_omitted,"first":f.first,"last":f.last,"numeric_pattern":if f.numeric>0{Some(pattern)}else{None}})
            }).collect::<Vec<_>>(),"meaning":"Units are literal attached suffixes. Empty unit means unknown. Mixed units must not be combined without conversion. State values are observed codes, not inferred business meaning. Recipes are candidates; inspect examples before choosing transition rules. Selector candidates are [label, pattern] pairs from a bounded source sample. Use the pattern directly in numeric or timeline fields."})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn discovers_units_states_and_valid_extraction_without_product_vocabulary() {
        let mut d=Discovery::new();
        d.observe("s",1,r#"20:05:09.865 {"VIN":5000mV,"I":1.2A,"state":"charging"}"#);
        d.observe("s",2,"20:06:00.000 VIN=4.9V I=-2e-3A state=idle");
        let r=d.report();let fields=r["fields"].as_array().unwrap();
        let vin=fields.iter().find(|f|f["name"]=="VIN").unwrap();
        assert_eq!(vin["units"]["mV"],1);assert_eq!(vin["units"]["V"],1);
        assert_eq!(vin["numeric"],2);assert_eq!(r["clock_lines"],2);
        let regex=Regex::new(vin["numeric_pattern"].as_str().unwrap()).unwrap();
        assert_eq!(&regex.captures("VIN=-2e-3V").unwrap()[1],"-2e-3");
        assert!(fields.iter().any(|f|f["states"]["idle"]==1));
    }
    #[test] fn schema_and_enum_memory_are_bounded() {
        let mut d=Discovery::new();
        for n in 0..100 { d.observe("s",n,&format!("field{n}={n} state=value{n}")); }
        let r=d.report();assert_eq!(r["fields_tracked"],64);assert!(r["untracked_field_occurrences"].as_u64().unwrap()>0);
        assert!(r["fields_list_truncated"].as_bool().unwrap());assert_eq!(r["fields"].as_array().unwrap().len(),8);
    }
}
