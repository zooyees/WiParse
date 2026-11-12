use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Weak,
};
use wiparse_core::log_analysis::timeline::{Dataset, Field};

#[derive(Default)]
pub(crate) struct ExtractCache {
    source: Weak<String>,
    query: String,
    dataset: Option<Arc<Dataset>>,
    full_table: Option<Arc<tempfile::NamedTempFile>>,
    curves: Option<Arc<Vec<HistoryPoints>>>,
}

impl ExtractCache{
    pub(super) fn full_curves(&self,query:&str,dataset:&Dataset)->Option<Arc<Vec<HistoryPoints>>>{
        if self.query!=query || !self.dataset.as_deref().is_some_and(|d|std::ptr::eq(d,dataset)){return None;}
        self.curves.clone()
    }
}

impl DataAnalysisPanel {
    pub(crate) fn file_dataset(&self, id: &str) -> Option<Arc<Dataset>> {
        let cache = self.extract_cache.lock().ok()?;
        if cache.query != id
            || self.source != DataSource::File
            || self.is_loading()
            || !cache
                .source
                .upgrade()
                .zip(self.file_text.as_ref())
                .is_some_and(|(a, b)| Arc::ptr_eq(&a, b))
        {
            return None;
        }
        cache.dataset.clone()
    }

    pub(crate) fn agent_extract(
        &mut self,
        params: &Value,
        ctx: &egui::Context,
        cancel: Arc<AtomicBool>,
    ) -> crate::ai::agent::HostReply {
        use crate::ai::agent::{failure, spawn_work, HostReply};
        let Some(text) = self
            .file_text
            .clone()
            .filter(|_| self.source == DataSource::File && !self.is_loading())
        else {
            return HostReply::value(failure("DATA.OPEN_TEXT_FILE_REQUIRED"));
        };
        let retain = params["retain_dataset"] == true;
        if retain {
            self.extract_cache = Arc::default();
        }
        let cache = self.extract_cache.clone();
        let params = params.clone();
        let source = self
            .last_open_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        if retain {
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::new("ai-file-dataset"), Arc::downgrade(&cache))
            });
        }
        let storage=ctx.data(|d|d.get_temp::<std::path::PathBuf>(egui::Id::new("wiparse-ai-project-root")).zip(d.get_temp::<String>(egui::Id::new("wiparse-ai-project-id"))));
        spawn_work(ctx, move || {
            let header = params["header"].as_str().unwrap_or("");
            let Some(fields) = params["fields"].as_array() else {
                return failure("DATA.INVALID_FIELDS");
            };
            let mut values = Vec::new();
            let mut dataset = Dataset {
                fields: Vec::new(),
                rows: Vec::new(),
                omitted: 0,
                provenance: json!({"x_axis":"record_index","clock_domain":"record_index","synchronized":false,"source":source}),
            };
            for (i, field) in fields.iter().enumerate() {
                let name = field.as_str().unwrap_or("");
                let selector = match Selector::compile(name, true, true) {
                    Ok(v) => v,
                    Err(e) => return failure(e),
                };
                dataset.fields.push(Field {
                    name: name.into(),
                    pattern: name.into(),
                    unit: params["units"]
                        .get(i)
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .into(),
                    scale: 1.,
                });
                values.push((name, selector, AgentReducer::default()));
            }
            let skip_first = params["skip_first_record"] == true;
            let timestamp = if let Some(t) = params.get("timestamp") {
                match Selector::compile(t["selector"].as_str().unwrap_or(""), true, true) {
                    Ok(s) => Some((s, t["unit"].as_str().unwrap_or(""))),
                    Err(e) => return failure(e),
                }
            } else {
                None
            };
            let axis = if timestamp.is_some() {
                "source_seconds"
            } else {
                "record_index"
            };
            dataset.provenance = json!({"x_axis":axis,"producer":"data.extract","clock_domain":if timestamp.is_some(){"source_local"}else{"record_index"},"external_alignment_verified":false,"source":source});
            let coordinate=if header.is_empty(){"source_line_1based"}else{"decoded_utf8_byte_offset_0based"};
            let count=if !retain{0}else if header.is_empty(){text.lines().count()}else{text.match_indices(header).count()};
            let width=if count>250_000 {count.div_ceil((250_000/(2+3*fields.len())).max(1))}else{1};
            let mut bucket=super::retained_rows::Bucket::new(width,fields.len());
            let mut archive=if width>1 {match super::retained_rows::TableWriter::new(coordinate,axis,&dataset.fields){
                Ok(table)=>Some(table),Err(e)=>return failure(&format!("DATA.ARCHIVE_OPEN: {e}")),
            }}else{None};
            let mut curves=(width>1).then(||vec![HistoryPoints::default();fields.len()]);
            let scales=if curves.is_none(){Vec::new()}else{match dataset.fields.iter().map(|f|super::agent_display_unit(&f.unit).map(|(_,scale)|scale)).collect::<Result<Vec<_>,_>>(){
                Ok(scales)=>scales,Err(e)=>return failure(&e),
            }};
            let mut missing_plotted_fields=0u64;
            let mut archive_error=None;
            let mut plottable_rows=0u64;
            let mut last_time = None;
            let mut missing_time = 0;
            let mut backward_time = 0;
            let mut records = 0u64;
            let mut skipped = false;
            let mut scan = |line: &str, position: u64| {
                if skip_first && !skipped {
                    skipped = true;
                    return;
                }
                records += 1;
                let record = Record::new(line);
                let x = match &timestamp {
                    None => Some(records as f64),
                    Some((selector, unit)) => match selector.first(&record).and_then(|raw| {
                        wiparse_core::log_analysis::timeline::parse_time(&raw, unit)
                    }) {
                        None => {
                            missing_time += 1;
                            None
                        }
                        Some(v) if last_time.is_some_and(|prev| v < prev) => {
                            backward_time += 1;
                            None
                        }
                        Some(v) => {
                            last_time = Some(v);
                            Some(v)
                        }
                    },
                };
                let save = retain && x.is_some();
                let mut row = Vec::new();
                for (_, selector, stats) in &mut values {
                    let raw = selector.first(&record);
                    let number = raw.as_deref().and_then(wiparse_core::extract::number);
                    if let Some(v) = number {
                        stats.count += 1;
                        stats.min = Some(stats.min.map_or(v, |old| old.min(v)));
                        stats.max = Some(stats.max.map_or(v, |old| old.max(v)));
                        let n = stats.count as f64;
                        stats.mean = stats.mean * ((n - 1.) / n) + v / n;
                    } else if raw.as_deref().is_some_and(|v| !v.trim().is_empty()) {
                        stats.invalid += 1;
                    }
                    if save {
                        row.push(number);
                    }
                }
                if let Some(x)=x.filter(|_|retain){
                    plottable_rows+=1;
                    if let Some(table)=archive.as_mut(){
                        if archive_error.is_none(){archive_error=table.append(x,position,&row).err();}
                    }
                    missing_plotted_fields+=row.iter().filter(|v|v.is_none()).count() as u64;
                    if let Some(curves)=curves.as_mut(){
                        for (i,curve) in curves.iter_mut().enumerate(){
                            if let Some(v)=row[i]{
                                let scaled=v*scales[i];
                                if !scaled.is_finite(){archive_error=Some(std::io::Error::other("DATA.NONFINITE_CONVERSION"));continue;}
                                curve.push([x,scaled]);
                                if curve.len()%8192==0{curve.seal();if let Err(e)=curve.page_out(){archive_error=Some(e);}}
                            }
                        }
                    }
                    bucket.push((source.clone(),x,position,row),&mut dataset.rows);
                }
            };
            if header.is_empty() {
                for (line_index,line) in text.lines().enumerate() {
                    if cancel.load(Ordering::Relaxed) {
                        return failure("AGENT.CANCELLED");
                    }
                    scan(line,line_index as u64+1);
                }
            } else {
                let mut starts = text.match_indices(header).peekable();
                while let Some((start, _)) = starts.next() {
                    if cancel.load(Ordering::Relaxed) {
                        return failure("AGENT.CANCELLED");
                    }
                    scan(&text[start..starts.peek().map_or(text.len(), |(end, _)| *end)],start as u64);
                }
            }
            bucket.flush(&mut dataset.rows);
            if let Some(e)=archive_error{return failure(&format!("DATA.ARCHIVE_WRITE: {e}"));}
            let full_table=match archive.map(|table|table.finish()).transpose(){
                Ok(table)=>table,Err(e)=>return failure(&format!("DATA.ARCHIVE_FLUSH: {e}")),
            };
            let overview=width>1;
            dataset.provenance["missing_plotted_fields"]=json!(missing_plotted_fields);
            dataset.provenance["overview"]=json!(overview);
            dataset.provenance["overview_rows"]=json!(dataset.rows.len());
            dataset.provenance["plottable_rows"]=json!(plottable_rows);
            dataset.provenance["bucket_records"]=json!(width);
            let matched = values.iter().any(|(_, _, s)| s.count > 0);
            let fields=values.into_iter().map(|(name,_,s)|json!({"selector":name,"matched":s.count,"missing_or_invalid":records-s.count,"missing":records-s.count-s.invalid,"invalid":s.invalid,"min":s.min,"max":s.max,"mean":(s.count>0).then_some(s.mean)})).collect::<Vec<_>>();
            let issue = if records == 0 {
                Some("DATA.NO_RECORDS: inspect header or skip_first_record")
            } else if !matched {
                Some("DATA.NO_VALID_VALUES: inspect selectors and source format")
            } else {
                None
            };
            let decoded_sha256 = wiparse_core::update::hex_sha256(text.as_bytes());
            dataset.provenance["source_coordinate"] = json!(if header.is_empty() {"source_line_1based"} else {"decoded_utf8_byte_offset_0based"});
            dataset.provenance["decoded_text_sha256"] = json!(decoded_sha256);
            dataset.provenance["sources"] = json!([{"source_id":source,"missing_timestamp_rows":missing_time,"backward_rows":backward_time}]);
            let plotted_values = dataset.rows.iter().any(|r| r.3.iter().any(Option::is_some));
            let omitted = dataset.omitted;
            let mut query = None;
            let mut archive_receipt=None;
            if retain && plotted_values && omitted == 0 {
                let id = uuid::Uuid::new_v4().to_string();
                let dataset=Arc::new(dataset);
                if let Some((root,project))=&storage{
                    match crate::ai::artifact::archive_dataset(root,project,&id,dataset.clone(),text.clone(),full_table.clone(),&cancel){
                        Ok(reference)=>archive_receipt=Some(reference),Err(e)=>return failure(&format!("DATA.ARCHIVE_FAILED: {e}"))
                    }
                }
                let Ok(mut slot) = cache.lock() else {
                    return failure("DATA.CACHE_UNAVAILABLE");
                };
                if cancel.load(Ordering::Relaxed) {
                    return failure("AGENT.CANCELLED");
                }
                *slot = ExtractCache {
                    source: Arc::downgrade(&text),
                    query: id.clone(),
                    dataset: Some(dataset),
                    full_table,curves:curves.map(Arc::new),
                };
                query = Some(id);
            }
            json!({"ok":issue.is_none(),"error":issue,"data":{"source":source,"decoded_text_sha256":decoded_sha256,"records_scanned":records,"fields":fields,"complete":true,"recognition":if matched{"matched"}else{"no_numeric_evidence"},"coverage":{"complete":true,"bytes_scanned":text.len(),"skipped_header_records":u64::from(skipped)},"query_id":query,"dataset_archive":archive_receipt,"overview":overview,"plottable_rows":plottable_rows,"bucket_records":width,"dataset_omitted_rows":omitted,"plot_unavailable":retain && query.is_none(),"missing_timestamp_rows":missing_time,"backward_timestamp_rows":backward_time,"x_axis":axis,"units":"source numeric values, no unit conversion","raw_uploaded":false}})
        })
    }
}
pub(crate) fn report_dataset(ctx: &egui::Context) -> Option<(String, Arc<Dataset>, Arc<String>, Option<Arc<tempfile::NamedTempFile>>)> {
    let weak = ctx.data(|d| {
        d.get_temp::<Weak<std::sync::Mutex<ExtractCache>>>(egui::Id::new("ai-file-dataset"))
    })?;
    let cache = weak.upgrade()?;
    let slot = cache.lock().ok()?;
    Some((slot.query.clone(), slot.dataset.clone()?, slot.source.upgrade()?,slot.full_table.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn extract(panel: &mut DataAnalysisPanel, p: Value, ctx: &egui::Context) -> Value {
        panel
            .agent_extract(&p, ctx, Arc::new(AtomicBool::new(false)))
            .receiver
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap()
    }
    #[test]
    fn extraction_status_and_source_bound_dataset() {
        let mut panel = DataAnalysisPanel::new(&AppConfig::default());
        panel.source = DataSource::File;
        panel.file_text = Some(Arc::new("ts,v\n0,5\n1,\n2,bad\n3,6".into()));
        let ctx = egui::Context::default();
        let none = extract(&mut panel, json!({"fields":["csv:2"],"header":"@@"}), &ctx);
        assert_eq!(none["ok"], false);
        assert_eq!(none["data"]["complete"], true);
        let all_missing = extract(&mut panel, json!({"fields":["csv:5"]}), &ctx);
        assert_eq!(all_missing["ok"], false);
        for timed in [false, true] {
            let mut p = json!({"skip_first_record":true,"fields":["csv:2"],"units":["V"],"retain_dataset":true});
            if timed {
                p["timestamp"] = json!({"selector":"csv:1","unit":"ms"});
            }
            let v = extract(&mut panel, p, &ctx);
            let d = &v["data"];
            assert_eq!(d["fields"][0]["missing"], 1);
            assert_eq!(d["fields"][0]["invalid"], 1);
            let id = d["query_id"].as_str().unwrap();
            let dataset = panel.file_dataset(id).unwrap();
            assert_eq!(dataset.rows.len(), 4);
            assert_eq!(dataset.rows[0].2,2);
            assert_eq!(dataset.rows[2].3[0], None);
            let result = panel.agent_present(&dataset, id, Lang::Zh).unwrap();
            assert_eq!(result["x_unit"], if timed { "s" } else { "record_index" });
            assert_eq!(
                panel.series[0].points[1],
                [if timed { 0.003 } else { 4. }, 6000.]
            );
            assert!(panel.file_dataset(id).is_some());
            assert_eq!(report_dataset(&ctx).unwrap().0, id);
        }
        let root =
            std::env::temp_dir().join(format!("wiparse-file-series-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let ev = json!({"evidence_id":"E1","ok":true,"method":"data.extract","result":{"data":{"query_id":report_dataset(&ctx).unwrap().0}}});
        let rx = crate::ai::artifact::start(
            root.clone(),
            "p".into(),
            json!({"title":"file","markdown":"summary"}),
            vec![ev],
            Arc::new(AtomicBool::new(false)),
            ctx.clone(),
            |v| v,
        )
        .unwrap();
        let report = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert_eq!(report["data"]["curves"], 1);
        let path = root.join(report["data"]["relative_path"].as_str().unwrap());
        let csv = std::fs::read_to_string(path.with_file_name("dataset-0-0.csv")).unwrap();
        assert_eq!(csv, "time_s,value\n0,5\n0.001,\n0.002,\n0.003,6\n");
        std::fs::remove_dir_all(root).unwrap();
        let id = report_dataset(&ctx).unwrap().0;
        panel.file_text = Some(Arc::new("different".into()));
        assert!(panel.file_dataset(&id).is_none());
    }
    #[test]
    fn million_row_spike_matches_full_curve_report_and_source() {
        use std::io::{BufRead,BufReader};
        let mut panel=DataAnalysisPanel::new(&AppConfig::default());
        panel.source=DataSource::File;
        let mut text="5\n".repeat(1_000_000);
        text.replace_range(1_137_246..1_137_247,"87");
        panel.file_text=Some(Arc::new(text));
        let ctx=egui::Context::default();
        let result=panel.agent_extract(&json!({"fields":["csv:1"],"units":["V"],"retain_dataset":true}),&ctx,
            Arc::new(AtomicBool::new(false))).receiver.unwrap().recv_timeout(std::time::Duration::from_secs(120)).unwrap();
        let query=result["data"]["query_id"].as_str().unwrap();
        let (_,dataset,_,table)=report_dataset(&ctx).unwrap();
        assert_eq!(dataset.provenance["plottable_rows"],1_000_000);
        assert!(dataset.rows.len()<=250_000);
        assert!(dataset.rows.iter().any(|r|r.2==568624 && r.3[0]==Some(87.0)));
        panel.agent_present(&dataset,query,Lang::Zh).unwrap();
        assert_eq!(panel.series[0].points.len(),1_000_000);
        assert_eq!(panel.series[0].points[568623],[568624.0,87000.0]);
        let mut lines=BufReader::new(table.unwrap().reopen().unwrap()).lines();
        assert_eq!(lines.nth(568624).unwrap().unwrap(),"568624,568624,87");
        assert_eq!(lines.count(),1_000_000-568624);
    }
    #[test]
    fn current_file_without_time_produces_counts_and_keeps_gaps() {
        let mut panel = DataAnalysisPanel::new(&AppConfig::default());
        panel.source = DataSource::File;
        panel.file_text = Some(Arc::new("rpm,load\n800,12\n,13\n900,14".into()));
        let params = json!({"fields":["csv:1"]});
        let reply = panel.agent_extract(
            &params,
            &egui::Context::default(),
            Arc::new(AtomicBool::new(false)),
        );
        let result = reply
            .receiver
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        let field = &result["data"]["fields"][0];
        assert_eq!(field["matched"], 2);
        assert_eq!(field["missing_or_invalid"], 2);
        assert_eq!(field["mean"], 850.);
        assert_eq!(result["data"]["raw_uploaded"], false);
    }
}
