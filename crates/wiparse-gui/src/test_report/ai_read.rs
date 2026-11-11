use super::*;
use serde_json::{json,Value};
use std::{io::{BufRead,BufReader,Read},sync::{Arc,atomic::{AtomicBool,Ordering}}};

impl TestReportPanel {
    pub(crate) fn agent_selected_label(&self)->String {
        format!("{} · {}",self.project_id,self.selected.as_ref().and_then(|p|p.file_name()).map(|s|s.to_string_lossy().into_owned()).unwrap_or_else(||"未打开报告 / No report selected".into()))
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
            let path=if let Some(name)=p["attachment"].as_str(){
                let attachment=||->Result<PathBuf,String>{
                    if !is_markdown(&path) || name.is_empty() || name.len()>180 || name.starts_with('.') ||
                        !name.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'.'|b'_'|b'-')){
                        return Err("REPORT.INVALID_ATTACHMENT".into());
                    }
                    let base=path.parent().ok_or("REPORT.NO_DIRECTORY")?.canonicalize().map_err(|e|e.to_string())?;
                    let target=base.join(name).canonicalize().map_err(|e|e.to_string())?;
                    if target.parent()!=Some(base.as_path()) || !path_in_project_jail(&root,&project,&target){return Err("REPORT.ATTACHMENT_OUTSIDE_SCOPE".into());}
                    let mut file=std::fs::File::open(&path).map_err(|e|e.to_string())?;
                    let mut input=Vec::new();
                    std::io::Read::by_ref(&mut file).take(4*1024*1024+1).read_to_end(&mut input).map_err(|e|e.to_string())?;
                    if input.len()>4*1024*1024{return Err("REPORT.ATTACHMENT_INDEX_TOO_LARGE".into());}
                    if !links_attachment(&String::from_utf8_lossy(&input),name){return Err("REPORT.ATTACHMENT_NOT_LINKED".into());}
                    Ok(target)
                };
                match attachment(){Ok(path)=>path,Err(e)=>return crate::ai::agent::failure(e)}
            }else{path};
            if !path.extension().and_then(|e|e.to_str()).is_some_and(|e|matches!(e.to_lowercase().as_str(),"md"|"markdown"|"txt"|"log"|"csv"|"json"|"jsonl")) {return crate::ai::agent::failure("REPORT.TEXT_REQUIRED: image pixels are not available to this text model");}
            match read_page(&path,p["start_line"].as_u64().unwrap_or(1),p["limit"].as_u64().unwrap_or(80),p["start_byte"].as_u64().unwrap_or(0),&cancel){Ok(v)=>json!({"ok":true,"data":v}),Err(e)=>crate::ai::agent::failure(e)}
        })
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

fn read_page(path:&Path,start:u64,limit:u64,start_byte:u64,cancel:&AtomicBool)->Result<Value,String> {
    if start==0||start>1_000_000||limit==0||limit>200||start_byte>16*1024*1024{return Err("REPORT.INVALID_READ".into());}
    let file=fs::File::open(path).map_err(|e|e.to_string())?;
    let before=file.metadata().map_err(|e|e.to_string())?;
    let mut reader=BufReader::new(file.take(16*1024*1024+1));
    let mut raw=Vec::new();let mut rows=Vec::new();let mut line=0;let mut bytes=0;let mut payload=0;let mut eof=false;let mut next=None;let mut next_byte=0;
    loop {
        if cancel.load(Ordering::Acquire){return Err("AGENT.CANCELLED".into());}
        raw.clear();let n=reader.read_until(b'\n',&mut raw).map_err(|e|e.to_string())?;
        bytes+=n;
        if bytes>16*1024*1024{return Err("REPORT.READ_BUDGET: use report.search for large reports".into());}
        if n==0{eof=true;break;}
        line+=1;if line<start{continue;}
        if rows.len()>=limit as usize {next=Some(line);break;}
        let text=String::from_utf8_lossy(&raw);let text=text.trim_end_matches(['\r','\n']);
        let begin=if line==start{start_byte as usize}else{0};
        if begin>text.len()||!text.is_char_boundary(begin){return Err("REPORT.INVALID_BYTE_CURSOR: use next_line and next_byte together".into());}
        let mut end=text.len().min(begin+4096);while !text.is_char_boundary(end){end-=1;}
        if payload+end-begin>16*1024&&!rows.is_empty(){next=Some(line);break;}
        payload+=end-begin;
        rows.push(json!({"line":line,"text":&text[begin..end],"start_byte":begin,"next_byte":if end<text.len(){Some(end)}else{None},"truncated":false,"line_complete":end==text.len(),"encoding_lossy":matches!(String::from_utf8_lossy(&raw),std::borrow::Cow::Owned(_))}));
        if end<text.len(){next=Some(line);next_byte=end;break;}
    }
    let after=fs::metadata(path).map_err(|e|e.to_string())?;
    if before.len()!=after.len()||before.modified().ok()!=after.modified().ok(){return Err("REPORT.SOURCE_CHANGED: read again".into());}
    Ok(json!({"file":path.file_name().map(|s|s.to_string_lossy()),"source_id":wiparse_core::log_analysis::source_id_for_path(path).ok(),"start_line":start,"lines":rows,"next_line":next,"next_byte":next_byte,"reached_eof":eof,"source_bytes":before.len(),"scope":"report text page; linked images/waveforms have not been inspected"}))
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
    fn attachment_must_be_a_rendered_link() {
        assert!(links_attachment("## Data\n\n- [CSV](data.csv)", "data.csv"));
        assert!(!links_attachment("```md\n[CSV](data.csv)\n```", "data.csv"));
        assert!(!links_attachment("Example `[CSV](data.csv)`", "data.csv"));
    }
}
