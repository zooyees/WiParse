use std::io::{self, Write};
use std::sync::Arc;

type Row=(String,f64,u64,Vec<Option<f64>>);

/// Keep original rows at bucket boundaries, extrema and the first gap per field.
/// The complete table is streamed separately for precise offline investigation.
pub(super) struct Bucket {
    width:usize,
    count:usize,
    first:Option<Row>,
    last:Option<Row>,
    min:Vec<Option<Row>>,
    max:Vec<Option<Row>>,
    gap:Vec<Option<Row>>,
}

impl Bucket {
    pub fn new(width:usize,fields:usize)->Self {
        Self{width,count:0,first:None,last:None,min:vec![None;fields],max:vec![None;fields],gap:vec![None;fields]}
    }
    pub fn push(&mut self,row:Row,out:&mut Vec<Row>) {
        if self.width==1{out.push(row);return;}
        self.count+=1;
        if self.first.is_none(){self.first=Some(row.clone());}
        for (i,value) in row.3.iter().enumerate(){
            if let Some(y)=value{
                if self.min[i].as_ref().is_none_or(|r|r.3[i].is_none_or(|v|*y<v)){self.min[i]=Some(row.clone());}
                if self.max[i].as_ref().is_none_or(|r|r.3[i].is_none_or(|v|*y>v)){self.max[i]=Some(row.clone());}
            }else if self.gap[i].is_none(){self.gap[i]=Some(row.clone());}
        }
        self.last=Some(row);
        if self.count==self.width{self.flush(out);}
    }
    pub fn flush(&mut self,out:&mut Vec<Row>){
        let mut keep=Vec::new();
        keep.extend(self.first.take());
        for row in self.min.iter_mut().chain(&mut self.max).chain(&mut self.gap){keep.extend(row.take());}
        keep.extend(self.last.take());
        keep.sort_by_key(|r|r.2);
        keep.dedup_by_key(|r|r.2);
        out.extend(keep);
        self.count=0;
    }
}

pub(super) struct TableWriter {
    file:Arc<tempfile::NamedTempFile>,
    output:io::BufWriter<std::fs::File>,
}
impl TableWriter {
    pub fn new(coordinate:&str,axis:&str,fields:&[wiparse_core::log_analysis::timeline::Field])->io::Result<Self>{
        let file=Arc::new(tempfile::NamedTempFile::new()?);
        let mut output=io::BufWriter::new(file.reopen()?);
        write!(output,"{coordinate},{axis}")?;
        for f in fields {write!(output,",\"{}\"",f.name.replace('"',"\"\""))?;}
        writeln!(output)?;
        Ok(Self{file,output})
    }
    pub fn append(&mut self,x:f64,position:u64,values:&[Option<f64>])->io::Result<()> {
        write!(self.output,"{position},{x}")?;
        for value in values{
            write!(self.output,",")?;
            if let Some(value)=value{write!(self.output,"{value}")?;}
        }
        writeln!(self.output)
    }
    pub fn finish(mut self)->io::Result<Arc<tempfile::NamedTempFile>>{
        self.output.flush()?;
        Ok(self.file)
    }
}
