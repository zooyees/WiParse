mod screenshot;
mod reviewed_preview;
use super::*;
use serde_json::{json,Value};
use std::{io::{BufRead,BufReader,Read},sync::{Arc,atomic::{AtomicBool,Ordering}}};

impl TestReportPanel {
    pub(crate) fn agent_selected_label(&self, lang: Lang)->String {
        format!("{} · {}",self.project_id,self.selected.as_ref().and_then(|p|p.file_name()).map(|s|s.to_string_lossy().into_owned()).unwrap_or_else(||if lang == Lang::Zh {"未打开报告".into()} else {"No report selected".into()}))
    }
    pub(crate) fn agent_read(&self,p:&Value,root:&Path,ctx:&egui::Context,cancel:Arc<AtomicBool>)->crate::ai::agent::HostReply {
        let path=if let Some(id)=p["report_id"].as_str() {
            if uuid::Uuid::parse_str(id).is_ok(){
                match crate::ai::artifact::report_path(root,&self.project_id,id){Ok(path)=>Some(path),Err(e)=>return crate::ai::agent::HostReply::value(crate::ai::agent::failure(e))}
            }else{match self.agent_report_files().into_iter().find(|(key,_)|key==id) {Some((_,p))=>Some(p),None=>return crate::ai::agent::HostReply::value(crate::ai::agent::failure("REPORT.UNKNOWN_OR_EXPIRED_ID"))}}
        }else{self.selected.clone()};let root=root.to_owned();let project=self.project_id.clone();let p=p.clone();
        crate::ai::agent::spawn_work(ctx,move||{
            let Some(path)=path else{return crate::ai::agent::failure("REPORT.NO_SELECTED_REPORT");};
            if !path_in_project_jail(&root,&project,&path){return crate::ai::agent::failure("REPORT.PATH_OUTSIDE_PROJECT");}
            let provenance = report_run_provenance(&root, &project, &path);
            let path=if let Some(name)=p["attachment"].as_str(){
                let attachment=||->Result<PathBuf,String>{
                    if !is_markdown(&path) || name.is_empty() || name.len()>180 || name.starts_with('.') ||
                        !name.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'.'|b'_'|b'-')){
                        return Err("REPORT.INVALID_ATTACHMENT".into());
                    }
                    let base=path.parent().ok_or("REPORT.NO_DIRECTORY")?.canonicalize().map_err(|e|e.to_string())?;
                    let target=base.join(name).canonicalize().map_err(|e|e.to_string())?;
                    if target.parent()!=Some(base.as_path()) || !path_in_project_jail(&root,&project,&target){return Err("REPORT.ATTACHMENT_OUTSIDE_SCOPE".into());}
                    let file=std::fs::File::open(&path).map_err(|e|e.to_string())?;
                    if !linked_in_reader(BufReader::new(file),name,&cancel)?{return Err("REPORT.ATTACHMENT_NOT_LINKED".into());}
                    Ok(target)
                };
                match attachment(){Ok(path)=>path,Err(e)=>return crate::ai::agent::failure(e)}
            }else{path};
            if path.extension().and_then(|e|e.to_str()).is_some_and(|s|matches!(s.to_ascii_lowercase().as_str(),"png"|"jpg"|"jpeg")){
                if p.get("scan").is_some(){return crate::ai::agent::failure("REPORT.CSV_REQUIRED");}
                return match screenshot::inspect(&path,&cancel){Ok(data)=>json!({"ok":true,"data":data}),Err(e)=>crate::ai::agent::failure(e)};
            }
            if !path.extension().and_then(|e|e.to_str()).is_some_and(|e|matches!(e.to_lowercase().as_str(),"md"|"markdown"|"txt"|"log"|"csv"|"json"|"jsonl")) {return crate::ai::agent::failure("REPORT.TEXT_REQUIRED: image pixels are not available to this text model");}
            if let Some(rule)=p.get("scan"){return match paging::scan_table(&path,rule,&cancel){
                Ok(mut v)=>{
                    let source=match paging::current_source(&path){Ok(source)=>source,Err(e)=>return crate::ai::agent::failure(e.to_string())};
                    if source["source_version"]!=v["source_version"] {return crate::ai::agent::failure("REPORT.SOURCE_CHANGED: discard counts and restart");}
                    v["source_id"]=source["source_id"].clone();v["source_bytes"]=source["source_bytes"].clone();
                    v["run_provenance"]=provenance.clone();
                    json!({"ok":true,"data":v})
                },Err(e)=>crate::ai::agent::failure(e)
            };}
            if p["attachment"]=="adapter-matrix.json" && !["cursor","start_line","start_byte","limit"].iter().any(|k|p.get(k).is_some()){
                let before=match paging::current_source(&path){Ok(source)=>source,Err(e)=>return crate::ai::agent::failure(e.to_string())};
                if let Ok(file)=fs::File::open(&path){
                    let mut bytes=Vec::new();
                    if file.take(131073).read_to_end(&mut bytes).is_ok() && bytes.len()<=131072 {
                        if cancel.load(Ordering::Acquire) {return crate::ai::agent::failure("AGENT.CANCELLED");}
                        if let Ok(matrix)=serde_json::from_slice::<Value>(&bytes){
                            if let Some(rows)=matrix["rows"].as_array().filter(|a|a.len()<=500){
                                let sha256=wiparse_core::update::hex_sha256(&bytes);
                                let after=match paging::current_source(&path){Ok(source)=>source,Err(e)=>return crate::ai::agent::failure(e.to_string())};
                                if before!=after || after["source_bytes"].as_u64()!=Some(bytes.len() as u64){return crate::ai::agent::failure("REPORT.SOURCE_CHANGED_OR_INVALID_CURSOR: restart this report");}
                                return json!({"ok":true,"data":{"source_id":after["source_id"],"source_version":after["source_version"],"source_bytes":bytes.len(),"sha256":sha256,"adapter_rows":rows,"mode":matrix["mode"],"run_id":matrix["run_id"],"industrial_approved":matrix["industrial_approved"],"run_provenance":provenance,"reached_eof":true,"coverage":{"complete":true}}});
                            }
                        }
                    }
                }
            }
            match read_page_at(&path,p["cursor"].as_str(),p["start_line"].as_u64().unwrap_or(1),p["limit"].as_u64().unwrap_or(80),p["start_byte"].as_u64().unwrap_or(0),&cancel){
                Ok(mut v)=>{v["run_provenance"]=provenance;json!({"ok":true,"data":v})},
                Err(e) if e == "REPORT.SOURCE_CHANGED_OR_INVALID_CURSOR: restart this report" => {
                    match paging::current_source(&path) {
                        Ok(source) => json!({"ok":false,"error":e,"data":source}),
                        Err(_) => crate::ai::agent::failure(e),
                    }
                },
                Err(e)=>crate::ai::agent::failure(e)
            }
        })
    }
}

/// Link only a report explicitly listed in its containing runner manifest.
/// Missing/older manifests remain unassociated; filenames or timestamps are
/// not evidence that a report belongs to a particular invocation.
pub(super) fn report_run_provenance(root:&Path, project:&str, report:&Path)->Value {
    let Ok(report_path)=report.canonicalize() else {return Value::Null;};
    for dir in report_path.ancestors().skip(1) {
        if !path_in_project_jail(root,project,dir) { break; }
        let manifest=dir.join("run.json");
        if !manifest.is_file() { continue; }
        if !path_in_project_jail(root,project,&manifest) { return Value::Null; }
        let Ok(meta)=fs::metadata(&manifest) else {return Value::Null;};
        if meta.len()>1024*1024 {return Value::Null;}
        let Ok(bytes)=fs::read(&manifest) else {return Value::Null;};
        let Ok(document)=serde_json::from_slice::<Value>(&bytes) else {return Value::Null;};
        if document["schema"]!="wiparse.run/v1" || document["project"]!=project {return Value::Null;}
        let listed=document["artifacts"]["items"].as_array().is_some_and(|items| items.iter().any(|item| {
            let relative = item["relative_path"].as_str().filter(|raw| {
                !Path::new(raw).is_absolute() && Path::new(raw).components().all(|part|
                    matches!(part, std::path::Component::Normal(_)))
            }).and_then(|raw|dir.join(raw).canonicalize().ok());
            relative.as_ref() == Some(&report_path) || item["path"].as_str()
                .and_then(|raw|Path::new(raw).canonicalize().ok()).as_ref()==Some(&report_path)
        }));
        if !listed {return Value::Null;}
        let Some(id)=document["host_run_id"].as_str().filter(|id|uuid::Uuid::parse_str(id).is_ok()) else {return Value::Null;};
        return json!({"host_run_id":id,"plugin_run_id":document["run_id"],
            "manifest_sha256":wiparse_core::update::hex_sha256(&bytes),
            "association":"runner artifact manifest; not independently authenticated"});
    }
    Value::Null
}

// Keep Markdown paragraph and inline-code boundaries. Discard fenced bodies as they stream in.
fn linked_in_reader(mut reader:impl BufRead,name:&str,cancel:&AtomicBool)->Result<bool,String>{
    let mut line=String::new();
    let mut block=String::new();
    let mut fence=None;
    loop {
        if cancel.load(Ordering::Relaxed){return Err("AGENT.CANCELLED".into());}
        line.clear();
        let eof=reader.read_line(&mut line).map_err(|e|e.to_string())?==0;
        if let Some((ch,n))=fence {
            if is_fence_close(line.trim(),ch,n){fence=None;}
        } else {
            let open=parse_fence_open(line.trim());
            if eof || line.trim().is_empty() || open.is_some() {
                if block.contains("](") && links_attachment(&block,name){return Ok(true);}
                block.clear();
                fence=open.map(|(ch,n,_)|(ch,n));
            }else{block.push_str(&line);}
        }
        if eof{return Ok(false);}
    }
}
fn links_attachment(markdown:&str,name:&str)->bool {
    let inline=|line:&str| {
        let chars=line.chars().collect::<Vec<_>>();
        let mut i=0;
        while i<chars.len() {
            if chars[i]=='`' {
                let n=chars[i..].iter().take_while(|&&c|c=='`').count();
                i+=n;
                while i<chars.len(){
                    if chars[i]=='`' {
                        let end=chars[i..].iter().take_while(|&&c|c=='`').count();
                        i+=end;
                        if end==n {break;}
                    } else {i+=1;}
                }
            } else if let Some((_,url,next))=parse_md_link(&chars,i){
                if decode_md_src(&url)==name{return true;} i=next;
            } else {i+=1;}
        }
        false
    };
    parse_md(markdown).blocks.iter().any(|block|match block {
        MdBlock::Heading{text,..}|MdBlock::Paragraph{text}|MdBlock::Bullet{text,..}|MdBlock::Ordered{text,..}
        |MdBlock::Task{text,..}|MdBlock::Quote{text}=>inline(text),
        MdBlock::Table{rows}=>rows.iter().flatten().any(|cell|inline(cell)),
        MdBlock::Image{src,..}=>decode_md_src(src)==name,
        MdBlock::Code{..}|MdBlock::Rule=>false,
    })
}

mod paging;
use paging::read_page_at;

#[cfg(test)]
fn read_page(path:&Path,start:u64,limit:u64,start_byte:u64,cancel:&AtomicBool)->Result<Value,String>{
    read_page_at(path,None,start,limit,start_byte,cancel)
}

#[cfg(test)]mod tests {
    use super::*;
    #[test]fn report_pages_preserve_original_lines_and_cancel(){
        let root=std::env::temp_dir().join(format!("report-ai-{}",uuid::Uuid::new_v4()));fs::create_dir_all(&root).unwrap();let path=root.join("report.md");
        fs::write(&path,(1..=205).map(|n|format!("第{n}行\n")).collect::<String>()).unwrap();
        let cancel=AtomicBool::new(false);
        let first=read_page(&path,1,80,0,&cancel).unwrap();assert_eq!(first["lines"][79]["line"],80);assert_eq!(first["next_line"],81);assert_eq!(first["reached_eof"],false);
        let mut start=1;let mut all=Vec::new();
        loop {let page=crate::ai::agent::bounded_evidence(json!({"ok":true,"data":read_page(&path,start,80,0,&cancel).unwrap()}),0);
            all.extend(page["data"]["lines"].as_array().unwrap().iter().map(|r|r["line"].as_u64().unwrap()));
            match page["data"]["next_line"].as_u64(){Some(n)=>start=n,None=>break}
        }
        assert_eq!(all,(1..=205).collect::<Vec<u64>>());
        let last=read_page(&path,201,80,0,&cancel).unwrap();assert_eq!(last["lines"].as_array().unwrap().len(),5);assert_eq!(last["lines"][0]["text"],"第201行");assert_eq!(last["reached_eof"],true);
        cancel.store(true,Ordering::Release);assert!(read_page(&path,1,80,0,&cancel).is_err());
        fs::remove_file(path).unwrap();fs::remove_dir(root).unwrap();
    }
    #[test]fn report_long_line_fragments_roundtrip_unicode() {
        let path=std::env::temp_dir().join(format!("report-fragment-{}.md",uuid::Uuid::new_v4()));
        let text="中文充电波形".repeat(3000);fs::write(&path,format!("{text}\n尾行\n")).unwrap();
        let cancel=AtomicBool::new(false);let(mut line,mut byte)=(1,0);let mut joined=String::new();let mut tail=false;
        loop {let page=crate::ai::agent::bounded_evidence(read_page(&path,line,80,byte,&cancel).unwrap(),0);
            for r in page["lines"].as_array().unwrap(){assert_eq!(r["truncated"],false);if r["line"]==1{joined.push_str(r["text"].as_str().unwrap())}else{tail=true;}}
            match page["next_line"].as_u64(){Some(n)=>{line=n;byte=page["next_byte"].as_u64().unwrap()},None=>break}
        }
        assert_eq!(joined,text);assert!(tail);fs::remove_file(path).unwrap();
    }
    #[test]fn read_rejects_outside_project_and_unknown_index_id(){
        let root=std::env::temp_dir().join(format!("report-ai-jail-{}",uuid::Uuid::new_v4()));fs::create_dir_all(&root).unwrap();
        let path=root.join("outside.md");fs::write(&path,"not in project").unwrap();
        let mut panel=TestReportPanel::new(&AppConfig::default());panel.selected=Some(path.clone());
        let ctx=egui::Context::default();let cancel=Arc::new(AtomicBool::new(false));
        let reply=panel.agent_read(&json!({}),&root,&ctx,cancel.clone()).receiver.unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(reply["ok"],false);assert_eq!(reply["error"],"REPORT.PATH_OUTSIDE_PROJECT");
        assert_eq!(panel.agent_read(&json!({"report_id":"report-0000000000000000"}),&root,&ctx,cancel).value["error"],"REPORT.UNKNOWN_OR_EXPIRED_ID");
        let identity=panel.agent_identity();fs::write(&path,"changed report length").unwrap();assert_ne!(panel.agent_identity(),identity);
        fs::remove_file(path).unwrap();fs::remove_dir(root).unwrap();
    }
}

#[cfg(test)]
mod attachment_tests {
    use super::*;
    #[test]
    fn linked_adapter_matrix_feeds_recipe_and_shares_paging_source_version() {
        let root=tempfile::tempdir().unwrap();
        let dir=wiparse_core::project::project_dir(root.path(),"default");
        fs::create_dir_all(&dir).unwrap();
        let report=dir.join("report.md");
        fs::write(&report,"# Results\n[Matrix](adapter-matrix.json)\n").unwrap();
        fs::write(dir.join("adapter-matrix.json"),json!({"run_id":"run-1","mode":"replay","rows":[{"case_id":"A","check_id":"1"},{"case_id":"A","check_id":"2"}]}).to_string()).unwrap();
        let mut panel=TestReportPanel::new(&AppConfig::default());panel.project_id="default".into();panel.selected=Some(report);
        let ctx=egui::Context::default();
        let read=|p:Value|panel.agent_read(&p,root.path(),&ctx,Arc::new(AtomicBool::new(false))).receiver.unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
        let structured=read(json!({"attachment":"adapter-matrix.json"}));
        let paged=read(json!({"attachment":"adapter-matrix.json","limit":1}));
        assert_eq!(structured["ok"],true);assert_eq!(paged["ok"],true);
        assert_eq!(structured["data"]["source_id"],paged["data"]["source_id"]);
        assert_eq!(structured["data"]["source_version"],paged["data"]["source_version"]);
        let mut workflow=wiparse_core::ai::workflow::Workflow::default();
        workflow.skill_ids=vec!["test_result".into()];
        workflow.record_call("A1","report.read",&json!({"attachment":"adapter-matrix.json"}),&structured);
        let result=workflow.run_recipe(&json!({"recipe":"recipe.adapter_compatibility","evidence_id":"A1"})).unwrap();
        assert_eq!(result["findings"]["cases"],1);assert_eq!(result["findings"]["check_rows"],2);
        assert_eq!(result["findings"]["source_mode"],"replay");
    }
    #[test]
    fn linked_attachment_requires_its_own_cursor_and_detects_replacement() {
        let root = tempfile::tempdir().unwrap();
        let dir = wiparse_core::project::project_dir(root.path(), "default");
        fs::create_dir_all(&dir).unwrap();
        let report = dir.join("report.md");
        let attachment = dir.join("data.csv");
        fs::write(&report, "# Demo\n[Data](data.csv)\nEnd\n").unwrap();
        fs::write(&attachment, "time,value\n1,5\n2,6\n").unwrap();
        let mut panel = TestReportPanel::new(&AppConfig::default());
        panel.project_id = "default".into();
        panel.selected = Some(report);
        let ctx = egui::Context::default();
        let read = |params: Value| panel.agent_read(&params, root.path(), &ctx, Arc::new(AtomicBool::new(false)))
            .receiver.unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
        let body = read(json!({"limit":1}));
        let data = read(json!({"attachment":"data.csv","limit":1}));
        let full_data = read(json!({"attachment":"data.csv","limit":80}));
        let scan = read(json!({"attachment":"data.csv","scan":{"column":"value","max":7}}));
        assert_eq!(scan["ok"],true);
        assert_eq!(scan["data"]["source_id"],full_data["data"]["source_id"]);
        assert_eq!(scan["data"]["source_version"],full_data["data"]["source_version"]);
        assert_eq!(scan["data"]["source_bytes"],full_data["data"]["source_bytes"]);
        assert_eq!(scan["data"]["run_provenance"],full_data["data"]["run_provenance"]);
        assert_eq!(body["ok"], true);
        assert_eq!(data["ok"], true);
        assert_ne!(body["data"]["source_id"], data["data"]["source_id"]);
        assert_eq!(body["data"]["scope"], "report text page; linked images/waveforms have not been inspected");
        let wrong = read(json!({"attachment":"data.csv","cursor":body["data"]["next_cursor"],"limit":1}));
        assert_eq!(wrong["ok"], false);
        assert!(wrong["error"].as_str().unwrap().contains("REPORT.SOURCE_CHANGED_OR_INVALID_CURSOR"));
        let mut workflow = wiparse_core::ai::workflow::Workflow::default();
        workflow.skill_ids = vec!["test_result".into()];
        workflow.initialize_skill_plan().unwrap();
        workflow.record_call("R1", "report.read", &json!({"limit":1}), &body);
        workflow.record_call("A0", "report.read", &json!({"attachment":"data.csv","limit":1}), &data);
        workflow.record_call("A1", "report.read", &json!({"attachment":"data.csv","limit":80}), &full_data);
        let plan = || json!({"steps":[{"id":"skill_test_result","title":"分析附件","state":"done","deliverable":"analysis","target":{"attachment":"data.csv"},"evidence_ids":["A1"]}]});
        assert!(workflow.update(plan()).is_ok());
        workflow.record_call("bad", "report.read", &json!({"attachment":"data.csv","cursor":body["data"]["next_cursor"],"limit":1}), &wrong);
        assert!(workflow.citation_method("A1").is_some(), "foreign cursor must not invalidate an unchanged attachment");
        fs::write(&attachment, "time,value\n1,7\n2,8\n3,9\n").unwrap();
        let arbitrary = read(json!({"attachment":"data.csv","cursor":"0.1.0.0000000000000000","limit":1}));
        assert_eq!(arbitrary["ok"], false);
        workflow.record_call("arbitrary", "report.read", &json!({"attachment":"data.csv","cursor":"0.1.0.0000000000000000","limit":1}), &arbitrary);
        assert!(workflow.citation_method("A1").is_some(), "unissued cursor must not invalidate evidence");
        let changed = read(json!({"attachment":"data.csv","cursor":data["data"]["next_cursor"],"limit":1}));
        assert_eq!(changed["ok"], false);
        assert!(changed["error"].as_str().unwrap().contains("REPORT.SOURCE_CHANGED_OR_INVALID_CURSOR"));
        workflow.record_call("changed", "report.read", &json!({"attachment":"data.csv","cursor":data["data"]["next_cursor"],"limit":1}), &changed);
        assert!(workflow.citation_method("A1").is_none(), "known cursor and verified new version must invalidate old evidence immediately");
        assert!(workflow.citation_method("R1").is_some(), "attachment replacement must not invalidate report body");
        let replacement = read(json!({"attachment":"data.csv","limit":1}));
        assert_eq!(replacement["ok"], true);
        workflow.record_call("A2", "report.read", &json!({"attachment":"data.csv","limit":1}), &replacement);
        assert!(workflow.report_input("A1").is_err());
        assert!(workflow.update(plan()).is_err());
        assert!(workflow.citation_method("A1").is_none(), "superseded attachment cannot be cited as current evidence");
    }
    #[test]
    fn attachment_must_be_a_rendered_link() {
        assert!(links_attachment("## Data\n\n- [CSV](data.csv)", "data.csv"));
        assert!(!links_attachment("```md\n[CSV](data.csv)\n```", "data.csv"));
        assert!(!links_attachment("Example `[CSV](data.csv)`", "data.csv"));
    }
}

#[cfg(test)]
mod large_parent {
    use super::*;
    #[test] fn late_link_is_resolved_while_code_is_excluded(){
        let mut md="\x60\x60\x60log\n".to_owned();
        for _ in 0..4100{md.push_str(&"x".repeat(1024));md.push('\n');}
        md.push_str("[fake](secret.csv)\n\x60\x60\x60\n\n[values](data.csv)\n");
        assert!(!linked_in_reader(md.as_bytes(),"secret.csv",&AtomicBool::new(false)).unwrap());
        assert!(linked_in_reader(md.as_bytes(),"data.csv",&AtomicBool::new(false)).unwrap());
    }
}
