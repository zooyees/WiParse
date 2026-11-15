//! Immutable in-process curve samples for an explicitly reviewed task.
use super::*;
use std::sync::atomic::{AtomicBool,Ordering};

struct Curve { label:String,unit:String,kind:String,scale:f64,offset:f64,points:HistoryPoints }
pub(crate) struct ReviewedDataSnapshot {
    id:String, instance:String, source_generation:u64, live_session:String, live_source_generation:Option<u64>, imported_query:Option<String>, source:String, x_axis:String, frame_count:usize,
    live_cursor:usize, window_x:Option<(f64,f64)>, window_y:Option<(f64,f64)>,
    imported_quality:Option<Value>,gap_notice:Option<String>,curves:Vec<Curve>,
}
impl DataAnalysisPanel {
    pub(crate) fn capture_review_data(&self)->Option<wiparse_core::ai::ReviewedSource<Arc<ReviewedDataSnapshot>>> {
        let view=self.visible_result().unwrap_or(self);
        if self.pending_load.is_some() || view.pending_load.is_some() || view.cache_quarantined || view.cache_rebuilding || view.history_error.is_some() || !view.series.iter().any(|series|!series.points.is_empty()) {return None;}
        wiparse_core::ai::ReviewedSource::capture(&self.data_review_lease,||Arc::new(ReviewedDataSnapshot {
            id:uuid::Uuid::new_v4().to_string(),instance:view.agent_instance.to_string(),source_generation:view.agent_generation,live_session:view.live_session_id.clone(),live_source_generation:view.live_source_generation,imported_query:view.imported_query.clone(),source:view.source.as_config_str().to_string(),
            x_axis:if view.imported_seconds(){"seconds parsed from source timestamps; independent sources not synchronized"}else{"frame/record index; not physical time"}.into(),
            frame_count:view.frame_count,live_cursor:view.live_cursor,window_x:view.last_x_range,window_y:view.last_y_range,
            imported_quality:view.imported_quality.clone(),gap_notice:view.gap_notice.clone(),
            curves:view.series.iter().map(|s|Curve{label:s.label.clone(),unit:s.unit_override.as_deref().unwrap_or(s.kind.unit()).to_string(),kind:s.kind.as_config_str().to_string(),scale:s.y_scale,offset:s.y_offset,points:s.points.clone()}).collect(),
        }))
    }
}
impl ReviewedDataSnapshot {
    pub(crate) fn manifest(&self)->Value {json!({"snapshot_id":self.id,"view_instance":self.instance,"source_generation":self.source_generation,"live_session":self.live_session,"live_source_generation":self.live_source_generation,"imported_query":self.imported_query,"source_mode":self.source,"x_axis":self.x_axis,
        "frame_count":self.frame_count,"live_cursor":self.live_cursor,"window_x":self.window_x,"window_y":self.window_y,
        "source_quality":self.imported_quality,"gap_notice":self.gap_notice,"curves":self.curves.iter().enumerate().map(|(i,c)|json!({"index":i,"curve_id":format!("{}:{i}",self.id),"label":c.label,"unit":c.unit,"kind":c.kind,"points":c.points.len(),"y_scale":c.scale,"y_offset":c.offset})).collect::<Vec<_>>(),
        "scope":"frozen displayed in-process points, not proof of external file contents or physical acquisition; later appends and later view changes excluded","raw_samples_included":false})}
    fn bounds(&self,p:&Value)->Result<(usize,usize,usize),String> {
        if p.get("snapshot_id").is_some_and(|id|id.as_str()!=Some(self.id.as_str())) {return Err("DATA.SNAPSHOT_CHANGED".into());}
        let series=p["series"].as_u64().and_then(|n|usize::try_from(n).ok()).ok_or("DATA.INVALID_SERIES")?;
        let curve=self.curves.get(series).ok_or("DATA.INVALID_SERIES")?;
        let start=p["start"].as_u64().unwrap_or(0).try_into().map_err(|_|"DATA.INVALID_WINDOW")?;
        let end=p["end"].as_u64().unwrap_or(curve.points.len() as u64).try_into().map_err(|_|"DATA.INVALID_WINDOW")?;
        if start>end || end>curve.points.len() {return Err("DATA.INVALID_WINDOW".into());}
        Ok((series,start,end))
    }
    pub(crate) fn read(&self,p:&Value)->Value {
        let (index,start,end)=match self.bounds(p){Ok(v)=>v,Err(e)=>return crate::ai::agent::failure(e)};
        let offset=p["offset"].as_u64().unwrap_or(start as u64);
        let Ok(offset)=usize::try_from(offset)else{return crate::ai::agent::failure("DATA.INVALID_PAGE");};
        let limit=p["limit"].as_u64().unwrap_or(32);
        if !(1..=64).contains(&limit)||offset<start||offset>end{return crate::ai::agent::failure("DATA.INVALID_PAGE");}
        let stop=offset.saturating_add(limit as usize).min(end);let curve=&self.curves[index];
        json!({"ok":true,"data":{"snapshot_id":self.id,"curve_id":format!("{}:{index}",self.id),"series":index,"unit":curve.unit,"x_axis":self.x_axis,
            "window":{"start":start,"end_exclusive":end,"total_points":curve.points.len()},"page_offset":offset,"page_count":stop-offset,
            "points":(offset..stop).map(|i|{let [x,y]=curve.points[i];json!({"index":i,"x":x.is_finite().then_some(x),"y":y.is_finite().then_some(y),"valid":x.is_finite()&&y.is_finite()})}).collect::<Vec<_>>(),
            "next_offset":(stop<end).then_some(stop),"reached_window_end":stop==end,"window_covered_by_page":offset==start && stop==end,"source_complete":false,"scope":"only frozen in-memory curve window; external capture completeness unverified"}})
    }
    pub(crate) fn stats(self:Arc<Self>,p:&Value,ctx:&egui::Context,cancel:Arc<AtomicBool>)->crate::ai::agent::HostReply {
        let (index,start,end)=match self.bounds(p){Ok(v)=>v,Err(e)=>return crate::ai::agent::HostReply::value(crate::ai::agent::failure(e))};
        crate::ai::agent::spawn_work(ctx,move||{
            let curve=&self.curves[index];let mut count=0u64;let mut invalid=0u64;let mut mean=0.;let(mut min,mut max)=(None::<f64>,None::<f64>);
            for n in start..end {if n%4096==0 && cancel.load(Ordering::Acquire){return crate::ai::agent::failure("AGENT.CANCELLED");}
                let [x,y]=curve.points[n];if !x.is_finite()||!y.is_finite(){invalid+=1;continue;}count+=1;mean=mean*(1.-1./count as f64)+y/count as f64;min=Some(min.map_or(y,|v|v.min(y)));max=Some(max.map_or(y,|v|v.max(y)));
            }
            json!({"ok":true,"data":{"snapshot_id":self.id,"curve_id":format!("{}:{index}",self.id),"series":index,"unit":curve.unit,"x_axis":self.x_axis,
                "window":{"start":start,"end_exclusive":end,"total_points":curve.points.len()},"examined_points":end-start,"valid_points":count,"invalid_points":invalid,
                "min":min,"max":max,"mean":(count>0&&mean.is_finite()).then_some(mean),"window_complete":true,"numeric_complete":count>0 && invalid==0,"source_complete":false,
                "scope":"computed on frozen displayed points; not proof of external source or physical acquisition completeness"}})
        })
    }
    pub(crate) fn matches(&self,params:&Value,result:&Value)->bool {
        self.bounds(params).ok().is_some_and(|(index,start,end)|result["ok"]==true && result["data"]["snapshot_id"]==self.id &&
            result["data"]["curve_id"]==format!("{}:{index}",self.id) && result["data"]["window"]["start"]==start && result["data"]["window"]["end_exclusive"]==end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_curve_pages_are_bound_to_one_snapshot_and_append_is_excluded() {
        let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.source=DataSource::SerialLive;
        panel.series_count=1;panel.ensure_series_shells(Lang::Zh);
        panel.series[0].points=vec![[0.,1.],[1.,f64::NAN],[2.,3.]].into();panel.frame_count=3;
        let source=panel.capture_review_data().unwrap();let snapshot=source.value().clone();
        let first=snapshot.read(&json!({"series":0,"limit":2}));
        assert_eq!(first["data"]["next_offset"],2);assert_eq!(first["data"]["points"][1]["valid"],false);
        panel.source_lease.clone().mutate_always(||panel.series[0].points.push([3.,4.]));
        assert!(source.is_current());assert_eq!(snapshot.manifest()["curves"][0]["points"],3);
        let second=snapshot.read(&json!({"series":0,"offset":2,"snapshot_id":snapshot.id}));
        assert_eq!(second["data"]["points"][0]["y"],3.);assert!(snapshot.matches(&json!({"series":0,"offset":2}),&second));
        assert_eq!(snapshot.read(&json!({"series":0,"snapshot_id":uuid::Uuid::new_v4().to_string()}))["ok"],false);
        panel.commit_thresholds((true,5.),(false,0.),(false,0.));assert!(!source.is_current());
    }
    #[test]
    fn separate_curves_keep_independent_windows_and_no_clock_claim() {
        let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.series_count=2;panel.ensure_series_shells(Lang::Zh);
        panel.series[0].points=vec![[0.,1.],[1.,2.]].into();panel.series[1].points=vec![[100.,5.]].into();
        panel.imported_query=Some("fixture".into());
        let snapshot=panel.capture_review_data().unwrap();let snap=snapshot.value();
        assert!(snap.manifest()["x_axis"].as_str().unwrap().contains("not synchronized"));
        let left=snap.read(&json!({"series":0}));let right=snap.read(&json!({"series":1}));
        assert_ne!(left["data"]["curve_id"],right["data"]["curve_id"]);
        assert_eq!(left["data"]["window"]["total_points"],2);assert_eq!(right["data"]["window"]["total_points"],1);
        assert!(!snap.matches(&json!({"series":1}),&left));
    }
    #[test]
    fn large_unsealed_tail_is_shared_without_reducing_coverage() {
        let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.series_count=1;panel.ensure_series_shells(Lang::Zh);
        panel.series[0].points=vec![[0.,1.];1_000_000].into();
        let source=panel.capture_review_data().unwrap();
        assert_eq!(source.value().manifest()["curves"][0]["points"],1_000_000);
        let page=source.value().read(&json!({"series":0,"offset":999_999}));
        assert_eq!(page["data"]["points"][0]["y"],1.);
        assert_eq!(page["data"]["next_offset"],Value::Null);
    }
    #[test]
    fn source_replacement_invalidates_and_loading_blocks_capture() {
        let mut panel=DataAnalysisPanel::new(&AppConfig::default());panel.series_count=1;panel.ensure_series_shells(Lang::Zh);
        panel.series[0].points=vec![[0.,1.]].into();let source=panel.capture_review_data().unwrap();
        panel.clear_data(Lang::Zh);assert!(!source.is_current());assert!(panel.capture_review_data().is_none());
    }
}
