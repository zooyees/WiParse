//! Correlate captured event streams using observed synchronization anchors.
use super::{stream::{EventRecord,StreamValidator},time::{MappingRequest,convert,fit}};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureWindow {
    reference_clock_id:String,
    start_reference_s:f64,
    end_reference_s:f64,
    streams:Vec<CaptureStream>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureStream {
    source:String,
    stream_id:String,
    mapping:MappingRequest,
    records:Vec<EventRecord>,
}
pub fn correlate(request:CaptureWindow)->Result<Value,String>{
    let (start,end)=(request.start_reference_s,request.end_reference_s);
    if !start.is_finite() || !end.is_finite() || start>=end || request.reference_clock_id.is_empty()
        || request.streams.is_empty() || request.streams.len()>32 ||
        request.streams.iter().map(|s|s.records.len()).sum::<usize>()>65536 {
        return Err("CAPTURE.INVALID_WINDOW_OR_SIZE".into());
    }
    let mut identities=HashSet::new();
    let mut results=Vec::new();
    let mut complete=true;
    for stream in request.streams {
        if !identities.insert((stream.source.clone(),stream.stream_id.clone())){return Err("CAPTURE.DUPLICATE_STREAM".into());}
        if stream.mapping.reference_clock_id!=request.reference_clock_id{return Err("CAPTURE.REFERENCE_CLOCK_MISMATCH".into());}
        let mapping=fit(&stream.mapping)?;
        let mut validator=StreamValidator::default();
        let (mut bad,mut unmatched,mut certain,mut boundary)=(0usize,0usize,0usize,0usize);
        let (mut covers_first,mut covers_last)=(false,false);
        let mut examples=Vec::new();
        let total=stream.records.len();
        for mut e in stream.records {
            if e.source!=stream.source || e.stream_id!=stream.stream_id || e.tick_hz!=stream.mapping.tick_hz{
                return Err("CAPTURE.STREAM_IDENTITY_MISMATCH".into());
            }
            validator.ingest(&mut e)?;
            if !e.quality.valid(){bad+=1;}
            let Ok(t)=convert(&mapping,&e.device_tick,&e.boot_id,&e.capture_session_id) else{unmatched+=1;continue;};
            let lo=t-mapping.uncertainty_s;
            let hi=t+mapping.uncertainty_s;
            if e.quality.valid(){covers_first|=hi<=start;covers_last|=lo>=end;}
            if lo>end || hi<start{continue;}
            let within=lo>=start && hi<=end;
            if within && e.quality.valid(){certain+=1;}else{boundary+=1;}
            if examples.len()<32{examples.push(json!({"seq":e.seq,"tick":e.device_tick,"event_type":e.event_type,
                "reference_s":t,"uncertainty_s":mapping.uncertainty_s,"within_window":within,"quality":e.quality}));}
        }
        let covered=bad==0 && unmatched==0 && covers_first && covers_last;
        complete &= covered;
        results.push(json!({"source":stream.source,"stream_id":stream.stream_id,"boot_id":mapping.boot_id,
            "capture_session_id":mapping.capture_session_id,"records":total,"records_with_quality_issues":bad,
            "unmapped_records":unmatched,"within_window":certain,"uncertain_or_degraded":boundary,
            "bracketed_with_clean_stream":covered,"examples":examples,"examples_omitted":(certain+boundary).saturating_sub(32)}));
    }
    Ok(json!({"reference_clock_id":request.reference_clock_id,"window_s":[start,end],
        "all_streams_bracket_window":complete,"streams":results,"physical_causality_verified":false}))
}

#[cfg(test)]
mod window_tests {
    use super::*;
    #[test]
    fn mapped_window_flags_missing_sequence_and_rejects_foreign_reference() {
        let records:Vec<_>=[0,1,3,4].into_iter().map(|seq|json!({
            "source":"uart","boot_id":"b","capture_session_id":"s","stream_id":"0",
            "seq":seq.to_string(),"device_tick":seq.to_string(),"tick_hz":1,
            "event_type":"edge","payload":{},"quality":{"issues":[]}
        })).collect();
        let mut input=json!({"reference_clock_id":"scope","start_reference_s":1.2,"end_reference_s":3.2,
            "streams":[{"source":"uart","stream_id":"0","mapping":{"clock_id":"dongle","reference_clock_id":"scope",
            "boot_id":"b","capture_session_id":"s","tick_hz":1,"sync_method":"pulse","sync_uncertainty_s":0.01,
            "max_residual_s":0.1,"quality":{"issues":[]},"pairs":[{"tick":"0","reference_s":0.0},
            {"tick":"2","reference_s":2.0},{"tick":"4","reference_s":4.0}]},"records":records}]});
        let value=correlate(serde_json::from_value(input.clone()).unwrap()).unwrap();
        assert_eq!(value["all_streams_bracket_window"],false);
        assert_eq!(value["streams"][0]["records_with_quality_issues"],1);
        input["reference_clock_id"]=json!("unrelated");
        assert!(correlate(serde_json::from_value(input).unwrap()).is_err());
    }
}
