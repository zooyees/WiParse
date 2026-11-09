import { invoke } from '@tauri-apps/api/core';

export async function saveAiSettings(input:{revision:string;fields:Record<string,unknown>;newKey:string;clearKey:boolean}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('ai_settings_set_safe',input);
}
export async function testAiConnection(revision:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('ai_test_safe',{revision});
}
export async function submitAiPrompt(revision:string,text:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('ai_chat_submit_safe',{revision,text,consent:true});
}
export async function sourceAction(input:{deviceId:number;resource:string;channel:number;action:'voltage'|'current'|'ovp'|'ocp'|'output';value:number|null;enabled:boolean|null}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_source_action_safe',input);
}
export async function loadAction(input:{deviceId:number;resource:string;action:'setpoint'|'input';mode:string|null;value:number|null;enabled:boolean|null}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_load_action_safe',input);
}
export async function captureWaveformSource(deviceId:number,resource:string):Promise<unknown|null>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_waveform_source_safe',{deviceId,resource});
}
export async function openSerialLog():Promise<unknown|null>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('serial_open_log_safe');
}
export async function closeSerialLog(tabId:number,viewId:number):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('serial_close_log_safe',{tabId,viewId});
}

export async function clearLiveSerialLog(viewId:number,contentEpoch:number):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('serial_clear_live_safe',{viewId,contentEpoch});
}

export async function setScopeRuleEnabled(project:string,expected:string,index:number,enabled:boolean):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_rule_enabled_safe',{project,expected,index,enabled});
}
export async function addScopeRule(project:string,expected:string,pattern:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_rule_add_safe',{project,expected,pattern});
}
export async function setScopeRuleText(project:string,expected:string,index:number,pattern:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_rule_text_safe',{project,expected,index,pattern});
}

export async function changeScopeExclusion(input:{project:string;expected:string;index:number;action:'add'|'remove';text:string|null;key:string|null;itemIndex:number|null}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_rule_exclusion_safe',input);
}
export async function changeScopeRuleOrder(project:string,expected:string,index:number,action:'up'|'down'|'remove'):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_rule_order_safe',{project,expected,index,action});
}
export async function exportScopeSettings(project:string,expected:Record<string,string>):Promise<string|null>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_settings_export',{project,expected});
}
export async function importScopeSettings(project:string,expected:Record<string,string>):Promise<unknown|null>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_settings_import',{project,expected});
}
export async function setScopeEdgeMode(project:string,expected:string,enabled:boolean):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_edge_mode_safe',{project,expected,enabled});
}
export async function setScopeSerialSettings(input:{project:string;expectedPort:string;expectedBaud:string;port:string;baud:number}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_serial_settings_safe',input);
}
export async function selectScopeInstrument(input:{project:string;deviceId:number;resource:string;expected:Record<string,string>}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_instrument_safe',input);
}
export async function preflightScopeSerial(input:{project:string;port:string;baud:string;triggers:string}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_preflight_safe',input);
}
export async function runScopeSerial(input:{project:string;port:string;baud:string;triggers:string}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_run_safe',input);
}
export async function stopScopeSerial(runningToken:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('scope_stop_safe',{runningToken});
}

export async function addProjectTest(project: string, pluginId: string): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('project_add_test_safe', { project, pluginId });
}

export async function removeProjectTest(project: string, id: string): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('project_remove_test_safe', { project, id });
}

export async function readLegacy(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动；浏览器预览没有设备服务连接。');
  return invoke('legacy_read', { method, params });
}

export async function calculateLc(fields: Record<string, string>): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('calculate_lc', { fields });
}

export async function calculateElectrical(card: string, fields: Record<string, string>): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('calculate_electrical', { card, fields });
}

export async function calculateCrc(fields: Record<string, string | number | boolean>): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('calculate_crc', { fields });
}

export async function calculateConverter(fields: Record<string, string | number>): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('calculate_converter', { fields });
}

export async function patchFpgaItem(input: {configId:string; project:string; revision:number; sha256:string; name:string; enabled:boolean}): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_fpga_item', input);
}

export async function patchFpgaLog(input: {configId:string; project:string; revision:number; sha256:string; enabled:boolean; startAtS:number; durationS:number}): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_fpga_log', input);
}

export async function patchFpgaWave(input: {configId:string; project:string; revision:number; sha256:string; captureId:string; atS:number; timeoutS:number; sourceEnabled:boolean; sourceFormat:string;channels:string[];create:boolean}): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_fpga_wave', input);
}

export async function deleteFpgaWave(input: {configId:string; project:string; revision:number; sha256:string; captureId:string}): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('delete_fpga_wave', input);
}

export async function patchFpgaRule(input: {configId:string; project:string; revision:number; sha256:string; rule:Record<string,string|number|boolean>}): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_fpga_rule', input);
}

export async function deleteFpgaRule(input: {configId:string; project:string; revision:number; sha256:string; ruleId:string}): Promise<unknown> {
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('delete_fpga_rule', input);
}

export async function genericPluginRun(input:{project:string;plugin:string;dataRoot:string;expected:Record<string,unknown>;values:Record<string,string>;lifecycle:'preflight'|'run'}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('generic_plugin_run',input);
}

export async function genericPluginStop(plugin:string,runningToken:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('generic_plugin_stop',{plugin,runningToken});
}

export async function genericSelectPath(directory:boolean):Promise<string|null>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('generic_select_path',{directory});
}

export async function cloneFpgaItem(input:{sourceId:string;project:string;revision:number;sha256:string;newId:string;name:string}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('clone_fpga_item',input);
}

export async function createFpgaItem(input:{project:string;configId:string;name:string}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('create_fpga_item',input);
}

export async function selectFpgaFirmware(input:{configId:string;project:string;revision:number;sha256:string}):Promise<unknown|null>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('select_fpga_firmware',input);
}

export async function patchFpgaProjectDut(input:{project:string;revision:number;sha256:string;dut:Record<string,string|boolean>}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_fpga_project_dut',input);
}

export async function patchFpgaProjectResource(input:{project:string;revision:number;sha256:string;role:string;resource:string}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_fpga_project_resource',input);
}

export async function patchFpgaProjectSerial(input:{project:string;revision:number;sha256:string;role:string;port:string;baud:number}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_fpga_project_serial',input);
}

export async function patchFpgaProjectDongleUart(input:{project:string;revision:number;sha256:string;role:string;serial:string;channel:number;baud:number}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_fpga_project_dongle_uart',input);
}

export async function patchFpgaReset(input:{configId:string;project:string;revision:number;sha256:string;projectRevision:number;projectSha256:string;method:string;role:string;channel:number;offTimeMs:number}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_fpga_reset',input);
}

export async function startSerial(port:string,baud:number):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('serial_start_safe',{port,baud});
}
export async function stopSerial(port:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('serial_stop_safe',{port});
}
export async function sendSerial(port:string,bytes:number[]):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('serial_send_safe',{port,bytes});
}
export async function scanInstruments():Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_scan_safe');
}
export async function setSerialRecording(enabled:boolean):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('serial_recording_safe',{enabled});
}
export async function connectInstrument(resource:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_connect_safe',{resource});
}
export async function connectManualInstrument(resource:string,kind:string|null):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_connect_manual_safe',{resource,kind});
}
export async function disconnectInstrument(deviceId:number,resource:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_disconnect_safe',{deviceId,resource});
}
export async function measureInstrument(deviceId:number,resource:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_measure_safe',{deviceId,resource});
}
export async function scopeAction(deviceId:number,resource:string,action:'run'|'stop'|'single'):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_scope_action_safe',{deviceId,resource,action});
}
export async function captureInstrument(deviceId:number,resource:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_capture_safe',{deviceId,resource});
}
export async function readInstrumentScreenshot(deviceId:number,resource:string,jobId:number):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('instrument_screenshot_safe',{deviceId,resource,jobId});
}
export async function saveScopeScreenshot(pngBase64:string):Promise<string|null>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('save_scope_screenshot',{pngBase64});
}
export async function selectPlugin(project:string,plugin:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('select_plugin_safe',{project,plugin});
}
export async function listProjects():Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('project_list_safe');
}
export async function selectProject(id:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('project_select_safe',{id});
}
export async function createProject(id:string,name:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('project_create_safe',{id,name});
}
export async function fpgaPreflight(input:{project:string;selectedIds:string[];identity:Record<string,string>;executionMode:string}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('fpga_preflight_safe',input);
}
export type WirelessRequest={project:string;selectedIds:string[];mode:string;logFile:string;dutSerial:string;firmwareVersion:string};
export async function patchWirelessLimits(input:{project:string;revision:number;sha256:string;limits:Record<string,unknown>}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_wireless_limits',input);
}
export async function patchWirelessStation(input:{project:string;revision:number;sha256:string;station:Record<string,unknown>}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('patch_wireless_station',input);
}
export async function wirelessPreflight(input:WirelessRequest):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('wireless_preflight_safe',input);
}
export async function importWirelessLog(project:string):Promise<string|null>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('import_wireless_log',{project});
}
export async function wirelessRun(input:WirelessRequest&{manifestSha256:string;preflightRevision:string}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('wireless_run_safe',input);
}
export async function wirelessStop(runningToken:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('wireless_stop_safe',{runningToken});
}
export async function fpgaRun(input:{project:string;selectedIds:string[];identity:Record<string,string>;executionMode:string;manifestSha256:string;preflightRevision:string}):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('fpga_run_safe',input);
}
export async function fpgaStop(runningToken:string):Promise<unknown>{
  if (!('__TAURI_INTERNALS__' in window)) throw new Error('请通过桌面应用启动。');
  return invoke('fpga_stop_safe',{runningToken});
}
