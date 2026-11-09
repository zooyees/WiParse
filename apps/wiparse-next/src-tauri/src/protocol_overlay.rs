//! Viewport-bounded protocol marks. Full decoder results remain in the native cache.
use serde::Serialize;
use wiparse_core::bus_decode::{BusDecodeResult,BusByteError,BusBitKind};
#[derive(Serialize)]pub struct Mark {pub id:usize,pub start:f64,pub end:f64,pub label:String,pub error:bool,pub one:Option<bool>,pub kind:String}
#[derive(Serialize)]pub struct Track {pub kind:String,pub total:usize,pub marks:Vec<Mark>}
#[derive(Serialize)]pub struct Overlay {pub tracks:Vec<Track>,pub sampled:bool}
fn visible(a:f64,b:f64,start:f64,end:f64)->bool {a.is_finite()&&b.is_finite()&&b>=a&&b>=start&&a<=end}
fn sampled<T,F>(items:&[T],start:f64,end:f64,limit:usize,time:F)->(usize,Vec<(usize,&T)>) where F:Fn(&T)->(f64,f64) {
    // Chip, bit and byte spans are emitted in time order by the existing decoders.
    let from=items.partition_point(|item|time(item).1<start);
    let to=from+items[from..].partition_point(|item|time(item).0<=end);
    let total=to-from;let stride=total.div_ceil(limit).max(1);
    (total,(from..to).step_by(stride).filter(|&id|{let(a,b)=time(&items[id]);visible(a,b,start,end)}).take(limit).map(|id|(id,&items[id])).collect())
}
pub fn render(result:&BusDecodeResult,start:f64,end:f64,budget:usize,selected:Option<usize>)->Result<Overlay,String>{
    if !start.is_finite()||!end.is_finite()||end<=start||!(32..=1200).contains(&budget){return Err("协议标注显示范围或预算无效".into());}
    let limit=(budget/4).max(8);let mut tracks=Vec::new();
    let all:Vec<_>=result.frames.iter().enumerate().filter(|(_,f)|visible(f.t_start,f.t_end,start,end)).collect();
    let stride=all.len().div_ceil(limit).max(1);let mut frames:Vec<_>=all.iter().step_by(stride).take(limit).copied().collect();
    if let Some(pair)=selected.and_then(|id|all.iter().find(|(index,_)|*index==id)).copied(){if !frames.iter().any(|(id,_)|*id==pair.0){if frames.len()==limit{frames.pop();}frames.push(pair);frames.sort_by_key(|(id,_)|*id);}}
    tracks.push(Track{kind:"frames".into(),total:all.len(),marks:frames.into_iter().map(|(id,f)|Mark{id,start:f.t_start,end:f.t_end,label:f.plot_label().chars().take(48).collect(),error:f.has_error(),one:None,kind:"frame".into()}).collect()});
    let(total,bytes)=sampled(&result.byte_spans,start,end,limit,|b|(b.t_start,b.t_end));
    if total>0{tracks.push(Track{kind:"bytes".into(),total,marks:bytes.into_iter().map(|(id,b)|Mark{id,start:b.t_start,end:b.t_end,label:b.label(),error:b.error!=BusByteError::None,one:None,kind:"byte".into()}).collect()});}
    let(total,bits)=sampled(&result.bits,start,end,limit,|b|(b.t_start,b.t_end));
    if total>0{tracks.push(Track{kind:"bits".into(),total,marks:bits.into_iter().map(|(id,b)|Mark{id,start:b.t_start,end:b.t_end,label:format!("{}:{}",b.label(),u8::from(b.one)),error:b.error,one:Some(b.one),kind:match b.kind{BusBitKind::Start=>"start".into(),BusBitKind::Data{index}=>format!("b{index}"),BusBitKind::Parity=>"parity".into(),BusBitKind::Stop=>"stop".into()}}).collect()});}
    let(total,chips)=sampled(&result.chips,start,end,limit,|b|(b.t_start,b.t_end));
    if total>0{tracks.push(Track{kind:"chips".into(),total,marks:chips.into_iter().map(|(id,b)|Mark{id,start:b.t_start,end:b.t_end,label:b.one.map_or("?",|one|if one{"1"}else{"0"}).into(),error:b.error,one:b.one,kind:"chip".into()}).collect()});}
    let sampled=tracks.iter().any(|track|track.total>track.marks.len());Ok(Overlay{tracks,sampled})
}
#[cfg(test)]mod tests{
 use super::*;use wiparse_core::bus_decode::{BusFrame,BusFrameError,BusChipMark};
 #[test]fn viewport_keeps_selected_frame_without_exceeding_budget(){
  let result=BusDecodeResult{frames:(0..4000).map(|i|BusFrame{t_start:i as f64,t_end:i as f64+0.8,summary:format!("frame{i}"),error:if i==999{BusFrameError::Checksum}else{BusFrameError::None},..Default::default()}).collect(),..Default::default()};
  let out=render(&result,100.,1200.,64,Some(999)).unwrap();assert!(out.sampled);assert_eq!(out.tracks[0].total,1101);assert!(out.tracks[0].marks.len()<=16);assert!(out.tracks[0].marks.iter().any(|m|m.id==999&&m.error));assert!(!out.tracks[0].marks.iter().any(|m|m.start<100.));
 }
 #[test]fn zoomed_chip_window_preserves_original_time_and_uncertain_polarity(){
  let result=BusDecodeResult{chips:(0..10000).map(|i|BusChipMark{t_start:i as f64,t_end:i as f64+0.5,one:if i==9001{None}else{Some(true)},error:i==9002}).collect(),..Default::default()};
  let out=render(&result,9000.,9003.,128,None).unwrap();let track=out.tracks.iter().find(|t|t.kind=="chips").unwrap();assert_eq!(track.total,4);assert!(track.marks.iter().any(|m|m.id==9001&&m.one.is_none()));assert!(track.marks.iter().any(|m|m.id==9002&&m.error));assert!(!out.sampled);
 }
 #[test]fn invalid_or_unbounded_overlay_requests_are_rejected(){let result=BusDecodeResult::default();assert!(render(&result,0.,1.,1201,None).is_err());assert!(render(&result,f64::NAN,1.,100,None).is_err());assert!(render(&result,2.,1.,100,None).is_err());}
}
