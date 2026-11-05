//! SCP single-wire NRZ, MSB first, odd parity, quarter-UI separators, CRC-8 0x39.
//! The default UI is 160 us; captured PINGs and separators track transmitter timing.
use super::charging::{append, byte_error, crc8, finish};
use super::digital::{LogicLevels, LogicWave};
use super::{
    BusByteError, BusByteSpan, BusDecodeResult, BusFrame, BusFrameError,
};
use crate::instrument::WaveformTrace;

#[derive(Debug, Clone)]
pub struct ScpConfig {
    pub ui_us: f64,
}
impl Default for ScpConfig {
    fn default() -> Self {
        Self { ui_us: 160.0 }
    }
}

pub fn decode_scp(
    trace: &WaveformTrace,
    threshold: Option<f64>,
    cfg: &ScpConfig,
) -> BusDecodeResult {
    let mut result = BusDecodeResult::default();
    if !cfg.ui_us.is_finite() || !(120.0..=200.0).contains(&cfg.ui_us) {
        result.error = Some("SCP UI must be 120..200 us".into());
        return result;
    }
    // SCP shares a wire between 1.8 V / 3.3 V transmitters. A midpoint
    // inferred from the largest swing can completely hide the 1.8 V sender.
    // Receiver limits in B.4.1 are VIL=0.9 V and VIH=1.3 V.
    let mid = threshold.filter(|t| t.is_finite()).unwrap_or(1.1);
    let wave = LogicWave::with_levels(trace, LogicLevels { vil: mid - 0.2, vih: mid + 0.2 });
    let edges = wave.edges();
    let mut ui = cfg.ui_us * 1e-6;
    let mut clock_acquired = false;
    let mut packet = Vec::new();
    let mut used = 0;
    let mut follow: Option<u8> = None;
    let mut i = 0;
    while i + 1 < edges.len() && !result.truncated {
        let width = edges[i + 1].time - edges[i].time;
        if edges[i].level_after && (ui * 77.0..=ui * 123.0).contains(&width) {
            follow = emit(&mut result, &mut used, &mut packet, false, follow);
            if !append(&mut result, &mut used, BusFrame {
                t_start: edges[i].time, t_end: edges[i+1].time,
                summary: "RESET".into(), ..Default::default()
            }, vec![]) { break; }
            clock_acquired = false;
            i += 1;
            continue;
        }
        // Acquire transmitter timing from PING before applying per-byte timing.
        // The configured UI is only a fallback when no PING was captured.
        if edges[i].level_after && (16.0 * 120e-6..=16.0 * 200e-6).contains(&width) {
            follow = emit(&mut result, &mut used, &mut packet, true, follow);
            ui = width / 16.0;
            clock_acquired = true;
            if !append(
                &mut result,
                &mut used,
                BusFrame {
                    t_start: edges[i].time,
                    t_end: edges[i + 1].time,
                    summary: "PING".into(),
                    ..Default::default()
                },
                vec![],
            ) {
                break;
            }
            i += 1;
            continue;
        }
        if !(ui * 0.17..=ui * 0.33).contains(&width) {
            i += 1;
            continue;
        }
        let first = i;
        while i + 1 < edges.len()
            && (ui * 0.17..=ui * 0.33).contains(&(edges[i + 1].time - edges[i].time))
        {
            i += 1;
        }
        if i - first < 2 {
            continue;
        }
        let separator_ui = (edges[i].time - edges[first].time) / (i - first) as f64 * 4.0;
        // PING averages timing over 16 UI; quarter-UI edge quantization
        // otherwise accumulates across nine bits and skips the next separator.
        let bit = if clock_acquired && (separator_ui / ui - 1.0).abs() < 0.15 {
            ui
        } else { separator_ui };
        let start = edges[i].time;
        // An end separator is followed by a high 16-UI PING, not another byte.
        if i + 1 < edges.len() && edges[i].level_after && edges[i + 1].time - start > bit * 12.0 {
            continue;
        }
        let end = start + 9.0 * bit;
        if wave.at(start + 8.5 * bit).is_none() {
            break;
        }
        // A following separator must begin at the end of the parity bit. A
        // quarter-UI glitch inside data must not manufacture an apparently valid byte.
        let next_index = edges.partition_point(|e| e.time < end - bit * 0.3);
        let boundary_ok = edges
            .get(next_index)
            .is_some_and(|e| (e.time - end).abs() < bit * 0.3);
        let byte = (0..8).fold(0u8, |v, b| {
            (v << 1) | (wave.at(start + (b as f64 + 0.5) * bit) == Some(true)) as u8
        });
        let parity = wave.at(start + 8.5 * bit) == Some(true);
        let error = if !boundary_ok {
            BusByteError::Framing
        } else if (byte.count_ones() + u32::from(parity)) % 2 != 1 {
            BusByteError::Parity
        } else {
            BusByteError::None
        };
        packet.push(BusByteSpan {
            t_start: start,
            t_end: end,
            byte,
            error,
        });
        if packet.len() >= super::decode_byte_limit() {
            follow = emit(&mut result, &mut used, &mut packet, false, follow);
            result.truncated = true;
            break;
        }
        i = next_index.max(i + 1);
    }
    if !packet.is_empty() && !result.truncated {
        emit(&mut result, &mut used, &mut packet, false, follow);
    }
    finish(&mut result, "SCP");
    result
}

fn emit(
    result: &mut BusDecodeResult,
    used: &mut usize,
    packet: &mut Vec<BusByteSpan>,
    complete: bool,
    follow: Option<u8>,
) -> Option<u8> {
    if packet.is_empty() {
        return follow;
    }
    let mut spans: Vec<BusByteSpan> = std::mem::take(packet);
    let bytes: Vec<u8> = spans.iter().map(|s| s.byte).collect();
    let error =
        if !complete || bytes.len() < 2 || spans.iter().any(|s| s.error == BusByteError::Framing) {
            BusFrameError::Framing
        } else if spans.iter().any(|s| s.error == BusByteError::Parity) {
            BusFrameError::Parity
        } else if crc8(&bytes[..bytes.len() - 1], 0x39) != bytes[bytes.len() - 1] {
            BusFrameError::Checksum
        } else {
            BusFrameError::None
        };
    if error == BusFrameError::Checksum {
        spans.last_mut().unwrap().error = byte_error(error);
    }
    let command = command_name(bytes[0]);
    let addressed = matches!(bytes[0], 0x0b | 0x0c | 0x1b | 0x1c);
    let reg = if addressed {
        bytes.get(1).copied()
    } else if command == "DATA" {
        follow
    } else {
        None
    };
    let data = payload_bytes(&bytes, reg.is_some() && !addressed);
    let mut summary = format!("SCP {command}");
    if let Some(addr) = reg {
        summary.push_str(&format!(" reg=0x{addr:02X}"));
        let name = register_name(addr);
        // 0x2B keeps the historical summary "SCP SBRRD reg=0x2B".
        if name != "寄存器" && addr != 0x2b {
            summary.push(' ');
            summary.push_str(name);
        }
        if let Some(meaning) = value_meaning(addr, data) {
            summary.push(' ');
            summary.push_str(&meaning);
        }
    }
    if !complete {
        summary.push_str(" incomplete");
    }
    let next = if matches!(bytes.first().copied(), Some(0x0c | 0x1c)) {
        bytes.get(1).copied()
    } else {
        None
    };
    append(
        result,
        used,
        BusFrame {
            t_start: spans[0].t_start,
            t_end: spans.last().unwrap().t_end,
            summary,
            bytes,
            error,
        },
        spans,
    );
    next
}

fn command_name(byte: u8) -> &'static str {
    match byte {
        0x0b => "SBRWR",
        0x0c => "SBRRD",
        0x1b => "MBRWR",
        0x1c => "MBRRD",
        0x08 => "ACK",
        0x03 => "NACK",
        _ => "DATA",
    }
}

fn payload_bytes(bytes: &[u8], response: bool) -> &[u8] {
    if bytes.len() < 2 {
        return bytes;
    }
    let body = &bytes[..bytes.len() - 1];
    if response || !matches!(body.first().copied(), Some(0x0b | 0x0c | 0x1b | 0x1c | 0x08 | 0x03)) {
        return body;
    }
    match body[0] {
        0x0b | 0x0c => body.get(2..).unwrap_or(&[]),
        0x1b | 0x1c => body.get(3..).unwrap_or(&[]),
        _ => &[],
    }
}

fn register_name(addr: u8) -> &'static str {
    register_names(addr).0
}

/// Names from the HarmonyOS charger stack (`adapter_protocol_fcp` / `adapter_protocol_scp`).
/// Addresses 0x00–0x3F are the shared discrete-register map; 0x7E and above are SCP Type-B.
fn register_names(addr: u8) -> (&'static str, &'static str) {
    match addr {
        0x00 => ("设备类型", "Device type"),
        0x01 => ("协议版本", "Specification version"),
        0x04 => ("厂商 ID", "Vendor ID"),
        0x20 => ("能力总览", "Capabilities"),
        0x21 => ("离散电压能力", "Discrete voltage capability"),
        0x22 => ("最大功率", "Maximum power"),
        0x28 => ("适配器状态", "Adapter status"),
        0x29 => ("输出电压状态", "Output voltage status"),
        0x2b => ("输出使能", "Output enable"),
        0x2c => ("电压配置", "Voltage setting"),
        0x2d => ("电流配置", "Current setting"),
        0x2f => ("车充辅助", "Car-charger auxiliary"),
        0x30..=0x3f => output_step_names(addr),
        0x54 => ("车充签名", "Car-charger signature"),
        0x7e => ("适配器类型", "Adapter type"),
        0x80 => ("能力检测", "Capability detect"),
        0x82 => ("厂商ID高字节", "Vendor ID high"),
        0x83 => ("厂商ID低字节", "Vendor ID low"),
        0x84 => ("模块ID高字节", "Module ID high"),
        0x85 => ("模块ID低字节", "Module ID low"),
        0x86 => ("序列号高字节", "Serial high"),
        0x87 => ("序列号低字节", "Serial low"),
        0x88 => ("芯片ID", "Chip ID"),
        0x89 => ("硬件版本", "Hardware version"),
        0x8a => ("固件版本高字节", "Firmware high"),
        0x8b => ("固件版本低字节", "Firmware low"),
        0x8d => ("适配器型号", "Adapter model"),
        0x8f => ("功率曲线档数", "Power-curve steps"),
        0xa0..=0xa7 => ("主机随机数", "Host random"),
        0xa8..=0xaf => ("从机随机数", "Slave random"),
        0xb8 => ("输出电压高字节", "Output voltage high"),
        0xb9 => ("输出电压低字节", "Output voltage low"),
        0xb0..=0xbf => ("认证Hash", "Auth hash"),
        0xca => ("输出电压", "Output voltage"),
        0xcf => ("加密状态", "Encryption"),
        0xd0..=0xef => ("功率曲线", "Power curve"),
        _ => ("寄存器", "Register"),
    }
}

fn output_step_names(addr: u8) -> (&'static str, &'static str) {
    const ZH: [&str; 16] = [
        "第0档输出电压", "第1档输出电压", "第2档输出电压", "第3档输出电压",
        "第4档输出电压", "第5档输出电压", "第6档输出电压", "第7档输出电压",
        "第8档输出电压", "第9档输出电压", "第10档输出电压", "第11档输出电压",
        "第12档输出电压", "第13档输出电压", "第14档输出电压", "第15档输出电压",
    ];
    const EN: [&str; 16] = [
        "Output voltage step 0", "Output voltage step 1", "Output voltage step 2", "Output voltage step 3",
        "Output voltage step 4", "Output voltage step 5", "Output voltage step 6", "Output voltage step 7",
        "Output voltage step 8", "Output voltage step 9", "Output voltage step 10", "Output voltage step 11",
        "Output voltage step 12", "Output voltage step 13", "Output voltage step 14", "Output voltage step 15",
    ];
    let index = (addr - 0x30) as usize;
    (ZH[index], EN[index])
}

fn register_phrase(addr: u8, zh: bool) -> String {
    let (zh_name, en_name) = register_names(addr);
    let name = if zh { zh_name } else { en_name };
    let extra = match addr {
        0x21 => if zh { "。1 = 5V/9V，2 = 5V/9V/12V" } else { ". 1 = 5V/9V, 2 = 5V/9V/12V" },
        0x22 => if zh { "。字节 × 500 mW" } else { ". byte × 500 mW" },
        0x28 => if zh { "。bit3 欠压，bit2 过压，bit1 过流，bit0 过温" } else { ". bit3 UVP, bit2 OVP, bit1 OCP, bit0 OTP" },
        0x2b => if zh { "。写入 1 使能输出" } else { ". writing 1 enables the output" },
        0x2c => if zh { "。字节 × 100 mV" } else { ". byte × 100 mV" },
        0x2d => if zh { "。字节 × 100 mA" } else { ". byte × 100 mA" },
        0x30 => if zh { "。字节 × 100 mV。鸿蒙充电代码把这一档当作 5V 档" } else { ". byte × 100 mV. HarmonyOS treats this step as 5 V" },
        0x31 => if zh { "。字节 × 100 mV。鸿蒙充电代码把这一档当作 9V 档" } else { ". byte × 100 mV. HarmonyOS treats this step as 9 V" },
        0x32 => if zh { "。字节 × 100 mV。鸿蒙充电代码把这一档当作 12V 档" } else { ". byte × 100 mV. HarmonyOS treats this step as 12 V" },
        0x33..=0x3f => if zh { "。字节 × 100 mV" } else { ". byte × 100 mV" },
        0x54 => if zh { "。0x1E 或 0x20 配合适配器型号识别 M5/M7 车充" } else { ". 0x1E or 0x20 with the adapter code marks an M5/M7 car charger" },
        0xca => if zh { "。单字节电压 (码 × 10 mV) + 3000 mV" } else { ". single-byte voltage (code × 10 mV) + 3000 mV" },
        0xb8 => if zh { "。与下一字节组成毫伏" } else { ". with the next byte, millivolts" },
        _ => "",
    };
    format!("{name}{extra}")
}

fn adapter_status(byte: u8) -> String {
    let mut parts = Vec::new();
    if byte & 0x08 != 0 { parts.push("欠压"); }
    if byte & 0x04 != 0 { parts.push("过压"); }
    if byte & 0x02 != 0 { parts.push("过流"); }
    if byte & 0x01 != 0 { parts.push("过温"); }
    if parts.is_empty() { "正常".into() } else { parts.join(" ") }
}

fn value_meaning(addr: u8, data: &[u8]) -> Option<String> {
    let byte = *data.first()?;
    match addr {
        0x21 => match byte {
            1 => Some("5V/9V".into()),
            2 => Some("5V/9V/12V".into()),
            _ => None,
        },
        0x22 => Some(format!("{:.2}W", f64::from(byte) * 0.5)),
        0x28 => Some(adapter_status(byte)),
        0x2b if byte == 1 => Some("输出使能".into()),
        0x2c | 0x30..=0x3f => Some(format!("{:.2}V", f64::from(byte) * 0.1)),
        0x2d => Some(format!("{:.2}A", f64::from(byte) * 0.1)),
        0x54 if matches!(byte, 0x1e | 0x20) => Some("M5/M7车充".into()),
        0x7e => Some(adapter_type(byte)),
        0x8d => Some(format!("0x{byte:02X} {}", adapter_code(byte))),
        0xca => Some(format!("{:.2}V", (3000 + u32::from(byte) * 10) as f64 / 1000.0)),
        0xb8 if data.len() >= 2 => Some(format!("{:.3}V", u32::from(u16::from_be_bytes([data[0], data[1]])) as f64 / 1000.0)),
        0xcf => Some(if byte & 0x40 != 0 { "加密完成".into() } else { format!("0x{byte:02X}") }),
        0xd0..=0xef if (addr - 0xd0) % 2 == 0 => Some(format!("{:.1}V", f64::from(byte) * 0.5)),
        0xd0..=0xef => Some(format!("{:.1}A", f64::from(byte) * 0.1)),
        _ => None,
    }
}

pub fn adapter_bit_lines(byte: u8, zh: bool) -> Vec<String> {
    let on = |value: bool| if zh { if value { "是" } else { "否" } } else if value { "yes" } else { "no" };
    vec![
        coded_line(zh, "Type-B", "Type-B", format!("0x{byte:02X} & 0x90"), on(byte & 0x90 != 0)),
        coded_line(zh, "不可拆卸线缆", "Undetachable cable", format!("0x{byte:02X} & 0x40"), on(byte & 0x40 != 0)),
        coded_line(zh, "2:1 直充", "2:1 direct charge", format!("0x{byte:02X} & 0x08"), on(byte & 0x08 != 0)),
        coded_line(zh, "1:1 直充", "1:1 direct charge", format!("bit2 = {}", u8::from(byte & 0x04 == 0)), on(byte & 0x04 == 0)),
        coded_line(zh, "接口", "Port", format!("0x{byte:02X} & 0x03"), if byte & 0x03 == 1 { "Type-C" } else { "Type-A" }),
    ]
}

fn adapter_type(byte: u8) -> String {
    let mut parts = vec![format!("0x{byte:02X}")];
    if byte & 0x90 != 0 {
        parts.push("Type-B".into());
    }
    if byte & 0x40 != 0 {
        parts.push("不可拆卸线缆".into());
    }
    if byte & 0x08 != 0 {
        parts.push("2:1直充".into());
    }
    if byte & 0x04 == 0 {
        parts.push("1:1直充".into());
    }
    parts.push(if byte & 0x03 == 1 { "Type-C".into() } else { "Type-A".into() });
    parts.join(" ")
}

fn adapter_code(byte: u8) -> &'static str {
    match byte {
        0x00 => "25W iWatt",
        0x01 => "25W Richtek",
        0x02 | 0x07 => "40W",
        0x05 => "65W",
        0x0d => "66W",
        0x0f => "60W 双口C",
        0x16 => "66W 三口A",
        0x19 => "40W",
        0x1d => "66W",
        0x26 => "135W",
        0x2c => "65W",
        0x3a => "100W 车充",
        0x3f => "66W",
        _ => "型号",
    }
}

fn coded_line(zh: bool, zh_name: &str, en_name: &str, code: impl std::fmt::Display, physical: impl std::fmt::Display) -> String {
    let name = if zh { zh_name } else { en_name };
    format!("{name}  码 {code}  →  {physical}")
}

fn command_phrase(command: &str, zh: bool) -> &'static str {
    if zh {
        match command {
            "SBRWR" => "单字节写 SBRWR",
            "SBRRD" => "单字节读 SBRRD",
            "MBRWR" => "多字节写 MBRWR",
            "MBRRD" => "多字节读 MBRRD",
            "ACK" => "确认 ACK",
            "NACK" => "否认 NACK",
            _ => "命令",
        }
    } else {
        match command {
            "SBRWR" => "single-byte write SBRWR",
            "SBRRD" => "single-byte read SBRRD",
            "MBRWR" => "multi-byte write MBRWR",
            "MBRRD" => "multi-byte read MBRRD",
            "ACK" => "ACK",
            "NACK" => "NACK",
            _ => "command",
        }
    }
}

/// Register payload of a command or of the adapter's data reply.
/// `start` is the first register address. Consecutive bytes are consecutive registers.
fn scp_value_fields(start: u8, data: &[u8], zh: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let mut index = 0;
    while index < data.len() {
        let addr = start.wrapping_add(index as u8);
        if matches!(addr, 0x82 | 0x84 | 0x86 | 0x8a | 0xb8) && index + 1 < data.len() {
            let raw = u16::from_be_bytes([data[index], data[index + 1]]);
            let (name_zh, name_en, code, physical) = match addr {
                0xb8 => ("输出电压", "Output voltage", format!("0x{raw:04X} × 1 mV"), format!("{:.3}V", f64::from(raw) / 1000.0)),
                0x82 => ("厂商 ID", "Vendor ID", format!("0x{:02X} 0x{:02X}", data[index], data[index + 1]), format!("0x{raw:04X}")),
                0x84 => ("模块 ID", "Module ID", format!("0x{:02X} 0x{:02X}", data[index], data[index + 1]), format!("0x{raw:04X}")),
                0x86 => ("序列号", "Serial number", format!("0x{:02X} 0x{:02X}", data[index], data[index + 1]), format!("0x{raw:04X}")),
                _ => ("固件版本", "Firmware version", format!("0x{:02X} 0x{:02X}", data[index], data[index + 1]), format!("0x{raw:04X}")),
            };
            lines.push(coded_line(zh, name_zh, name_en, code, physical));
            index += 2;
            continue;
        }
        let byte = data[index];
        match addr {
            0x7e => {
                lines.push(coded_line(zh, "适配器类型", "Adapter type", format!("0x{byte:02X}"), adapter_type(byte)));
                lines.extend(adapter_bit_lines(byte, zh));
            }
            0x8d => lines.push(coded_line(zh, "适配器型号", "Adapter model", format!("0x{byte:02X}"), adapter_code(byte))),
            0xca => {
                let mv = 3000 + u32::from(byte) * 10;
                lines.push(coded_line(zh, "输出电压", "Output voltage", format!("0x{byte:02X} × 10 mV + 3000 mV"), format!("{:.2}V", mv as f64 / 1000.0)));
            }
            0xcf => lines.push(coded_line(
                zh,
                "加密状态",
                "Encryption",
                format!("0x{byte:02X} bit6={}", u8::from(byte & 0x40 != 0)),
                if byte & 0x40 != 0 { if zh { "加密完成" } else { "encryption done" } } else { if zh { "未完成" } else { "not done" } },
            )),
            0xd0..=0xef if (addr - 0xd0) % 2 == 0 => {
                lines.push(coded_line(zh, "功率曲线电压", "Power-curve voltage", format!("0x{byte:02X} × 0.5 V"), format!("{:.1}V", f64::from(byte) * 0.5)));
            }
            0xd0..=0xef => {
                lines.push(coded_line(zh, "功率曲线电流", "Power-curve current", format!("0x{byte:02X} × 0.1 A"), format!("{:.1}A", f64::from(byte) * 0.1)));
            }
            0x8f => lines.push(coded_line(zh, "功率曲线档数", "Power-curve steps", format!("0x{byte:02X}"), byte)),
            0x21 => {
                let physical = match byte {
                    1 => "5V/9V",
                    2 => "5V/9V/12V",
                    _ => if zh { "原码，规格只列出 1 和 2" } else { "raw; only 1 and 2 are defined" },
                };
                lines.push(coded_line(zh, "离散电压能力", "Discrete voltage capability", format!("0x{byte:02X}"), physical));
            }
            0x22 => lines.push(coded_line(zh, "最大功率", "Maximum power", format!("0x{byte:02X} × 500 mW"), format!("{:.2}W", f64::from(byte) * 0.5))),
            0x28 => {
                lines.push(coded_line(zh, "适配器状态", "Adapter status", format!("0x{byte:02X}"), adapter_status(byte)));
                let flag = |on: bool| if zh { if on { "触发" } else { "未触发" } } else if on { "active" } else { "inactive" };
                for (mask, bit, zh_name, en_name) in [
                    (0x08u8, 3u8, "欠压保护", "UVP"),
                    (0x04, 2, "过压保护", "OVP"),
                    (0x02, 1, "过流保护", "OCP"),
                    (0x01, 0, "过温保护", "OTP"),
                ] {
                    let on = byte & mask != 0;
                    lines.push(coded_line(zh, zh_name, en_name, format!("bit{bit}={}", u8::from(on)), flag(on)));
                }
            }
            0x2b => lines.push(coded_line(
                zh,
                "输出使能",
                "Output enable",
                format!("0x{byte:02X}"),
                if byte == 1 { if zh { "输出使能" } else { "output enabled" } } else if zh { "原码。规格示例只把 1 定义为使能输出" } else { "raw; the published example only defines 1 as output enable" },
            )),
            0x2c => lines.push(coded_line(zh, "电压配置", "Voltage setting", format!("0x{byte:02X} × 100 mV"), format!("{:.2}V", f64::from(byte) * 0.1))),
            0x2d => lines.push(coded_line(zh, "电流配置", "Current setting", format!("0x{byte:02X} × 100 mA"), format!("{:.2}A", f64::from(byte) * 0.1))),
            0x30..=0x3f => {
                let (zh_name, en_name) = register_names(addr);
                lines.push(coded_line(zh, zh_name, en_name, format!("0x{byte:02X} × 100 mV"), format!("{:.2}V", f64::from(byte) * 0.1)));
            }
            0x54 => lines.push(coded_line(
                zh,
                "车充签名",
                "Car-charger signature",
                format!("0x{byte:02X}"),
                if matches!(byte, 0x1e | 0x20) { if zh { "M5/M7 车充签名" } else { "M5/M7 car-charger signature" } } else if zh { "原码。只有 0x1E 和 0x20 被用来识别 M5/M7 车充" } else { "raw; only 0x1E and 0x20 mark an M5/M7 car charger" },
            )),
            _ => {
                let name = register_name(addr);
                let (name_zh, name_en) = if name == "寄存器" { ("寄存器", "Register") } else { (name, name) };
                lines.push(coded_line(zh, name_zh, name_en, format!("0x{byte:02X}"), if zh { "原码，规格没有给出换算" } else { "raw code; the specification defines no scale" }));
            }
        }
        index += 1;
    }
    lines
}

/// Field decode of one SCP frame. `follow` is the register a bare adapter data reply answers.
pub fn explain_scp(bytes: &[u8], follow: Option<u8>, zh: bool) -> String {
    if bytes.is_empty() {
        return if zh { "无字节载荷".into() } else { "No payload bytes".into() };
    }
    let command = command_name(bytes[0]);
    let addressed = matches!(bytes[0], 0x0b | 0x0c | 0x1b | 0x1c);
    let is_data = command == "DATA";
    let reg = if addressed { bytes.get(1).copied() } else if is_data { follow } else { None };
    let data = payload_bytes(bytes, reg.is_some() && !addressed);
    let mut lines = vec![coded_line(
        zh,
        "同步",
        "Sync",
        if zh { "起始双沿" } else { "start edges" },
        if zh { "随后 8 位 MSB 与奇校验" } else { "then 8 MSB bits and odd parity" },
    )];
    if is_data {
        if let Some(addr) = reg {
            lines.push(coded_line(zh, "应答寄存器", "Reply register", format!("0x{addr:02X}"), register_phrase(addr, zh)));
        } else {
            lines.push(coded_line(zh, "应答", "Reply", "DATA", if zh { "前面没有读命令，寄存器未知" } else { "no preceding read; register unknown" }));
        }
    } else {
        lines.push(coded_line(zh, "命令", "Command", format!("0x{:02X}", bytes[0]), command_phrase(command, zh)));
        if let Some(addr) = reg {
            lines.push(coded_line(zh, "寄存器", "Register", format!("0x{addr:02X}"), register_phrase(addr, zh)));
        }
        if matches!(bytes[0], 0x1b | 0x1c) {
            if let Some(&len) = bytes.get(2) {
                lines.push(coded_line(zh, "长度", "Length", format!("0x{len:02X}"), if zh { format!("{len} 字节") } else { format!("{len} bytes") }));
            }
        }
    }
    if let Some(addr) = reg {
        if !data.is_empty() {
            lines.extend(scp_value_fields(addr, data, zh));
        }
    } else if is_data {
        for (index, byte) in data.iter().copied().enumerate() {
            lines.push(coded_line(zh, &format!("数据{index}"), &format!("Data {index}"), format!("0x{byte:02X}"), if zh { "原码" } else { "raw" }));
        }
    }
    let crc = bytes.len() >= 2;
    for (index, byte) in bytes.iter().copied().enumerate() {
        let bits: String = (0..8).rev().map(|bit| if byte >> bit & 1 == 1 { '1' } else { '0' }).collect();
        let role = if index == 0 && !is_data {
            command_phrase(command, zh)
        } else if crc && index + 1 == bytes.len() {
            "CRC-8"
        } else if addressed && index == 1 {
            let (zh_name, en_name) = register_names(bytes[1]);
            if zh { zh_name } else { en_name }
        } else if matches!(bytes[0], 0x1b | 0x1c) && index == 2 {
            if zh { "长度" } else { "length" }
        } else if zh {
            "数据"
        } else {
            "data"
        };
        lines.push(coded_line(zh, &format!("波形字节{index}"), &format!("Wire byte {index}"), format!("MSB {bits}"), format!("0x{byte:02X} {role}")));
    }
    if crc {
        lines.push(coded_line(
            zh,
            "CRC-8",
            "CRC-8",
            format!("0x{:02X}", bytes[bytes.len() - 1]),
            if zh { "帧末校验字节，多项式 0x39" } else { "trailing checksum, polynomial 0x39" },
        ));
    }
    lines.push(coded_line(zh, "奇校验", "Odd parity", if zh { "每字节 1 位" } else { "one bit per byte" }, if zh { "该位使 8 位数据与校验位的 1 的个数为奇数" } else { "makes the number of 1s in the byte plus parity odd" }));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    // Independent transmitter following B.3.3: at least three separator
    // edges, two rising edges at packet start, then 8 MSB bits + odd parity.
    fn packet_wave(y: &mut Vec<f64>, bytes: &[u8], high: f64, bad_parity: bool) {
        let mut previous = false;
        y.extend(std::iter::repeat_n(0.0, 64));
        for (index, &byte) in bytes.iter().enumerate() {
            let msb = byte & 0x80 != 0;
            let mut level = previous;
            for _ in 0..2 {
                level = !level;
                y.extend(std::iter::repeat_n(if level { high } else { 0.0 }, 8));
            }
            if level == msb {
                level = !level;
                y.extend(std::iter::repeat_n(if level { high } else { 0.0 }, 8));
            }
            for bit in (0..8).rev() {
                y.extend(std::iter::repeat_n(if byte >> bit & 1 != 0 { high } else { 0.0 }, 32));
            }
            previous = (byte.count_ones() % 2 == 0) ^ (bad_parity && index == 0);
            y.extend(std::iter::repeat_n(if previous { high } else { 0.0 }, 32));
        }
        // End separator terminates at a high 16 UI ping.
        if previous { y.extend(std::iter::repeat_n(0.0, 8)); }
        y.extend(std::iter::repeat_n(high, 8));
        y.extend(std::iter::repeat_n(0.0, 8));
        y.extend(std::iter::repeat_n(high, 16 * 32));
        y.extend(std::iter::repeat_n(0.0, 64));
    }

    fn waveform(y: Vec<f64>, ui: f64) -> WaveformTrace {
        WaveformTrace { channel: "D-".into(), x: (0..y.len()).map(|i| i as f64 * ui / 32.0).collect::<Vec<_>>().into(), y: y.into(), x_unit: "s".into(), y_unit: "V".into() }
    }

    #[test]
    fn all_bytes_and_mixed_voltage_senders_decode_from_waveform() {
        for ui in [144e-6, 160e-6, 176e-6] {
            let mut y = Vec::new();
            let mut bytes: Vec<u8> = (0..=255).collect();
            bytes.push(crc8(&bytes, 0x39));
            packet_wave(&mut y, &bytes, 1.8, false);
            packet_wave(&mut y, &[0x08, crc8(&[0x08], 0x39)], 3.3, false);
            let result = decode_scp(&waveform(y, ui), None, &ScpConfig::default());
            let data: Vec<_> = result.frames.iter().filter(|f| !f.bytes.is_empty()).collect();
            assert_eq!(data.len(), 2, "{result:?}");
            assert_eq!(data[0].bytes, bytes);
            assert!(data.iter().all(|f| f.error == BusFrameError::None), "{data:?}");
        }
    }

    #[test]
    fn physical_parity_and_crc_errors_are_reported() {
        for (bad_parity, crc, expected) in [(true, 0xc4, BusFrameError::Parity), (false, 0xc5, BusFrameError::Checksum)] {
            let mut y = Vec::new();
            packet_wave(&mut y, &[0x0c, 0x2b, 0x10, crc], 1.8, bad_parity);
            let result = decode_scp(&waveform(y, 160e-6), None, &ScpConfig::default());
            assert_eq!(result.frames[0].error, expected, "{result:?}");
        }
    }
    #[test]
    fn ping_clock_survives_quantized_separator_and_decodes_every_byte() {
        let mut y = vec![0.0; 100];
        y.extend(std::iter::repeat_n(1.8, 2548));
        y.extend(std::iter::repeat_n(0.0, 300));
        // Independent 1 us sampling: quarter pulses 40/41 us, data 159 us.
        let bytes = [0x08u8, 0x34, crc8(&[0x08, 0x34], 0x39)];
        let mut previous = false;
        for byte in bytes {
            let msb = byte & 0x80 != 0;
            for duration in [41, 40] {
                previous = !previous;
                y.extend(std::iter::repeat_n(if previous { 1.8 } else { 0.0 }, duration));
            }
            if previous == msb {
                previous = !previous;
                y.extend(std::iter::repeat_n(if previous { 1.8 } else { 0.0 }, 41));
            }
            for bit in (0..8).rev() {
                y.extend(std::iter::repeat_n(if byte >> bit & 1 != 0 { 1.8 } else { 0.0 }, 159));
            }
            previous = byte.count_ones() % 2 == 0;
            y.extend(std::iter::repeat_n(if previous { 1.8 } else { 0.0 }, 159));
        }
        if previous { y.extend(std::iter::repeat_n(0.0, 41)); }
        y.extend(std::iter::repeat_n(1.8, 40));
        y.extend(std::iter::repeat_n(0.0, 41));
        y.extend(std::iter::repeat_n(1.8, 2548));
        y.extend(std::iter::repeat_n(0.0, 100));
        let result = decode_scp(&waveform(y, 32e-6), None, &ScpConfig::default());
        let data: Vec<_> = result.frames.iter().filter(|f| !f.bytes.is_empty()).collect();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].bytes, bytes);
        assert_eq!(data[0].error, BusFrameError::None);
    }

    #[test]
    fn scp_crc_uses_0x39_polynomial() {
        assert_eq!(crc8(&[0x0c, 0x2b, 0x10], 0x39), 0xc4);
    }
    #[test]
    fn assembles_complete_command_with_crc() {
        let mut result = BusDecodeResult::default();
        let mut used = 0;
        let mut packet = [0x0c, 0x2b, 0x10, 0xc4]
            .into_iter()
            .enumerate()
            .map(|(i, byte)| BusByteSpan {
                t_start: i as f64,
                t_end: i as f64 + 1.0,
                byte,
                error: BusByteError::None,
            })
            .collect();
        emit(&mut result, &mut used, &mut packet, true, None);
        assert_eq!(result.frames[0].error, BusFrameError::None);
        assert_eq!(result.frames[0].summary, "SCP SBRRD reg=0x2B");
    }
    #[test]
    fn rejects_invalid_ui() {
        let trace = WaveformTrace {
            channel: "D+".into(),
            x: vec![0.0, 1.0].into(),
            y: vec![0.0, 3.3].into(),
            x_unit: "s".into(),
            y_unit: "V".into(),
        };
        let r = decode_scp(&trace, None, &ScpConfig { ui_us: 250.0 });
        assert!(r.error.unwrap().contains("120..200"));
    }

    #[test]
    fn recognizes_reset_pulse() {
        let mut y = vec![0.0; 64];
        y.extend(std::iter::repeat_n(1.8, 100 * 32));
        y.extend(std::iter::repeat_n(0.0, 64));
        let r = decode_scp(&waveform(y, 160e-6), None, &ScpConfig::default());
        assert_eq!(r.frames.len(), 1);
        assert_eq!(r.frames[0].summary, "RESET");
    }
    #[test]
    #[ignore = "Set WIPARSE_SCP_CAPTURE to the external Normal.isf capture"]
    fn real_normal_capture_regression() {
        let path = std::env::var("WIPARSE_SCP_CAPTURE").expect("external capture path");
        let traces = crate::waveform_file::load_waveform_file_all(&path).unwrap();
        let trace = traces.iter().find(|t| t.channel == "CH2").unwrap();
        let r = decode_scp(trace, None, &ScpConfig::default());
        assert_eq!(r.frames.len(), 278);
        assert!(r.frames.iter().all(|f| !f.has_error()), "{}", r.info);
        assert_eq!(r.frames.iter().filter(|f| !f.bytes.is_empty()).count(), 60);
        assert!(r.frames.iter().any(|f| f.bytes == [0x0c, 0x7e, 0x12] && f.summary == "SCP SBRRD reg=0x7E 适配器类型"));
    }

    #[test]
    fn translates_read_request_and_voltage_write() {
        let read = explain_scp(&[0x0c, 0x7e, 0x12], None, true);
        assert!(read.contains("单字节读"), "{read}");
        assert!(read.contains("适配器类型"), "{read}");
        assert!(read.contains("CRC-8"), "{read}");
        let body = [0x0bu8, 0xca, 200];
        let mut frame = body.to_vec();
        frame.push(crc8(&body, 0x39));
        let write = explain_scp(&frame, None, true);
        assert!(write.contains("单字节写"), "{write}");
        assert!(write.contains("5.00V"), "{write}");
        let reply = explain_scp(&[0x91, 0x00], Some(0x7e), true);
        assert!(reply.contains("适配器类型"), "{reply}");
        assert!(reply.contains("Type-B"), "{reply}");
        assert!(reply.contains("Type-C"), "{reply}");
        assert!(reply.contains("0x91"), "{reply}");
        assert!(reply.contains("→"), "{reply}");
        let volts = explain_scp(&[0x13, 0x88, 0x00], Some(0xb8), true);
        assert!(volts.contains("5.000V"), "{volts}");
        assert!(volts.contains("0x1388"), "{volts}");
    }

    #[test]
    fn output_step_one_is_hundred_millivolts() {
        let mut read = vec![0x0cu8, 0x31];
        read.push(crc8(&read, 0x39));
        let text = explain_scp(&read, None, true);
        assert!(text.contains("第1档输出电压"), "{text}");
        assert!(text.contains("100 mV"), "{text}");
        assert!(text.contains("9V 档"), "{text}");
        assert!(!text.contains("9.00V"), "{text}");
        let mut result = BusDecodeResult::default();
        let mut used = 0;
        let mut packet = read
            .iter()
            .enumerate()
            .map(|(i, byte)| BusByteSpan {
                t_start: i as f64,
                t_end: i as f64 + 1.0,
                byte: *byte,
                error: BusByteError::None,
            })
            .collect();
        emit(&mut result, &mut used, &mut packet, true, None);
        assert_eq!(result.frames[0].summary, "SCP SBRRD reg=0x31 第1档输出电压");

        let mut reply = vec![90u8];
        reply.push(crc8(&reply, 0x39));
        let value = explain_scp(&reply, Some(0x31), true);
        assert!(value.contains("第1档输出电压"), "{value}");
        assert!(value.contains("0x5A × 100 mV"), "{value}");
        assert!(value.contains("9.00V"), "{value}");

        let mut power = vec![36u8];
        power.push(crc8(&power, 0x39));
        let watts = explain_scp(&power, Some(0x22), true);
        assert!(watts.contains("18.00W"), "{watts}");
        assert!(watts.contains("500 mW"), "{watts}");

        let mut status = vec![0x04u8];
        status.push(crc8(&status, 0x39));
        let flags = explain_scp(&status, Some(0x28), true);
        assert!(flags.contains("过压"), "{flags}");
        assert!(flags.contains("bit2=1"), "{flags}");
        assert!(flags.contains("bit3=0"), "{flags}");
    }
}
