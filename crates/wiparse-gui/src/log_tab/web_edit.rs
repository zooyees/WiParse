//! Paged adapter for the existing line editor; device ownership is unchanged.
use super::*;
use serde_json::{json,Value};
pub(super) struct WebEditSession {token:String,revision:u64,source_hash:String,path:PathBuf,total_bytes:usize}
const MAX_EDIT_BYTES:u64=64*1024*1024;

fn source_bytes(path:&Path)->Result<Vec<u8>,String>{
    use std::io::Read;
    let file=std::fs::File::open(path).map_err(|_|"LOG.READ_FAILED")?;
    if file.metadata().map_err(|_|"LOG.READ_FAILED")?.len()>MAX_EDIT_BYTES{return Err("LOG.EDIT_TOO_LARGE: editor supports files up to 64 MiB".into());}
    let mut bytes=Vec::new();file.take(MAX_EDIT_BYTES+1).read_to_end(&mut bytes).map_err(|_|"LOG.READ_FAILED")?;
    if bytes.len() as u64>MAX_EDIT_BYTES{return Err("LOG.EDIT_TOO_LARGE".into());}Ok(bytes)
}
impl LogTabPage {
    pub(crate) fn has_web_editor(&self)->bool{self.web_edit.is_some()}
    fn web_edit_page(&self,from:usize)->Result<Value,String>{
        let session=self.web_edit.as_ref().ok_or("LOG.EDIT_CHANGED")?;
        let start=from.min(self.edit.lines.len().saturating_sub(1));let mut bytes=0usize;let mut rows=Vec::new();
        for line in self.edit.lines.iter().skip(start).take(80){
            if bytes+line.len()>1024*1024 {if rows.is_empty(){return Err("LOG.EDIT_LINE_TOO_LARGE: line exceeds the 1 MiB editor-page budget".into());}break;}
            bytes+=line.len();rows.push(line.clone());
        }
        Ok(json!({"token":session.token,"revision":session.revision,"source_sha256":session.source_hash,"filename":self.title,"from_row":start,"next_row":start+rows.len(),"total_rows":self.edit.lines.len(),"rows":rows,"dirty":self.edit_dirty}))
    }
    pub(crate) fn web_edit_action(&mut self,action:&str,params:&Value)->Result<Value,String>{
        if action=="begin"{
            if self.live||self.loading||self.edit_mode||self.has_background_filter(){return Err("LOG.EDIT_IN_PROGRESS_OR_UNAVAILABLE".into());}
            let path=PathBuf::from(self.filepath.as_ref().ok_or("LOG.NOT_A_FILE")?).canonicalize().map_err(|_|"LOG.READ_FAILED")?;
            let bytes=source_bytes(&path)?;let hash=wiparse_core::update::hex_sha256(&bytes);
            let text=String::from_utf8(bytes).map_err(|_|"LOG.INVALID_UTF8")?;
            let rows=text.as_bytes().iter().filter(|byte|**byte==b'\n').count().saturating_add(1);
            if text.len().saturating_add(rows.saturating_mul(std::mem::size_of::<String>()))>256*1024*1024{return Err("LOG.EDIT_TOO_LARGE: estimated line-editor storage exceeds the 256 MiB budget; file remains available for viewing".into());}
            if text.split('\n').any(|line|line.len()>1024*1024){return Err("LOG.EDIT_LINE_TOO_LARGE: file contains a line above the 1 MiB editor-page budget".into());}
            self.release_mapped_file();for pane in &mut self.panes {pane.filter_rx=None;pane.filter_busy=false;pane.clear_sel();}
            self.edit=LineEditorSession::from_text(&text);self.edit_dirty=false;self.edit_mode=true;
            self.edit_status.clear();self.edit_match_status.clear();self.edit_cursor_status.clear();
            self.web_edit=Some(WebEditSession{token:uuid::Uuid::new_v4().to_string(),revision:1,source_hash:hash,path,total_bytes:self.edit.lines.iter().map(String::len).sum::<usize>()+self.edit.lines.len().saturating_sub(1)*self.edit.newline.len()});
            return self.web_edit_page(0);
        }
        let session=self.web_edit.as_ref().filter(|session|params["token"].as_str()==Some(session.token.as_str())).ok_or("LOG.EDIT_CHANGED")?;
        if !self.edit_mode{return Err("LOG.EDIT_CHANGED".into());}
        let from=params["from_row"].as_u64().and_then(|value|usize::try_from(value).ok()).unwrap_or(0);
        if action=="read"{return self.web_edit_page(from);}
        if action=="discard"{self.web_edit=None;self.exit_edit_mode();return Ok(json!({"closed":true,"saved":false}));}
        if params["revision"].as_u64()!=Some(session.revision){return Err("LOG.EDIT_REVISION_CHANGED".into());}
        match action {
            "patch"=>{
                let count=params["delete_count"].as_u64().and_then(|value|usize::try_from(value).ok()).ok_or("LOG.INVALID_EDIT")?;
                let input=params["lines"].as_array().filter(|lines|lines.len()<=5000).ok_or("LOG.INVALID_EDIT")?;
                let mut bytes=0usize;let mut replacement=Vec::new();
                for value in input {let line=value.as_str().filter(|line|!line.contains(['\n','\r','\0'])).ok_or("LOG.INVALID_EDIT")?;bytes+=line.len();if bytes>1024*1024{return Err("LOG.EDIT_PAGE_TOO_LARGE".into());}replacement.push(line.to_string());}
                if from>self.edit.lines.len()||count>self.edit.lines.len()-from{return Err("LOG.INVALID_EDIT_RANGE".into());}
                let old_bytes=self.edit.lines[from..from+count].iter().map(String::len).sum::<usize>();
                let next_count=(self.edit.lines.len()-count+replacement.len()).max(1);
                let total=session.total_bytes.saturating_sub(old_bytes+self.edit.lines.len().saturating_sub(1)*self.edit.newline.len()).saturating_add(bytes+next_count.saturating_sub(1)*self.edit.newline.len());
                if total>MAX_EDIT_BYTES as usize{return Err("LOG.EDIT_TOO_LARGE".into());}
                let max_chars=replacement.iter().map(|line|line.chars().count()).max().unwrap_or(1);
                self.edit.lines.splice(from..from+count,replacement);if self.edit.lines.is_empty(){self.edit.lines.push(String::new());}
                self.edit_dirty=true;let session=self.web_edit.as_mut().unwrap();session.revision+=1;session.total_bytes=total;
                self.edit.max_chars=self.edit.max_chars.max(max_chars);
                self.web_edit_page(from)
            },
            "find"|"find_previous"|"replace"|"replace_all"=>{
                let query=params["query"].as_str().filter(|query|!query.is_empty()&&query.len()<=4096&&!query.contains(['\r','\n','\0'])).ok_or("LOG.INVALID_QUERY")?;
                let sensitive=params["case_sensitive"].as_bool().unwrap_or(false);
                if action=="replace_all"{
                    let replacement=params["replacement"].as_str().filter(|value|value.len()<=4096&&!value.contains(['\r','\n','\0'])).ok_or("LOG.INVALID_REPLACEMENT")?;
                    // Evaluate into a separate document first; reject oversized
                    // expansions before changing the original editor buffer.
                    let (lines,count,total)=bounded_replace(&self.edit.lines,query,replacement,sensitive,self.edit.newline.len())?;
                    self.edit.max_chars=lines.iter().map(|line|line.chars().count()).max().unwrap_or(1).max(1);self.edit.lines=lines;self.edit_dirty|=count>0;let session=self.web_edit.as_mut().unwrap();session.revision+=1;session.total_bytes=total;
                    let mut page=self.web_edit_page(from)?;page["replaced"]=json!(count);return Ok(page);
                }
                let row=params["after_row"].as_u64().and_then(|value|usize::try_from(value).ok()).unwrap_or(from);
                let col=params["after_col"].as_u64().and_then(|value|usize::try_from(value).ok()).unwrap_or(0);
                let hit=if action=="find_previous"{find_prev_in_lines(&self.edit.lines,query,row,col,sensitive)}else{find_next_in_lines(&self.edit.lines,query,row,col,sensitive)};
                if action=="replace"{
                    let replacement=params["replacement"].as_str().filter(|value|value.len()<=4096&&!value.contains(['\r','\n','\0'])).ok_or("LOG.INVALID_REPLACEMENT")?;
                    if let Some((row,start,end,_,_))=hit{
                        let old=self.edit.lines[row].len();let mut line=self.edit.lines[row].clone();replace_char_range(&mut line,start,end,replacement);
                        let total=session.total_bytes-old+line.len();if total>MAX_EDIT_BYTES as usize||line.len()>1024*1024{return Err("LOG.EDIT_TOO_LARGE".into());}
                        self.edit.max_chars=self.edit.max_chars.max(line.chars().count());self.edit.lines[row]=line;self.edit_dirty=true;let session=self.web_edit.as_mut().unwrap();session.revision+=1;session.total_bytes=total;
                        let mut page=self.web_edit_page(row.saturating_sub(3))?;page["replaced"]=json!(1);return Ok(page);
                    }
                    let mut page=self.web_edit_page(from)?;page["replaced"]=json!(0);return Ok(page);
                }
                let mut page=self.web_edit_page(hit.map(|hit|hit.0.saturating_sub(3)).unwrap_or(from))?;
                page["match"]=hit.map(|(row,start,end,ordinal,total)|json!({"row":row,"start":start,"end":end,"ordinal":ordinal,"total":total})).unwrap_or(Value::Null);Ok(page)
            },
            "save"|"save_as"=>{
                let source=session.path.clone();
                if session.total_bytes>MAX_EDIT_BYTES as usize{return Err("LOG.EDIT_TOO_LARGE".into());}
                if action=="save"{
                    if self.filepath.as_deref().and_then(|value|Path::new(value).canonicalize().ok()).as_ref()!=Some(&source){return Err("LOG.EDIT_CHANGED".into());}
                    if wiparse_core::update::hex_sha256(&source_bytes(&source)?)!=session.source_hash{return Err("LOG.SOURCE_CHANGED: disk file changed; use Save As or discard and reload".into());}
                    if !self.save_edit(){return Err(format!("LOG.SAVE_FAILED: {}",self.edit_status));}
                }else{
                    let path=params["path"].as_str().filter(|path|!path.is_empty()&&path.len()<=4096&&!path.chars().any(char::is_control)).ok_or("LOG.INVALID_SAVE_PATH")?;
                    let path=PathBuf::from(path);if !path.extension().and_then(|value|value.to_str()).is_some_and(|value|["txt","log","csv","json","jsonl"].iter().any(|extension|value.eq_ignore_ascii_case(extension))){return Err("LOG.INVALID_SAVE_PATH".into());}
                    if !self.save_edit_as(path){return Err(format!("LOG.SAVE_FAILED: {}",self.edit_status));}
                }
                self.web_edit=None;self.exit_edit_mode();Ok(json!({"closed":true,"saved":true}))
            },
            _=>Err("LOG.INVALID_EDIT_ACTION".into()),
        }
    }
}

fn bounded_replace(lines:&[String],query:&str,replacement:&str,sensitive:bool,newline:usize)->Result<(Vec<String>,usize,usize),String>{
    let mut total=lines.len().saturating_sub(1)*newline;let mut count=0;let mut result=Vec::with_capacity(lines.len());
    for line in lines {
        let ranges=find_match_ranges(line,query,sensitive);
        if ranges.is_empty(){total=total.saturating_add(line.len());if total>MAX_EDIT_BYTES as usize{return Err("LOG.EDIT_TOO_LARGE".into());}result.push(line.clone());continue;}
        let mut offsets=line.char_indices().map(|(index,_)|index).collect::<Vec<_>>();offsets.push(line.len());
        let removed=ranges.iter().map(|(start,end)|offsets[*end]-offsets[*start]).sum::<usize>();
        let size=line.len().saturating_sub(removed).saturating_add(ranges.len().saturating_mul(replacement.len()));
        total=total.saturating_add(size);if total>MAX_EDIT_BYTES as usize||size>1024*1024{return Err("LOG.EDIT_TOO_LARGE".into());}
        let mut next=String::with_capacity(size);let mut position=0;
        for (start,end) in &ranges{next.push_str(&line[position..offsets[*start]]);next.push_str(replacement);position=offsets[*end];}
        next.push_str(&line[position..]);count+=ranges.len();result.push(next);
    }
    Ok((result,count,total))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paged_edits_preserve_original_until_save_and_detect_external_changes(){
        let root=std::env::temp_dir().join(format!("wiparse-editor-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let source=root.join("capture.log");let saved=root.join("edited.log");let bytes=b"\xef\xbb\xbfhello\r\nASK sample\r\n";std::fs::write(&source,bytes).unwrap();
        let mut tab=LogTabPage::file_tab_empty(source.to_string_lossy().into_owned());tab.loading=false;
        let initial=tab.web_edit_action("begin",&json!({})).unwrap();
        let changed=tab.web_edit_action("patch",&json!({"token":initial["token"],"revision":initial["revision"],"from_row":1,"delete_count":1,"lines":["ASK changed"]})).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(),bytes);assert_eq!(changed["dirty"],true);
        assert!(tab.web_edit_action("patch",&json!({"token":initial["token"],"revision":initial["revision"],"from_row":0,"delete_count":1,"lines":["stale"]})).unwrap_err().contains("REVISION_CHANGED"));
        std::fs::write(&source,b"written elsewhere").unwrap();
        assert!(tab.web_edit_action("save",&json!({"token":changed["token"],"revision":changed["revision"]})).unwrap_err().contains("SOURCE_CHANGED"));
        tab.web_edit_action("save_as",&json!({"token":changed["token"],"revision":changed["revision"],"path":saved})).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(),b"written elsewhere");assert_eq!(std::fs::read(&saved).unwrap(),b"\xef\xbb\xbfhello\r\nASK changed\r\n");
        drop(tab);
        let canonical=root.canonicalize().unwrap();let temp_root=std::env::temp_dir().canonicalize().unwrap();assert_eq!(canonical.parent(),Some(temp_root.as_path()));
        let mut removed=false;
        for _ in 0..40 {
            for entry in std::fs::read_dir(&canonical).unwrap(){let entry=entry.unwrap();if entry.file_type().unwrap().is_file(){let _=std::fs::remove_file(entry.path());}}
            if std::fs::remove_dir(&canonical).is_ok(){removed=true;break;}std::thread::sleep(Duration::from_millis(10));
        }
        assert!(removed,"editor test temporary files still held by a worker");
    }
    #[test]
    fn replacement_bounds_are_checked_before_allocating_expanded_document(){
        let lines=vec!["a".repeat(20_000)];let replacement="b".repeat(4096);
        assert!(bounded_replace(&lines,"a",&replacement,true,1).unwrap_err().contains("TOO_LARGE"));assert_eq!(lines[0].len(),20_000);
        assert!(bounded_replace(&["a".repeat(1000)],"a",&replacement,true,1).unwrap_err().contains("TOO_LARGE"));
        let (result,count,_)=bounded_replace(&["启动 Ask ASK".into()],"ask","中文",false,1).unwrap();assert_eq!(count,2);assert_eq!(result,["启动 中文 中文"]);
    }
    #[test]
    fn atomic_log_writer_preserves_existing_readonly_file(){
        let root=std::env::temp_dir().join(format!("wiparse-editor-write-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();let path=root.join("existing.log");
        std::fs::write(&path,b"original").unwrap();let original=std::fs::metadata(&path).unwrap().permissions();let mut readonly=original.clone();readonly.set_readonly(true);std::fs::set_permissions(&path,readonly).unwrap();
        assert!(write_log_file(&path,b"replacement").is_err());assert_eq!(std::fs::read(&path).unwrap(),b"original");
        std::fs::set_permissions(&path,original).unwrap();write_log_file(&path,b"replacement").unwrap();assert_eq!(std::fs::read(&path).unwrap(),b"replacement");
        std::fs::remove_file(path).unwrap();std::fs::remove_dir(root).unwrap();
    }
}
