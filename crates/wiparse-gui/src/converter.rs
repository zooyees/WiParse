use crate::theme::{self as ui_theme, Tokens};
use egui::{Color32, CornerRadius, Frame, Margin, Stroke};
use std::f64::consts::{E, PI};
use wiparse_core::i18n::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConverterTab {
    Ascii,
    Radix,
    Scientific,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteDirection {
    TextToBytes,
    BytesToText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteBase {
    Decimal,
    Hex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AngleMode {
    Radians,
    Degrees,
}

fn parse_byte_sequence(input: &str, base: ByteBase) -> Result<Vec<u8>, String> {
    let normalized = input.replace([',', ';'], " ");
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    if tokens.is_empty() {
        return Err("输入不能为空 / Input cannot be empty".into());
    }
    tokens
        .into_iter()
        .map(|token| {
            let (digits, radix) = if let Some(rest) = token
                .strip_prefix("0x")
                .or_else(|| token.strip_prefix("0X"))
            {
                (rest, 16)
            } else {
                (
                    token,
                    match base {
                        ByteBase::Decimal => 10,
                        ByteBase::Hex => 16,
                    },
                )
            };
            if digits.is_empty() {
                return Err(format!("无效字节 / Invalid byte: {token}"));
            }
            let value = u16::from_str_radix(digits, radix)
                .map_err(|_| format!("无效字节 / Invalid byte: {token}"))?;
            u8::try_from(value).map_err(|_| format!("字节超出 0..255 / Byte out of range: {token}"))
        })
        .collect()
}

fn bytes_to_utf8(input: &str, base: ByteBase) -> Result<String, String> {
    let bytes = parse_byte_sequence(input, base)?;
    String::from_utf8(bytes).map_err(|_| "字节不是有效 UTF-8 / Invalid UTF-8 byte sequence".into())
}

fn utf8_to_bytes(input: &str, base: ByteBase) -> Result<String, String> {
    if input.is_empty() {
        return Err("输入不能为空 / Input cannot be empty".into());
    }
    Ok(input
        .as_bytes()
        .iter()
        .map(|byte| match base {
            ByteBase::Decimal => byte.to_string(),
            ByteBase::Hex => format!("{byte:02X}"),
        })
        .collect::<Vec<_>>()
        .join(" "))
}

fn parse_i128_radix(input: &str, source_base: u32) -> Result<i128, String> {
    if !(2..=36).contains(&source_base) {
        return Err("源进制必须在 2..36 / Source base must be 2..36".into());
    }
    let compact: String = input
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '_')
        .collect();
    if compact.is_empty() {
        return Err("输入不能为空 / Input cannot be empty".into());
    }
    let (negative, unsigned) = if let Some(rest) = compact.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = compact.strip_prefix('+') {
        (false, rest)
    } else {
        (false, compact.as_str())
    };
    let digits = match source_base {
        2 => unsigned
            .strip_prefix("0b")
            .or_else(|| unsigned.strip_prefix("0B"))
            .unwrap_or(unsigned),
        8 => unsigned
            .strip_prefix("0o")
            .or_else(|| unsigned.strip_prefix("0O"))
            .unwrap_or(unsigned),
        16 => unsigned
            .strip_prefix("0x")
            .or_else(|| unsigned.strip_prefix("0X"))
            .unwrap_or(unsigned),
        _ => unsigned,
    };
    if digits.is_empty() {
        return Err("缺少数字 / Missing digits".into());
    }
    let magnitude = u128::from_str_radix(digits, source_base).map_err(|_| {
        "数字与源进制不匹配或超出 i128 / Invalid digit or i128 overflow".to_string()
    })?;
    if negative {
        let min_magnitude = 1_u128 << 127;
        if magnitude == min_magnitude {
            Ok(i128::MIN)
        } else if magnitude <= i128::MAX as u128 {
            Ok(-(magnitude as i128))
        } else {
            Err("超出 i128 范围 / i128 overflow".into())
        }
    } else if magnitude <= i128::MAX as u128 {
        Ok(magnitude as i128)
    } else {
        Err("超出 i128 范围 / i128 overflow".into())
    }
}

fn format_i128_radix(value: i128, target_base: u32) -> Result<String, String> {
    if !(2..=36).contains(&target_base) {
        return Err("目标进制必须在 2..36 / Target base must be 2..36".into());
    }
    if value == 0 {
        return Ok("0".into());
    }
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let negative = value < 0;
    let mut magnitude = value.unsigned_abs();
    let mut output = Vec::new();
    while magnitude > 0 {
        output.push(DIGITS[(magnitude % target_base as u128) as usize] as char);
        magnitude /= target_base as u128;
    }
    if negative {
        output.push('-');
    }
    output.reverse();
    Ok(output.into_iter().collect())
}

fn convert_radix(input: &str, source_base: u32, target_base: u32) -> Result<String, String> {
    format_i128_radix(parse_i128_radix(input, source_base)?, target_base)
}

struct ExprParser<'a> {
    input: &'a str,
    position: usize,
    angle_mode: AngleMode,
}

impl<'a> ExprParser<'a> {
    fn new(input: &'a str, angle_mode: AngleMode) -> Self {
        Self {
            input,
            position: 0,
            angle_mode,
        }
    }

    fn parse(mut self) -> Result<f64, String> {
        let value = self.expression()?;
        self.skip_space();
        if self.position != self.input.len() {
            return Err(format!(
                "尾随字符 / Unexpected trailing input: {}",
                &self.input[self.position..]
            ));
        }
        finite(value)
    }

    fn expression(&mut self) -> Result<f64, String> {
        let mut value = self.term()?;
        loop {
            if self.consume('+') {
                value = finite(value + self.term()?)?;
            } else if self.consume('-') {
                value = finite(value - self.term()?)?;
            } else {
                return Ok(value);
            }
        }
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut value = self.unary()?;
        loop {
            if self.consume('*') {
                value = finite(value * self.unary()?)?;
            } else if self.consume('/') {
                let divisor = self.unary()?;
                if divisor == 0.0 {
                    return Err("除数不能为零 / Division by zero".into());
                }
                value = finite(value / divisor)?;
            } else if self.consume('%') {
                let divisor = self.unary()?;
                if divisor == 0.0 {
                    return Err("模数不能为零 / Modulo by zero".into());
                }
                value = finite(value % divisor)?;
            } else {
                return Ok(value);
            }
        }
    }

    fn unary(&mut self) -> Result<f64, String> {
        if self.consume('+') {
            self.unary()
        } else if self.consume('-') {
            finite(-self.unary()?)
        } else {
            self.power()
        }
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.primary()?;
        if self.consume('^') {
            finite(base.powf(self.unary()?))
        } else {
            Ok(base)
        }
    }

    fn primary(&mut self) -> Result<f64, String> {
        self.skip_space();
        if self.consume('(') {
            let value = self.expression()?;
            if !self.consume(')') {
                return Err("括号不匹配 / Missing ')'".into());
            }
            return Ok(value);
        }
        if self
            .peek()
            .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
        {
            return self.number();
        }
        if self
            .peek()
            .is_some_and(|ch| ch.is_alphabetic() || ch == 'π')
        {
            let name = self.identifier();
            return match name.as_str() {
                "pi" | "π" => Ok(PI),
                "e" => Ok(E),
                _ => {
                    if !self.consume('(') {
                        return Err(format!("未知常量或函数 / Unknown name: {name}"));
                    }
                    let argument = self.expression()?;
                    if !self.consume(')') {
                        return Err("括号不匹配 / Missing ')'".into());
                    }
                    self.function(&name, argument)
                }
            };
        }
        Err("表达式语法错误 / Expression syntax error".into())
    }

    fn number(&mut self) -> Result<f64, String> {
        self.skip_space();
        let start = self.position;
        if self.input[self.position..].starts_with("0x")
            || self.input[self.position..].starts_with("0X")
        {
            self.position += 2;
            let hex_start = self.position;
            while self
                .peek()
                .is_some_and(|ch| ch.is_ascii_hexdigit())
            {
                self.position += self.peek().unwrap().len_utf8();
            }
            if hex_start == self.position {
                return Err("十六进制数字无效 / Invalid hex number".into());
            }
            return parse_hex_digits(&self.input[hex_start..self.position]);
        }

        let mut has_hex_alpha = false;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() || ch == '.' {
                self.position += ch.len_utf8();
            } else if matches!(ch, 'a'..='d' | 'f' | 'A'..='D' | 'F') {
                has_hex_alpha = true;
                self.position += ch.len_utf8();
            } else {
                break;
            }
        }
        if self.position == start {
            return Err("数字无效 / Invalid number".into());
        }

        if matches!(self.peek(), Some('h') | Some('H')) {
            let digits = &self.input[start..self.position];
            self.position += 1;
            return parse_hex_digits(digits);
        }

        if !has_hex_alpha {
            let bytes = self.input.as_bytes();
            if self.position < bytes.len() && matches!(bytes[self.position], b'e' | b'E') {
                self.position += 1;
                if self.position < bytes.len() && matches!(bytes[self.position], b'+' | b'-') {
                    self.position += 1;
                }
                let exponent_start = self.position;
                while self.position < bytes.len() && bytes[self.position].is_ascii_digit() {
                    self.position += 1;
                }
                if exponent_start == self.position {
                    return Err("科学计数法指数无效 / Invalid exponent".into());
                }
            }
        }

        self.input[start..self.position]
            .parse::<f64>()
            .map_err(|_| "数字无效 / Invalid number".into())
            .and_then(finite)
    }

    fn identifier(&mut self) -> String {
        self.skip_space();
        let start = self.position;
        for (offset, ch) in self.input[self.position..].char_indices() {
            if ch.is_alphabetic() || ch.is_ascii_digit() || ch == 'π' {
                self.position = start + offset + ch.len_utf8();
            } else {
                break;
            }
        }
        self.input[start..self.position].to_ascii_lowercase()
    }

    fn function(&self, name: &str, value: f64) -> Result<f64, String> {
        let radians = |argument: f64| match self.angle_mode {
            AngleMode::Radians => argument,
            AngleMode::Degrees => argument.to_radians(),
        };
        let inverse = |result: f64| match self.angle_mode {
            AngleMode::Radians => result,
            AngleMode::Degrees => result.to_degrees(),
        };
        let output = match name {
            "sqrt" if value >= 0.0 => value.sqrt(),
            "sqrt" => return Err("sqrt 定义域要求 x≥0 / sqrt domain is x>=0".into()),
            "abs" => value.abs(),
            "exp" => value.exp(),
            "ln" if value > 0.0 => value.ln(),
            "ln" => return Err("ln 定义域要求 x>0 / ln domain is x>0".into()),
            "log" | "log10" if value > 0.0 => value.log10(),
            "log" | "log10" => return Err("log10 定义域要求 x>0 / log10 domain is x>0".into()),
            "log2" if value > 0.0 => value.log2(),
            "log2" => return Err("log2 定义域要求 x>0 / log2 domain is x>0".into()),
            "sin" => radians(value).sin(),
            "cos" => radians(value).cos(),
            "tan" => {
                let angle = radians(value);
                if angle.cos().abs() < 1.0e-12 {
                    return Err("tan 在该角度无定义 / tan is undefined at this angle".into());
                }
                angle.tan()
            }
            "asin" if (-1.0..=1.0).contains(&value) => inverse(value.asin()),
            "asin" => return Err("asin 定义域为 [-1,1] / asin domain is [-1,1]".into()),
            "acos" if (-1.0..=1.0).contains(&value) => inverse(value.acos()),
            "acos" => return Err("acos 定义域为 [-1,1] / acos domain is [-1,1]".into()),
            "atan" => inverse(value.atan()),
            "floor" => value.floor(),
            "ceil" => value.ceil(),
            _ => return Err(format!("未知函数 / Unknown function: {name}")),
        };
        finite(output)
    }

    fn consume(&mut self, expected: char) -> bool {
        self.skip_space();
        if self.peek() == Some(expected) {
            self.position += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.position..].chars().next()
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.position += self.peek().unwrap().len_utf8();
        }
    }
}

fn finite(value: f64) -> Result<f64, String> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err("结果不是有限值 / Result is not finite".into())
    }
}

fn parse_hex_digits(digits: &str) -> Result<f64, String> {
    let clean: String = digits.chars().filter(|c| *c != '_').collect();
    if clean.is_empty() {
        return Err("十六进制数字无效 / Invalid hex number".into());
    }
    if !clean.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("十六进制数字无效 / Invalid hex number".into());
    }
    u128::from_str_radix(&clean, 16)
        .map(|value| value as f64)
        .map_err(|_| "十六进制超出范围 / Hex value out of range".into())
        .and_then(finite)
}

/// Parse a numeric literal: `0x`/`0X` prefix or `H`/`h` suffix => hex; otherwise decimal.
pub(crate) fn parse_numeric_literal(text: &str) -> Result<f64, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("数字不能为空 / Number cannot be empty".into());
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return parse_hex_digits(hex);
    }
    if trimmed.len() >= 2 {
        let last = trimmed.as_bytes()[trimmed.len() - 1];
        if last == b'H' || last == b'h' {
            let body = &trimmed[..trimmed.len() - 1];
            if !body.is_empty() {
                return parse_hex_digits(body);
            }
        }
    }
    trimmed
        .parse::<f64>()
        .map_err(|_| "数字无效 / Invalid number".into())
        .and_then(finite)
}

fn evaluate_expression(input: &str, angle_mode: AngleMode) -> Result<f64, String> {
    if input.trim().is_empty() {
        return Err("表达式不能为空 / Expression cannot be empty".into());
    }
    ExprParser::new(input, angle_mode).parse()
}

pub struct ConverterPanel {
    tab: ConverterTab,
    byte_direction: ByteDirection,
    byte_base: ByteBase,
    byte_input: String,
    byte_result: String,
    byte_error: Option<String>,
    radix_input: String,
    source_base: u32,
    target_base: u32,
    radix_result: String,
    radix_error: Option<String>,
    expression: String,
    angle_mode: AngleMode,
    expression_result: String,
    expression_error: Option<String>,
}

impl ConverterPanel {
    pub fn new() -> Self {
        Self {
            tab: ConverterTab::Ascii,
            byte_direction: ByteDirection::TextToBytes,
            byte_base: ByteBase::Decimal,
            byte_input: "Hello".into(),
            byte_result: String::new(),
            byte_error: None,
            radix_input: "255".into(),
            source_base: 10,
            target_base: 16,
            radix_result: String::new(),
            radix_error: None,
            expression: "sin(pi/2)+log10(100)+sqrt(9)".into(),
            angle_mode: AngleMode::Radians,
            expression_result: String::new(),
            expression_error: None,
        }
    }

    pub(crate) fn api_get(&self) -> serde_json::Value {
        serde_json::json!({
            "tab": format!("{:?}", self.tab),
            "radix_input": self.radix_input,
            "source_base": self.source_base,
            "target_base": self.target_base,
            "radix_result": self.radix_result,
            "expression": self.expression,
            "expression_result": self.expression_result,
            "byte_input": self.byte_input,
            "byte_result": self.byte_result,
        })
    }

    pub(crate) fn api_set(&mut self, fields: &serde_json::Value) {
        if let Some(v) = fields.get("radix_input").and_then(|x| x.as_str()) {
            self.radix_input = v.to_string();
        }
        if let Some(v) = fields.get("source_base").and_then(|x| x.as_u64()) {
            self.source_base = v.clamp(2, 36) as u32;
        }
        if let Some(v) = fields.get("target_base").and_then(|x| x.as_u64()) {
            self.target_base = v.clamp(2, 36) as u32;
        }
        if let Some(v) = fields.get("expression").and_then(|x| x.as_str()) {
            self.expression = v.to_string();
        }
        if let Some(v) = fields.get("byte_input").and_then(|x| x.as_str()) {
            self.byte_input = v.to_string();
        }
        if let Some(tab) = fields.get("tab").and_then(|x| x.as_str()) {
            self.tab = match tab.to_ascii_lowercase().as_str() {
                "ascii" | "byte" => ConverterTab::Ascii,
                "radix" => ConverterTab::Radix,
                "scientific" | "expr" => ConverterTab::Scientific,
                _ => self.tab,
            };
        }
        match self.tab {
            ConverterTab::Radix => match convert_radix(&self.radix_input, self.source_base, self.target_base)
            {
                Ok(r) => {
                    self.radix_result = r;
                    self.radix_error = None;
                }
                Err(e) => {
                    self.radix_result.clear();
                    self.radix_error = Some(e);
                }
            },
            ConverterTab::Scientific => match evaluate_expression(&self.expression, self.angle_mode) {
                Ok(v) => {
                    self.expression_result = format!("{v}");
                    self.expression_error = None;
                }
                Err(e) => {
                    self.expression_result.clear();
                    self.expression_error = Some(e);
                }
            },
            ConverterTab::Ascii => {
                let result = match self.byte_direction {
                    ByteDirection::TextToBytes => utf8_to_bytes(&self.byte_input, self.byte_base),
                    ByteDirection::BytesToText => bytes_to_utf8(&self.byte_input, self.byte_base),
                };
                match result {
                    Ok(r) => {
                        self.byte_result = r;
                        self.byte_error = None;
                    }
                    Err(e) => {
                        self.byte_result.clear();
                        self.byte_error = Some(e);
                    }
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        ui.heading(
            egui::RichText::new(text(
                lang,
                "6. 转换与科学计算",
                "6. Conversion & Scientific",
            ))
            .size(16.0)
            .color(t.text_primary),
        );
        ui.label(
            egui::RichText::new(text(
                lang,
                "UTF-8 字节、整数进制与安全表达式计算",
                "UTF-8 bytes, integer radix, and safe expressions",
            ))
            .small()
            .color(t.text_muted),
        );
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, ConverterTab::Ascii, "ASCII/UTF-8");
            ui.selectable_value(
                &mut self.tab,
                ConverterTab::Radix,
                text(lang, "进制转换", "Radix"),
            );
            ui.selectable_value(
                &mut self.tab,
                ConverterTab::Scientific,
                text(lang, "科学计算", "Scientific"),
            );
        });
        ui.separator();
        match self.tab {
            ConverterTab::Ascii => self.ascii_ui(ui, lang, t),
            ConverterTab::Radix => self.radix_ui(ui, lang, t),
            ConverterTab::Scientific => self.scientific_ui(ui, lang, t),
        }
    }

    fn ascii_ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.byte_direction,
                ByteDirection::TextToBytes,
                text(lang, "字符串 → 字节", "Text → bytes"),
            );
            ui.selectable_value(
                &mut self.byte_direction,
                ByteDirection::BytesToText,
                text(lang, "字节 → 字符串", "Bytes → text"),
            );
        });
        ui.horizontal(|ui| {
            ui.label(text(lang, "显示/输入基数", "Byte base"));
            ui.selectable_value(&mut self.byte_base, ByteBase::Decimal, "DEC");
            ui.selectable_value(&mut self.byte_base, ByteBase::Hex, "HEX");
        });
        ui.add_sized(
            [ui.available_width(), 58.0],
            egui::TextEdit::multiline(&mut self.byte_input),
        );
        if ui_theme::accent_button(ui, t, text(lang, "转换", "Convert")).clicked() {
            let result = match self.byte_direction {
                ByteDirection::TextToBytes => utf8_to_bytes(&self.byte_input, self.byte_base),
                ByteDirection::BytesToText => bytes_to_utf8(&self.byte_input, self.byte_base),
            };
            match result {
                Ok(result) => {
                    self.byte_result = result;
                    self.byte_error = None;
                }
                Err(error) => {
                    self.byte_result.clear();
                    self.byte_error = Some(error);
                }
            }
        }
        error(ui, self.byte_error.as_deref());
        result_box(ui, t, &self.byte_result);
        note(
            ui,
            t,
            text(
                lang,
                "非 ASCII 字符按 UTF-8 bytes 转换；字节输入范围 0..255。",
                "Non-ASCII text uses UTF-8 bytes; input bytes must be 0..255.",
            ),
        );
    }

    fn radix_ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        ui.horizontal(|ui| {
            ui.label(text(lang, "源进制", "Source base"));
            ui.add(egui::DragValue::new(&mut self.source_base).range(2..=36));
            ui.label(text(lang, "目标进制", "Target base"));
            ui.add(egui::DragValue::new(&mut self.target_base).range(2..=36));
        });
        ui.text_edit_singleline(&mut self.radix_input);
        if ui_theme::accent_button(ui, t, text(lang, "转换", "Convert")).clicked() {
            match convert_radix(&self.radix_input, self.source_base, self.target_base) {
                Ok(result) => {
                    self.radix_result = result;
                    self.radix_error = None;
                }
                Err(error) => {
                    self.radix_result.clear();
                    self.radix_error = Some(error);
                }
            }
        }
        error(ui, self.radix_error.as_deref());
        result_box(ui, t, &self.radix_result);
        if let Ok(value) = parse_i128_radix(&self.radix_input, self.source_base) {
            let summary = format!(
                "BIN {}  ·  OCT {}  ·  DEC {}  ·  HEX {}",
                format_i128_radix(value, 2).unwrap_or_default(),
                format_i128_radix(value, 8).unwrap_or_default(),
                value,
                format_i128_radix(value, 16).unwrap_or_default()
            );
            note(ui, t, &summary);
        }
        note(
            ui,
            t,
            text(
                lang,
                "有符号 i128；支持负号、0b/0o/0x 前缀、下划线及 2..36 进制。",
                "Signed i128; supports sign, 0b/0o/0x prefixes, underscores, and bases 2..36.",
            ),
        );
    }

    fn scientific_ui(&mut self, ui: &mut egui::Ui, lang: Lang, t: &Tokens) {
        ui.horizontal(|ui| {
            ui.label(text(lang, "角度模式", "Angle mode"));
            ui.selectable_value(&mut self.angle_mode, AngleMode::Radians, "RAD");
            ui.selectable_value(&mut self.angle_mode, AngleMode::Degrees, "DEG");
        });
        let response = ui.add_sized(
            [ui.available_width(), 54.0],
            egui::TextEdit::multiline(&mut self.expression),
        );
        scientific_keypad(ui, &mut self.expression, lang, t);
        let enter = response.has_focus()
            && ui.input(|input| input.key_pressed(egui::Key::Enter) && !input.modifiers.shift);
        if ui_theme::accent_button(ui, t, text(lang, "计算", "Calculate")).clicked() || enter {
            match evaluate_expression(&self.expression, self.angle_mode) {
                Ok(result) => {
                    self.expression_result = format!("{result:.12}");
                    self.expression_error = None;
                }
                Err(error) => {
                    self.expression_result.clear();
                    self.expression_error = Some(error);
                }
            }
        }
        error(ui, self.expression_error.as_deref());
        result_box(ui, t, &self.expression_result);
        note(
            ui,
            t,
            text(
                lang,
                "+ − * / % ^；0x/0X 前缀或 H/h 后缀为十六进制，其余为十进制；pi/π, e；sqrt abs exp ln log/log10 log2；sin cos tan asin acos atan；floor ceil。^ 右结合，-2^2 = -(2^2)。",
                "+ − * / % ^; 0x/0X prefix or H/h suffix => hex, otherwise decimal; pi/π, e; sqrt abs exp ln log/log10 log2; sin cos tan asin acos atan; floor ceil. ^ is right-associative; -2^2 = -(2^2).",
            ),
        );
    }
}

fn scientific_keypad(ui: &mut egui::Ui, expression: &mut String, lang: Lang, t: &Tokens) {
    ui.add_space(4.0);
    let gap = 4.0;
    let cols = 7_usize;
    let btn_w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).max(30.0);
    let btn_h = 28.0;

    let mut row = |ui: &mut egui::Ui, labels: &[&str]| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for label in labels {
                if keypad_button(ui, t, label, btn_w, btn_h) {
                    append_keypad_token(expression, label);
                }
            }
        });
    };

    row(ui, &["7", "8", "9", "/", "*", "^", "⌫"]);
    row(ui, &["4", "5", "6", "-", "+", "%", "C"]);
    row(ui, &["1", "2", "3", "(", ")", ".", "H"]);
    row(ui, &["0", "0x", "e", "pi", "sin(", "cos(", "tan("]);
    row(
        ui,
        &["sqrt(", "ln(", "log(", "log2(", "asin(", "acos(", "atan("],
    );
    row(ui, &["abs(", "exp(", "floor(", "ceil("]);
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(text(
            lang,
            "⌫ 退格 · C 清空 · H 追加后缀（如 10H）",
            "⌫ backspace · C clear · H appends suffix (e.g. 10H)",
        ))
        .small()
        .color(t.text_muted),
    );
}

fn keypad_button(ui: &mut egui::Ui, t: &Tokens, label: &str, width: f32, height: f32) -> bool {
    ui.add_sized(
        [width, height],
        egui::Button::new(egui::RichText::new(label).size(12.0).color(t.text_primary))
            .fill(t.surface_bg)
            .stroke(Stroke::new(1.0_f32, t.border.gamma_multiply(0.65))),
    )
    .clicked()
}

fn append_keypad_token(expression: &mut String, token: &str) {
    match token {
        "⌫" => {
            expression.pop();
        }
        "C" => expression.clear(),
        "0x" => expression.push_str("0x"),
        "H" => expression.push('H'),
        "pi" => expression.push_str("pi"),
        "e" => expression.push('e'),
        _ => expression.push_str(token),
    }
}

fn result_box(ui: &mut egui::Ui, t: &Tokens, result: &str) {
    Frame::NONE
        .fill(t.surface_bg)
        .stroke(Stroke::new(1.0_f32, t.border.gamma_multiply(0.5)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::same(8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(
                egui::RichText::new(if result.is_empty() { "—" } else { result })
                    .strong()
                    .color(t.accent),
            );
        });
}

fn note(ui: &mut egui::Ui, t: &Tokens, value: &str) {
    ui.label(egui::RichText::new(value).small().color(t.text_muted));
}

fn error(ui: &mut egui::Ui, value: Option<&str>) {
    ui.allocate_ui(egui::vec2(ui.available_width(), 34.0), |ui| {
        if let Some(value) = value {
            ui.label(
                egui::RichText::new(value)
                    .small()
                    .color(Color32::from_rgb(239, 68, 68)),
            );
        }
    });
}

fn text<'a>(lang: Lang, zh: &'a str, en: &'a str) -> &'a str {
    match lang {
        Lang::Zh => zh,
        Lang::En => en,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(left: f64, right: f64) {
        assert!((left - right).abs() < 1.0e-10, "{left} != {right}");
    }

    #[test]
    fn utf8_bytes_roundtrip_ascii_and_chinese() {
        assert_eq!(
            utf8_to_bytes("Hello", ByteBase::Decimal).unwrap(),
            "72 101 108 108 111"
        );
        assert_eq!(
            bytes_to_utf8("48 65 6C 6C 6F", ByteBase::Hex).unwrap(),
            "Hello"
        );
        let encoded = utf8_to_bytes("中文", ByteBase::Hex).unwrap();
        assert_eq!(encoded, "E4 B8 AD E6 96 87");
        assert_eq!(bytes_to_utf8(&encoded, ByteBase::Hex).unwrap(), "中文");
        assert!(bytes_to_utf8("FF", ByteBase::Hex).is_err());
        assert!(parse_byte_sequence("256", ByteBase::Decimal).is_err());
    }

    #[test]
    fn radix_conversion_handles_common_arbitrary_and_negative_values() {
        assert_eq!(convert_radix("0b1111", 2, 16).unwrap(), "F");
        assert_eq!(convert_radix("17", 8, 10).unwrap(), "15");
        assert_eq!(convert_radix("255", 10, 16).unwrap(), "FF");
        assert_eq!(convert_radix("0xFF", 16, 2).unwrap(), "11111111");
        assert_eq!(convert_radix("-0xFF", 16, 10).unwrap(), "-255");
        assert_eq!(convert_radix("Z", 36, 10).unwrap(), "35");
        assert!(convert_radix("2", 2, 10).is_err());
        assert!(convert_radix("1", 1, 10).is_err());
        assert!(parse_i128_radix("170141183460469231731687303715884105728", 10).is_err());
    }

    #[test]
    fn expression_precedence_unary_power_and_scientific_notation() {
        close(
            evaluate_expression("2+3*4", AngleMode::Radians).unwrap(),
            14.0,
        );
        close(
            evaluate_expression("(2+3)*4", AngleMode::Radians).unwrap(),
            20.0,
        );
        close(
            evaluate_expression("2^3^2", AngleMode::Radians).unwrap(),
            512.0,
        );
        close(
            evaluate_expression("-2^2", AngleMode::Radians).unwrap(),
            -4.0,
        );
        close(
            evaluate_expression("1.5e2 + 2.5E-1", AngleMode::Radians).unwrap(),
            150.25,
        );
        close(
            evaluate_expression("0xFF+1", AngleMode::Radians).unwrap(),
            256.0,
        );
        close(
            evaluate_expression("10H*2", AngleMode::Radians).unwrap(),
            32.0,
        );
        close(parse_numeric_literal("0x10").unwrap(), 16.0);
        close(parse_numeric_literal("FFH").unwrap(), 255.0);
        close(parse_numeric_literal("10h").unwrap(), 16.0);
        close(parse_numeric_literal("255").unwrap(), 255.0);
    }

    #[test]
    fn expression_constants_logs_and_nested_functions() {
        close(
            evaluate_expression("ln(e)", AngleMode::Radians).unwrap(),
            1.0,
        );
        close(
            evaluate_expression("log10(100)+log2(8)", AngleMode::Radians).unwrap(),
            5.0,
        );
        close(
            evaluate_expression("sin(pi/2)+log10(100)+sqrt(9)", AngleMode::Radians).unwrap(),
            6.0,
        );
        close(
            evaluate_expression("exp(ln(5))", AngleMode::Radians).unwrap(),
            5.0,
        );
    }

    #[test]
    fn trig_and_inverse_trig_respect_angle_mode() {
        close(
            evaluate_expression("sin(90)", AngleMode::Degrees).unwrap(),
            1.0,
        );
        close(
            evaluate_expression("asin(1)", AngleMode::Degrees).unwrap(),
            90.0,
        );
        close(
            evaluate_expression("cos(pi)", AngleMode::Radians).unwrap(),
            -1.0,
        );
        close(
            evaluate_expression("atan(1)", AngleMode::Radians).unwrap(),
            PI / 4.0,
        );
    }

    #[test]
    fn expression_reports_math_and_syntax_errors() {
        for expression in [
            "1/0",
            "1%0",
            "sqrt(-1)",
            "ln(0)",
            "asin(2)",
            "tan(90)",
            "unknown(1)",
            "(1+2",
            "1+",
        ] {
            let mode = if expression == "tan(90)" {
                AngleMode::Degrees
            } else {
                AngleMode::Radians
            };
            assert!(
                evaluate_expression(expression, mode).is_err(),
                "{expression}"
            );
        }
    }
}
