#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod commit_file;
mod serial_events;
mod protocol_overlay;
mod wave_view;
mod wave_extrema;
mod probe_firmware;
mod offline_browser;
mod runtime_update;
mod resource_policy;
mod log_browser;
mod hub_files;
mod report_view;
mod data_view;

use serde_json::{json, Value};
use std::time::Duration;

// Temporary compatibility bridge. The existing application still owns device
// sessions. Read access is allowlisted; mutations use separately typed commands.
const ENDPOINT: &str = "http://127.0.0.1:7878/v1/invoke";
const ALLOWED: &[&str] = &["ui.market.get_safe","ui.instrument.io_log.get_safe","ui.test_tool.execution_log_safe","ui.data.preferences.get_safe","ui.navigation.get_safe","ui.preferences.get_safe","ui.state", "ui.calc.get", "instrument.list", "instrument.job.get", "serial.ports", "serial.monitor.status", "log.tabs.list", "log.lines.get", "ui.ai.settings.get_safe", "ui.ai.chat.state_safe", "ui.ai.serial.state_safe", "ui.report.tree_safe", "ui.test_tool.state_safe", "ui.test_tool.config.list", "ui.test_tool.config.get", "ui.test_tool.project.get", "ui.test_tool.matrix.get", "ui.test_tool.result.get", "ui.test_tool.preflight.state"];

fn validate_request(method: &str, params: &Value) -> Result<(), String> {
    if !ALLOWED.contains(&method) { return Err("仅允许读取已批准的状态接口".into()); }
    if !params.is_object() { return Err("请求参数无效".into()); }
    Ok(())
}

#[tauri::command]
fn application_info()->Value{json!({"version":wiparse_core::VERSION,"shell_version":env!("CARGO_PKG_VERSION"),"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH})}

#[tauri::command]
async fn scope_rules_replace_safe(project:String,expected:String,replacement:String,validate_only:bool)->Result<Value,String>{
    if !valid_identifier(&project)||expected.len()>1_000_000||replacement.len()>1_000_000||serde_json::from_str::<Value>(&replacement).is_err(){return Err("规则配置必须为有效 JSON，且不超过 1 MiB".into());}
    let params=json!({"project":project,"expected":expected,"replacement":replacement,"validate_only":validate_only});
    if params.to_string().len()>3*1024*1024{return Err("规则内容超过单次传输上限，请使用设置导入".into());}
    invoke_legacy("ui.test_tool.scope_rules_replace_safe".into(),params).await
}

#[tauri::command]
async fn navigation_ack_safe(owner:u32,revision:u64,state:String)->Result<Value,String>{
    if owner==0||revision==0||!matches!(state.as_str(),"applied"|"blocked"){return Err("页面请求身份无效".into());}
    invoke_legacy("ui.navigation.ack_safe".into(),json!({"owner":owner,"revision":revision,"state":state})).await
}

#[tauri::command]
async fn confirm_unsaved_exit(language:Option<String>)->Result<bool,String>{
    let english=language.as_deref()==Some("en");
    tauri::async_runtime::spawn_blocking(move||rfd::MessageDialog::new().set_title(if english{"Close WiParse?"}else{"关闭 WiParse？"}).set_description(if english{"There are unsaved edits or unconfirmed operations. Closing discards this window's unsaved content. Already submitted capture and device operations are not automatically cancelled. Close anyway?"}else{"当前有未保存的修改或尚未确认的操作。关闭将放弃本窗口的未保存内容；已经提交的采集和设备任务不会因此自动撤销。是否仍然关闭？"}).set_level(rfd::MessageLevel::Warning).set_buttons(rfd::MessageButtons::YesNo).show()==rfd::MessageDialogResult::Yes).await.map_err(|_|"无法显示关闭确认".into())
}

#[tauri::command]
async fn legacy_read(method: String, params: Value) -> Result<Value, String> {
    validate_request(&method, &params)?;
    invoke_legacy(method, params).await
}

#[tauri::command]
async fn ai_settings_set_safe(revision:String,fields:Value,new_key:String,clear_key:bool)->Result<Value,String>{
    if revision.len()!=16||!revision.bytes().all(|b|b.is_ascii_hexdigit())||!fields.is_object()||new_key.len()>16384||new_key.contains(['\r','\n'])||clear_key&&!new_key.is_empty(){return Err("AI 设置无效".into());}
    // Protect in the Tauri process so plaintext never crosses the compatibility HTTP bridge.
    let key=new_key.trim();
    let protected=if key.is_empty(){None}else{Some(wiparse_core::ai::protect_key(key).map_err(|e|e.to_string())?)};
    invoke_legacy("ui.ai.settings.set_safe".into(),json!({"revision":revision,"fields":fields,"protected_key":protected,"clear_key":clear_key})).await
}
#[tauri::command]
async fn ai_test_safe(revision:String)->Result<Value,String>{
    if revision.len()!=16||!revision.bytes().all(|b|b.is_ascii_hexdigit()){return Err("AI 设置版本无效".into());}
    invoke_legacy("ui.ai.test_safe".into(),json!({"revision":revision})).await
}
#[tauri::command]
async fn ai_chat_submit_safe(revision:String,text:String,consent:bool)->Result<Value,String>{
    if revision.len()!=16||!revision.bytes().all(|b|b.is_ascii_hexdigit())||!consent||text.trim().is_empty()||text.len()>65536||text.contains('\0'){return Err("AI 请求无效".into());}
    invoke_legacy("ui.ai.chat.submit_safe".into(),json!({"revision":revision,"text":text,"consent":true})).await
}

#[tauri::command]
async fn data_preferences_save(revision:String,fields:Value)->Result<Value,String>{
    if revision.len()!=16||!fields.is_object()||fields.to_string().len()>16384{return Err("数据规则参数无效".into());}
    invoke_legacy("ui.data.preferences.set_safe".into(),json!({"revision":revision,"fields":fields})).await
}

#[tauri::command]
async fn preferences_save(revision:String,fields:Value)->Result<Value,String>{
    if revision.len()!=16||!revision.bytes().all(|value|value.is_ascii_hexdigit())||!fields.is_object()||fields.to_string().len()>4096{return Err("默认值参数无效".into());}
    invoke_legacy("ui.preferences.set_safe".into(),json!({"revision":revision,"fields":fields})).await
}

#[tauri::command]
async fn instrument_acquisition(device_id:u64,resource:String,action:String,interval_ms:Option<u64>,from_row:Option<u64>)->Result<Value,String>{
    if resource.is_empty()||resource.len()>4096||!matches!(action.as_str(),"state"|"start"|"pause"|"resume"|"stop"|"clear"){return Err("采集参数无效".into());}
    invoke_legacy("ui.instrument.acquisition_safe".into(),json!({"device_id":device_id,"resource":resource,"action":action,"interval_ms":interval_ms,"from_row":from_row,"limit":2000})).await
}

#[tauri::command]
async fn instrument_export_samples(device_id:u64,resource:String)->Result<Option<Value>,String>{
    if resource.is_empty()||resource.len()>4096{return Err("设备会话无效".into());}
    let path=tauri::async_runtime::spawn_blocking(||rfd::FileDialog::new().add_filter("采集记录 CSV",&["csv"]).set_file_name("instrument-samples.csv").save_file()).await.map_err(|_|"文件选择失败")?;
    let Some(path)=path else{return Ok(None)};
    invoke_legacy("ui.instrument.export_samples_safe".into(),json!({"device_id":device_id,"resource":resource,"path":path})).await.map(Some)
}

#[tauri::command]
async fn hub_run_control(project:String,plugin:String,run_id:String,action:String)->Result<Value,String>{
    if !valid_identifier(&project)||!valid_identifier(&plugin)||run_id.is_empty()||run_id.len()>256||run_id.chars().any(char::is_control)||!matches!(action.as_str(),"pause"|"resume"){return Err("测试运行控制参数无效".into());}
    let state=invoke_legacy("ui.test_tool.state_safe".into(),json!({})).await?;
    if state["project"]!=project||state["selected"]!=plugin{return Err("工程或插件已切换，请刷新后再操作".into());}
    invoke_legacy("ui.test_tool.run.control".into(),json!({"plugin":plugin,"run_id":run_id,"action":action})).await
}

#[tauri::command]
async fn serial_select_log_safe(tab_id:u64,view_id:u64)->Result<Value,String>{
    invoke_legacy("ui.serial.select_log_safe".into(),json!({"tab_id":tab_id,"view_id":view_id})).await
}

#[tauri::command]
async fn hub_document_save(target:String,plugin:String,project:String,config_id:String,revision:u64,sha256:String,schema_fingerprint:String,value:Value,validate_only:bool)->Result<Value,String>{
    if !matches!(target.as_str(),"config"|"project")||!valid_identifier(&project)||!valid_identifier(&config_id)||(revision>0&&(sha256.len()!=64||!sha256.bytes().all(|byte|byte.is_ascii_hexdigit()))){return Err("配置标识或版本无效".into());}
    if !valid_identifier(&plugin)||schema_fingerprint.len()!=64||!schema_fingerprint.bytes().all(|byte|byte.is_ascii_hexdigit())||!value.is_object()||serde_json::to_vec(&value).map_or(true,|bytes|bytes.len()>1_048_576){return Err("配置参数无效".into());}
    invoke_legacy(format!("ui.test_tool.{target}.edit"),json!({"plugin":plugin,"expected_project":project,"config_id":config_id,"expected_revision":revision,"expected_sha256":sha256,"schema_fingerprint":schema_fingerprint,"value":value,"validate_only":validate_only})).await
}

#[tauri::command]
async fn instrument_log_clear(device_id:u64,resource:String,revision:String)->Result<Value,String>{
    if resource.is_empty()||resource.len()>4096||revision.len()!=16||!revision.bytes().all(|byte|byte.is_ascii_hexdigit()){return Err("通信记录版本无效".into());}
    invoke_legacy("ui.instrument.io_log.clear_safe".into(),json!({"device_id":device_id,"resource":resource,"revision":revision})).await
}

#[tauri::command]
async fn instrument_control_safe(device_id:u64,resource:String,command:Value)->Result<Value,String>{
    if resource.is_empty()||resource.len()>4096||resource.contains(['\0','\r','\n'])||serde_json::to_vec(&command).map_or(true,|bytes|bytes.len()>16*1024){return Err("设备控制参数无效".into());}
    wiparse_core::instrument::parse_control_command(&command).map_err(|_|"不支持的仪器控制参数")?;
    invoke_legacy("ui.instrument.control_safe".into(),json!({"device_id":device_id,"resource":resource,"command":command})).await
}

#[tauri::command]
async fn serial_ai_action(action:String,params:Value)->Result<Value,String>{
    if !["control","preview","send","cancel","stop","clear","resume","close"].contains(&action.as_str())||!params.is_object()||serde_json::to_vec(&params).map_or(true,|bytes|bytes.len()>64*1024){return Err("AI 请求无效".into());}
    invoke_legacy(format!("ui.ai.serial.{action}_safe"),params).await
}

async fn invoke_legacy(method: String, params: Value) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        invoke_legacy_at(ENDPOINT, &method, params)
    }).await.map_err(|_| "读取任务异常结束".to_string())?
}

fn invoke_legacy_at(endpoint: &str, method: &str, params: Value) -> Result<Value, String> {
    // Hub configuration work is dispatched to a worker with a 65 s backend
    // deadline. Do not time it out early and then encourage a duplicate save.
    let read_timeout=if method.starts_with("ui.test_tool.config.")||method.starts_with("ui.test_tool.project."){70}else{18};
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(2)).timeout_read(Duration::from_secs(read_timeout)).build();
    let mut request=agent.post(endpoint);
    if endpoint==ENDPOINT {
        let health_response=agent.get("http://127.0.0.1:7878/v1/health").call().map_err(|_|"无法确认本机设备服务身份".to_string())?;
        let health=read_bounded_bridge_json(health_response.into_reader(),64*1024)?;
        let pid=host_identity(&health)?;
        let owned=OWNED_AUTH.lock().unwrap_or_else(|error|error.into_inner()).as_ref().filter(|(id,_)|*id==pid).map(|(_,token)|token.clone());
        let token=if let Some(token)=owned{token}else{
            let reference=health.pointer("/data/ai_auth_ref").and_then(Value::as_str).ok_or("受保护的本机服务凭据不可用，请检查 Windows 用户环境")?;
            wiparse_core::ai::load_local_bridge_token(reference)?
        };
        request=request.set("Authorization",&format!("Bearer {token}"));
    } else if method.starts_with("ui.ai.") {return Err("AI 接口只允许连接本机 WiParse".into());}
    let response = match request.send_json(json!({"method":method,"params":params})) {
        Ok(response)=>response,
        Err(ureq::Error::Status(_,response))=>response,
        Err(error)=>return Err(format!("无法读取现有 WiParse 服务：{error}；操作结果未确认，请刷新检查，勿重复提交")),
    };
    let status=response.status();
    let reply = read_bounded_bridge_json(response.into_reader(),16*1024*1024)?;
    if !(200..300).contains(&status)||reply.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(legacy_error(&reply));
    }
    Ok(reply.get("data").cloned().unwrap_or(Value::Null))
}
fn read_bounded_bridge_json(reader:impl std::io::Read,limit:usize)->Result<Value,String>{
    use std::io::Read;
    let mut bytes=Vec::new();
    reader.take(limit.saturating_add(1) as u64).read_to_end(&mut bytes).map_err(|_|"读取 WiParse 返回数据失败；操作结果未确认，请刷新检查，勿重复提交".to_string())?;
    if bytes.len()>limit{return Err("WiParse 返回的数据超过大小限制；操作结果未确认，请刷新检查，勿重复提交".into());}
    serde_json::from_slice(&bytes).map_err(|_|"WiParse 返回了无效数据；操作结果未确认，请刷新检查，勿重复提交".to_string())
}
fn legacy_error(reply:&Value)->String{
    let message=reply.pointer("/data/message").and_then(Value::as_str)
        .or_else(||reply.pointer("/error/code").and_then(Value::as_str).filter(|code|*code!="ERROR"))
        .or_else(||reply.pointer("/error/message").and_then(Value::as_str))
        .or_else(||reply.get("error").and_then(Value::as_str)).unwrap_or("读取失败").to_owned();
    let errors=reply.pointer("/error/details/errors").and_then(Value::as_array);
    let detail=errors.map(|errors|errors.iter().take(5).map(|error|{
        let path=error.get("path").and_then(Value::as_str).unwrap_or("/");
        let text=error.get("message").or_else(||error.get("keyword")).and_then(Value::as_str).unwrap_or("配置字段无效");
        format!("{}: {}",path.chars().take(160).collect::<String>(),text.chars().take(240).collect::<String>())
    }).collect::<Vec<_>>().join("；")).unwrap_or_default();
    if detail.is_empty(){message}else{format!("{message}；{detail}")}
}

// Calculator mutations are exposed separately; the generic compatibility
// command above remains read-only.
#[tauri::command]
async fn calculate_lc(fields: Value) -> Result<Value, String> {
    validate_lc_fields(&fields)?;
    invoke_legacy("ui.calc.set".into(), json!({"card":"lc", "fields":fields})).await
}

#[tauri::command]
async fn calculate_electrical(card: String, fields: Value) -> Result<Value, String> {
    validate_electrical_fields(&card,&fields)?;
    invoke_legacy("ui.calc.set".into(), json!({"card":card, "fields":fields})).await
}

#[tauri::command]
async fn calculate_crc(fields: Value) -> Result<Value, String> {
    validate_crc_fields(&fields)?;
    invoke_legacy("ui.calc.set".into(),json!({"card":"crc","fields":fields})).await
}

#[tauri::command]
async fn calculate_converter(fields: Value) -> Result<Value, String> {
    validate_converter_fields(&fields)?;
    invoke_legacy("ui.calc.set".into(),json!({"card":"convert","fields":fields})).await
}

fn validate_converter_fields(fields:&Value)->Result<(),String>{
    let values=fields.as_object().ok_or("转换参数无效")?;
    let tab=values.get("tab").and_then(Value::as_str).ok_or("请选择转换类型")?;
    let expected:&[&str]=match tab {
        "radix"=>&["tab","radix_input","source_base","target_base"],
        "scientific"=>&["tab","expression","angle_mode"],
        "ascii"=>&["tab","byte_input","byte_direction","byte_base"],
        _=>return Err("转换类型无效".into()),
    };
    if values.len()!=expected.len()||values.keys().any(|key|!expected.contains(&key.as_str())){return Err("转换参数不完整".into());}
    match tab {
        "radix"=>{
            if !values.get("radix_input").and_then(Value::as_str).is_some_and(|s|!s.is_empty()&&s.len()<=256){return Err("进制输入无效".into());}
            for key in ["source_base","target_base"]{if !values.get(key).and_then(Value::as_u64).is_some_and(|n|(2..=36).contains(&n)){return Err("进制必须在 2 至 36 之间".into());}}
        }
        "scientific"=>{
            if !values.get("expression").and_then(Value::as_str).is_some_and(|s|!s.is_empty()&&s.len()<=256){return Err("表达式无效或过长".into());}
            if !["radians","degrees"].contains(&values.get("angle_mode").and_then(Value::as_str).unwrap_or("")){return Err("角度单位无效".into());}
        }
        "ascii"=>{
            if !values.get("byte_input").and_then(Value::as_str).is_some_and(|s|!s.is_empty()&&s.len()<=65_536){return Err("字节输入无效或过长".into());}
            if !["text_to_bytes","bytes_to_text"].contains(&values.get("byte_direction").and_then(Value::as_str).unwrap_or(""))||!["decimal","hex"].contains(&values.get("byte_base").and_then(Value::as_str).unwrap_or("")){return Err("字节转换设置无效".into());}
        }
        _=>unreachable!(),
    }
    Ok(())
}

fn validate_crc_fields(fields:&Value)->Result<(),String>{
    let values=fields.as_object().ok_or("CRC 参数无效")?;
    let expected=["data","preset","input_mode","custom_width","custom_poly","custom_init","custom_xorout","custom_refin","custom_refout"];
    if values.len()!=expected.len()||values.keys().any(|key|!expected.contains(&key.as_str())) {return Err("CRC 参数不完整".into());}
    if !values.get("data").and_then(Value::as_str).is_some_and(|v|v.len()<=1_048_576) {return Err("CRC 数据超过 1 MiB 上限".into());}
    if !["CRC-8","CRC-8/MAXIM-DOW","CRC-16/IBM (ARC)","CRC-16/MODBUS","CRC-16/CCITT-FALSE","CRC-16/XMODEM","CRC-32/ISO-HDLC","CRC-32C","Custom"].contains(&values.get("preset").and_then(Value::as_str).unwrap_or("")) {return Err("CRC 类型无效".into());}
    if !["text","hex"].contains(&values.get("input_mode").and_then(Value::as_str).unwrap_or("")) {return Err("CRC 输入模式无效".into());}
    if !values.get("custom_width").and_then(Value::as_u64).is_some_and(|value|matches!(value,8|16|32)) {return Err("CRC 位宽无效".into());}
    for key in ["custom_poly","custom_init","custom_xorout"] {if !values.get(key).and_then(Value::as_str).is_some_and(|value|!value.is_empty()&&value.len()<=32) {return Err("CRC 参数无效".into());}}
    for key in ["custom_refin","custom_refout"] {if !values.get(key).is_some_and(Value::is_boolean){return Err("CRC 反射设置无效".into());}}
    Ok(())
}

fn validate_electrical_fields(card: &str, fields: &Value) -> Result<(), String> {
    let values=fields.as_object().ok_or("计算参数无效")?;
    let (numeric,units): (&[&str], &[(&str,&[&str])]) = match card {
        "q" => (&["v1","v2","bias","intervals"], &[]),
        "bandpass" => (&["r_hp","c_hp","r_lp","c_lp"], &[
            ("r_hp_unit", &["Ω","kΩ","MΩ"]), ("r_lp_unit", &["Ω","kΩ","MΩ"]),
            ("c_hp_unit", &["pF","nF","uF","mF","F"]), ("c_lp_unit", &["pF","nF","uF","mF","F"])]),
        "rc" => (&["resistance","capacitance","initial_voltage","final_voltage","time"], &[
            ("resistance_unit", &["mΩ","Ω","kΩ","MΩ"]), ("capacitance_unit", &["pF","nF","uF","mF","F"]), ("time_unit", &["us","ms","s"])]),
        _ => return Err("计算类型不支持".into()),
    };
    if values.len()!=numeric.len()+units.len() { return Err("计算参数不完整".into()); }
    for key in numeric {
        let input=values.get(*key).and_then(Value::as_str).ok_or("计算参数无效")?;
        if input.len()>64 { return Err("计算参数过长".into()); }
        let n=parse_numeric_literal(input)?;
        if !n.is_finite() { return Err("参数必须是有限数字".into()); }
    }
    for (key,choices) in units {
        if !choices.contains(&values.get(*key).and_then(Value::as_str).unwrap_or("")) { return Err("计算单位无效".into()); }
    }
    Ok(())
}

#[tauri::command]
async fn ai_page_summary(page:String)->Result<Value,String>{
    if !matches!(page.as_str(),"serial"|"instruments"|"calculator"|"tests"){return Err("请先打开对应工具，读取新工作台的当前摘要".into());}
    invoke_legacy("ui.ai.page_summary_safe".into(),json!({"page":page})).await
}

#[tauri::command]
async fn market_action(revision:String,action:String,id:Option<String>,version:Option<String>,consent:bool)->Result<Value,String>{
    if revision.len()!=16||!matches!(action.as_str(),"refresh"|"install"|"uninstall")||!consent||id.as_ref().is_some_and(|id|!valid_identifier(id))||version.as_ref().is_some_and(|version|version.len()>128||version.chars().any(char::is_control)){return Err("插件市场请求无效".into());}
    invoke_legacy("ui.market.action_safe".into(),json!({"revision":revision,"action":action,"id":id,"version":version,"consent":true})).await
}
#[tauri::command]
async fn market_configure(revision:String,fields:Value)->Result<Value,String>{
    if revision.len()!=16||!fields.is_object()||fields.to_string().len()>16384{return Err("插件市场设置无效".into());}
    invoke_legacy("ui.market.configure_safe".into(),json!({"revision":revision,"fields":fields})).await
}

#[tauri::command]
async fn hub_config_batch(project:String,plugin:String,ids:Vec<String>,expected:Value,enabled:Option<bool>,tags:Option<Vec<String>>)->Result<Value,String>{
    if !valid_identifier(&project)||!valid_identifier(&plugin)||ids.is_empty()||ids.len()>500||ids.iter().any(|id|!valid_identifier(id))||ids.iter().collect::<std::collections::HashSet<_>>().len()!=ids.len()||enabled.is_some()==tags.is_some(){return Err("批量操作参数无效".into());}
    if expected.as_object().map_or(true,|items|items.len()!=ids.len())||ids.iter().any(|id|!expected[id]["expected_revision"].as_u64().is_some_and(|revision|revision>0)||!expected[id]["expected_sha256"].as_str().is_some_and(|hash|hash.len()==64&&hash.bytes().all(|byte|byte.is_ascii_hexdigit()))){return Err("批量配置版本无效".into());}
    if tags.as_ref().is_some_and(|tags|tags.len()>64||tags.iter().any(|tag|tag.len()>128||tag.chars().any(char::is_control))){return Err("批量标签无效".into());}
    let mut params=json!({"plugin":plugin,"expected_project":project,"ids":ids,"expected":expected});
    if let Some(value)=enabled{params["enabled"]=json!(value);}if let Some(value)=tags{params["tags"]=json!(value);}
    invoke_legacy("ui.test_tool.config.batch".into(),params).await
}

#[tauri::command]
async fn patch_fpga_item(config_id: String, project: String, revision: u64, sha256: String, name: String, enabled: bool) -> Result<Value, String> {
    validate_fpga_patch(&config_id, &project, revision, &sha256, &name)?;
    invoke_legacy("ui.test_tool.config.patch".into(), json!({
        "plugin":"fpga-firmware-test", "config_id":config_id, "expected_project":project,
        "expected_revision":revision, "expected_sha256":sha256,
        "patch":{"name":name,"enabled":enabled}
    })).await
}

#[tauri::command]
async fn patch_wireless_limits(project:String,revision:u64,sha256:String,limits:Value)->Result<Value,String>{
    if !valid_identifier(&project)||sha256.len()!=64||!sha256.bytes().all(|b|b.is_ascii_hexdigit()){return Err("测试项版本信息无效，请刷新后重试".into());}
    let Some(fields)=limits.as_object() else{return Err("无线限值无效".into())};
    if fields.is_empty()||fields.keys().any(|k|!matches!(k.as_str(),"limit"|"rx_vout_control"|"physical_vout_limit"|"load_cc"))||serde_json::to_vec(&limits).map_or(true,|v|v.len()>4096){return Err("无线限值无效".into());}
    invoke_legacy("ui.test_tool.config.patch".into(),json!({"plugin":"wireless-whitebox","config_id":"wcv_07","expected_project":project,"expected_revision":revision,"expected_sha256":sha256,"patch":{"wireless_limits":limits}})).await
}

#[tauri::command]
async fn patch_wireless_station(project:String,revision:u64,sha256:String,station:Value)->Result<Value,String>{
    if !valid_identifier(&project)||!(revision==0&&sha256.is_empty()||revision>0&&sha256.len()==64&&sha256.bytes().all(|b|b.is_ascii_hexdigit())){return Err("工程版本无效，请刷新后重试".into());}
    let Some(fields)=station.as_object() else{return Err("工位设置无效".into())};
    const KEYS:&[&str]=&["station_id","rx_i2c_interface","log_source","rx_start_marker","rx_identification_marker","rx_running_marker","scope_resource","power_resource","load_resource","log_baud","rx_i2c_clock_hz","rx_ready_timeout_s","scope_capture_enabled","scope_source_enabled"];
    if fields.is_empty()||fields.keys().any(|k|!KEYS.contains(&k.as_str()))||serde_json::to_vec(&station).map_or(true,|v|v.len()>16384){return Err("工位设置无效".into());}
    invoke_legacy("ui.test_tool.project.patch_wireless_station".into(),json!({"plugin":"wireless-whitebox","expected_project":project,"expected_revision":revision,"expected_sha256":sha256,"patch":station})).await
}

#[tauri::command]
async fn patch_fpga_log(config_id:String,project:String,revision:u64,sha256:String,enabled:bool,start_at_s:f64,duration_s:f64)->Result<Value,String>{
    validate_fpga_patch(&config_id,&project,revision,&sha256,"valid")?;
    if !start_at_s.is_finite()||!(0.0..=1_000_000.0).contains(&start_at_s)||!duration_s.is_finite()||duration_s<=0.0||duration_s>1_000_000.0{return Err("日志时间无效".into());}
    invoke_legacy("ui.test_tool.config.patch".into(),json!({
        "plugin":"fpga-firmware-test","config_id":config_id,"expected_project":project,
        "expected_revision":revision,"expected_sha256":sha256,
        "patch":{"log_capture":{"enabled":enabled,"start_at_s":start_at_s,"duration_s":duration_s}}
    })).await
}

#[tauri::command]
async fn patch_fpga_wave(config_id:String,project:String,revision:u64,sha256:String,capture_id:String,at_s:f64,timeout_s:f64,source_enabled:bool,source_format:String,channels:Vec<String>,create:bool)->Result<Value,String>{
    validate_fpga_patch(&config_id,&project,revision,&sha256,"valid")?;
    let unique:std::collections::HashSet<_>=channels.iter().collect();
    if capture_id.is_empty()||capture_id.len()>128||!capture_id.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b))||!at_s.is_finite()||!(0.0..=1_000_000.0).contains(&at_s)||!timeout_s.is_finite()||timeout_s<=0.0||timeout_s>1_000_000.0||!["csv","instrument_native"].contains(&source_format.as_str())||channels.is_empty()||channels.len()>32||unique.len()!=channels.len()||channels.iter().any(|c|c.trim().is_empty()||c.len()>384||c.chars().any(char::is_control)){return Err("波形采集参数无效".into());}
    invoke_legacy("ui.test_tool.config.patch".into(),json!({"plugin":"fpga-firmware-test","config_id":config_id,"expected_project":project,"expected_revision":revision,"expected_sha256":sha256,
      "patch":{"waveform_capture":{"id":capture_id,"at_s":at_s,"timeout_s":timeout_s,"source_enabled":source_enabled,"source_format":source_format,"channels":channels,"create":create}}})).await
}

#[tauri::command]
async fn delete_fpga_wave(config_id:String,project:String,revision:u64,sha256:String,capture_id:String)->Result<Value,String>{
    validate_fpga_patch(&config_id,&project,revision,&sha256,"valid")?;
    if capture_id.is_empty()||capture_id.len()>128||!capture_id.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b)){return Err("波形任务标识无效".into());}
    invoke_legacy("ui.test_tool.config.patch".into(),json!({"plugin":"fpga-firmware-test","config_id":config_id,"expected_project":project,"expected_revision":revision,"expected_sha256":sha256,"patch":{"waveform_capture_delete":{"id":capture_id}}})).await
}

#[tauri::command]
async fn patch_fpga_rule(config_id:String,project:String,revision:u64,sha256:String,rule:Value)->Result<Value,String>{
    validate_fpga_patch(&config_id,&project,revision,&sha256,"valid")?;
    validate_fpga_rule(&rule)?;
    invoke_legacy("ui.test_tool.config.patch".into(),json!({"plugin":"fpga-firmware-test","config_id":config_id,"expected_project":project,"expected_revision":revision,"expected_sha256":sha256,"patch":{"verdict_rule":rule}})).await
}

#[tauri::command]
async fn delete_fpga_rule(config_id:String,project:String,revision:u64,sha256:String,rule_id:String)->Result<Value,String>{
    validate_fpga_patch(&config_id,&project,revision,&sha256,"valid")?;
    if rule_id.is_empty()||rule_id.len()>384||rule_id.chars().any(char::is_control){return Err("判定条件标识无效".into());}
    invoke_legacy("ui.test_tool.config.patch".into(),json!({"plugin":"fpga-firmware-test","config_id":config_id,"expected_project":project,"expected_revision":revision,"expected_sha256":sha256,"patch":{"verdict_rule_delete":{"id":rule_id}}})).await
}

#[tauri::command]
async fn generic_plugin_run(project:String,plugin:String,data_root:String,expected:Value,values:Value,lifecycle:String)->Result<Value,String>{
    if !valid_identifier(&project)||!valid_identifier(&plugin)||data_root.is_empty()||data_root.len()>4096||!expected.is_object()||!values.is_object()||!matches!(lifecycle.as_str(),"preflight"|"run")||serde_json::to_vec(&values).map_or(true,|bytes|bytes.len()>1_048_576){return Err("插件参数无效".into());}
    invoke_legacy("ui.test_tool.generic_run_safe".into(),json!({"project":project,"plugin":plugin,"data_root":data_root,"expected":expected,"values":values,"lifecycle":lifecycle})).await
}

#[tauri::command]
async fn generic_plugin_stop(plugin:String,running_token:String)->Result<Value,String>{
    if !valid_identifier(&plugin)||running_token.is_empty()||running_token.len()>128{return Err("任务标识无效".into());}
    invoke_legacy("ui.test_tool.generic_stop_safe".into(),json!({"plugin":plugin,"running_token":running_token})).await
}

#[tauri::command]
async fn generic_select_path(directory:bool)->Result<Option<String>,String>{
    let selected=tauri::async_runtime::spawn_blocking(move||{
        let dialog=rfd::FileDialog::new();
        if directory {dialog.pick_folder()} else {dialog.pick_file()}
    }).await.map_err(|_|"无法打开路径选择窗口")?;
    Ok(selected.map(|path|path.to_string_lossy().into_owned()))
}

#[tauri::command]
async fn clone_fpga_item(source_id:String,project:String,revision:u64,sha256:String,new_id:String,name:String)->Result<Value,String>{
    validate_fpga_patch(&source_id,&project,revision,&sha256,&name)?;
    if !valid_identifier(&new_id)||source_id.eq_ignore_ascii_case(&new_id){return Err("新测试项标识无效或与原标识重复".into());}
    invoke_legacy("ui.test_tool.config.clone".into(),json!({"plugin":"fpga-firmware-test","expected_project":project,"source_id":source_id,"source_revision":revision,"source_sha256":sha256,"new_id":new_id,"name":name})).await
}

#[tauri::command]
async fn create_fpga_item(project:String,config_id:String,name:String)->Result<Value,String>{
    if !valid_identifier(&project)||!valid_identifier(&config_id)||name.trim().is_empty()||name.len()>512||name.chars().any(char::is_control){return Err("测试项标识或名称无效".into());}
    invoke_legacy("ui.test_tool.config.create_saved".into(),json!({"plugin":"fpga-firmware-test","expected_project":project,"config_id":config_id,"name":name})).await
}

#[tauri::command]
async fn select_fpga_firmware(config_id:String,project:String,revision:u64,sha256:String)->Result<Option<Value>,String>{
    validate_fpga_patch(&config_id,&project,revision,&sha256,"valid")?;
    let selected=tauri::async_runtime::spawn_blocking(||rfd::FileDialog::new().add_filter("FPGA 固件",&["hex","bin"]).pick_file()).await.map_err(|_|"无法打开固件选择窗口")?;
    let Some(path)=selected else{return Ok(None)};
    let path=path.to_str().ok_or("文件路径无法表示")?.to_owned();
    let result=invoke_legacy("ui.test_tool.config.select_firmware".into(),json!({"plugin":"fpga-firmware-test","expected_project":project,"config_id":config_id,"expected_revision":revision,"expected_sha256":sha256,"path":path})).await?;
    Ok(Some(result))
}

#[tauri::command]
async fn patch_fpga_project_dut(project:String,revision:u64,sha256:String,dut:Value)->Result<Value,String>{
    validate_fpga_patch("valid",&project,revision,&sha256,"valid")?;
    let values=dut.as_object().ok_or("DUT 参数无效")?;let fields=["model","board_revision","fpga_model","fixture_id","station_id","serial_required"];
    if values.len()!=fields.len()||values.keys().any(|key|!fields.contains(&key.as_str()))||!values.get("serial_required").is_some_and(Value::is_boolean){return Err("DUT 参数不完整".into());}
    for key in &fields[..5]{if !values.get(*key).and_then(Value::as_str).is_some_and(|s|s.len()<=768&&!s.chars().any(char::is_control)&&s!="[REDACTED]"&&s!="[LOCAL_PATH]"){return Err("DUT 参数无效".into());}}
    invoke_legacy("ui.test_tool.project.patch_dut".into(),json!({"plugin":"fpga-firmware-test","expected_project":project,"expected_revision":revision,"expected_sha256":sha256,"dut":dut})).await
}

#[tauri::command]
async fn patch_fpga_project_resource(project:String,revision:u64,sha256:String,role:String,resource:String)->Result<Value,String>{
    validate_fpga_patch("valid",&project,revision,&sha256,"valid")?;
    if role.is_empty()||role.len()>64||!role.as_bytes()[0].is_ascii_alphanumeric()||!role.bytes().all(|b|b.is_ascii_alphanumeric()||b"_-".contains(&b))||resource.trim().is_empty()||resource.len()>1024||resource.chars().any(char::is_control)||resource=="[REDACTED]"||resource=="[LOCAL_PATH]"{return Err("仪器资源无效".into());}
    invoke_legacy("ui.test_tool.project.patch_resource".into(),json!({"plugin":"fpga-firmware-test","expected_project":project,"expected_revision":revision,"expected_sha256":sha256,"role":role,"resource":resource})).await
}

#[tauri::command]
async fn patch_fpga_project_serial(project:String,revision:u64,sha256:String,role:String,port:String,baud:u32)->Result<Value,String>{
    validate_fpga_patch("valid",&project,revision,&sha256,"valid")?;
    let upper=port.to_ascii_uppercase();let suffix=upper.strip_prefix("COM").unwrap_or("");
    if role.is_empty()||role.len()>64||!role.as_bytes()[0].is_ascii_alphanumeric()||!role.bytes().all(|b|b.is_ascii_alphanumeric()||b"_-".contains(&b))||suffix.is_empty()||suffix.len()>6||suffix.starts_with('0')||!suffix.bytes().all(|b|b.is_ascii_digit())||baud==0||baud>12_000_000{return Err("串口参数无效".into());}
    invoke_legacy("ui.test_tool.project.patch_serial".into(),json!({"plugin":"fpga-firmware-test","expected_project":project,"expected_revision":revision,"expected_sha256":sha256,"role":role,"port":upper,"baud":baud})).await
}

#[tauri::command]
async fn patch_fpga_project_dongle_uart(project:String,revision:u64,sha256:String,role:String,serial:String,channel:u32,baud:u32)->Result<Value,String>{
    validate_fpga_patch("valid",&project,revision,&sha256,"valid")?;
    if role.is_empty()||role.len()>64||!role.as_bytes()[0].is_ascii_alphanumeric()||!role.bytes().all(|b|b.is_ascii_alphanumeric()||b"_-".contains(&b))||serial.is_empty()||serial.len()>128||!serial.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b))||channel!=0||baud==0||baud>12_000_000{return Err("Dongle UART 参数无效".into());}
    invoke_legacy("ui.test_tool.project.patch_dongle_uart".into(),json!({"plugin":"fpga-firmware-test","expected_project":project,"expected_revision":revision,"expected_sha256":sha256,"role":role,"serial":serial,"channel":channel,"baud":baud})).await
}

#[tauri::command]
async fn patch_fpga_reset(config_id:String,project:String,revision:u64,sha256:String,project_revision:u64,project_sha256:String,method:String,role:String,channel:u32,off_time_ms:f64)->Result<Value,String>{
    validate_fpga_patch(&config_id,&project,revision,&sha256,"valid")?;
    validate_fpga_patch("valid",&project,project_revision,&project_sha256,"valid")?;
    if !["dc_power","wiparse_dongle"].contains(&method.as_str())||!valid_identifier(&role)||channel==0||channel>256||!off_time_ms.is_finite()||!(0.0..=1_000_000.0).contains(&off_time_ms){return Err("复位参数无效".into());}
    invoke_legacy("ui.test_tool.config.patch".into(),json!({"plugin":"fpga-firmware-test","expected_project":project,"config_id":config_id,"expected_revision":revision,"expected_sha256":sha256,"expected_project_revision":project_revision,"expected_project_sha256":project_sha256,"patch":{"reset":{"method":method,"role":role,"channel":channel,"off_time_ms":off_time_ms}}})).await
}

fn valid_serial_port(port:&str)->bool{
    if port.is_empty()||port.len()>512||port.chars().any(char::is_control){return false;}
    let upper=port.to_ascii_uppercase();
    if let Some(number)=upper.strip_prefix("COM"){return !number.is_empty()&&number.len()<=6&&!number.starts_with('0')&&number.bytes().all(|b|b.is_ascii_digit());}
    if let Some(serial)=wiparse_core::serial::parse_winusb_uart_device(port){
        let prefix=wiparse_core::serial::WINUSB_UART_PREFIX;
        return if serial.is_empty(){port.eq_ignore_ascii_case(prefix)}else{serial.len()<=128&&serial.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b))&&port.eq_ignore_ascii_case(&format!("{prefix}?serial={serial}"))};
    }
    if let Some((host,_))=wiparse_core::serial::parse_tcp_uart_device(port){return host.len()<=253&&host.bytes().all(|b|b.is_ascii_alphanumeric()||b".:-_".contains(&b));}
    false
}
#[tauri::command]
async fn serial_start_safe(port:String,baud:u32)->Result<Value,String>{
    if !valid_serial_port(&port)||baud==0||baud>12_000_000{return Err("串口参数无效".into());}
    invoke_legacy("ui.serial.monitor.start_safe".into(),json!({"port":port,"baud":baud})).await
}
#[tauri::command]
async fn serial_stop_safe(port:String)->Result<Value,String>{
    if !valid_serial_port(&port){return Err("串口参数无效".into());}
    invoke_legacy("ui.serial.monitor.stop_safe".into(),json!({"port":port})).await
}
#[tauri::command]
async fn serial_send_safe(port:String,bytes:Vec<u8>)->Result<Value,String>{
    if !valid_serial_port(&port)||bytes.is_empty()||bytes.len()>4096{return Err("串口发送参数无效".into());}
    invoke_legacy("ui.serial.monitor.send_safe".into(),json!({"port":port,"bytes":bytes})).await
}

#[tauri::command]
async fn instrument_scan_safe()->Result<Value,String>{
    invoke_legacy("ui.instrument.scan_safe".into(),json!({})).await
}
#[tauri::command]
async fn serial_edit_safe(tab_id:u64,view_id:u64,action:String,params:Value)->Result<Option<Value>,String>{
    if !matches!(action.as_str(),"begin"|"read"|"patch"|"find"|"find_previous"|"replace"|"replace_all"|"save"|"save_as"|"discard")||!params.is_object()||params.to_string().len()>3*1024*1024{return Err("日志编辑请求无效或当前页超过传输上限".into());}
    let mut params=params;params["tab_id"]=json!(tab_id);params["view_id"]=json!(view_id);params["action"]=json!(action);
    if action=="save_as"{
        let path=tauri::async_runtime::spawn_blocking(||rfd::FileDialog::new().add_filter("日志文本",&["txt","log","csv","json","jsonl"]).set_file_name("edited-log.txt").save_file()).await.map_err(|_|"文件选择失败")?;
        let Some(path)=path else{return Ok(None)};params["path"]=json!(path);
    }
    invoke_legacy("ui.serial.edit_safe".into(),params).await.map(Some)
}

#[tauri::command]
fn serial_explain_line(text:String)->Result<Value,String>{
    if text.len()>65536||text.contains('\0'){return Err("日志行超过说明长度上限".into());}
    let details=wiparse_core::protocol::decode_qi_message(&text).map(|decoded|wiparse_core::protocol::format_qi_tooltip(&decoded).into_iter().map(|line|json!({"role":format!("{:?}",line.role).to_ascii_lowercase(),"text":line.text})).collect::<Vec<_>>()).unwrap_or_default();
    Ok(json!({"parsed":wiparse_core::protocol::parse_qi_line(&text),"details":details}))
}

#[tauri::command]
async fn serial_new_log_safe(view_id:u64,content_epoch:u64,consent:bool)->Result<Value,String>{
    if !consent{return Err("需要确认新建实时日志页".into());}
    invoke_legacy("ui.serial.new_log_safe".into(),json!({"view_id":view_id,"content_epoch":content_epoch,"consent":true})).await
}

#[tauri::command]
async fn serial_recording_settings_safe(revision:String,enabled:bool,file_name:String,directory:String)->Result<Value,String>{
    if revision.len()!=16||!revision.bytes().all(|value|value.is_ascii_hexdigit())||file_name.is_empty()||file_name.len()>120||directory.is_empty()||directory.len()>4096||directory.chars().any(char::is_control){return Err("日志记录设置无效".into());}
    invoke_legacy("ui.serial.recording_settings_safe".into(),json!({"revision":revision,"enabled":enabled,"file_name":file_name,"directory":directory})).await
}

#[tauri::command]
async fn serial_recording_directory()->Result<Option<String>,String>{
    tauri::async_runtime::spawn_blocking(||{
        let Some(path)=rfd::FileDialog::new().set_title("选择日志保存目录").pick_folder() else{return Ok(None)};
        let root=wiparse_core::paths::app_root();
        Ok(Some(if path==root{".".into()}else{path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().into_owned()}))
    }).await.map_err(|_|"目录选择失败".to_string())?
}

#[tauri::command]
async fn serial_recording_safe(enabled:bool)->Result<Value,String>{
    invoke_legacy("ui.serial.recording_safe".into(),json!({"enabled":enabled})).await
}

#[tauri::command]
async fn instrument_connect_safe(resource:String)->Result<Value,String>{
    if resource.is_empty()||resource.len()>1024||resource.chars().any(char::is_control){return Err("仪器地址无效".into());}
    invoke_legacy("ui.instrument.connect_safe".into(),json!({"resource":resource})).await
}

#[tauri::command]
async fn instrument_connect_manual_safe(resource:String,kind:Option<String>)->Result<Value,String>{
    let resource=resource.trim();
    if resource.is_empty()||resource.len()>1024||resource.chars().any(char::is_control){return Err("仪器地址无效".into());}
    if kind.as_deref().is_some_and(|value|!matches!(value,"oscilloscope"|"dc_source"|"electronic_load"|"multimeter"|"debug_probe"|"usb_bridge"|"wiparse_dongle"|"generic")){return Err("仪器类型无效".into());}
    invoke_legacy("ui.instrument.connect_manual_safe".into(),json!({"resource":resource,"kind":kind})).await
}

#[tauri::command]
async fn instrument_disconnect_safe(device_id:u64,resource:String)->Result<Value,String>{
    if device_id==0||resource.is_empty()||resource.len()>1024||resource.chars().any(char::is_control){return Err("设备会话无效".into());}
    invoke_legacy("ui.instrument.disconnect_safe".into(),json!({"device_id":device_id,"resource":resource})).await
}

#[tauri::command]
async fn instrument_measure_safe(device_id:u64,resource:String)->Result<Value,String>{
    if device_id==0||resource.is_empty()||resource.len()>1024||resource.chars().any(char::is_control){return Err("设备会话无效".into());}
    invoke_legacy("ui.instrument.measure_safe".into(),json!({"device_id":device_id,"resource":resource})).await
}

#[tauri::command]
async fn instrument_scope_action_safe(device_id:u64,resource:String,action:String)->Result<Value,String>{
    if device_id==0||resource.is_empty()||resource.len()>1024||resource.chars().any(char::is_control)||!matches!(action.as_str(),"run"|"stop"|"single"){return Err("示波器操作无效".into());}
    invoke_legacy("ui.instrument.scope_action_safe".into(),json!({"device_id":device_id,"resource":resource,"action":action})).await
}
#[tauri::command]
async fn instrument_source_action_safe(device_id:u64,resource:String,channel:u8,action:String,value:Option<f64>,enabled:Option<bool>)->Result<Value,String>{
    if resource.trim().is_empty()||resource.len()>1024||resource.chars().any(char::is_control)||!(1..=4).contains(&channel){return Err("电源或通道无效".into());}
    match action.as_str(){
        "voltage"|"ovp" if value.is_some_and(|v|v.is_finite()&&(0.0..=60.0).contains(&v))&&enabled.is_none()=>{},
        "current"|"ocp" if value.is_some_and(|v|v.is_finite()&&(0.0..=20.0).contains(&v))&&enabled.is_none()=>{},
        "output" if enabled.is_some()&&value.is_none()=>{},
        _=>return Err("电源命令无效".into()),
    }
    invoke_legacy("ui.instrument.source_action_safe".into(),json!({"device_id":device_id,"resource":resource,"channel":channel,"action":action,"value":value,"enabled":enabled})).await
}
#[tauri::command]
async fn instrument_load_action_safe(device_id:u64,resource:String,action:String,mode:Option<String>,value:Option<f64>,enabled:Option<bool>)->Result<Value,String>{
    if resource.trim().is_empty()||resource.len()>1024||resource.chars().any(char::is_control){return Err("负载设备无效".into());}
    match action.as_str(){
        "setpoint" if mode.as_deref().is_some_and(|m|["CC","CV","CR","CP"].contains(&m))&&value.is_some_and(|v|v.is_finite()&&(0.0..=1_000_000.0).contains(&v)&&(mode.as_deref()!=Some("CR")||v>=0.001))&&enabled.is_none()=>{},
        "input" if enabled.is_some()&&mode.is_none()&&value.is_none()=>{},
        _=>return Err("负载命令无效".into()),
    }
    invoke_legacy("ui.instrument.load_action_safe".into(),json!({"device_id":device_id,"resource":resource,"action":action,"mode":mode,"value":value,"enabled":enabled})).await
}

#[tauri::command]
async fn instrument_capture_safe(device_id:u64,resource:String)->Result<Value,String>{
    if device_id==0||resource.is_empty()||resource.len()>1024||resource.chars().any(char::is_control){return Err("设备会话无效".into());}
    invoke_legacy("ui.instrument.capture_safe".into(),json!({"device_id":device_id,"resource":resource})).await
}

#[tauri::command]
async fn serial_open_log_safe()->Result<Option<Value>,String>{
    let file=tauri::async_runtime::spawn_blocking(||rfd::FileDialog::new().set_title("打开日志").add_filter("日志和文本",&["log","txt"]).pick_file()).await.map_err(|_|"选择日志文件失败".to_owned())?;
    let Some(file)=file else{return Ok(None)};
    let path=std::fs::canonicalize(file).map_err(|error|format!("无法读取日志：{error}"))?;
    if !path.is_file(){return Err("所选路径不是文件".into());}
    let path=path.to_str().filter(|s|s.len()<=4096&&!s.chars().any(char::is_control)).ok_or("日志文件路径无效")?;
    invoke_legacy("ui.serial.open_safe".into(),json!({"path":path})).await.map(Some)
}

#[tauri::command]
async fn serial_close_log_safe(tab_id:u64,view_id:u64)->Result<Value,String>{
    if tab_id==0||view_id==0{return Err("日志页无效".into());}
    invoke_legacy("ui.serial.close_safe".into(),json!({"tab_id":tab_id,"view_id":view_id})).await
}

#[tauri::command]
async fn serial_clear_live_safe(view_id:u64,content_epoch:u64)->Result<Value,String>{
    if view_id==0{return Err("日志页无效".into());}
    invoke_legacy("ui.serial.clear_live_safe".into(),json!({"view_id":view_id,"content_epoch":content_epoch})).await
}

#[tauri::command]
async fn instrument_waveform_source_safe(device_id:u64,resource:String)->Result<Option<Value>,String>{
    if device_id==0||resource.is_empty()||resource.len()>1024||resource.chars().any(char::is_control){return Err("设备会话无效".into());}
    let dir=tauri::async_runtime::spawn_blocking(move||->Result<Option<std::path::PathBuf>,String>{
        let Some(parent)=rfd::FileDialog::new().set_title("选择波形保存目录").pick_folder() else{return Ok(None)};
        let parent=std::fs::canonicalize(parent).map_err(|error|format!("无法读取目录：{error}"))?;
        let dir=parent.join(format!("WiParse-wave-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).map_err(|error|format!("无法创建新目录：{error}"))?;
        Ok(Some(dir))
    }).await.map_err(|_|"选择保存目录失败".to_owned())??;
    let Some(dir)=dir else{return Ok(None)};
    let path=dir.to_str().ok_or("目录路径无法表示")?.to_owned();
    let result=invoke_legacy("ui.instrument.waveform_source_safe".into(),json!({"device_id":device_id,"resource":resource,"dir":path})).await
        .map_err(|error|{
            let rejected=["INSTRUMENT.TEST_IN_PROGRESS","INSTRUMENT.SESSION_CHANGED","INSTRUMENT.BUSY","INSTRUMENT.WAVEFORM_UNSUPPORTED","INSTRUMENT.INVALID_DIRECTORY","INSTRUMENT.INVALID_REQUEST"].iter().any(|code|error.contains(code));
            if rejected&&std::fs::remove_dir(&dir).is_ok(){error}else{format!("{error}；保存目录 {} 可能已创建，请检查",dir.display())}
        })?;
    Ok(Some(json!({"job_id":result["job_id"],"device_id":device_id,"directory":path})))
}

#[tauri::command]
async fn instrument_screenshot_safe(device_id:u64,resource:String,job_id:u64)->Result<Value,String>{
    if device_id==0||job_id==0||resource.is_empty()||resource.len()>1024||resource.chars().any(char::is_control){return Err("截图任务无效".into());}
    let result=invoke_legacy("ui.instrument.screenshot_safe".into(),json!({"device_id":device_id,"resource":resource,"job_id":job_id})).await?;
    match result.get("state").and_then(Value::as_str){
        Some("pending")=>Ok(json!({"state":"pending"})),
        Some("completed")=>{
            let png=result.get("png_base64").and_then(Value::as_str).ok_or("截图数据无效")?;
            if png.len()>5_592_408||png.is_empty()||!png.bytes().all(|b|b.is_ascii_alphanumeric()||b"+/=".contains(&b)){return Err("截图数据无效或过大".into());}
            Ok(json!({"state":"completed","png_base64":png}))
        },
        _=>Err("截图状态无效".into())
    }
}

#[tauri::command]
async fn save_scope_screenshot(png_base64:String)->Result<Option<String>,String>{
    use base64::Engine;
    if png_base64.is_empty()||png_base64.len()>5_592_408{return Err("截图数据无效或过大".into());}
    let bytes=base64::engine::general_purpose::STANDARD.decode(png_base64).map_err(|_|"截图数据无效")?;
    if bytes.len()>4*1024*1024||!bytes.starts_with(b"\x89PNG\r\n\x1a\n"){return Err("截图不是有效的 PNG 文件".into());}
    tauri::async_runtime::spawn_blocking(move||{
        let Some(path)=rfd::FileDialog::new().add_filter("PNG 图片",&["png"]).set_file_name("screenshot.png").save_file() else{return Ok(None)};
        save_png_new(&path,&bytes)?;
        Ok(Some(path.file_name().and_then(|n|n.to_str()).unwrap_or("screenshot.png").to_owned()))
    }).await.map_err(|_|"截图保存失败".to_owned())?
}
fn save_png_new(path:&std::path::Path,bytes:&[u8])->Result<(),String>{
    use std::io::Write;
    if !path.extension().and_then(|ext|ext.to_str()).is_some_and(|ext|ext.eq_ignore_ascii_case("png")){return Err("请选择 PNG 文件名".into());}
    let parent=path.parent().ok_or("保存目录无效")?;
    let temp=parent.join(format!(".wiparse-screenshot-{}.tmp",uuid::Uuid::new_v4()));
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e|format!("无法创建临时文件：{e}"))?;
    let result=(||->Result<(),String>{file.write_all(bytes).and_then(|_|file.sync_all()).map_err(|e|format!("截图写入失败：{e}"))?;drop(file);commit_file::publish_new(&temp,path).map_err(|e|if e.kind()==std::io::ErrorKind::AlreadyExists{"目标文件已存在，请选择其他文件名".into()}else{format!("无法安全创建文件：{e}")})?;Ok(())})();
    let _=std::fs::remove_file(&temp);
    result
}

#[tauri::command]
async fn select_plugin_safe(project:String,plugin:String)->Result<Value,String>{
    if project.is_empty() || project.len()>128 || plugin.is_empty() || plugin.len()>128 || !plugin.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b)) {return Err("项目或插件标识无效".into());}
    invoke_legacy("ui.test_tool.select_safe".into(),json!({"project":project,"plugin":plugin})).await
}
#[tauri::command]
async fn scope_rule_enabled_safe(project:String,expected:String,index:usize,enabled:bool)->Result<Value,String>{
    if !valid_identifier(&project)||expected.len()>1_000_000||index>10000{return Err("规则参数无效".into());}
    invoke_legacy("ui.test_tool.scope_rule_enabled_safe".into(),json!({"project":project,"expected":expected,"index":index,"enabled":enabled})).await
}
#[tauri::command]
async fn scope_rule_add_safe(project:String,expected:String,pattern:String)->Result<Value,String>{
    if !valid_identifier(&project)||expected.len()>1_000_000||pattern.trim().is_empty()||pattern.len()>512||pattern.chars().any(char::is_control){return Err("条件内容无效".into());}
    invoke_legacy("ui.test_tool.scope_rule_add_safe".into(),json!({"project":project,"expected":expected,"pattern":pattern})).await
}
#[tauri::command]
async fn scope_rule_text_safe(project:String,expected:String,index:usize,pattern:String)->Result<Value,String>{
    if !valid_identifier(&project)||expected.len()>1_000_000||index>10000||pattern.trim().is_empty()||pattern.len()>512||pattern.chars().any(char::is_control){return Err("条件内容无效".into());}
    invoke_legacy("ui.test_tool.scope_rule_text_safe".into(),json!({"project":project,"expected":expected,"index":index,"pattern":pattern})).await
}

#[tauri::command]
async fn scope_rule_exclusion_safe(project:String,expected:String,index:usize,action:String,text:Option<String>,key:Option<String>,item_index:Option<usize>)->Result<Value,String>{
    if !valid_identifier(&project)||expected.len()>1_000_000||index>=500{return Err("条件信息无效".into());}
    match action.as_str(){
        "add"=>if text.as_deref().is_none_or(|s|s.trim().is_empty()||s.len()>512||s.chars().any(char::is_control))||key.is_some()||item_index.is_some(){return Err("忽略内容无效".into());},
        "remove"=>if !key.as_deref().is_some_and(|s|matches!(s,"unless"|"except"|"exclude"))||item_index.is_none_or(|i|i>=500)||text.is_some(){return Err("忽略条件无效".into());},
        _=>return Err("条件操作无效".into()),
    }
    invoke_legacy("ui.test_tool.scope_rule_exclusion_safe".into(),json!({"project":project,"expected":expected,"index":index,"action":action,"text":text,"key":key,"item_index":item_index})).await
}
#[tauri::command]
async fn scope_rule_order_safe(project:String,expected:String,index:usize,action:String)->Result<Value,String>{
    if project.is_empty()||project.len()>64||expected.len()>1_000_000||index>=500||!matches!(action.as_str(),"up"|"down"|"remove"){return Err("CONFIG.INVALID_REQUEST".into());}
    invoke_legacy("ui.test_tool.scope_rule_order_safe".into(),json!({"project":project,"expected":expected,"index":index,"action":action})).await
}

fn validate_scope_settings(values:&Value)->Result<(),String>{
    let fields=values.as_object().ok_or("设置文件无效")?;
    let keys=["port","baud","triggers","scope_resource","scope_model","scope_kind","rising_edge"];
    if fields.len()!=keys.len()||keys.iter().any(|key|!fields.get(*key).is_some_and(Value::is_string)){return Err("设置文件字段无效".into());}
    let text=|key:&str|fields[key].as_str().unwrap_or("");
    if text("port").trim().is_empty()||text("port").len()>256||text("port").chars().any(char::is_control)||text("baud").parse::<u64>().ok().is_none_or(|b|!(300..=12_000_000).contains(&b))||text("triggers").len()>1_000_000||text("scope_resource").len()>1024||text("scope_model").len()>256||text("scope_resource").chars().any(char::is_control)||text("scope_model").chars().any(char::is_control)||!["oscilloscope","dc_source","electronic_load","multimeter","generic"].contains(&text("scope_kind"))||!["true","false"].contains(&text("rising_edge")){return Err("设置文件内容无效".into());}
    let rules:Value=serde_json::from_str(text("triggers")).map_err(|_|"抓取条件格式无效")?;
    if rules.as_array().is_none_or(|items|items.len()>500||!items.iter().any(|item|item.is_object()&&item.get("enabled")!=Some(&Value::Bool(false)))){return Err("请保留至少一条启用的抓取条件".into());}
    Ok(())
}
#[tauri::command]
async fn scope_edge_mode_safe(project:String,expected:String,enabled:bool)->Result<Value,String>{
    if project.is_empty()||project.len()>64||!["true","false"].contains(&expected.as_str()){return Err("抓取设置无效".into());}
    invoke_legacy("ui.test_tool.scope_edge_mode_safe".into(),json!({"project":project,"expected":expected,"enabled":enabled})).await
}
fn write_scope_settings_new(path:&std::path::Path,values:&Value)->Result<(),String>{
    use std::io::Write;validate_scope_settings(values)?;
    if !path.extension().and_then(|v|v.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("json")){return Err("请选择 JSON 文件名".into());}
    let bytes=serde_json::to_vec_pretty(&json!({"format":"wiparse.scope-monitor-settings/1","values":values})).map_err(|_|"无法生成设置文件")?;
    if bytes.len()>1_100_000{return Err("设置文件过大，无法导出".into());}
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e|format!("无法创建新文件（请勿选择已有文件）：{e}"))?;
    file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|e|format!("写入未完成，请检查文件：{e}"))
}
#[tauri::command]
async fn scope_settings_export(project:String,expected:Value)->Result<Option<String>,String>{
    if project.is_empty()||project.len()>64||!expected.is_object()||serde_json::to_vec(&expected).map_err(|_|"当前设置无效")?.len()>1_100_000{return Err("当前设置无效".into());}
    let values=invoke_legacy("ui.test_tool.scope_settings_export_safe".into(),json!({"project":project,"expected":expected})).await?;
    validate_scope_settings(&values)?;
    tauri::async_runtime::spawn_blocking(move||{
        let Some(path)=rfd::FileDialog::new().set_title("导出监控设置").add_filter("JSON 设置",&["json"]).set_file_name("scope-monitor-settings.json").save_file() else{return Ok(None)};
        write_scope_settings_new(&path,&values)?;
        Ok(Some(path.file_name().and_then(|s|s.to_str()).unwrap_or("settings.json").to_owned()))
    }).await.map_err(|_|"导出任务异常结束".to_owned())?
}
#[tauri::command]
async fn scope_settings_import(project:String,expected:Value)->Result<Option<Value>,String>{
    if project.is_empty()||project.len()>64||!expected.is_object()||serde_json::to_vec(&expected).map_err(|_|"当前设置无效")?.len()>1_100_000{return Err("当前设置无效".into());}
    let values=tauri::async_runtime::spawn_blocking(||->Result<Option<Value>,String>{
        use std::io::Read;
        let Some(path)=rfd::FileDialog::new().set_title("导入监控设置").add_filter("JSON 设置",&["json"]).pick_file() else{return Ok(None)};
        let file=std::fs::File::open(path).map_err(|e|format!("无法打开设置文件：{e}"))?;let mut bytes=Vec::new();
        file.take(1_100_001).read_to_end(&mut bytes).map_err(|e|format!("无法读取设置文件：{e}"))?;
        if bytes.len()>1_100_000{return Err("设置文件过大".into());}
        let bundle:Value=serde_json::from_slice(&bytes).map_err(|_|"设置文件不是有效 JSON")?;
        if bundle.get("format").and_then(Value::as_str)!=Some("wiparse.scope-monitor-settings/1")||bundle.as_object().is_none_or(|o|o.len()!=2){return Err("设置文件格式不受支持".into());}
        let values=bundle.get("values").cloned().ok_or("设置文件缺少内容")?;validate_scope_settings(&values)?;Ok(Some(values))
    }).await.map_err(|_|"导入任务异常结束".to_owned())??;
    let Some(values)=values else{return Ok(None)};
    invoke_legacy("ui.test_tool.scope_settings_import_safe".into(),json!({"project":project,"expected":expected,"values":values})).await.map(Some)
}
#[tauri::command]
async fn scope_serial_settings_safe(project:String,expected_port:String,expected_baud:String,port:String,baud:u64)->Result<Value,String>{
    if !valid_identifier(&project)||port.trim().is_empty()||port.len()>256||port.chars().any(char::is_control)||!(300..=12_000_000).contains(&baud)||expected_port.len()>256||expected_baud.len()>32{return Err("串口设置无效".into());}
    invoke_legacy("ui.test_tool.scope_serial_settings_safe".into(),json!({"project":project,"expected_port":expected_port,"expected_baud":expected_baud,"port":port,"baud":baud})).await
}
#[tauri::command]
async fn scope_instrument_safe(project:String,device_id:u64,resource:String,expected:Value)->Result<Value,String>{
    let keys=["prefer_device_id","scope_resource","scope_model","scope_kind"];
    if !valid_identifier(&project)||resource.trim().is_empty()||resource.len()>1024||resource.chars().any(char::is_control)||expected.as_object().is_none_or(|map|map.len()!=4||keys.iter().any(|key|map.get(*key).and_then(Value::as_str).is_none_or(|s|s.len()>1024))){return Err("示波器设置无效".into());}
    invoke_legacy("ui.test_tool.scope_instrument_safe".into(),json!({"project":project,"device_id":device_id,"resource":resource,"expected":expected})).await
}
#[tauri::command]
async fn scope_preflight_safe(project:String,port:String,baud:String,triggers:String)->Result<Value,String>{
    if !valid_identifier(&project)||port.len()>256||baud.len()>32||triggers.len()>1_000_000{return Err("预检参数无效".into());}
    invoke_legacy("ui.test_tool.scope_preflight_safe".into(),json!({"project":project,"port":port,"baud":baud,"triggers":triggers})).await
}
#[tauri::command]
async fn scope_run_safe(project:String,port:String,baud:String,triggers:String)->Result<Value,String>{
    if !valid_identifier(&project)||port.len()>256||baud.len()>32||triggers.len()>1_000_000{return Err("运行参数无效".into());}
    invoke_legacy("ui.test_tool.scope_run_safe".into(),json!({"project":project,"port":port,"baud":baud,"triggers":triggers})).await
}
#[tauri::command]
async fn scope_stop_safe(running_token:String)->Result<Value,String>{
    if running_token.len()!=36||!running_token.bytes().all(|b|b.is_ascii_hexdigit()||b==b'-'){return Err("任务标识无效".into());}
    invoke_legacy("ui.test_tool.scope_stop_safe".into(),json!({"running_token":running_token})).await
}
#[tauri::command]
async fn project_list_safe()->Result<Value,String>{invoke_legacy("ui.project.list_safe".into(),json!({})).await}
#[tauri::command]
async fn project_select_safe(id:String)->Result<Value,String>{
    if !valid_identifier(&id){return Err("项目标识无效".into());}
    invoke_legacy("ui.project.select_safe".into(),json!({"id":id})).await
}
#[tauri::command]
async fn project_create_safe(id:String,name:String)->Result<Value,String>{
    if !valid_identifier(&id)||name.trim().is_empty()||name.len()>512||name.chars().any(char::is_control){return Err("项目名称或标识无效".into());}
    invoke_legacy("ui.project.create_safe".into(),json!({"id":id,"name":name})).await
}
#[tauri::command]
async fn project_remove_test_safe(project:String,id:String)->Result<Value,String>{
    if !removable_test_id(&project,&id){return Err("工程或测试项标识无效".into());}
    invoke_legacy("ui.project.remove_test_safe".into(),json!({"project":project,"id":id})).await
}
#[tauri::command]
async fn project_add_test_safe(project:String,plugin_id:String)->Result<Value,String>{
    if !removable_test_id(&project,&plugin_id){return Err("工程或测试标识无效".into());}
    invoke_legacy("ui.project.add_test_safe".into(),json!({"project":project,"plugin_id":plugin_id})).await
}

#[tauri::command]
async fn fpga_preflight_safe(project:String,selected_ids:Vec<String>,identity:Value,execution_mode:String)->Result<Value,String>{
    validate_fpga_preflight(&project,&selected_ids,&identity,&execution_mode)?;
    invoke_legacy("ui.test_tool.preflight_safe".into(),json!({"project":project,"selected_ids":selected_ids,"identity":identity,"execution_mode":execution_mode})).await
}
fn validate_fpga_preflight(project:&str,selected_ids:&[String],identity:&Value,execution_mode:&str)->Result<(),String>{
    if !valid_identifier(&project)||selected_ids.is_empty()||selected_ids.len()>500||selected_ids.iter().any(|id|!valid_identifier(id))||selected_ids.iter().collect::<std::collections::HashSet<_>>().len()!=selected_ids.len()||!["hardware","simulation"].contains(&execution_mode){return Err("预检参数无效".into());}
    let Some(fields)=identity.as_object() else{return Err("DUT 身份无效".into())};let expected=["dut_serial","board_revision","fpga_model","fixture_id","station_id","operator"];
    if fields.len()!=expected.len()||fields.iter().any(|(key,value)|!expected.contains(&key.as_str())||!value.as_str().is_some_and(|s|s.len()<=512&&!s.chars().any(char::is_control))){return Err("DUT 身份无效".into());}
    Ok(())
}
#[tauri::command]
async fn fpga_run_safe(project:String,selected_ids:Vec<String>,identity:Value,execution_mode:String,manifest_sha256:String,preflight_revision:String)->Result<Value,String>{
    validate_fpga_preflight(&project,&selected_ids,&identity,&execution_mode)?;
    for hash in [&manifest_sha256,&preflight_revision]{if hash.len()!=64||!hash.bytes().all(|b|b.is_ascii_hexdigit()){return Err("预检记录无效，请重新预检".into());}}
    invoke_legacy("ui.test_tool.run_safe".into(),json!({"project":project,"selected_ids":selected_ids,"identity":identity,"execution_mode":execution_mode,"manifest_sha256":manifest_sha256,"preflight_revision":preflight_revision})).await
}

fn wireless_request(project:&str,selected_ids:&[String],mode:&str,log_file:&str,dut_serial:&str,firmware_version:&str)->Result<Value,String>{
    if project.is_empty()||project.len()>128||selected_ids.is_empty()||selected_ids.len()>500||selected_ids.iter().any(|id|id.is_empty()||id.len()>64||!id.bytes().all(|b|b.is_ascii_alphanumeric()||b"._-".contains(&b)))||!matches!(mode,"review"|"log_analysis"|"rx_read"|"rx_control"|"rx_load")||[log_file,dut_serial,firmware_version].iter().any(|s|s.len()>1024||s.chars().any(char::is_control)){return Err("无线测试参数无效".into());}
    let unique:std::collections::HashSet<_>=selected_ids.iter().collect();if unique.len()!=selected_ids.len(){return Err("测试项重复".into());}
    Ok(json!({"project":project,"selected_ids":selected_ids,"mode":mode,"log_file":log_file,"dut_serial":dut_serial,"firmware_version":firmware_version}))
}
#[tauri::command]
async fn wireless_preflight_safe(project:String,selected_ids:Vec<String>,mode:String,log_file:String,dut_serial:String,firmware_version:String)->Result<Value,String>{
    let params=wireless_request(&project,&selected_ids,&mode,&log_file,&dut_serial,&firmware_version)?;
    invoke_legacy("ui.test_tool.wireless_preflight_safe".into(),params).await
}

#[tauri::command]
async fn import_wireless_log(project:String)->Result<Option<String>,String>{
    if !valid_identifier(&project){return Err("项目标识无效".into());}
    tauri::async_runtime::spawn_blocking(move||{
        use std::io::Read;
        let context=||->Result<std::path::PathBuf,String>{
            let state=invoke_legacy_at(ENDPOINT,"ui.state",json!({}))?;
            let tool=&state["test_tool"];
            if tool["project"].as_str()!=Some(project.as_str())||tool["selected"].as_str()!=Some("wireless-whitebox")||tool["running"].as_bool()!=Some(false){return Err("项目或测试状态已变化，请刷新后重试".into());}
            let root=tool["data_root_resolved"].as_str().filter(|s|!s.is_empty()).ok_or("需要更新现有 WiParse 后端")?;
            std::fs::canonicalize(root).map_err(|e|format!("无法访问数据目录：{e}"))
        };
        let root=context()?;
        let Some(source)=rfd::FileDialog::new().add_filter("日志文件",&["txt","log"]).pick_file() else{return Ok(None)};
        let file=std::fs::File::open(&source).map_err(|e|format!("无法打开日志：{e}"))?;
        if !file.metadata().map_err(|e|e.to_string())?.is_file(){return Err("请选择普通日志文件".into());}
        let mut bytes=Vec::new();file.take(16*1024*1024+1).read_to_end(&mut bytes).map_err(|e|format!("日志读取失败：{e}"))?;
        if bytes.len()>16*1024*1024{return Err("日志超过 16 MiB".into());}
        if context()?!=root{return Err("数据目录已变化，请重新选择日志".into());}
        Ok(Some(save_wireless_log(&root,&bytes)?))
    }).await.map_err(|_|"日志导入失败".to_owned())?
}
fn save_wireless_log(root:&std::path::Path,bytes:&[u8])->Result<String,String>{
    use std::io::Write;use sha2::{Digest,Sha256};
    if bytes.len()>16*1024*1024{return Err("日志超过 16 MiB".into());}
    let dir=root.join("wireless-log-imports");
    match std::fs::create_dir(&dir){Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>{},Err(e)=>return Err(format!("无法创建导入目录：{e}"))};
    let meta=std::fs::symlink_metadata(&dir).map_err(|e|e.to_string())?;
    if meta.file_type().is_symlink()||!meta.is_dir()||std::fs::canonicalize(&dir).map_err(|e|e.to_string())?.parent()!=Some(root){return Err("导入目录不安全".into());}
    let digest=format!("{:x}",Sha256::digest(bytes));let name=format!("{digest}.txt");let target=dir.join(&name);
    if target.exists(){let meta=std::fs::symlink_metadata(&target).map_err(|e|e.to_string())?;if meta.file_type().is_symlink()||!meta.is_file()||meta.len()!=bytes.len() as u64||std::fs::read(&target).map_err(|e|e.to_string())?!=bytes{return Err("导入目标已存在但内容不同".into());}return Ok(format!("wireless-log-imports/{name}"));}
    let temp=dir.join(format!(".import-{}.tmp",uuid::Uuid::new_v4()));
    let mut output=std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e|e.to_string())?;
    let result=(||->Result<(),String>{output.write_all(bytes).and_then(|_|output.sync_all()).map_err(|e|e.to_string())?;drop(output);commit_file::publish_new(&temp,&target).map_err(|e|format!("无法安全导入日志：{e}"))?;Ok(())})();
    let _=std::fs::remove_file(&temp);result?;
    Ok(format!("wireless-log-imports/{name}"))
}
#[tauri::command]
async fn wireless_run_safe(project:String,selected_ids:Vec<String>,mode:String,log_file:String,dut_serial:String,firmware_version:String,manifest_sha256:String,preflight_revision:String)->Result<Value,String>{
    let mut params=wireless_request(&project,&selected_ids,&mode,&log_file,&dut_serial,&firmware_version)?;
    for hash in [&manifest_sha256,&preflight_revision]{if hash.len()!=64||!hash.bytes().all(|b|b.is_ascii_hexdigit()){return Err("预检记录无效，请重新预检".into());}}
    params["manifest_sha256"]=json!(manifest_sha256);params["preflight_revision"]=json!(preflight_revision);
    invoke_legacy("ui.test_tool.wireless_run_safe".into(),params).await
}
#[tauri::command]
async fn wireless_stop_safe(running_token:String)->Result<Value,String>{
    if uuid::Uuid::parse_str(&running_token).is_err(){return Err("测试任务标识无效".into());}
    invoke_legacy("ui.test_tool.wireless_stop_safe".into(),json!({"running_token":running_token})).await
}
#[tauri::command]
async fn fpga_stop_safe(running_token:String)->Result<Value,String>{
    if uuid::Uuid::parse_str(&running_token).is_err(){return Err("任务标识无效，请刷新状态".into());}
    invoke_legacy("ui.test_tool.stop_safe".into(),json!({"running_token":running_token})).await
}
fn validate_fpga_rule(rule:&Value)->Result<(),String>{
    let fields=rule.as_object().ok_or("判定条件无效")?;
    let id=fields.get("id").and_then(Value::as_str).ok_or("规则标识无效")?;
    if id.is_empty()||id.len()>128||id.chars().any(char::is_control)||!fields.get("required").is_some_and(Value::is_boolean)||!fields.get("create").is_some_and(Value::is_boolean){return Err("规则标识无效".into());}
    let kind=fields.get("type").and_then(Value::as_str).unwrap_or("");
    if fields.get("create").and_then(Value::as_bool)==Some(true)&&!["log_contains","log_regex","log_not_regex"].contains(&kind){return Err("此规则类型尚不支持新建".into());}
    let expected:&[&str]=match kind{"log_contains"=>&["id","type","required","text","create"],"log_regex"|"log_not_regex"=>&["id","type","required","pattern","create"],"waveform_metric_range"|"instrument_measure_range"=>&["id","type","required","minimum","maximum","create"],_=>return Err("此规则类型尚不支持编辑".into())};
    if fields.len()!=expected.len()||fields.keys().any(|key|!expected.contains(&key.as_str())){return Err("规则字段无效".into());}
    if let Some(key)=expected.iter().find(|key|**key=="text"||**key=="pattern"){
        if !fields.get(*key).and_then(Value::as_str).is_some_and(|s|!s.is_empty()&&s.len()<=4096){return Err("匹配内容无效".into());}
    }else{
        let min=fields.get("minimum").and_then(Value::as_f64).ok_or("限值无效")?;let max=fields.get("maximum").and_then(Value::as_f64).ok_or("限值无效")?;
        if !min.is_finite()||!max.is_finite()||min>max{return Err("下限不能大于上限".into());}
    }Ok(())
}

fn removable_test_id(project: &str, id: &str) -> bool {
    let valid=|value:&str| value.len()<=128 && wiparse_core::project::is_valid_id(value);
    valid(project) && valid(id)
}

fn valid_identifier(input: &str) -> bool {
    !input.is_empty() && input.len()<=64 && input.as_bytes()[0].is_ascii_alphanumeric()
        && !input.ends_with('.') && input.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn validate_fpga_patch(config_id: &str, project: &str, revision: u64, sha256: &str, name: &str) -> Result<(), String> {
    if !valid_identifier(config_id) || !valid_identifier(project) { return Err("配置或项目标识无效".into()); }
    if revision==0 || sha256.len()!=64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("配置版本信息无效，请刷新后重试".into()); }
    if name.trim().is_empty() || name.len()>512 || name.chars().any(char::is_control) { return Err("测试名称无效".into()); }
    Ok(())
}

fn validate_lc_fields(fields: &Value) -> Result<(), String> {
    let values=fields.as_object().ok_or("计算参数无效")?;
    let allowed=["inductance", "capacitance", "inductance_unit", "capacitance_unit", "inductor_esr", "capacitor_esr", "inductor_esr_unit", "capacitor_esr_unit"];
    if values.len()!=allowed.len() || values.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("计算参数不完整或包含不支持的字段".into());
    }
    for key in ["inductance", "capacitance"] {
        let input=values.get(key).and_then(Value::as_str).ok_or("参数必须是数字")?;
        if input.len()>64 { return Err("计算参数过长".into()); }
        let number=parse_numeric_literal(input)?;
        if !number.is_finite() || number<=0.0 { return Err("电感和电容必须是有限正数".into()); }
    }
    for key in ["inductor_esr", "capacitor_esr"] {
        let input=values.get(key).and_then(Value::as_str).ok_or("参数必须是数字")?;
        if input.len()>64 { return Err("计算参数过长".into()); }
        let number=parse_numeric_literal(input)?;
        if !number.is_finite() || number<0.0 { return Err("等效串联电阻必须是有限非负数".into()); }
    }
    if !["uH", "mH", "H"].contains(&values.get("inductance_unit").and_then(Value::as_str).unwrap_or("")) {
        return Err("电感单位无效".into());
    }
    if !["pF", "nF", "uF", "mF", "F"].contains(&values.get("capacitance_unit").and_then(Value::as_str).unwrap_or("")) {
        return Err("电容单位无效".into());
    }
    for key in ["inductor_esr_unit", "capacitor_esr_unit"] {
        if !["mΩ", "Ω", "kΩ"].contains(&values.get(key).and_then(Value::as_str).unwrap_or("")) {
            return Err("等效串联电阻单位无效".into());
        }
    }
    Ok(())
}

// Match the legacy calculator's accepted decimal and unsigned hex literals.
fn parse_numeric_literal(input: &str) -> Result<f64, String> {
    let trimmed=input.trim();
    let hexadecimal=trimmed.strip_prefix("0x").or_else(||trimmed.strip_prefix("0X"))
        .or_else(||trimmed.strip_suffix('h')).or_else(||trimmed.strip_suffix('H'));
    let value=if let Some(digits)=hexadecimal {
        let clean=digits.replace('_', "");
        if clean.is_empty() || !clean.bytes().all(|b|b.is_ascii_hexdigit()) { return Err("参数必须是数字".into()); }
        u128::from_str_radix(&clean,16).map_err(|_| "参数必须是数字")? as f64
    } else { trimmed.parse::<f64>().map_err(|_| "参数必须是数字")? };
    if !value.is_finite() { return Err("参数必须是有限数字".into()); }
    Ok(value)
}

static OWNED_AUTH: std::sync::Mutex<Option<(u32,String)>> = std::sync::Mutex::new(None);
static OWNED_HOST: std::sync::Mutex<Option<std::process::Child>> = std::sync::Mutex::new(None);

fn host_executable(current: &std::path::Path) -> Option<std::path::PathBuf> {
    let current_canon = current.canonicalize().unwrap_or_else(|_| current.to_path_buf());
    let mut directories = Vec::new();
    if let Some(parent) = current.parent() { directories.push(parent.to_path_buf()); }
    let mut cursor = current.parent();
    for _ in 0..8 {
        let Some(directory) = cursor else { break };
        directories.push(directory.join("target").join("release"));
        cursor = directory.parent();
    }
    for directory in directories {
        for name in ["WiParse-Host.exe", "wiparse-gui.exe"] {
            let path = directory.join(name);
            if path.is_file() && path.canonicalize().ok().filter(|found| found != &current_canon).is_some() { return Some(path); }
        }
    }
    None
}

fn host_project_root(host: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut cursor = host.parent();
    for _ in 0..10 {
        let Some(directory) = cursor else { break };
        if directory.join("test-tools").is_dir() && (directory.join("config.json").is_file() || directory.join("config.default.json").is_file() || directory.join("Cargo.toml").is_file()) {
            return Some(directory.to_path_buf());
        }
        cursor = directory.parent();
    }
    None
}

const REQUIRED_HOST_FEATURES:&[&str]=&["companion-graceful-exit-v1","typed-control-v1","serial-ai-v1","config-document-edit-v1","project-transfer-v1","live-source-identity-v1","scope-rules-document-v1","ui-navigation-v1","serial-log-editor-v1","serial-recording-settings-v1","serial-log-lifecycle-v1","instrument-acquisition-v1","manual-connect-v1","document-origin-bindings-v1","preferences-v1","migration-complete-api-v1"];

fn host_identity(reply: &Value) -> Result<u32, String> {
    let data = &reply["data"];
    if reply["ok"]==true&&data["product_id"]=="com.wiparse.host"&&data["control_auth_required"]==false {
        return Err("当前端口由独立旧界面兼容模式占用。请正常关闭旧界面后，通过新界面启动受保护的设备宿主。".into());
    }
    if data["product_id"]=="com.wiparse.host"&&data["recovery_mode"]==true{
        return Err("已有恢复中的设备宿主窗口。请先在该窗口完成或停止任务、保存编辑并正常关闭，然后刷新新界面。".into());
    }
    let valid = reply["ok"] == true && data["product_id"] == "com.wiparse.host"
        && data["bridge_protocol"] == 2 && data["control_auth_required"] == true
        && data["instance_id"].as_str().is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        && data["features"].as_array().is_some_and(|features| REQUIRED_HOST_FEATURES.iter().all(|required|features.iter().any(|feature|feature.as_str()==Some(*required))));
    let pid = data["process_id"].as_u64().and_then(|id| u32::try_from(id).ok()).filter(|id| *id > 0);
    if !valid || pid.is_none() { return Err("7878 端口上的服务不是兼容的 WiParse 设备宿主，请更新匹配的 WiParse-Host.exe；没有发送设备命令。".into()); }
    Ok(pid.unwrap())
}

fn probe_host() -> Result<Option<u32>, String> {
    probe_host_at("127.0.0.1:7878".parse().unwrap())
}
fn probe_host_at(address:std::net::SocketAddr) -> Result<Option<u32>, String> {
    // Windows can take about two seconds to report WSAECONNREFUSED even on
    // loopback. A 400 ms timeout mistakes an unused port for an unknown host
    // and prevents every cold launch. Only explicit refusal permits spawning.
    match std::net::TcpStream::connect_timeout(&address, Duration::from_secs(3)) {
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => return Ok(None),
        Err(_) => return Err("无法确认设备服务是否运行，没有再次启动宿主。".into()),
        Ok(_) => {}
    }
    let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_millis(400)).timeout_read(Duration::from_millis(800)).build();
    let response = agent.get(&format!("http://{address}/v1/health")).call().map_err(|_| "设备服务身份检查失败，没有再次启动宿主。".to_string())?;
    let reply = read_bounded_bridge_json(response.into_reader(), 64 * 1024)?;
    host_identity(&reply).map(Some)
}

fn stop_owned_host() {
    // Dropping Child does not kill it. The companion watches this process and
    // closes itself through the ordinary GUI shutdown path when idle, or
    // becomes visible while active work still needs recovery.
    let _ = OWNED_HOST.lock().unwrap_or_else(|error| error.into_inner()).take();
}

fn wait_for_host(child: &mut std::process::Child) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait().map_err(|error| error.to_string())?.is_some() {
            return Err("设备宿主已退出，请检查宿主启动日志。".into());
        }
        if let Some(pid) = probe_host()? {
            return if pid == child.id() { Ok(()) } else { Err("端口已被另一个宿主接管；没有向该实例发送设备命令。".into()) };
        }
        let now = std::time::Instant::now();
        if now >= deadline { return Err("设备服务未在启动时限内就绪，进程已保留以避免中断正在初始化的任务；不会再次启动。".into()); }
        std::thread::sleep((deadline - now).min(Duration::from_millis(250)));
    }
}

fn ensure_owned_host() -> Result<Value, String> {
    let mut slot = OWNED_HOST.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(pid) = probe_host()? {
        if let Some(child) = slot.as_mut() {
            if child.try_wait().map_err(|error| error.to_string())?.is_none() && child.id() != pid {
                return Err("当前服务实例与本窗口启动的宿主不一致，请检查后再连接。".into());
            }
        }
        return Ok(json!({"started": false, "process_id": pid}));
    }
    if let Some(child) = slot.as_mut() {
        if child.try_wait().map_err(|error| error.to_string())?.is_none() {
            wait_for_host(child)?;
            return Ok(json!({"started": true, "process_id": child.id()}));
        }
    }
    if let Some(mut previous) = slot.take() { let _ = previous.wait(); }
    let current = std::env::current_exe().map_err(|error| error.to_string())?;
    let host = host_executable(&current).ok_or("找不到匹配的 WiParse-Host.exe")?;
    let mut command = std::process::Command::new(&host);
    command.arg("--companion").arg("--companion-parent").arg(std::process::id().to_string());
    let control_token=uuid::Uuid::new_v4().to_string();
    command.env("WIPARSE_LOCAL_API_TOKEN",&control_token);
    if let Some(root) = host_project_root(&host) {
        command.current_dir(&root);
        if std::env::var("WIPARSE_PROJECT_ROOT").ok().map(|value| value.trim().is_empty()).unwrap_or(true) {
            command.env("WIPARSE_PROJECT_ROOT", &root);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    *slot = Some(command.spawn().map_err(|error| format!("无法启动设备服务：{error}"))?);
    let child = slot.as_mut().unwrap();
    *OWNED_AUTH.lock().unwrap_or_else(|error|error.into_inner())=Some((child.id(),control_token));
    wait_for_host(child)?;
    Ok(json!({"started": true, "process_id": child.id()}))
}

#[tauri::command]
async fn host_ensure() -> Result<Value, String> {
    runtime_update::recovery_required()?;
    tauri::async_runtime::spawn_blocking(ensure_owned_host).await.map_err(|error| format!("设备服务启动被中断：{error}"))?
}

fn main() {
    resource_policy::init();
    tauri::Builder::default().manage(serial_events::SerialEvents::default()).manage(runtime_update::UpdateState::default()).manage(probe_firmware::ProbeFirmwareState::default()).manage(offline_browser::OfflineBrowserState::default()).manage(log_browser::LogBrowserState::default()).manage(wave_view::WaveState::default()).manage(report_view::ReportState::default()).manage(data_view::DataState::default()).invoke_handler(tauri::generate_handler![serial_events::serial_events_start,serial_events::serial_events_stop,resource_policy::resource_policy_get,resource_policy::resource_policy_save,runtime_update::runtime_update_status,runtime_update::runtime_update_check,runtime_update::runtime_update_prepare,runtime_update::runtime_update_apply,application_info,legacy_read,host_ensure,scope_rules_replace_safe,navigation_ack_safe,data_preferences_save,serial_edit_safe,confirm_unsaved_exit,serial_explain_line,serial_new_log_safe,offline_browser::offline_browser_pick,offline_browser::offline_browser_status,offline_browser::offline_browser_list,log_browser::serial_browser_pick,log_browser::serial_browser_status,log_browser::serial_browser_list,log_browser::serial_browser_open_file,serial_recording_settings_safe,serial_recording_directory,preferences_save,instrument_acquisition,instrument_export_samples,hub_run_control,serial_select_log_safe,hub_files::hub_file_transfer,hub_document_save,probe_firmware::probe_select_firmware,probe_firmware::probe_execute_safe,instrument_log_clear,instrument_control_safe,serial_ai_action,ai_settings_set_safe,ai_test_safe,ai_chat_submit_safe,calculate_lc,calculate_electrical,calculate_crc,calculate_converter,ai_page_summary,market_action,market_configure,hub_config_batch,patch_fpga_item,patch_wireless_limits,patch_wireless_station,patch_fpga_log,patch_fpga_wave,delete_fpga_wave,patch_fpga_rule,delete_fpga_rule,generic_plugin_run,generic_plugin_stop,generic_select_path,clone_fpga_item,create_fpga_item,select_fpga_firmware,patch_fpga_project_dut,patch_fpga_project_resource,patch_fpga_project_serial,patch_fpga_project_dongle_uart,patch_fpga_reset,serial_start_safe,serial_stop_safe,serial_send_safe,serial_recording_safe,serial_open_log_safe,serial_close_log_safe,serial_clear_live_safe,instrument_scan_safe,instrument_connect_safe,instrument_connect_manual_safe,instrument_disconnect_safe,instrument_measure_safe,instrument_scope_action_safe,instrument_source_action_safe,instrument_load_action_safe,instrument_capture_safe,instrument_waveform_source_safe,instrument_screenshot_safe,save_scope_screenshot,select_plugin_safe,scope_rule_enabled_safe,scope_rule_add_safe,scope_rule_text_safe,scope_rule_exclusion_safe,scope_rule_order_safe,scope_settings_export,scope_settings_import,scope_edge_mode_safe,scope_serial_settings_safe,scope_instrument_safe,scope_preflight_safe,scope_run_safe,scope_stop_safe,project_list_safe,project_select_safe,project_create_safe,project_remove_test_safe,project_add_test_safe,fpga_preflight_safe,fpga_run_safe,fpga_stop_safe,wireless_preflight_safe,wireless_run_safe,wireless_stop_safe,import_wireless_log,wave_view::wave_open,wave_view::wave_add_files,wave_view::wave_remove_file,wave_view::wave_open_indexed,wave_view::wave_close,wave_view::wave_status,wave_view::wave_view,wave_view::wave_measure,wave_view::wave_lane_remove,wave_view::wave_protocol_report,wave_view::wave_analysis_context,wave_view::wave_decode,wave_view::wave_explain,wave_view::wave_overlay,wave_view::wave_sample,wave_view::wave_export_csv,wave_view::wave_export_file,report_view::report_analysis_context,report_view::report_open,report_view::report_back,report_view::report_open_linked,report_view::report_open_external,report_view::report_show_linked,wave_view::wave_open_linked,report_view::report_open_indexed,report_view::report_show,report_view::report_close,report_view::report_status,report_view::report_image,data_view::data_save_wida,data_view::data_open,data_view::data_close,data_view::data_status,data_view::data_extract,data_view::data_view_range,data_view::data_export_csv,data_view::data_live_reset,data_view::data_live_push,data_view::data_live_range])
        .build(tauri::generate_context!()).expect("无法启动 WiParse Next")
        .run(|_app, event| { if matches!(event, tauri::RunEvent::Exit) { stop_owned_host(); } });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::{Read,Write},net::TcpListener,thread};

    #[test]
    fn unused_loopback_port_allows_cold_host_start_on_windows() {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();drop(listener);
        assert_eq!(probe_host_at(address).unwrap(),None);
    }

    #[test]
    fn host_identity_rejects_old_wrong_or_incomplete_services() {
        let good = json!({"ok":true,"data":{"product_id":"com.wiparse.host","bridge_protocol":2,"control_auth_required":true,"instance_id":uuid::Uuid::new_v4().to_string(),"process_id":42,"features":REQUIRED_HOST_FEATURES}});
        assert_eq!(host_identity(&good).unwrap(),42);
        let mut recovering=good.clone();recovering["data"]["recovery_mode"]=json!(true);assert!(host_identity(&recovering).is_err());
        for field in ["product_id","bridge_protocol","instance_id","process_id","features"] {
            let mut invalid=good.clone();invalid["data"][field]=Value::Null;
            assert!(host_identity(&invalid).is_err(),"{field}");
        }
        assert!(host_identity(&json!({"ok":true,"data":{"version":"1.2.2"}})).is_err());
    }

    #[test]
    fn host_lookup_uses_the_packaged_sibling_and_the_workspace_binary() {
        let root = std::env::temp_dir().join(format!("wiparse-host-{}", uuid::Uuid::new_v4()));
        let bin = root.join("bin");
        let release = root.join("target").join("release");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&release).unwrap();
        std::fs::create_dir(root.join("test-tools")).unwrap();
        std::fs::write(root.join("config.default.json"), b"{}").unwrap();
        let packaged = bin.join("WiParse-Host.exe");
        let dev = release.join("wiparse-gui.exe");
        let launcher = bin.join("WiParse.exe");
        std::fs::write(&packaged, b"host").unwrap();
        std::fs::write(&dev, b"dev").unwrap();
        std::fs::write(&launcher, b"next").unwrap();
        let next = bin.join("WiParse-Next.exe");
        std::fs::write(&next, b"old").unwrap();
        assert_eq!(host_executable(&launcher).unwrap().canonicalize().unwrap(), packaged.canonicalize().unwrap());
        assert_eq!(host_executable(&next).unwrap().canonicalize().unwrap(), packaged.canonicalize().unwrap());
        assert_eq!(host_project_root(&packaged).unwrap(), root);
        std::fs::remove_file(&packaged).unwrap();
        let nested = root.join("apps").join("wiparse-next").join("src-tauri").join("target").join("release").join("wiparse-next.exe");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        assert_eq!(host_executable(&nested).unwrap().canonicalize().unwrap(), dev.canonicalize().unwrap());
        assert_eq!(host_project_root(&dev).unwrap(), root);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removable_test_id_matches_project_identifiers(){
        assert!(removable_test_id("lot1","example-smoke"));
        assert!(!removable_test_id("","example-smoke"));
        assert!(!removable_test_id("lot1","../other"));
        assert!(!removable_test_id("lot1",&"a".repeat(129)));
    }

    #[test]
    fn bridge_response_reader_rejects_oversized_and_invalid_json(){
        assert_eq!(read_bounded_bridge_json(&b"{\"ok\":true}"[..],11).unwrap()["ok"],true);
        assert!(read_bounded_bridge_json(&b"{\"ok\":true}"[..],10).unwrap_err().contains("超过大小限制"));
        assert!(read_bounded_bridge_json(&b"not json"[..],20).unwrap_err().contains("无效数据"));
    }

    #[test]
    fn scope_settings_export_creates_new_file_without_overwriting(){
        let root=std::env::temp_dir().join(format!("wiparse-scope-settings-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&root).unwrap();
        let file=root.join("settings.json");let values=json!({"port":"COM3","baud":"115200","triggers":"[{\"id\":\"x\",\"type\":\"contains\",\"pattern\":\"READY\"}]","scope_resource":"USB::SCOPE","scope_model":"MDO3014","scope_kind":"oscilloscope","rising_edge":"true"});
        write_scope_settings_new(&file,&values).unwrap();let original=std::fs::read(&file).unwrap();let parsed:Value=serde_json::from_slice(&original).unwrap();assert_eq!(parsed["values"],values);
        assert!(write_scope_settings_new(&file,&values).is_err());assert_eq!(std::fs::read(&file).unwrap(),original);
        assert!(validate_scope_settings(&json!({"port":"COM3","baud":"0","triggers":"[]","scope_resource":"","scope_model":"","scope_kind":"oscilloscope","rising_edge":"true"})).is_err());
        std::fs::remove_file(file).unwrap();std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn screenshot_export_preserves_bytes_and_never_replaces_existing_file(){
        let root=std::env::temp_dir().join(format!("wiparse-screenshot-test-{}",uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path=root.join("capture.png");
        let bytes=b"\x89PNG\r\n\x1a\nexample";
        save_png_new(&path,bytes).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(),bytes);
        assert!(save_png_new(&path,b"changed").is_err());
        assert_eq!(std::fs::read(&path).unwrap(),bytes);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(),1);
        std::fs::remove_file(&path).unwrap();std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn wireless_input_excludes_unimplemented_modes_and_invalid_selection(){
        assert!(wireless_request("default",&["wcv_07".into()],"rx_read","","SN","v1").is_ok());
        assert!(wireless_request("default",&["wcv_07".into()],"live","","SN","v1").is_err());
        assert!(wireless_request("default",&["wcv_07".into(),"wcv_07".into()],"review","","SN","v1").is_err());
    }

    #[test]
    fn wireless_log_import_is_bounded_idempotent_and_never_replaces_data(){
        let temp=std::env::temp_dir().join(format!("wiparse-log-import-test-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&temp).unwrap();
        let root=std::fs::canonicalize(&temp).unwrap();let bytes=b"TX log\nfirst line\n";
        let relative=save_wireless_log(&root,bytes).unwrap();assert_eq!(save_wireless_log(&root,bytes).unwrap(),relative);
        let path=root.join(relative);assert_eq!(std::fs::read(&path).unwrap(),bytes);
        std::fs::write(&path,b"different contents").unwrap();assert!(save_wireless_log(&root,bytes).is_err());
        assert_eq!(std::fs::read(&path).unwrap(),b"different contents");assert!(save_wireless_log(&root,&vec![0;16*1024*1024+1]).is_err());
        std::fs::remove_file(path).unwrap();std::fs::remove_dir(root.join("wireless-log-imports")).unwrap();std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn optional_supplied_tx_log_import_preserves_all_bytes(){
        let Some(sample)=std::env::var_os("WIPARSE_ANALYSIS_SAMPLE") else{return};
        let bytes=std::fs::read(sample).unwrap();assert!(!bytes.is_empty());
        let temp=std::env::temp_dir().join(format!("wiparse-real-log-test-{}",uuid::Uuid::new_v4()));std::fs::create_dir(&temp).unwrap();let root=std::fs::canonicalize(&temp).unwrap();
        let path=root.join(save_wireless_log(&root,&bytes).unwrap());assert_eq!(std::fs::read(&path).unwrap(),bytes);
        std::fs::remove_file(path).unwrap();std::fs::remove_dir(root.join("wireless-log-imports")).unwrap();std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn converter_accepts_only_bounded_explicit_fields() {
        assert!(validate_converter_fields(&json!({"tab":"radix","radix_input":"255","source_base":10,"target_base":16})).is_ok());
        assert!(validate_converter_fields(&json!({"tab":"radix","radix_input":"255","source_base":1,"target_base":16})).is_err());
        assert!(validate_converter_fields(&json!({"tab":"scientific","expression":"sin(90)","angle_mode":"degrees"})).is_ok());
        assert!(validate_converter_fields(&json!({"tab":"scientific","expression":"x".repeat(257),"angle_mode":"degrees"})).is_err());
        assert!(validate_converter_fields(&json!({"tab":"ascii","byte_input":"Hi","byte_direction":"text_to_bytes","byte_base":"hex","method":"device.write"})).is_err());
    }

    #[test]
    fn only_explicit_read_methods_are_allowed() {
        assert!(validate_request("log.lines.get", &json!({"limit": 500})).is_ok());
        assert!(validate_request("instrument.command", &json!({})).is_err());
        assert!(validate_request("ui.test_tool.run.control", &json!({})).is_err());
        assert!(validate_request("log.lines.get", &json!([])).is_err());
    }
    #[test]
    fn serial_controls_reject_arbitrary_resources(){
        assert!(valid_serial_port("COM3"));assert!(valid_serial_port("winusb://wiparse/uart?serial=ABC"));assert!(valid_serial_port("tcp://192.168.1.2:1234"));
        assert!(!valid_serial_port("COM0"));assert!(!valid_serial_port("file:///secret"));assert!(!valid_serial_port("tcp://host/path:1234"));
    }
    #[test]
    fn legacy_error_retains_specific_code_or_message(){
        assert_eq!(legacy_error(&json!({"error":{"code":"ERROR","message":"SERIAL.TEST_IN_PROGRESS"}})),"SERIAL.TEST_IN_PROGRESS");
        assert_eq!(legacy_error(&json!({"error":{"code":"CONFIG.REVISION_CONFLICT","message":"changed"}})),"CONFIG.REVISION_CONFLICT");
    }

    #[test]
    fn lc_validation_excludes_arbitrary_commands_and_bad_numbers() {
        let fields=|esr:&str,unit:&str| json!({"inductance":"10","capacitance":"100","inductance_unit":"uH","capacitance_unit":"nF","inductor_esr":esr,"capacitor_esr":"0","inductor_esr_unit":unit,"capacitor_esr_unit":"mΩ"});
        assert!(validate_lc_fields(&fields("50","mΩ")).is_ok());
        assert!(validate_lc_fields(&json!({"inductance":"0x10","capacitance":"FFH","inductance_unit":"uH","capacitance_unit":"nF","inductor_esr":"0","capacitor_esr":"1","inductor_esr_unit":"kΩ","capacitor_esr_unit":"Ω"})).is_ok());
        assert!(validate_lc_fields(&json!({"inductance":"-1","capacitance":"100","inductance_unit":"uH","capacitance_unit":"nF","inductor_esr":"50","capacitor_esr":"0","inductor_esr_unit":"mΩ","capacitor_esr_unit":"mΩ"})).is_err());
        assert!(validate_lc_fields(&fields("-1","mΩ")).is_err());
        assert!(validate_lc_fields(&fields("50","MΩ")).is_err());
        assert!(validate_lc_fields(&json!({"inductance":"10","capacitance":"100","inductance_unit":"uH","capacitance_unit":"nF","inductor_esr":"50","capacitor_esr":"0","inductor_esr_unit":"mΩ","capacitor_esr_unit":"mΩ","command":"erase"})).is_err());
    }

    #[test]
    fn fpga_patch_requires_project_and_snapshot() {
        let hash="a".repeat(64);
        assert!(validate_fpga_patch("fpga_01", "default", 1, &hash, "测试名称").is_ok());
        assert!(validate_fpga_patch("../escape", "default", 1, &hash, "测试名称").is_err());
        assert!(validate_fpga_patch("fpga_01", "default", 0, &hash, "测试名称").is_err());
        assert!(validate_fpga_patch("fpga_01", "default", 1, "bad", "测试名称").is_err());
        assert!(validate_fpga_patch("fpga_01", "default", 1, &hash, "\n").is_err());
    }

    #[test]
    fn fpga_rules_reject_unrelated_fields_and_inverted_limits() {
        assert!(validate_fpga_rule(&json!({"id":"r1","type":"log_contains","required":true,"text":"OK","create":true})).is_ok());
        assert!(validate_fpga_rule(&json!({"id":"r1","type":"log_contains","required":true,"text":"OK","create":true,"device":"COM3"})).is_err());
        assert!(validate_fpga_rule(&json!({"id":"r2","type":"waveform_metric_range","required":true,"minimum":0,"maximum":10,"create":false})).is_ok());
        assert!(validate_fpga_rule(&json!({"id":"r2","type":"waveform_metric_range","required":true,"minimum":10,"maximum":0,"create":false})).is_err());
    }

    #[test]
    fn electrical_calculator_validates_operation_and_all_fields() {
        assert!(validate_electrical_fields("q",&json!({"v1":"10","v2":"8","bias":"0","intervals":"1"})).is_ok());
        assert!(validate_electrical_fields("erase",&json!({})).is_err());
        assert!(validate_electrical_fields("q",&json!({"v1":"NaN","v2":"8","bias":"0","intervals":"1"})).is_err());
        assert!(validate_electrical_fields("q",&json!({"v1":"10","v2":"8","bias":"0","unexpected":"1"})).is_err());
    }
    #[test]
    fn crc_rejects_unrecognised_settings(){
        let mut fields=json!({"data":"123456789","preset":"CRC-32/ISO-HDLC","input_mode":"text","custom_width":16,"custom_poly":"0x1021","custom_init":"0xFFFF","custom_xorout":"0x0000","custom_refin":false,"custom_refout":false});
        assert!(validate_crc_fields(&fields).is_ok());fields["preset"]=json!("unknown");assert!(validate_crc_fields(&fields).is_err());
    }

    #[test]
    fn legacy_response_is_interpreted_without_fabricating_a_result() {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();
        let address=listener.local_addr().unwrap();
        let worker=thread::spawn(move || {
            let (mut stream,_)=listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut buffer=[0u8;4096];
            let mut request=Vec::new();
            loop {
                let read=stream.read(&mut buffer).unwrap();
                assert!(read>0);
                request.extend_from_slice(&buffer[..read]);
                if let Some(end)=request.windows(4).position(|part| part==b"\r\n\r\n") {
                    let headers=String::from_utf8_lossy(&request[..end]);
                    let length=headers.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:").and_then(|value| value.trim().parse::<usize>().ok())).unwrap_or(0);
                    if request.len()>=end+4+length {break;}
                }
                assert!(request.len()<65536);
            }
            let request=String::from_utf8_lossy(&request);
            assert!(request.starts_with("POST /v1/invoke HTTP/1.1"));
            let body=r#"{"ok":false,"error":{"code":"CONFIG.REVISION_CONFLICT","message":"changed"}}"#;
            write!(stream,"HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        });
        let error=invoke_legacy_at(&format!("http://{address}/v1/invoke"),"ui.calc.get",json!({})).unwrap_err();
        assert_eq!(error,"CONFIG.REVISION_CONFLICT");
        worker.join().unwrap();
    }
}
