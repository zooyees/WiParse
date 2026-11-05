//! Field-level meaning for a decoded frame. Values use the protocol scale, not raw codes.
use super::scp::explain_scp;
use super::{BusFrame, BusFrameError, BusKind};
use crate::protocol::decode_qi_message;

pub fn explain_frame(kind: BusKind, frames: &[BusFrame], index: usize, zh: bool) -> Vec<String> {
    let Some(frame) = frames.get(index) else {
        return Vec::new();
    };
    let mut lines = match kind {
        BusKind::UsbPd => explain_pd(frames, index, zh),
        BusKind::Scp => explain_scp_lines(frames, index, zh),
        BusKind::Ufcs => explain_ufcs(&frame.bytes, zh),
        BusKind::Uart => explain_uart(frame, zh),
        BusKind::I2c => explain_i2c(frame, zh),
        BusKind::Spi => explain_spi(frame, zh),
        BusKind::I2s => explain_i2s(frame, zh),
        BusKind::Ddsss => explain_qi(&frame.bytes, zh),
        BusKind::Off => Vec::new(),
    };
    if lines.is_empty() {
        lines.push(if zh { "无字段".into() } else { "No fields".into() });
    }
    if !frame.bytes.is_empty() {
        let hex = frame.bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
        lines.push(if zh { format!("原始字节  {hex}") } else { format!("Raw  {hex}") });
    }
    lines
}

fn explain_scp_lines(frames: &[BusFrame], index: usize, zh: bool) -> Vec<String> {
    let frame = &frames[index];
    let follow = if frame.bytes.first().is_some_and(|b| matches!(b, 0x0b | 0x0c | 0x1b | 0x1c | 0x08 | 0x03)) {
        None
    } else {
        frames[..index].iter().rev().find_map(|previous| {
            let cmd = previous.bytes.first().copied()?;
            if matches!(cmd, 0x0c | 0x1c) { previous.bytes.get(1).copied() } else { None }
        })
    };
    let mut lines: Vec<String> = explain_scp(&frame.bytes, follow, zh).lines().map(str::to_string).collect();
    if frame.bytes.len() >= 2 {
        let body = &frame.bytes[..frame.bytes.len() - 1];
        let crc = frame.bytes[frame.bytes.len() - 1];
        let expect = super::charging::crc8(body, 0x39);
        lines.push(coded(
            zh,
            "CRC-8",
            "CRC-8",
            format!("收到 {crc:02X} / 计算 {expect:02X}"),
            if crc == expect { if zh { "一致" } else { "match" } } else { if zh { "不一致" } else { "mismatch" } },
        ));
    }
    lines
}

fn explain_uart(frame: &BusFrame, zh: bool) -> Vec<String> {
    if frame.summary == "BREAK" {
        return vec![if zh { "线路空闲被拉住，超过一个字符时间，这是 BREAK。".into() } else { "Line held through a character time: BREAK.".into() }];
    }
    let mut lines = Vec::new();
    if let Some(&byte) = frame.bytes.first() {
        let bits: String = (0..8).map(|bit| if byte >> bit & 1 == 1 { '1' } else { '0' }).collect();
        let width = frame.t_end - frame.t_start;
        lines.push(coded(zh, "起始位", "Start bit", "0", if zh { "空闲为高，起始位把线拉低" } else { "idle is high; the start bit pulls the line low" }));
        lines.push(coded(zh, "数据位", "Data bits", format!("0b{bits} 低位在前"), format!("0x{byte:02X} = {byte}")));
        for bit in 0..8 {
            let one = (byte >> bit) & 1;
            lines.push(coded(zh, &format!("b{bit}"), &format!("b{bit}"), one, format!("权重 {}", 1u32 << bit)));
        }
        lines.push(coded(zh, "停止位", "Stop bit", "1", if zh { "回到空闲" } else { "return to idle" }));
        if width > 0.0 {
            lines.push(coded(zh, "位时间", "Bit time", "8N1 × 10", format!("{:.3} µs", width / 10.0 * 1e6)));
        }
        lines.push(coded(zh, "数据", "Data", format!("0x{byte:02X}"), if zh { format!("{byte}，起始位和停止位之间的载荷") } else { format!("{byte}, the payload between start and stop") }));
        if (0x20..0x7f).contains(&byte) {
            lines.push(coded(zh, "字符", "Character", format!("0x{byte:02X}"), format!("'{}'", byte as char)));
        }
    }
    if frame.bytes.len() == 2 {
        let word = u16::from(frame.bytes[0]) | u16::from(frame.bytes[1]) << 8;
        lines.push(if zh { format!("9 位数据  {word}") } else { format!("9-bit data  {word}") });
    }
    lines.push(integrity(frame.error, zh));
    lines
}

fn explain_i2c(frame: &BusFrame, zh: bool) -> Vec<String> {
    let summary = frame.summary.as_str();
    let ack = if summary.contains("NAK") {
        if zh { "从机 NAK" } else { "slave NAK" }
    } else if summary.contains("ACK") {
        if zh { "从机 ACK" } else { "slave ACK" }
    } else {
        ""
    };
    let mut lines = vec![summary.to_string()];
    if summary == "START" || summary.starts_with("Sr ") || summary.starts_with("START") {
        lines.push(coded(zh, "起始条件", "Start", "SDA↓ SCL=1", if zh { "开始一次传输" } else { "begins a transfer" }));
    }
    if summary == "STOP" {
        lines.push(coded(zh, "停止条件", "Stop", "SDA↑ SCL=1", if zh { "SCL 为高时 SDA 从低到高，主设备结束传输" } else { "SDA rises while SCL is high: the controller ends the transfer" }));
    }
    if summary.starts_with("General call") {
        lines.push(coded(zh, "广播地址", "General call", "0x00", if zh { "7 位地址为 0，后面的字节发给总线上所有从机" } else { "7-bit address 0; the following byte is for every target" }));
    }
    if summary.starts_with("10-bit") {
        lines.push(coded(zh, "10 位地址", "10-bit address", "11110", if zh { "首字节高 5 位固定为 11110，其余位和第 2 字节组成 10 位地址" } else { "the first byte starts with 11110; the rest plus the second byte form the 10-bit address" }));
    }
    if let Some(rest) = summary.strip_prefix("Dev 0x") {
        let addr = u8::from_str_radix(rest.get(..2).unwrap_or("00"), 16).unwrap_or(0);
        let write = summary.contains(" W ");
        lines.push(coded(zh, "7 位地址", "7-bit address", format!("0x{addr:02X}"), format!("{addr} {}", if write { if zh { "写" } else { "write" } } else { if zh { "读" } else { "read" } })));
        let rw = u8::from(!write);
        lines.push(coded(zh, "地址字节", "Address byte", format!("0x{:02X}", (addr << 1) | rw), if zh { format!("7 位地址左移后加上读/写位 {rw}") } else { format!("7-bit address shifted, R/W bit {rw}") }));
        lines.push(coded(zh, "读/写位", "R/W bit", rw, if write { if zh { "0，主设备写" } else { "0, controller writes" } } else { if zh { "1，主设备读" } else { "1, controller reads" } }));
        let bits: String = (0..8).rev().map(|bit| if ((addr << 1) | rw) >> bit & 1 == 1 { '1' } else { '0' }).collect();
        lines.push(coded(zh, "地址位", "Address bits", format!("MSB {bits}"), if zh { "高位先发。前 7 位是地址，第 8 位是读/写，第 9 个时钟是应答" } else { "MSB first. Seven address bits, then R/W; the ninth clock is ACK" }));
    }
    if let Some(rest) = summary.strip_prefix("Reg 0x") {
        let digits = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect::<String>();
        if let Ok(reg) = u32::from_str_radix(&digits, 16) {
            lines.push(coded(zh, "寄存器地址", "Register address", format!("0x{reg:X}"), if zh { format!("{reg}。从机内部地址，后面的数据字节读写这里") } else { format!("{reg}. The target's internal address; the following data byte reads or writes it") }));
        }
    }
    if let Some(rest) = summary.strip_prefix("Data 0x") {
        if let Ok(data) = u8::from_str_radix(rest.get(..2).unwrap_or("00"), 16) {
            let bits: String = (0..8).rev().map(|bit| if data >> bit & 1 == 1 { '1' } else { '0' }).collect();
            lines.push(coded(zh, "数据字节", "Data byte", format!("0x{data:02X}"), if zh { format!("{data}。从机寄存器或载荷的一个字节") } else { format!("{data}. One byte of the target register or payload") }));
            lines.push(coded(zh, "数据位", "Data bits", format!("MSB {bits}"), if zh { "高位先发，随后一个应答位" } else { "MSB first, then one ACK bit" }));
        }
    }
    if !ack.is_empty() {
        let ack_bit = u8::from(summary.contains("NAK"));
        lines.push(coded(zh, "应答位", "ACK bit", ack_bit, if ack_bit == 0 { if zh { "SDA=0，接收方拉低，表示应答" } else { "SDA=0, the receiver pulled the line low: ACK" } } else { if zh { "SDA=1，接收方未拉低，表示无应答" } else { "SDA=1, the receiver left the line high: NACK" } }));
        lines.push(ack.into());
    }
    lines
}

fn explain_spi(frame: &BusFrame, zh: bool) -> Vec<String> {
    let mut lines = vec![frame.summary.clone()];
    if !frame.bytes.is_empty() {
        let value = frame.bytes.iter().fold(0u64, |v, b| v << 8 | u64::from(*b));
        let bits = frame.bytes.iter().map(|byte| format!("{byte:08b}")).collect::<Vec<_>>().join(" ");
        lines.push(coded(zh, "片选字", "Chip-select word", bits, if zh { format!("0x{value:X} = {value}。片选有效期间按时钟移出；SPI 不规定这些位属于哪个寄存器") } else { format!("0x{value:X} = {value}. Bits shifted while chip-select is active; SPI does not assign them to a register") }));
    }
    lines.push(integrity(frame.error, zh));
    lines
}

fn explain_i2s(frame: &BusFrame, zh: bool) -> Vec<String> {
    let channel = if frame.summary.contains(" R ") || frame.summary.contains("I2S R") {
        if zh { "右声道" } else { "right" }
    } else {
        if zh { "左声道" } else { "left" }
    };
    let mut lines = vec![coded(
        zh,
        "声道",
        "Channel",
        "WS",
        if zh { format!("{channel}。字选择 WS 区分左右声道，数据是二进制补码采样") } else { format!("{channel}. Word-select WS picks the channel; the data is a two's-complement sample") },
    )];
    if !frame.bytes.is_empty() {
        let value = frame.bytes.iter().fold(0u32, |v, b| v << 8 | u32::from(*b));
        lines.push(if zh { format!("采样  {value}（0x{value:X}）") } else { format!("Sample  {value} (0x{value:X})") });
        if frame.bytes.len() == 2 {
            let signed = i16::from_be_bytes([frame.bytes[0], frame.bytes[1]]);
            let scale = signed as f64 / 32768.0 * 100.0;
            lines.push(coded(zh, "采样码", "Sample code", format!("0x{value:04X}"), format!("{signed}")));
            lines.push(coded(zh, "满量程", "Full scale", "16 bit", format!("{scale:.2} %")));
            lines.push(if zh { format!("有符号 16 位  {signed}") } else { format!("Signed 16-bit  {signed}") });
        }
    }
    if frame.summary.contains("partial") {
        lines.push(if zh { "采样位数不完整。".into() } else { "Partial sample.".into() });
    }
    lines
}

fn explain_qi(bytes: &[u8], zh: bool) -> Vec<String> {
    let hex = bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
    let Some(msg) = decode_qi_message(&format!("ASK {hex}")) else {
        return vec![if zh { "不是可识别的 Qi 报文。".into() } else { "Not a recognized Qi packet.".into() }];
    };
    let mut lines = vec![
        coded(zh, "前导", "Preamble", "1…1", if zh { "至少 11 个 1，供接收端同步；本帧从包头字节开始" } else { "at least eleven 1s for sync; this frame starts at the header byte" }),
        format!("{}  {}", msg.name, msg.desc),
    ];
    if let Some(&header) = bytes.first() {
        lines.insert(1, coded(zh, "包头", "Header", format!("0x{header:02X}"), msg.name.clone()));
    }
    lines.push(coded(zh, "字节编码", "Byte coding", "起始0 + LSB + 偶校验 + 停止1", if zh { "每个字节 11 位。前导是至少 11 个 1。校验是包头与消息逐字节异或" } else { "eleven bits per byte. The preamble is at least eleven 1s. The checksum is the XOR of the header and message" }));
    for (index, byte) in bytes.iter().copied().enumerate() {
        let bits: String = (0..8).map(|bit| if byte >> bit & 1 == 1 { '1' } else { '0' }).collect();
        let parity = byte.count_ones() & 1;
        lines.push(coded(zh, &format!("Qi字节{index}"), &format!("Qi byte {index}"), format!("LSB {bits} 偶校验 {parity}"), format!("0x{byte:02X}")));
    }
    for field in msg.fields {
        if field.name.ends_with("_desc") || field.name.ends_with("_valid") {
            continue;
        }
        let value = match field.value {
            serde_json::Value::String(text) => text,
            other => other.to_string(),
        };
        let unit = field.unit.unwrap_or_default();
        let physical = if unit.is_empty() { value.clone() } else { format!("{value} {unit}") };
        let label = qi_field_label(&field.name, zh);
        if let Some(raw) = field.raw.clone() {
            lines.push(coded(zh, label, &field.name, raw, physical));
        } else if !unit.is_empty() {
            lines.push(coded(zh, label, &field.name, value, physical));
        } else {
            lines.push(coded(zh, label, &field.name, &physical, &physical));
        }
    }
    if let Some(cs) = msg.checksum {
        lines.push(        match msg.checksum_ok {
            Some(true) => coded(zh, "校验", "Checksum", format!("0x{cs:02X}"), if zh { "一致" } else { "match" }),
            Some(false) => coded(zh, "校验", "Checksum", format!("0x{cs:02X}"), if zh { "不一致" } else { "mismatch" }),
            None => coded(zh, "校验", "Checksum", format!("0x{cs:02X}"), if zh { "包内校验字节" } else { "packet checksum" }),
        });
    }
    let _ = zh;
    lines
}

fn integrity(error: BusFrameError, zh: bool) -> String {
    match error {
        BusFrameError::None => if zh { "完整性  正常".into() } else { "Integrity  OK".into() },
        BusFrameError::Checksum => if zh { "完整性  校验错误".into() } else { "Integrity  checksum error".into() },
        BusFrameError::Parity => if zh { "完整性  奇偶错误".into() } else { "Integrity  parity error".into() },
        BusFrameError::Framing => if zh { "完整性  帧格式错误".into() } else { "Integrity  framing error".into() },
    }
}

fn explain_ufcs(bytes: &[u8], zh: bool) -> Vec<String> {
    if bytes.len() < 4 {
        return vec![if zh { "UFCS 报文不完整。".into() } else { "Incomplete UFCS message.".into() }];
    }
    let header = u16::from_be_bytes([bytes[1], bytes[2]]);
    let kind = header & 7;
    let target = match (header >> 13) & 7 {
        1 => if zh { "电源" } else { "Source" },
        2 => if zh { "受电" } else { "Sink" },
        3 => if zh { "线缆" } else { "Cable" },
        _ => if zh { "保留" } else { "Reserved" },
    };
    let version = (header >> 3) & 63;
    let version_text = if version == 1 { "1.0".to_string() } else { version.to_string() };
    let mut lines = vec![
        coded(zh, "起始", "SOP", format!("0x{:02X}", bytes[0]), if bytes[0] == 0xaa { if zh { "UFCS 同步码" } else { "UFCS sync" } } else { if zh { "不是 0xAA" } else { "not 0xAA" } }),
        coded(zh, "帧头", "Header", format!("0x{header:04X}"), if zh { "16 位大端，先发高字节" } else { "16-bit big-endian, high byte first" }),
        coded(zh, "接收方", "Address", format!("{:03b}", (header >> 13) & 7), target),
        coded(zh, "报文序号", "Message number", (header >> 9) & 15, (header >> 9) & 15),
        coded(zh, "协议版本", "Protocol version", version, version_text),
        coded(zh, "报文类型", "Message type", kind, match kind { 0 => if zh { "控制" } else { "control" }, 1 => if zh { "数据" } else { "data" }, _ => if zh { "厂商自定义" } else { "vendor" } }),
        coded(zh, "命令", "Command", format!("0x{:02X}", bytes[3]), ufcs_command(kind, bytes[3], zh)),
        coded(zh, "命令含义", "Command meaning", format!("0x{:02X}", bytes[3]), ufcs_command_meaning(kind, bytes[3], zh)),
    ];
    if kind == 1 && bytes.len() > 5 {
        let len = bytes[4] as usize;
        lines.push(coded(zh, "数据长度", "Data length", format!("0x{len:02X}"), if zh { format!("{len} 字节") } else { format!("{len} bytes") }));
        let data = bytes.get(5..5 + len).unwrap_or(&[]);
        lines.extend(ufcs_payload(bytes[3], data, zh));
    }
    if bytes.len() >= 2 {
        let crc = *bytes.last().unwrap();
        let expect = super::charging::crc8(&bytes[1..bytes.len() - 1], 0x29);
        lines.push(coded(zh, "CRC-8", "CRC-8", format!("收到 {crc:02X} / 计算 {expect:02X}"), if crc == expect { if zh { "一致" } else { "match" } } else { if zh { "不一致" } else { "mismatch" } }));
    }
    lines
}

fn ufcs_be(data: &[u8]) -> Option<u64> {
    if data.len() < 8 {
        return None;
    }
    Some(u64::from_be_bytes(data[..8].try_into().ok()?))
}

fn ufcs_temp(code: u8, zh: bool) -> String {
    if code == 0 {
        if zh { "无温度数据".into() } else { "no temperature".into() }
    } else {
        format!("{} °C", i32::from(code) - 50)
    }
}

/// T/TAF 083 data-message payloads. Multi-byte fields are big-endian.
fn ufcs_payload(command: u8, data: &[u8], zh: bool) -> Vec<String> {
    match command {
        1 => ufcs_output_capabilities(data, zh),
        2 => ufcs_request(data, zh),
        3 => ufcs_source_info(data, zh),
        4 => ufcs_sink_info(data, zh),
        5 => ufcs_cable_info(data, zh),
        6 => ufcs_device_info(data, zh),
        7 => ufcs_error_info(data, zh),
        8 => ufcs_watchdog(data, zh),
        9 => ufcs_refuse(data, zh),
        0x0a if !data.is_empty() => {
            let mut lines = vec![coded(zh, "密钥编号", "Key index", format!("0x{:02X}", data[0]), data[0])];
            if data.len() >= 17 {
                let hex = data[1..17].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
                lines.push(coded(zh, "随机数据", "Random data", hex, if zh { "16 字节" } else { "16 bytes" }));
            }
            lines
        }
        _ => Vec::new(),
    }
}

fn ufcs_output_capabilities(data: &[u8], zh: bool) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, chunk) in data.chunks_exact(8).enumerate() {
        let word = u64::from_be_bytes(chunk.try_into().unwrap());
        let n = index + 1;
        let step = (word >> 57) & 7;
        let step_text = match step {
            0 => "10 mA",
            1 => "20 mA",
            2 => "30 mA",
            3 => "40 mA",
            4 => "50 mA",
            _ => if zh { "保留" } else { "reserved" },
        };
        let v_step = if (word >> 56) & 1 == 0 { "10 mV" } else { "20 mV" };
        let vmax = (word >> 40) & 0xffff;
        let vmin = (word >> 24) & 0xffff;
        let imax = (word >> 8) & 0xffff;
        let imin = word & 0xff;
        lines.push(coded(zh, &format!("输出模式 {n}"), &format!("Output mode {n}"), format!("0x{word:016X}"), word >> 60));
        lines.push(coded(zh, &format!("模式 {n} 电流步进"), &format!("Mode {n} current step"), format!("{step:03b}"), step_text));
        lines.push(coded(zh, &format!("模式 {n} 电压步进"), &format!("Mode {n} voltage step"), (word >> 56) & 1, v_step));
        lines.push(coded(zh, &format!("模式 {n} 最高电压"), &format!("Mode {n} maximum voltage"), format!("0x{vmax:04X} × 10 mV"), format!("{:.2} V", vmax as f64 * 0.01)));
        lines.push(coded(zh, &format!("模式 {n} 最低电压"), &format!("Mode {n} minimum voltage"), format!("0x{vmin:04X} × 10 mV"), format!("{:.2} V", vmin as f64 * 0.01)));
        lines.push(coded(zh, &format!("模式 {n} 最大电流"), &format!("Mode {n} maximum current"), format!("0x{imax:04X} × 10 mA"), format!("{:.2} A", imax as f64 * 0.01)));
        lines.push(coded(zh, &format!("模式 {n} 最小电流"), &format!("Mode {n} minimum current"), format!("0x{imin:02X} × 10 mA"), format!("{:.2} A", imin as f64 * 0.01)));
    }
    lines
}

fn ufcs_request(data: &[u8], zh: bool) -> Vec<String> {
    let Some(request) = ufcs_be(data) else { return Vec::new(); };
    let v_code = (request >> 16) & 65535;
    let i_code = request & 65535;
    vec![
        coded(zh, "请求原码", "Request raw", format!("0x{request:016X}"), if zh { "64 位大端" } else { "64-bit big-endian" }),
        coded(zh, "输出模式", "Output mode", request >> 60, if zh { "高 4 位，对应输出能力里的模式编号" } else { "top 4 bits, the mode number from Output_Capabilities" }),
        coded(zh, "请求电压", "Requested voltage", format!("0x{v_code:04X} × 10 mV"), format!("{:.2} V", v_code as f64 * 0.01)),
        coded(zh, "请求电流", "Requested current", format!("0x{i_code:04X} × 10 mA"), format!("{:.2} A", i_code as f64 * 0.01)),
    ]
}

fn ufcs_source_info(data: &[u8], zh: bool) -> Vec<String> {
    let Some(word) = ufcs_be(data) else { return Vec::new(); };
    let voltage = (word >> 16) & 65535;
    let current = word & 65535;
    vec![
        coded(zh, "设备温度", "Device temperature", format!("0x{:02X}", (word >> 40) & 255), ufcs_temp(((word >> 40) & 255) as u8, zh)),
        coded(zh, "接口温度", "Port temperature", format!("0x{:02X}", (word >> 32) & 255), ufcs_temp(((word >> 32) & 255) as u8, zh)),
        coded(zh, "输出电压", "Output voltage", format!("0x{voltage:04X} × 10 mV"), format!("{:.2} V", voltage as f64 * 0.01)),
        coded(zh, "输出电流", "Output current", format!("0x{current:04X} × 10 mA"), format!("{:.2} A", current as f64 * 0.01)),
    ]
}

fn ufcs_sink_info(data: &[u8], zh: bool) -> Vec<String> {
    let Some(word) = ufcs_be(data) else { return Vec::new(); };
    let voltage = (word >> 16) & 65535;
    let current = word & 65535;
    vec![
        coded(zh, "电池温度", "Battery temperature", format!("0x{:02X}", (word >> 40) & 255), ufcs_temp(((word >> 40) & 255) as u8, zh)),
        coded(zh, "接口温度", "Port temperature", format!("0x{:02X}", (word >> 32) & 255), ufcs_temp(((word >> 32) & 255) as u8, zh)),
        coded(zh, "电池电压", "Battery voltage", format!("0x{voltage:04X} × 10 mV"), format!("{:.2} V", voltage as f64 * 0.01)),
        coded(zh, "充电电流", "Charge current", format!("0x{current:04X} × 10 mA"), format!("{:.2} A", current as f64 * 0.01)),
    ]
}

fn ufcs_cable_info(data: &[u8], zh: bool) -> Vec<String> {
    let Some(word) = ufcs_be(data) else { return Vec::new(); };
    vec![
        coded(zh, "线缆厂家", "Cable vendor", format!("0x{:04X}", (word >> 48) & 65535), format!("0x{:04X}", (word >> 48) & 65535)),
        coded(zh, "标签厂家", "E-label vendor", format!("0x{:04X}", (word >> 32) & 65535), format!("0x{:04X}", (word >> 32) & 65535)),
        coded(zh, "线缆阻抗", "Cable resistance", format!("0x{:04X} × 1 mΩ", (word >> 16) & 65535), format!("{} mΩ", (word >> 16) & 65535)),
        coded(zh, "最大电压", "Maximum voltage", format!("0x{:02X} × 1 V", (word >> 8) & 255), format!("{} V", (word >> 8) & 255)),
        coded(zh, "最大电流", "Maximum current", format!("0x{:02X} × 1 A", word & 255), format!("{} A", word & 255)),
    ]
}

fn ufcs_device_info(data: &[u8], zh: bool) -> Vec<String> {
    let Some(word) = ufcs_be(data) else { return Vec::new(); };
    vec![
        coded(zh, "设备厂家", "Device vendor", format!("0x{:04X}", (word >> 48) & 65535), format!("0x{:04X}", (word >> 48) & 65535)),
        coded(zh, "芯片厂家", "Protocol-chip vendor", format!("0x{:04X}", (word >> 32) & 65535), format!("0x{:04X}", (word >> 32) & 65535)),
        coded(zh, "硬件版本", "Hardware version", format!("0x{:04X}", (word >> 16) & 65535), if zh { "厂家自定义，0 表示未填写" } else { "vendor-defined; 0 means blank" }),
        coded(zh, "软件版本", "Software version", format!("0x{:04X}", word & 65535), if zh { "厂家自定义，0 表示未填写" } else { "vendor-defined; 0 means blank" }),
    ]
}

fn ufcs_error_info(data: &[u8], zh: bool) -> Vec<String> {
    if data.len() < 4 {
        return Vec::new();
    }
    let word = u32::from_be_bytes(data[..4].try_into().unwrap());
    let names: &[(&str, &str)] = &[
        ("输出过压", "output OVP"),
        ("输出欠压", "output UVP"),
        ("输出过流", "output OCP"),
        ("输出短路", "output SCP"),
        ("接口过温", "port OTP"),
        ("设备过温", "device OTP"),
        ("CC 过压", "CC OVP"),
        ("D- 过压", "D- OVP"),
        ("D+ 过压", "D+ OVP"),
        ("输入过压", "input OVP"),
        ("输入欠压", "input UVP"),
        ("漏电流", "leakage"),
        ("输入掉电", "input power loss"),
        ("CRC 关闭输出", "CRC shut output down"),
        ("看门狗溢出", "watchdog"),
    ];
    names.iter().enumerate().map(|(bit, (zh_name, en_name))| {
        let on = word & (1 << bit) != 0;
        coded(zh, zh_name, en_name, format!("bit{bit}={}", u8::from(on)), if on { if zh { "异常" } else { "fault" } } else { if zh { "正常" } else { "normal" } })
    }).collect()
}

fn ufcs_watchdog(data: &[u8], zh: bool) -> Vec<String> {
    if data.len() < 2 {
        return Vec::new();
    }
    let ms = u16::from_be_bytes([data[0], data[1]]);
    let text = if ms == 0 { if zh { "0 ms，关闭看门狗".to_string() } else { "0 ms, watchdog disabled".to_string() } } else { format!("{ms} ms") };
    vec![coded(zh, "看门狗时间", "Watchdog time", format!("0x{ms:04X} × 1 ms"), text)]
}

fn ufcs_refuse(data: &[u8], zh: bool) -> Vec<String> {
    if data.len() < 4 {
        return Vec::new();
    }
    let word = u32::from_be_bytes(data[..4].try_into().unwrap());
    let reason = (word & 255) as u8;
    let reason_text = match reason {
        0x01 => if zh { "无法识别的命令或数据" } else { "unrecognized command or data" },
        0x02 => if zh { "不支持的命令或数据" } else { "unsupported command or data" },
        0x03 => if zh { "设备忙" } else { "busy" },
        0x04 => if zh { "请求超出输出范围" } else { "request is outside the output range" },
        0x05 => if zh { "其它原因" } else { "other" },
        _ => if zh { "保留" } else { "reserved" },
    };
    vec![
        coded(zh, "被拒报文序号", "Refused message number", (word >> 24) & 15, (word >> 24) & 15),
        coded(zh, "被拒报文类型", "Refused message type", (word >> 16) & 7, (word >> 16) & 7),
        coded(zh, "被拒命令", "Refused command", format!("0x{:02X}", (word >> 8) & 255), format!("0x{:02X}", (word >> 8) & 255)),
        coded(zh, "拒绝原因", "Refuse reason", format!("0x{reason:02X}"), reason_text),
    ]
}

fn ufcs_command_meaning(kind: u16, command: u8, zh: bool) -> &'static str {
    if zh {
        return match (kind, command) {
            (0, 0) => "链路保活，不改变输出",
            (0, 1) => "确认刚收到的报文",
            (0, 2) => "否认刚收到的报文",
            (0, 3) => "接受对方的请求",
            (0, 4) => "复位协议状态，输出可以保持",
            (0, 5) => "输出已经稳定在请求的电压",
            (0, 6) => "请电源报出可提供的电压电流档",
            (0, 7) => "请电源报出当前输出和温度",
            (0, 8) => "请受电端报出电池和充电电流",
            (0, 9) => "读取线缆的阻抗和耐压",
            (0, 10) => "读取设备和芯片厂家",
            (0, 11) => "读取过压、过流等异常位",
            (0, 12) => "探测线缆是否在位",
            (0, 13) => "开始识别线缆",
            (0, 14) => "结束线缆识别",
            (0, 15) => "退出 UFCS，回到默认输出",
            (1, 1) => "每一档的电压、电流范围和步进",
            (1, 2) => "按某一档请求电压和电流",
            (1, 3) => "电源当前的输出电压、电流和温度",
            (1, 4) => "受电端的电池电压、充电电流和温度",
            (1, 5) => "线缆阻抗、耐压和最大电流",
            (1, 6) => "设备和芯片的厂家与版本",
            (1, 7) => "过压、过流、过温等位",
            (1, 8) => "看门狗超时，0 表示关闭",
            (1, 9) => "拒绝刚才的请求，并给出原因",
            (1, 10) => "带密钥编号和随机数的校验请求",
            (1, 11) => "校验响应",
            _ => "这份报文表没有该命令的字段定义",
        };
    }
    match (kind, command) {
        (0, 0) => "Keep the link up without changing the output",
        (0, 5) => "The output has settled at the requested voltage",
        (1, 1) => "Voltage, current range, and step of each mode",
        (1, 2) => "Request voltage and current from one mode",
        _ => "This command has no field table here",
    }
}

fn qi_field_label<'a>(name: &'a str, zh: bool) -> &'a str {
    if !zh {
        return name;
    }
    match name {
        "signal_strength" => "信号强度",
        "reason_code" => "结束原因码",
        "reason" => "结束原因",
        "received_power" => "接收功率",
        "p_rect" => "整流功率",
        "v_rect" => "整流电压",
        "ref_power" => "参考功率码",
        "ref_power_w" => "参考功率",
        "guaranteed_power" => "保证功率码",
        "guaranteed_power_w" => "保证功率",
        "potential_power" => "潜在功率码",
        "potential_power_w" => "潜在功率",
        "selector" => "选择位",
        "request" => "请求",
        "request_code" => "请求码",
        "request_type" => "请求类型",
        "response" => "应答",
        "response_code" => "应答码",
        "parameter" => "参数",
        "profile" => "功率档",
        "window_size" => "窗口大小",
        "window_offset" => "窗口偏移",
        "prmc" => "厂家码",
        "vendor" => "厂家",
        "crc16" => "CRC-16",
        "inverter_voltage" => "逆变器电压",
        "estimated_k" => "耦合系数",
        "alpha_fm_mohm" => "异物系数",
        "fsk_depth" => "FSK 调制深度",
        "buffer_size" => "缓冲区大小",
        "status" => "状态",
        "payload" => "载荷",
        "data" => "数据",
        "operation" => "操作",
        "report_id" => "报告号",
        "report_data" => "报告数据",
        "power_limit_reason" => "功率限制原因",
        "mode_cfg" => "模式配置",
        "preference" => "偏好",
        "main_mode" => "主模式",
        "clear_flag" => "清除标志",
        "calibration_action" => "校准动作",
        "packet_type" => "包类型",
        "pattern" => "图案",
        "reserved" => "保留",
        "ptx_id" => "发射端标识",
        "uid" => "唯一标识",
        _ => name,
    }
}

fn ufcs_command(kind: u16, command: u8, zh: bool) -> &'static str {
    if zh {
        return match (kind, command) {
            (0, 0) => "心跳",
            (0, 1) => "确认",
            (0, 2) => "否认",
            (0, 3) => "接受",
            (0, 4) => "软复位",
            (0, 5) => "电源就绪",
            (0, 6) => "读取输出能力",
            (0, 7) => "读取电源信息",
            (0, 8) => "读取受电信息",
            (0, 9) => "读取线缆信息",
            (0, 10) => "读取设备信息",
            (0, 11) => "读取错误信息",
            (0, 12) => "探测线缆",
            (0, 13) => "开始线缆识别",
            (0, 14) => "结束线缆识别",
            (0, 15) => "退出 UFCS",
            (1, 1) => "输出能力",
            (1, 2) => "功率请求",
            (1, 3) => "电源信息",
            (1, 4) => "受电信息",
            (1, 5) => "线缆信息",
            (1, 6) => "设备信息",
            (1, 7) => "异常信息",
            (1, 8) => "看门狗配置",
            (1, 9) => "拒绝",
            (1, 10) => "校验请求",
            (1, 11) => "校验响应",
            _ => "命令",
        };
    }
    match (kind, command) {
        (0, 0) => "PING",
        (0, 1) => "ACK",
        (0, 2) => "NCK",
        (0, 3) => "ACCEPT",
        (0, 5) => "POWER_READY",
        (0, 6) => "GET_OUTPUT_CAPABILITIES",
        (1, 2) => "REQUEST",
        _ => "command",
    }
}

fn explain_pd(frames: &[BusFrame], index: usize, zh: bool) -> Vec<String> {
    let frame = &frames[index];
    if frame.bytes.len() < 2 {
        return vec![frame.summary.clone(), if zh { "有序集，没有报文头。".into() } else { "Ordered set, no message header.".into() }];
    }
    let header = u16::from_le_bytes([frame.bytes[0], frame.bytes[1]]);
    let extended = header & 0x8000 != 0;
    let count = ((header >> 12) & 7) as usize;
    let kind = header & 31;
    let mut lines = vec![
        coded(zh, "同步码", "Sync", "SOP 4b5b", if zh { "K-code，位于 64 位 BMC 前导之后" } else { "K-codes after the 64-bit BMC preamble" }),
        coded(zh, "帧头", "Header", format!("0x{header:04X}"), if zh { "16 位小端消息头" } else { "16-bit little-endian message header" }),
        coded(zh, "编码", "Coding", "BMC", if zh { "规格位率约 300 kbps，本帧起止以解码结果为准" } else { "about 300 kbps; this frame uses the decoded interval" }),
        coded(zh, "报文", "Message", format!("0x{kind:02X}"), pd_name(header, zh)),
        coded(zh, "含义", "Meaning", pd_name(header, zh), pd_meaning(header, zh)),
        coded(zh, "扩展报文", "Extended", u8::from(extended), yes(extended, zh)),
        coded(zh, "数据对象数", "Data objects", count, if zh { format!("{count} 个 32 位对象") } else { format!("{count} 32-bit objects") }),
        coded(zh, "报文序号", "Message ID", (header >> 9) & 7, if zh { format!("{}，GoodCRC 用同一序号确认", (header >> 9) & 7) } else { format!("{}, GoodCRC confirms this number", (header >> 9) & 7) }),
        coded(zh, "电源角色", "Power role", format!("bit8={}", u8::from(header & 0x0100 != 0)), if header & 0x0100 != 0 { if zh { "Source，发送时端口在供电" } else { "Source, the port is supplying" } } else { if zh { "Sink，发送时端口在受电" } else { "Sink, the port is consuming" } }),
        coded(zh, "规范版本", "Spec revision", (header >> 6) & 3, if zh { ["1.0", "2.0", "3.0", "保留"][((header >> 6) & 3) as usize] } else { ["1.0", "2.0", "3.0", "reserved"][((header >> 6) & 3) as usize] }),
        coded(zh, "数据角色", "Data role", format!("bit5={}", u8::from(header & 0x20 != 0)), if header & 0x20 != 0 { if zh { "DFP，面向下游" } else { "DFP, downstream" } } else { if zh { "UFP，面向上游" } else { "UFP, upstream" } }),
    ];
    let payload_end = frame.bytes.len().saturating_sub(4);
    if extended && frame.bytes.len() >= 4 {
        let ext = u16::from_le_bytes([frame.bytes[2], frame.bytes[3]]);
        lines.push(coded(zh, "分块", "Chunked", u8::from(ext & 0x8000 != 0), yes(ext & 0x8000 != 0, zh)));
        lines.push(coded(zh, "块序号", "Chunk number", (ext >> 11) & 15, (ext >> 11) & 15));
        lines.push(coded(zh, "请求块", "Request chunk", u8::from(ext & 0x400 != 0), yes(ext & 0x400 != 0, zh)));
        lines.push(coded(zh, "数据长度", "Data size", ext & 0x1ff, if zh { format!("{} 字节", ext & 0x1ff) } else { format!("{} bytes", ext & 0x1ff) }));
        let size = (ext & 0x1ff) as usize;
        let data_end = (4 + size).min(frame.bytes.len().saturating_sub(4));
        if (ext & 0x8000 == 0 || (ext >> 11) & 15 == 0) && data_end > 4 {
            lines.extend(pd_extended(kind, &frame.bytes[4..data_end], zh));
        }
    }
    let objects = if extended { &[] } else { frame.bytes.get(2..payload_end).unwrap_or(&[]) };
    if !extended && header & 0x7000 != 0 {
        for (n, data) in objects.chunks_exact(4).enumerate() {
            let word = u32::from_le_bytes(data.try_into().unwrap());
            lines.extend(pd_object(kind, n, word, frames, index, zh));
        }
    }
        if frame.bytes.len() >= 6 {
        let received = u32::from_le_bytes(frame.bytes[payload_end..].try_into().unwrap_or([0, 0, 0, 0]));
        let expect = super::usb_pd::crc32(&frame.bytes[..payload_end]);
        lines.push(coded(zh, "CRC-32", "CRC-32", format!("收到 {received:08X} / 计算 {expect:08X}"), if received == expect { if zh { "一致" } else { "match" } } else { if zh { "不一致" } else { "mismatch" } }));
        lines.push(if zh {
            format!("CRC-32  收到 {received:08X}  计算 {expect:08X}  {}", if received == expect { "一致" } else { "不一致" })
        } else {
            format!("CRC-32  received {received:08X}  calculated {expect:08X}  {}", if received == expect { "match" } else { "mismatch" })
        });
    }
    if !frame.summary.contains("incomplete") && frame.bytes.len() >= 2 {
        lines.push(coded(zh, "结束码", "EOP", "K-code 13", if zh { "4b5b 结束符" } else { "4b5b end symbol" }));
    }
    lines
}

fn yes(value: bool, zh: bool) -> &'static str {
    if zh { if value { "是" } else { "否" } } else if value { "yes" } else { "no" }
}

fn pd_name(header: u16, zh: bool) -> &'static str {
    let kind = header & 31;
    let data = header & 0x7000 != 0 || header & 0x8000 != 0;
    if zh {
        if header & 0x8000 != 0 {
            return match kind {
                1 => "扩展源能力",
                2 => "状态",
                12 => "PPS 状态",
                15 => "扩展受电能力",
                _ => "扩展报文",
            };
        }
        return if data {
            match kind {
                1 => "源能力",
                2 => "功率请求",
                3 => "BIST",
                4 => "受电能力",
                15 => "厂商自定义",
                _ => "数据报文",
            }
        } else {
            match kind {
                1 => "GoodCRC",
                2 => "GotoMin",
                3 => "接受",
                4 => "拒绝",
                5 => "Ping",
                6 => "电源就绪",
                7 => "读取源能力",
                8 => "读取受电能力",
                9 => "数据角色交换",
                10 => "电源角色交换",
                11 => "VCONN 交换",
                12 => "等待",
                13 => "软复位",
                16 => "不支持",
                _ => "控制报文",
            }
        };
    }
    if header & 0x8000 != 0 {
        return match kind {
            1 => "Source_Capabilities_Extended",
            2 => "Status",
            12 => "PPS_Status",
            15 => "Sink_Capabilities_Extended",
            _ => "Extended",
        };
    }
    if data {
        match kind {
            1 => "Source_Capabilities",
            2 => "Request",
            3 => "BIST",
            4 => "Sink_Capabilities",
            15 => "Vendor_Defined",
            _ => "Data",
        }
    } else {
        match kind { 1 => "GoodCRC", 3 => "Accept", 4 => "Reject", 6 => "PS_RDY", 13 => "Soft_Reset", _ => "Control" }
    }
}

fn pd_object(kind: u16, index: usize, word: u32, frames: &[BusFrame], frame_index: usize, zh: bool) -> Vec<String> {
    let n = index + 1;
    if kind == 2 {
        return pd_request(word, frames, frame_index, zh);
    }
    if kind == 3 && index == 0 {
        let mode = word >> 28;
        let name = match mode {
            5 => if zh { "连续发送 BMC 载波，用来测物理层" } else { "continuous BMC carrier for the physical layer" },
            8 => if zh { "发送规定的测试数据图案" } else { "sends the specified test pattern" },
            _ => if zh { "保留" } else { "reserved" },
        };
        return vec![coded(zh, "BIST 模式", "BIST mode", format!("{mode:04b}"), name)];
    }
    if kind == 15 && index == 0 {
        return pd_vdm(word, zh);
    }
    if kind == 1 || kind == 4 {
        return pd_supply(n, word, kind == 4, zh);
    }
    vec![coded(zh, &format!("对象 {n}"), &format!("Object {n}"), format!("0x{word:08X}"), if zh { "这个报文类型没有公开的对象换算" } else { "this message type has no public object scale" })]
}

fn pd_meaning(header: u16, zh: bool) -> &'static str {
    let kind = header & 31;
    let extended = header & 0x8000 != 0;
    let data = header & 0x7000 != 0;
    if zh {
        if extended {
            return match kind {
                1 => "扩展源能力：厂家、版本和可提供功率",
                2 => "端口当前状态",
                12 => "PPS 正在输出的电压和电流",
                15 => "扩展受电能力",
                _ => "扩展报文，这一类没有更细的换算",
            };
        }
        return if data {
            match kind {
                1 => "源列出可提供的电压和电流档",
                2 => "受电端选定一档，并给出要的电流或功率",
                3 => "物理层测试，不用于供电",
                4 => "受电端列出能接受的电压和电流",
                15 => "厂商自定义报文，先看厂商号和命令",
                _ => "数据报文，对象按该类型解释",
            }
        } else {
            match kind {
                1 => "确认刚收到的那一帧，序号与被确认的帧相同",
                2 => "请源回到最小功率",
                3 => "接受对方刚才的请求",
                4 => "拒绝对方刚才的请求",
                5 => "保活，不改变输出",
                6 => "输出已经稳定在协商后的电压",
                7 => "请对方发送源能力",
                8 => "请对方发送受电能力",
                9 => "请求交换数据角色",
                10 => "请求交换电源角色",
                11 => "请求交换 VCONN",
                12 => "暂时不能处理，请稍后重试",
                13 => "复位协议状态，电源可以保持",
                16 => "不支持刚才的报文",
                _ => "控制报文，没有数据对象",
            }
        };
    }
    if extended {
        return match kind {
            1 => "Extended source capability: vendor, version, and power",
            12 => "Voltage and current the PPS supply is producing",
            _ => "Extended message without a finer scale",
        };
    }
    if data {
        match kind {
            1 => "The source lists the voltage and current it can supply",
            2 => "The sink picks one PDO and states the current or power it wants",
            3 => "Physical-layer test, not used to supply power",
            4 => "The sink lists the voltage and current it can accept",
            15 => "Vendor message; read the SVID and command",
            _ => "Data message; objects follow that message type",
        }
    } else {
        match kind {
            1 => "Acknowledges the previous frame and repeats its message number",
            3 => "Accepts the previous request",
            4 => "Rejects the previous request",
            6 => "The output has settled at the negotiated voltage",
            13 => "Resets the protocol state; power may stay on",
            _ => "Control message, no data objects",
        }
    }
}

fn pd_vdm(word: u32, zh: bool) -> Vec<String> {
    let svid = word >> 16;
    let structured = word & 0x8000 != 0;
    let command = word & 31;
    let command_type = (word >> 6) & 3;
    let position = (word >> 8) & 7;
    let svid_text = if svid == 0xff00 {
        if zh { "0xFF00，USB-IF 标准厂商号" } else { "0xFF00, the USB-IF standard vendor ID" }
    } else if zh {
        "厂商号"
    } else {
        "vendor ID"
    };
    let command_text = match command {
        1 => if zh { "发现身份" } else { "Discover Identity" },
        2 => if zh { "发现 SVID" } else { "Discover SVIDs" },
        3 => if zh { "发现模式" } else { "Discover Modes" },
        4 => if zh { "进入模式" } else { "Enter Mode" },
        5 => if zh { "退出模式" } else { "Exit Mode" },
        6 => if zh { "注意" } else { "Attention" },
        _ => if zh { "厂商命令" } else { "vendor command" },
    };
    let type_text = match command_type {
        0 => if zh { "请求" } else { "request" },
        1 => if zh { "确认" } else { "ACK" },
        2 => if zh { "否认" } else { "NAK" },
        _ => if zh { "忙" } else { "busy" },
    };
    vec![
        coded(zh, "厂商号", "SVID", format!("0x{svid:04X}"), svid_text),
        coded(zh, "结构化 VDM", "Structured VDM", format!("bit15={}", u8::from(structured)), if structured { if zh { "按标准命令头解释" } else { "standard command header" } } else { if zh { "后面是厂商自定义数据" } else { "the rest is vendor data" } }),
        coded(zh, "VDM 版本", "VDM version", (word >> 13) & 3, match (word >> 13) & 3 { 0 => "1.0", 1 => "2.0", _ => if zh { "保留" } else { "reserved" } }),
        coded(zh, "对象位置", "Object position", format!("{position:03b}"), position),
        coded(zh, "命令类型", "Command type", format!("{command_type:02b}"), type_text),
        coded(zh, "VDM 命令", "VDM command", command, command_text),
    ]
}

fn pd_extended(kind: u16, data: &[u8], zh: bool) -> Vec<String> {
    if kind == 12 && data.len() >= 3 {
        let voltage = u16::from_le_bytes([data[0], data[1]]) as f64 * 0.02;
        let current = data[2] as f64 * 0.05;
        let v_code = u16::from_le_bytes([data[0], data[1]]);
        return vec![
            coded(zh, "输出电压", "Output voltage", format!("0x{v_code:04X} × 20 mV"), format!("{voltage:.2} V")),
            coded(zh, "输出电流", "Output current", format!("0x{:02X} × 50 mA", data[2]), format!("{current:.2} A")),
        ];
    }
    if kind == 1 && data.len() >= 4 {
        let vid = u16::from_le_bytes([data[0], data[1]]);
        let pid = u16::from_le_bytes([data[2], data[3]]);
        let mut lines = vec![
            coded(zh, "厂商 ID", "Vendor ID", format!("{:02X} {:02X}", data[0], data[1]), format!("0x{vid:04X}")),
            coded(zh, "产品 ID", "Product ID", format!("{:02X} {:02X}", data[2], data[3]), format!("0x{pid:04X}")),
        ];
        if data.len() >= 10 {
            lines.push(field(zh, "固件版本", "Firmware version", format!("0x{:02X}", data[8])));
            lines.push(field(zh, "硬件版本", "Hardware version", format!("0x{:02X}", data[9])));
        }
        if data.len() >= 24 {
            lines.push(field(zh, "源输入", "Source inputs", format!("0x{:02X}", data[21])));
            lines.push(coded(zh, "SPR 功率", "SPR PDP", format!("0x{:02X}", data[23]), format!("{} W", data[23])));
        }
        return lines;
    }
    Vec::new()
}

fn pd_supply(index: usize, word: u32, sink: bool, zh: bool) -> Vec<String> {
    match word >> 30 {
        0 => {
            let v_code = (word >> 10) & 1023;
            let i_code = word & 1023;
            let voltage = v_code as f64 * 0.05;
            let current = i_code as f64 * 0.01;
            let on = |bit: u32| u8::from(word & 1 << bit != 0);
            let mut lines = vec![
                coded(zh, &format!("PDO {index} 类型"), &format!("PDO {index} type"), format!("{:02b}", word >> 30), if zh { "固定电源" } else { "fixed supply" }),
                coded(zh, &format!("PDO {index} 原码"), &format!("PDO {index} raw"), format!("0x{word:08X}"), if zh { "32 位小端" } else { "32-bit little-endian" }),
                coded(zh, &format!("PDO {index} 电压"), &format!("PDO {index} voltage"), format!("0x{v_code:03X} × 50 mV"), format!("{voltage:.2} V")),
                coded(
                    zh,
                    &format!("PDO {index} {}", if sink { "工作电流" } else { "最大电流" }),
                    &format!("PDO {index} {}", if sink { "operating current" } else { "maximum current" }),
                    format!("0x{i_code:03X} × 10 mA"),
                    format!("{current:.2} A"),
                ),
                coded(zh, &format!("PDO {index} 双角色电源"), &format!("PDO {index} dual-role power"), format!("bit29={}", on(29)), yes(word & 1 << 29 != 0, zh)),
                coded(
                    zh,
                    &format!("PDO {index} {}", if sink { "更高能力" } else { "USB 挂起" }),
                    &format!("PDO {index} {}", if sink { "higher capability" } else { "USB suspend" }),
                    format!("bit28={}", on(28)),
                    if sink {
                        if word & 1 << 28 != 0 { if zh { "还需要更高电压" } else { "a higher voltage is required" } } else { if zh { "当前电压即可" } else { "this voltage is enough" } }
                    } else {
                        yes(word & 1 << 28 != 0, zh)
                    },
                ),
                coded(zh, &format!("PDO {index} 不受限功率"), &format!("PDO {index} unconstrained power"), format!("bit27={}", on(27)), yes(word & 1 << 27 != 0, zh)),
                coded(zh, &format!("PDO {index} USB 通信"), &format!("PDO {index} USB communications"), format!("bit26={}", on(26)), yes(word & 1 << 26 != 0, zh)),
                coded(zh, &format!("PDO {index} 双角色数据"), &format!("PDO {index} dual-role data"), format!("bit25={}", on(25)), yes(word & 1 << 25 != 0, zh)),
            ];
            if sink {
                let frs = (word >> 23) & 3;
                let frs_text = match frs {
                    0 => if zh { "不支持快速角色交换" } else { "fast role swap is not supported" },
                    1 => if zh { "默认 USB 电流" } else { "default USB current" },
                    2 => "1.5 A",
                    _ => "3.0 A",
                };
                lines.push(coded(zh, &format!("PDO {index} 快速角色交换"), &format!("PDO {index} fast role swap"), format!("{frs:02b}"), frs_text));
            } else {
                let peak = match (word >> 20) & 3 {
                    0 => "100%",
                    1 => "130%",
                    2 => "150%",
                    _ => "200%",
                };
                lines.push(coded(zh, &format!("PDO {index} 不分块扩展"), &format!("PDO {index} unchunked extended"), format!("bit24={}", on(24)), if zh { "PD 3.0 可以不分块发送扩展报文" } else { "PD 3.0 may send extended messages unchunked" }));
                lines.push(coded(zh, &format!("PDO {index} EPR"), &format!("PDO {index} EPR"), format!("bit23={}", on(23)), if word & 1 << 23 != 0 { if zh { "支持扩展功率范围" } else { "extended power range" } } else { if zh { "不支持扩展功率范围" } else { "no extended power range" } }));
                lines.push(coded(zh, &format!("PDO {index} 峰值电流"), &format!("PDO {index} peak current"), format!("{:02b}", (word >> 20) & 3), if zh { format!("{peak}，短时过载相对最大电流") } else { format!("{peak} of the maximum current for a short overload") }));
            }
            lines
        }
        1 => {
            let max_c = (word >> 20) & 1023;
            let min_c = (word >> 10) & 1023;
            let power_c = word & 1023;
            let max_v = max_c as f64 * 0.05;
            let min_v = min_c as f64 * 0.05;
            let power = power_c as f64 * 0.25;
            vec![
                field(zh, &format!("PDO {index} 类型"), &format!("PDO {index} type"), if zh { "电池" } else { "battery" }),
                coded(zh, &format!("PDO {index} 原码"), &format!("PDO {index} raw"), format!("0x{word:08X}"), if zh { "32 位小端" } else { "32-bit little-endian" }),
                coded(zh, &format!("PDO {index} 最低电压"), &format!("PDO {index} minimum voltage"), format!("0x{min_c:03X} × 50 mV"), format!("{min_v:.2} V")),
                coded(zh, &format!("PDO {index} 最高电压"), &format!("PDO {index} maximum voltage"), format!("0x{max_c:03X} × 50 mV"), format!("{max_v:.2} V")),
                coded(zh, &format!("PDO {index} {}", if sink { "工作功率" } else { "最大功率" }), &format!("PDO {index} {}", if sink { "operating power" } else { "maximum power" }), format!("0x{power_c:03X} × 250 mW"), format!("{power:.2} W")),
            ]
        }
        2 => {
            let max_c = (word >> 20) & 1023;
            let min_c = (word >> 10) & 1023;
            let i_code = word & 1023;
            let max_v = max_c as f64 * 0.05;
            let min_v = min_c as f64 * 0.05;
            let current = i_code as f64 * 0.01;
            vec![
                field(zh, &format!("PDO {index} 类型"), &format!("PDO {index} type"), if zh { "可变电源" } else { "variable supply" }),
                coded(zh, &format!("PDO {index} 原码"), &format!("PDO {index} raw"), format!("0x{word:08X}"), if zh { "32 位小端" } else { "32-bit little-endian" }),
                coded(zh, &format!("PDO {index} 最低电压"), &format!("PDO {index} minimum voltage"), format!("0x{min_c:03X} × 50 mV"), format!("{min_v:.2} V")),
                coded(zh, &format!("PDO {index} 最高电压"), &format!("PDO {index} maximum voltage"), format!("0x{max_c:03X} × 50 mV"), format!("{max_v:.2} V")),
                coded(zh, &format!("PDO {index} {}", if sink { "工作电流" } else { "最大电流" }), &format!("PDO {index} {}", if sink { "operating current" } else { "maximum current" }), format!("0x{i_code:03X} × 10 mA"), format!("{current:.2} A")),
            ]
        }
        _ if word >> 28 == 0b1100 => {
            let max_c = (word >> 17) & 255;
            let min_c = (word >> 8) & 255;
            let i_code = word & 127;
            let max_v = max_c as f64 * 0.1;
            let min_v = min_c as f64 * 0.1;
            let current = i_code as f64 * 0.05;
            vec![
                field(zh, &format!("PDO {index} 类型"), &format!("PDO {index} type"), "PPS"),
                coded(zh, &format!("PDO {index} 原码"), &format!("PDO {index} raw"), format!("0x{word:08X}"), "APDO"),
                coded(zh, &format!("PDO {index} 最低电压"), &format!("PDO {index} minimum voltage"), format!("0x{min_c:02X} × 100 mV"), format!("{min_v:.1} V")),
                coded(zh, &format!("PDO {index} 最高电压"), &format!("PDO {index} maximum voltage"), format!("0x{max_c:02X} × 100 mV"), format!("{max_v:.1} V")),
                coded(zh, &format!("PDO {index} 最大电流"), &format!("PDO {index} maximum current"), format!("0x{i_code:02X} × 50 mA"), format!("{current:.2} A")),
            ]
        }
        _ => vec![field(zh, &format!("PDO {index}"), &format!("PDO {index}"), format!("0x{word:08X}"))],
    }
}

fn field(zh: bool, zh_name: &str, en_name: &str, value: impl std::fmt::Display) -> String {
    if zh { format!("{zh_name}  {value}") } else { format!("{en_name}  {value}") }
}

/// `码 {code}  →  {physical}` so the page can show the raw code and the scaled value together.
fn coded(zh: bool, zh_name: &str, en_name: &str, code: impl std::fmt::Display, physical: impl std::fmt::Display) -> String {
    let name = if zh { zh_name } else { en_name };
    format!("{name}  码 {code}  →  {physical}")
}

fn pd_request(word: u32, frames: &[BusFrame], index: usize, zh: bool) -> Vec<String> {
    let object = (word >> 28) as usize;
    let supply = frames[..index].iter().rev().find_map(|frame| referenced_supply(frame, object));
    let mut lines = vec![coded(zh, "选择对象", "Object position", format!("{object:04b}"), object)];
    if supply == Some(SupplyKind::Pps) {
        let v_code = (word >> 9) & 0x7ff;
        let i_code = word & 0x7f;
        let voltage = v_code as f64 * 0.02;
        let current = i_code as f64 * 0.05;
        lines.push(coded(zh, "请求电压", "Output voltage", format!("0x{v_code:03X} × 20 mV"), format!("{voltage:.2} V")));
        lines.push(coded(zh, "工作电流", "Operating current", format!("0x{i_code:02X} × 50 mA"), format!("{current:.2} A")));
    } else {
        let op_code = (word >> 10) & 1023;
        let max_code = word & 1023;
        let operate = op_code as f64 * 0.01;
        let maximum = max_code as f64 * 0.01;
        if supply == Some(SupplyKind::Battery) {
            lines.push(coded(zh, "工作功率", "Operating power", format!("0x{op_code:03X} × 250 mW"), format!("{:.2} W", op_code as f64 * 0.25)));
            lines.push(coded(zh, "最大功率", "Maximum power", format!("0x{max_code:03X} × 250 mW"), format!("{:.2} W", max_code as f64 * 0.25)));
        } else {
            lines.push(coded(zh, "工作电流", "Operating current", format!("0x{op_code:03X} × 10 mA"), format!("{operate:.2} A")));
            lines.push(coded(zh, "最大电流", "Maximum current", format!("0x{max_code:03X} × 10 mA"), format!("{maximum:.2} A")));
            lines.push(coded(zh, "归还功率", "GiveBack", u8::from(word & 1 << 27 != 0), yes(word & 1 << 27 != 0, zh)));
        }
    }
    lines.push(coded(zh, "能力不匹配", "Capability mismatch", format!("bit26={}", u8::from(word & 1 << 26 != 0)), if word & 1 << 26 != 0 { if zh { "所选档不能满足负载" } else { "the selected PDO cannot satisfy the load" } } else { if zh { "能力匹配" } else { "capability matches" } }));
    lines.push(coded(zh, "USB 通信", "USB communications", format!("bit25={}", u8::from(word & 1 << 25 != 0)), yes(word & 1 << 25 != 0, zh)));
    lines.push(coded(zh, "禁止 USB 挂起", "No USB suspend", format!("bit24={}", u8::from(word & 1 << 24 != 0)), yes(word & 1 << 24 != 0, zh)));
    lines
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SupplyKind { Fixed, Battery, Variable, Pps }

fn referenced_supply(frame: &BusFrame, object: usize) -> Option<SupplyKind> {
    if frame.bytes.len() < 6 { return None; }
    let header = u16::from_le_bytes([frame.bytes[0], frame.bytes[1]]);
    if header & 31 != 1 || header & 0x7000 == 0 { return None; }
    let end = frame.bytes.len().saturating_sub(4);
    let data = frame.bytes.get(2..end)?;
    let word = data.chunks_exact(4).nth(object.saturating_sub(1))?;
    let pdo = u32::from_le_bytes(word.try_into().ok()?);
    Some(match pdo >> 30 {
        0 => SupplyKind::Fixed,
        1 => SupplyKind::Battery,
        2 => SupplyKind::Variable,
        _ if pdo >> 28 == 0b1100 => SupplyKind::Pps,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_capability_pdo_is_five_volts_three_amps() {
        let bytes = vec![0x01, 0x10, 0x2c, 0x91, 0x01, 0x00, 0, 0, 0, 0];
        let frame = BusFrame { bytes, summary: "Source_Capabilities".into(), ..Default::default() };
        let lines = explain_frame(BusKind::UsbPd, &[frame], 0, true);
        let text = lines.join("\n");
        assert!(text.contains("5.00 V"), "{text}");
        assert!(text.contains("3.00 A"), "{text}");
        assert!(text.contains("源能力"), "{text}");
        assert!(text.contains("源列出可提供的电压和电流档"), "{text}");
        assert!(text.contains("bit29="), "{text}");
    }

    #[test]
    fn ufcs_output_capability_uses_ten_millivolt_steps() {
        let word = (1u64 << 60) | (1100u64 << 40) | (500u64 << 24) | (300u64 << 8) | 100;
        let mut data = word.to_be_bytes().to_vec();
        let mut bytes = vec![0xaa, 0x40, 0x09, 0x01, 8];
        bytes.append(&mut data);
        bytes.push(0);
        let frame = BusFrame { bytes, summary: "UFCS".into(), ..Default::default() };
        let text = explain_frame(BusKind::Ufcs, &[frame], 0, true).join("\n");
        assert!(text.contains("11.00 V"), "{text}");
        assert!(text.contains("5.00 V"), "{text}");
        assert!(text.contains("3.00 A"), "{text}");
        assert!(text.contains("1.00 A"), "{text}");
        assert!(text.contains("10 mV"), "{text}");
        assert!(text.contains("10 mA"), "{text}");
    }
}
