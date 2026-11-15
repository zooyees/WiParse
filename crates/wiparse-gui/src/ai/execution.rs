//! Host-owned lifecycle of one tool call, shared by the panel and serial bridge.
use super::agent::{failure, HostReply};
use serde_json::Value;
use std::time::{Duration, Instant};
use wiparse_core::engineering_agent::Call;

// Keep normal segments below the model-facing response cap so their
// continuation tokens and evidence IDs remain usable without rereading.
const SKILL_SEGMENT_BYTES: usize = 32 * 1024;
const SKILL_SEGMENT_PAGES: usize = 256;

pub(crate) fn timeout(method: &str) -> Duration {
    Duration::from_secs(match method {
        "testing.wait" => 600,
        "serial.start" | "serial.monitor.start" => 15,
        _ => 90,
    })
}

pub(crate) struct HostItem {
    receiver: Option<crossbeam_channel::Receiver<Value>>,
    pub continuation: Option<Call>,
    next: Instant,
    frame_pages: Option<FramePages>,
    report_pages: Option<ReportPages>,
    log_scan: Option<LogScan>,
}

struct LogScan {
    call: Call,
    query: Option<String>,
    bytes: u64,
    slices: usize,
    last: Option<Value>,
}
impl LogScan {
    fn partial(&self, error: &str) -> Value {
        serde_json::json!({"ok":false,"error":error,"data":self.last,"partial":true})
    }
}

struct ReportPages {
    call: Call,
    source: Option<Value>,
    source_version: Option<Value>,
    run_provenance: Option<Value>,
    bytes: Option<Value>,
    next_cursor: Option<String>,
    seen: std::collections::HashSet<String>,
    lines: Vec<Value>,
    pages: Vec<Value>,
}
impl ReportPages {
    fn partial(&self, error: &str) -> Value {
        serde_json::json!({"ok":false,"error":error,"data":{"partial":true,
            "lines":self.lines,"page_evidence":self.pages,"next_cursor":self.next_cursor,
            "source_id":self.source,"source_bytes":self.bytes,"reached_eof":false}})
    }
    fn segment(&self) -> Value {
        serde_json::json!({"ok":true,"data":{"partial":true,"lines":self.lines,
            "page_evidence":self.pages,"next_cursor":self.next_cursor,
            "source_id":self.source,"source_bytes":self.bytes,"run_provenance":self.run_provenance,
            "source_version":self.source_version,
            "reached_eof":false,"coverage_note":"Segment only; continue with next_cursor before claiming full coverage."}})
    }
}

struct FramePages {
    call: Call,
    source: Option<Value>,
    protocol: Option<Value>,
    total: Option<u64>,
    statistics: Option<Value>,
    next_offset: u64,
    frames: Vec<Value>,
    pages: Vec<Value>,
}
impl FramePages {
    fn partial(&self, error: &str) -> Value {
        serde_json::json!({"ok":false,"error":error,"data":{"partial":true,
            "frames":self.frames,"page_evidence":self.pages,"next_offset":self.next_offset,
            "total_matches":self.total,"provenance":self.source,"protocol":self.protocol}})
    }
    fn segment(&self, next_offset:u64) -> Value {
        serde_json::json!({"ok":true,"data":{"partial":true,"frames":self.frames,
            "page_evidence":self.pages,"next_offset":next_offset,"total_matches":self.total,
            "provenance":self.source,"protocol":self.protocol,"protocol_statistics":self.statistics,
            "decoder_truncated":false,"decoder_error":null,
            "coverage_note":"Segment only; continue at next_offset and combine all segments before claiming full coverage."}})
    }
}

impl Default for HostItem {
    fn default() -> Self {
        Self {
            receiver: None,
            continuation: None,
            next: Instant::now(),
            frame_pages: None,
            report_pages: None,
            log_scan: None,
        }
    }
}

impl HostItem {
    pub fn partial_result(&self, error: &str) -> Option<Value> {
        self.frame_pages.as_ref().filter(|pages| !pages.pages.is_empty()).map(|pages| pages.partial(error))
            .or_else(|| self.report_pages.as_ref().filter(|pages| !pages.pages.is_empty()).map(|pages| pages.partial(error)))
            .or_else(|| self.log_scan.as_ref().filter(|scan| scan.last.is_some()).map(|scan| scan.partial(error)))
    }
    pub fn collect_log(&mut self, call: &Call) {
        if call.method == "log.analyze" && call.params["cursor"].is_null() {
            self.log_scan=Some(LogScan {call:call.clone(),query:None,bytes:0,slices:0,last:None});
        }
    }
    pub fn collect_report(&mut self, call: &Call) {
        if call.method == "report.read" && call.params["scan"].is_null()
            && !structured_adapter_request(call)
            && call.params["start_line"].as_u64().unwrap_or(1) == 1 && call.params["start_byte"].as_u64().unwrap_or(0) == 0 {
            self.report_pages = Some(ReportPages { call:call.clone(), source:None, source_version:None, run_provenance:None, bytes:None, next_cursor:call.params["cursor"].as_str().map(str::to_owned),
                seen:Default::default(), lines:Vec::new(), pages:Vec::new() });
        }
    }
    pub fn collect_frames(&mut self, call: &Call) {
        if call.method == "wave.frames" {
            self.frame_pages = Some(FramePages { call: call.clone(), source: None, protocol: None,
                total: None, statistics:None, next_offset:call.params["offset"].as_u64().unwrap_or(0), frames: Vec::new(), pages: Vec::new() });
        }
    }
    pub fn receiving(&self) -> bool {
        self.receiver.is_some()
    }

    pub fn attach(&mut self, reply: HostReply) -> Option<Value> {
        self.receiver = reply.receiver;
        self.receiver.is_none().then_some(reply.value)
    }

    pub fn receive(&mut self) -> Option<Value> {
        let result = match self.receiver.as_ref()?.try_recv() {
            Ok(value) => value,
            Err(crossbeam_channel::TryRecvError::Empty) => return None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                failure("AGENT.WORKER_DISCONNECTED")
            }
        };
        self.receiver = None;
        Some(result)
    }

    /// A status read may be repeated. An acknowledgement can never replay the original write.
    pub fn advance(&mut self, value: Value, current: &Call, internal: bool) -> Option<Value> {
        if let Some(scan) = &mut self.log_scan {
            let data=&value["data"];
            let query=data["query_id"].as_str();
            let bytes=data["coverage"]["bytes_read"].as_u64();
            if value["ok"]!=true || query.is_none() || bytes.is_none()
                || scan.query.as_deref().is_some_and(|old|Some(old)!=query)
                || (scan.slices>0 && current.params["cursor"].as_str()!=scan.query.as_deref()) {
                let result=if scan.last.is_some(){scan.partial("SKILL.LOG_SCAN_CHANGED_OR_FAILED")}else{value};
                self.log_scan=None;return Some(result);
            }
            let previous=scan.bytes; scan.bytes=bytes.unwrap(); scan.slices+=1;
            scan.query=query.map(str::to_owned); scan.last=Some(data.clone());
            if let Some(cursor)=data["continuation"].as_str() {
                if Some(cursor)!=scan.query.as_deref() || scan.bytes<=previous || scan.slices>=32 {
                    let result=scan.partial("SKILL.LOG_SCAN_LIMIT_OR_NO_PROGRESS");self.log_scan=None;return Some(result);
                }
                self.continuation=Some(Call{method:scan.call.method.clone(),params:serde_json::json!({"cursor":cursor})});
                self.next=Instant::now();return None;
            }
            if data["coverage"]["complete"]!=true {
                let result=scan.partial("SKILL.LOG_SCAN_INCOMPLETE");self.log_scan=None;return Some(result);
            }
            self.log_scan=None;return Some(value);
        }
        if let Some(pages) = &mut self.report_pages {
            let data=&value["data"];
            let Some(lines)=data["lines"].as_array() else {
                let result=if pages.pages.is_empty(){value}else{pages.partial("SKILL.REPORT_PAGE_READ_FAILED")};
                self.report_pages=None; return Some(result);
            };
            let source=&data["source_id"]; let size=&data["source_bytes"];
            if value["ok"]!=true || source.as_str().is_none() || size.as_u64().is_none()
                || pages.source.as_ref().is_some_and(|old|old!=source)
                || pages.source_version.as_ref().is_some_and(|old|old!=&data["source_version"])
                || pages.run_provenance.as_ref().is_some_and(|old|old!=&data["run_provenance"])
                || pages.bytes.as_ref().is_some_and(|old|old!=size)
                || current.params["cursor"].as_str()!=pages.next_cursor.as_deref() {
                let result=pages.partial("SKILL.REPORT_PAGE_CHANGED_OR_INVALID"); self.report_pages=None; return Some(result);
            }
            pages.source=Some(source.clone()); pages.source_version=Some(data["source_version"].clone()); pages.run_provenance=Some(data["run_provenance"].clone()); pages.bytes=Some(size.clone());
            let existing=serde_json::to_vec(&pages.lines).map_or(usize::MAX,|bytes|bytes.len());
            let incoming=serde_json::to_vec(lines).map_or(usize::MAX,|bytes|bytes.len());
            if existing.saturating_add(incoming)>SKILL_SEGMENT_BYTES {
                let result=if pages.lines.is_empty(){pages.partial("SKILL.REPORT_SINGLE_PAGE_TOO_LARGE")}else{pages.segment()};
                self.report_pages=None; return Some(result);
            }
            pages.lines.extend(lines.iter().cloned());
            pages.pages.push(serde_json::json!({"start_line":data["start_line"],"count":lines.len()}));
            if let Some(next)=data["next_cursor"].as_str() {
                pages.next_cursor=Some(next.to_owned());
                if !pages.seen.insert(next.to_owned()) {
                    let result=pages.partial("SKILL.REPORT_PAGING_LIMIT_OR_NO_PROGRESS"); self.report_pages=None; return Some(result);
                }
                if pages.pages.len()>=SKILL_SEGMENT_PAGES {let result=pages.segment();self.report_pages=None;return Some(result);}
                let mut call=pages.call.clone(); call.params["cursor"]=serde_json::json!(next);
                self.continuation=Some(call); self.next=Instant::now(); return None;
            }
            if data["reached_eof"]!=true {let result=pages.partial("SKILL.REPORT_PAGES_INCOMPLETE");self.report_pages=None;return Some(result);}
            let mut result=value.clone(); result["data"]["lines"]=serde_json::json!(pages.lines);
            result["data"]["page_evidence"]=serde_json::json!(pages.pages);
            if pages.call.params["cursor"].is_null() { result["data"]["start_line"]=serde_json::json!(1); }
            else { result["data"]["partial"]=serde_json::json!(true);
                result["data"]["coverage_note"]=serde_json::json!("Final segment only; combine prior segments before claiming full coverage."); }
            self.report_pages=None; return Some(result);
        }
        if let Some(pages) = &mut self.frame_pages {
            let data = &value["data"];
            let offset = current.params["offset"].as_u64().unwrap_or(0);
            let Some(frames) = data["frames"].as_array() else {
                let result = if pages.pages.is_empty() { value } else {
                    pages.partial("SKILL.PROTOCOL_PAGE_READ_FAILED")
                };
                self.frame_pages = None;
                return Some(result);
            };
            let source = &data["provenance"];
            let total = data["total_matches"].as_u64();
            let valid = value["ok"] == true && data["decoder_truncated"] == false && data["decoder_error"].is_null()
                && offset == pages.next_offset && !source.is_null() && total.is_some()
                && pages.source.as_ref().is_none_or(|old| old == source)
                && pages.protocol.as_ref().is_none_or(|old| old == &data["protocol"])
                && pages.statistics.as_ref().is_none_or(|old| old == &data["protocol_statistics"])
                && pages.total.is_none_or(|old| Some(old) == total);
            if !valid { let result=pages.partial("SKILL.PROTOCOL_PAGE_CHANGED_OR_INVALID"); self.frame_pages = None; return Some(result); }
            pages.source = Some(source.clone()); pages.protocol = Some(data["protocol"].clone()); pages.total = total;
            pages.statistics = Some(data["protocol_statistics"].clone());
            let existing = serde_json::to_vec(&pages.frames).map_or(usize::MAX, |bytes| bytes.len());
            let incoming = serde_json::to_vec(frames).map_or(usize::MAX, |bytes| bytes.len());
            if existing.saturating_add(incoming) > SKILL_SEGMENT_BYTES {
                let result=if pages.frames.is_empty() { pages.partial("SKILL.PROTOCOL_SINGLE_PAGE_TOO_LARGE") }
                    else { pages.segment(offset) };
                self.frame_pages = None; return Some(result);
            }
            pages.frames.extend(frames.iter().cloned());
            pages.pages.push(serde_json::json!({"offset":offset,"count":frames.len()}));
            let end = offset.saturating_add(frames.len() as u64);
            if let Some(next) = data["next_offset"].as_u64() {
                if next != end || next <= offset {
                    pages.next_offset=end;
                    let result=pages.partial("SKILL.PROTOCOL_PAGING_LIMIT_OR_NO_PROGRESS"); self.frame_pages = None; return Some(result);
                }
                if pages.pages.len() >= SKILL_SEGMENT_PAGES {
                    let result=pages.segment(next);self.frame_pages=None;return Some(result);
                }
                pages.next_offset = next;
                let mut call = pages.call.clone(); call.params["offset"] = serde_json::json!(next);
                self.continuation = Some(call); self.next = Instant::now(); return None;
            }
            if Some(end) != total { pages.next_offset=end; let result=pages.partial("SKILL.PROTOCOL_PAGES_INCOMPLETE"); self.frame_pages = None; return Some(result); }
            let mut result = value.clone();
            result["data"]["frames"] = serde_json::json!(pages.frames);
            result["data"]["page_evidence"] = serde_json::json!(pages.pages);
            if pages.call.params["offset"].as_u64().unwrap_or(0)>0 {
                result["data"]["partial"] = serde_json::json!(true);
                result["data"]["coverage_note"] = serde_json::json!("Final segment only; combine prior segments before claiming full coverage.");
            }
            self.frame_pages = None;
            return Some(result);
        }
        let delay;
        if let Some(wait) = value.get("_agent_wait") {
            let method = wait["method"].as_str().unwrap_or("");
            if !matches!(
                method,
                "serial.ready"
                    | "testing.result"
                    | "wave.decode.result"
                    | "data.statistics.result"
                    | "instrument.result"
            ) || !wait["params"].is_object()
            {
                return Some(failure("AI.INVALID_HOST_CONTINUATION"));
            }
            self.continuation = Some(Call {
                method: method.into(),
                params: wait["params"].clone(),
            });
            delay = wait["interval_ms"].as_u64().unwrap_or(250).clamp(16, 1000);
        } else if value["state"] == "pending" || value["data"]["state"] == "pending_or_unknown" {
            if !internal {
                return Some(failure("AI.MISSING_STATUS_CONTINUATION: outcome unknown"));
            }
            self.continuation = Some(current.clone());
            delay = 250;
        } else {
            return Some(value);
        }
        self.next = Instant::now() + Duration::from_millis(delay);
        None
    }

    pub fn take_due(&mut self, now: Instant) -> Option<Call> {
        if now < self.next || self.receiving() {
            return None;
        }
        self.continuation.take()
    }
}

/// The report reader exposes a bounded structured adapter matrix only when
/// callers have not requested pagination or a table scan. Do not silently add
/// paging parameters to this request: that changes its result contract.
pub(crate) fn structured_adapter_request(call: &Call) -> bool {
    call.method == "report.read" && call.params["attachment"] == "adapter-matrix.json"
        && ["scan", "cursor", "start_line", "start_byte", "limit"].iter().all(|key| call.params.get(*key).is_none())
}

/// Repairs only rejected model turns. Tool receipts reset the consecutive error budget.
#[derive(Default)]
pub(crate) struct ProtocolRecovery {
    consecutive: u8,
    pub json_fallback: bool,
}

impl ProtocolRecovery {
    pub fn made_progress(&mut self) {
        self.consecutive = 0;
    }
    pub fn retry(&mut self, native: bool, reason: &str) -> bool {
        if self.consecutive >= 3 {
            return false;
        }
        self.consecutive += 1;
        if native && self.consecutive >= 2 && !reason.starts_with("AGENT.UNFINISHED_OUTCOMES") && !reason.starts_with("AGENT.PLOT_") {
            self.json_fallback = true;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adapter_matrix_structured_request_preserves_contract_and_explicit_paging() {
        let call=Call{method:"report.read".into(),params:json!({"attachment":"adapter-matrix.json"})};
        assert!(structured_adapter_request(&call));
        let mut item=HostItem::default();item.collect_report(&call);
        let receipt=json!({"ok":true,"data":{"adapter_rows":[{"case_id":"A"}],"reached_eof":true}});
        assert_eq!(item.advance(receipt.clone(),&call,false),Some(receipt));
        for params in [json!({"attachment":"adapter-matrix.json","limit":8}),json!({"attachment":"adapter-matrix.json","cursor":"next"}),json!({"attachment":"adapter-matrix.json","scan":{}})] {
            assert!(!structured_adapter_request(&Call{method:"report.read".into(),params}));
        }
    }

    #[test]
    fn skill_frame_paging_preserves_pages_and_rejects_source_change() {
        let call=Call{method:"wave.frames".into(),params:json!({"offset":0,"limit":1})};
        let mut item=HostItem::default();item.collect_frames(&call);
        let first=json!({"ok":true,"data":{"frames":[{"index":0}],"provenance":{"generation":3},"protocol":"Qi","total_matches":2,"decoder_truncated":false,"decoder_error":null,"next_offset":1}});
        assert!(item.advance(first,&call,false).is_none());
        let next=item.take_due(Instant::now()).unwrap();
        let changed=json!({"ok":true,"data":{"frames":[{"index":1}],"provenance":{"generation":4},"protocol":"Qi","total_matches":2,"decoder_truncated":false,"decoder_error":null,"next_offset":null}});
        let result=item.advance(changed,&next,true).unwrap();
        assert_eq!(result["ok"],false);
        assert_eq!(result["data"]["frames"][0]["index"],0);
        assert_eq!(result["data"]["partial"],true);
    }

    #[test]
    fn skill_frame_paging_never_reports_complete_at_page_limit() {
        let mut call=Call{method:"wave.frames".into(),params:json!({"offset":0,"limit":1})};
        let mut item=HostItem::default();item.collect_frames(&call);
        for offset in 0..SKILL_SEGMENT_PAGES {
            let page=json!({"ok":true,"data":{"frames":[{"index":offset}],"provenance":{"generation":1},"protocol":"Qi","total_matches":SKILL_SEGMENT_PAGES+1,"decoder_truncated":false,"decoder_error":null,"next_offset":offset+1}});
            let result=item.advance(page,&call,offset>0);
            if offset==SKILL_SEGMENT_PAGES-1 {
                let result=result.unwrap();assert_eq!(result["ok"],true);
                assert_eq!(result["data"]["partial"],true);
                assert_eq!(result["data"]["frames"].as_array().unwrap().len(),SKILL_SEGMENT_PAGES);
                assert_eq!(result["data"]["next_offset"],SKILL_SEGMENT_PAGES);
            } else {assert!(result.is_none());call=item.take_due(Instant::now()).unwrap();}
        }
    }

    #[test]
    fn skill_report_paging_preserves_lines_and_rejects_repeated_cursor() {
        let call=Call{method:"report.read".into(),params:json!({})};
        let mut item=HostItem::default();item.collect_report(&call);
        let first=json!({"ok":true,"data":{"source_id":"report-a","source_bytes":20,"start_line":1,
            "lines":[{"line":1,"text":"first"}],"next_cursor":"cursor-a","reached_eof":false}});
        assert!(item.advance(first,&call,false).is_none());
        let next=item.take_due(Instant::now()).unwrap();
        let second=json!({"ok":true,"data":{"source_id":"report-a","source_bytes":20,"start_line":2,
            "lines":[{"line":2,"text":"second"}],"next_cursor":"cursor-a","reached_eof":false}});
        let result=item.advance(second,&next,true).unwrap();
        assert_eq!(result["ok"],false);
        assert_eq!(result["data"]["lines"].as_array().unwrap().len(),2);
        assert_eq!(result["data"]["reached_eof"],false);
    }

    #[test]
    fn immediate_and_worker_replies_have_the_same_lifecycle() {
        for asynchronous in [false, true] {
            let mut item = HostItem::default();
            let write = Call {
                method: "instrument.command".into(),
                params: json!({}),
            };
            let ack = json!({"_agent_wait":{"method":"instrument.result","params":{"job_id":7},"interval_ms":16}});
            let reply = if asynchronous {
                let (tx, rx) = crossbeam_channel::bounded(1);
                tx.send(ack.clone()).unwrap();
                HostReply::work(rx)
            } else {
                HostReply::value(ack)
            };
            let value = item.attach(reply).or_else(|| item.receive()).unwrap();
            assert!(item.advance(value, &write, false).is_none());
            let query = item
                .take_due(Instant::now() + Duration::from_secs(1))
                .unwrap();
            assert_eq!(query.method, "instrument.result");
            assert!(item
                .advance(json!({"state":"pending"}), &query, true)
                .is_none());
            assert_eq!(
                item.take_due(Instant::now() + Duration::from_secs(1)),
                Some(query.clone())
            );
            let completed = json!({"ok":true,"data":{"physical_verified":false}});
            assert_eq!(
                item.advance(completed.clone(), &query, true),
                Some(completed)
            );
            assert_eq!(
                item.advance(json!({"state":"pending"}), &write, false)
                    .unwrap()["ok"],
                false
            );
        }
    }
}
