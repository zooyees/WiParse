//! WiParse CLI — JSON envelope compatible with Python WiParseCLI.

mod attach;
mod hw;
mod output;

use clap::{Parser, Subcommand};
use output::{emit_error, emit_ndjson, emit_ok, OutputOptions};
use serde_json::json;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use wiparse_core::config::load_config;
use wiparse_core::db::{
    close_session, create_session, get_session, insert_log, insert_metric, list_sessions, open_db,
    session_log_count, session_metric_count,
};
use wiparse_core::metrics::parse_metric_frame;
use wiparse_core::protocol::parse_qi_line;
use wiparse_core::scope::{self, scope_capabilities};
use wiparse_core::serial::{list_ports, CapturedEvent, SerialSession};
use wiparse_core::wave::{
    export_metrics_csv, export_metrics_json, fetch_session_metrics, metrics_to_wave, MetricRow,
    DEFAULT_CHANNELS,
};
use wiparse_core::VERSION;

#[derive(Parser, Debug)]
#[command(
    name = "wiparse",
    version = VERSION,
    about = "WiParse JSON CLI. Prefers a running WiParse.exe; use --local for headless.",
    after_help = "Docs: docs/CLI_REFERENCE.md  |  API: docs/DEPLOY_API.md",
    arg_required_else_help = true
)]
struct Cli {
    /// Accepted for compatibility; output is always JSON.
    #[arg(long, default_value_t = true, global = true, hide = true)]
    json: bool,
    /// Pretty-print JSON.
    #[arg(long, global = true)]
    pretty: bool,
    /// Print only the data / error body (no envelope).
    #[arg(short, long, global = true)]
    quiet: bool,
    /// Config file (sets WCM_CONFIG).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// GUI API URL (default WIPARSE_URL or http://127.0.0.1:7878).
    #[arg(long, global = true)]
    url: Option<String>,
    /// Do not attach to WiParse.exe; open devices in this process.
    #[arg(long, global = true)]
    local: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Print version.
    Version,
    /// List serial ports.
    Ports,
    /// Call the GUI embedded API (requires WiParse.exe).
    #[command(subcommand, arg_required_else_help = true)]
    Api(ApiCmd),
    /// Serial capture / send (attaches to GUI monitor when available).
    #[command(subcommand, arg_required_else_help = true)]
    Serial(SerialCmd),
    /// Compact live-log brief (requires WiParse.exe).
    #[command(subcommand, arg_required_else_help = true)]
    Log(LogCmd),
    /// Closed-loop test runner (requires WiParse.exe).
    #[command(subcommand, arg_required_else_help = true)]
    Test(TestCmd),
    /// Parse Qi ASK/FSK lines or AA55 metric frames.
    #[command(subcommand, arg_required_else_help = true)]
    Parse(ParseCmd),
    /// SQLite capture sessions.
    #[command(subcommand, arg_required_else_help = true)]
    Session(SessionCmd),
    /// Metrics waveform JSON / CSV.
    #[command(subcommand, arg_required_else_help = true)]
    Wave(WaveCmd),
    /// VISA oscilloscope shortcuts (local). GUI instruments: api invoke instrument.*.
    #[command(subcommand, arg_required_else_help = true)]
    Scope(ScopeCmd),
    /// Debug probe (J-Link / ST-Link / CMSIS-DAP). Prefers GUI session; `--local` is one-shot.
    #[command(subcommand, arg_required_else_help = true)]
    Probe(ProbeCmd),
    /// FT4222 USB-SPI/I2C/GPIO. Prefers GUI session; `--local` is one-shot.
    #[command(subcommand, arg_required_else_help = true)]
    Bridge(BridgeCmd),
    /// Drive the running WiParse.exe UI (tabs, panels, inputs). Requires GUI.
    #[command(subcommand, arg_required_else_help = true)]
    Ui(UiCmd),
}

#[derive(Subcommand, Debug)]
enum ApiCmd {
    /// GET /v1/health
    Health,
    /// GET /v1/capabilities
    Capabilities,
    /// POST /v1/invoke — `wiparse api invoke serial.ports`
    Invoke {
        /// Method name (or pass --method).
        #[arg(value_name = "METHOD")]
        method_pos: Option<String>,
        #[arg(long)]
        method: Option<String>,
        #[arg(short, long, default_value = "{}")]
        params: String,
    },
    /// GET /v1/events NDJSON stream
    Events {
        #[arg(long, default_value_t = 0)]
        since_seq: u64,
    },
}

#[derive(Subcommand, Debug)]
enum SerialCmd {
    /// Start GUI serial monitor (requires WiParse.exe).
    Start {
        #[arg(long)]
        port: String,
        #[arg(long, default_value_t = 2_000_000)]
        baud: u32,
    },
    /// Stop GUI serial monitor.
    Stop,
    /// GUI serial monitor status.
    Status,
    /// Set port/baud on the GUI without opening (requires WiParse.exe).
    Select {
        #[arg(long)]
        port: Option<String>,
        #[arg(long)]
        baud: Option<u32>,
    },
    /// Capture metrics/logs. Local mode needs --duration, --max-metrics, or --max-logs.
    Read {
        #[arg(long)]
        port: String,
        #[arg(long, default_value_t = 2_000_000)]
        baud: u32,
        #[arg(long)]
        duration: Option<f64>,
        #[arg(long)]
        max_metrics: Option<usize>,
        #[arg(long)]
        max_logs: Option<usize>,
        #[arg(long)]
        demo: bool,
        #[arg(long)]
        save_db: bool,
    },
    /// Endless NDJSON stream (Ctrl+C to stop). Local only.
    Stream {
        #[arg(long)]
        port: String,
        #[arg(long, default_value_t = 2_000_000)]
        baud: u32,
        #[arg(long, default_value = "metrics,logs")]
        types: String,
        #[arg(long)]
        demo: bool,
    },
    /// Send hex bytes. Attached: GUI monitor (port/baud ignored). `--local`: opens --port.
    Send {
        #[arg(long)]
        port: String,
        #[arg(long, default_value_t = 2_000_000)]
        baud: u32,
        #[arg(long = "hex")]
        hex_data: String,
    },
}

#[derive(Subcommand, Debug)]
enum LogCmd {
    /// List log tabs.
    Tabs,
    /// Compact session brief for Agent (no raw lines).
    Brief {
        #[arg(long, default_value_t = 0)]
        since: u64,
        #[arg(long)]
        detail: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum TestCmd {
    /// Start a plan (JSON file). Starts the GUI monitor if needed.
    Run {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        port: Option<String>,
        #[arg(long)]
        baud: Option<u32>,
    },
    /// Running / last test status.
    Status,
    /// Abort the running test.
    Abort {
        #[arg(long, default_value = "user")]
        reason: String,
    },
    /// Evidence pack summary (writes skeleton if the run just finished).
    Pack,
}

#[derive(Subcommand, Debug)]
enum ParseCmd {
    /// Parse one Qi ASK/FSK log line.
    Line {
        #[arg(long)]
        text: String,
    },
    /// Parse one AA55 metric frame.
    Metrics {
        #[arg(long)]
        text: String,
    },
    /// Parse Qi / metrics lines from a file.
    File {
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Parse Qi / metrics lines from stdin.
    Stdin {
        #[arg(long)]
        limit: Option<usize>,
    },
}

#[derive(Subcommand, Debug)]
enum SessionCmd {
    /// List recent SQLite sessions.
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show one session by id.
    Show {
        #[arg(long)]
        id: i64,
    },
}

#[derive(Subcommand, Debug)]
enum WaveCmd {
    /// Capture live metrics into waveform JSON (local serial; use --local if GUI holds the port).
    Live {
        #[arg(long)]
        port: String,
        #[arg(long, default_value_t = 2_000_000)]
        baud: u32,
        #[arg(long, default_value_t = 5.0)]
        duration: f64,
        #[arg(long, default_value = "v_in,i_in,v_out,i_out,p")]
        channels: String,
        #[arg(long)]
        demo: bool,
    },
    /// Waveform JSON (or CSV) from a saved session.
    Session {
        #[arg(long)]
        session_id: i64,
        #[arg(long = "from")]
        rel_from: Option<f64>,
        #[arg(long = "to")]
        rel_to: Option<f64>,
        #[arg(long, default_value = "v_in,i_in,v_out,i_out,v_bat,i_bat,p")]
        channels: String,
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Write session metrics to a file.
    Export {
        #[arg(long)]
        session_id: i64,
        #[arg(long = "from")]
        rel_from: Option<f64>,
        #[arg(long = "to")]
        rel_to: Option<f64>,
        #[arg(long, default_value = "csv")]
        format: String,
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum ScopeCmd {
    /// List VISA oscilloscopes.
    List,
    /// Capture a screenshot.
    Shot {
        #[arg(long, default_value_t = 0)]
        index: usize,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Read a channel waveform.
    Wave {
        #[arg(long, default_value_t = 0)]
        index: usize,
        #[arg(long, default_value = "CH1")]
        channel: String,
        #[arg(long)]
        points: Option<u32>,
    },
}

#[derive(Subcommand, Debug)]
enum ProbeCmd {
    /// List J-Link / ST-Link / CMSIS-DAP (GUI: connected + scan cache; --local: USB).
    List,
    Connect {
        #[arg(long)]
        resource: String,
        #[arg(long)]
        chip: Option<String>,
    },
    Halt {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        demo: bool,
    },
    Run {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        demo: bool,
    },
    Reset {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        hardware: bool,
        #[arg(long)]
        demo: bool,
    },
    Status {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        demo: bool,
    },
    Regs {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        demo: bool,
    },
    Flash {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        path: String,
        #[arg(long)]
        verify: bool,
        #[arg(long)]
        demo: bool,
    },
    #[command(name = "mem-read")]
    MemRead {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        address: String,
        #[arg(long, default_value_t = 64)]
        len: u32,
        #[arg(long)]
        demo: bool,
    },
    #[command(name = "mem-write")]
    MemWrite {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        address: String,
        #[arg(long)]
        hex: String,
        #[arg(long)]
        demo: bool,
    },
        Erase {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        demo: bool,
    },
    Speed {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long, default_value_t = 4_000)]
        khz: u32,
        #[arg(long)]
        demo: bool,
    },
    #[command(name = "rtt-start")]
    RttStart {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long, default_value_t = 0)]
        channel: u32,
        #[arg(long)]
        demo: bool,
    },
    #[command(name = "rtt-stop")]
    RttStop {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        demo: bool,
    },
    #[command(name = "rtt-read")]
    RttRead {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        chip: Option<String>,
        #[arg(long)]
        demo: bool,
    },
}

#[derive(Subcommand, Debug)]
enum BridgeCmd {
    List,
    Connect {
        #[arg(long)]
        resource: String,
    },
    Info {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        demo: bool,
    },
    Spi {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long, default_value_t = 0)]
        mode: u8,
        #[arg(long, default_value_t = 1_000_000)]
        clock_hz: u32,
        #[arg(long, default_value = "")]
        write: String,
        #[arg(long, default_value_t = 4)]
        read_len: u32,
        #[arg(long)]
        demo: bool,
    },
    I2c {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long)]
        addr: String,
        #[arg(long, default_value = "")]
        write: String,
        #[arg(long, default_value_t = 0)]
        read_len: u32,
        #[arg(long, default_value_t = 100_000)]
        clock_hz: u32,
        #[arg(long)]
        demo: bool,
    },
    Gpio {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        resource: Option<String>,
        #[arg(long, default_value_t = 0)]
        pin: u8,
        #[arg(long)]
        output: Option<bool>,
        #[arg(long)]
        high: Option<bool>,
        #[arg(long)]
        demo: bool,
    },
}

#[derive(Subcommand, Debug)]
enum UiCmd {
    /// Snapshot active tab, panels, serial, instruments, waveform, data analysis, calculator.
    State,
    /// Switch the main tab (also unhides it).
    Show {
        #[arg(long)]
        tab: String,
    },
    /// Show or hide main tools (`--serial true`).
    Panels {
        #[arg(long, num_args = 1, value_parser = clap::builder::BoolishValueParser::new())]
        serial: Option<bool>,
        #[arg(long, num_args = 1, value_parser = clap::builder::BoolishValueParser::new())]
        calculator: Option<bool>,
        #[arg(long, num_args = 1, value_parser = clap::builder::BoolishValueParser::new())]
        instruments: Option<bool>,
        #[arg(long, num_args = 1, value_parser = clap::builder::BoolishValueParser::new())]
        waveform: Option<bool>,
        #[arg(long, num_args = 1, value_parser = clap::builder::BoolishValueParser::new())]
        data_analysis: Option<bool>,
        #[arg(long, num_args = 1, value_parser = clap::builder::BoolishValueParser::new())]
        test_report: Option<bool>,
        #[arg(long, num_args = 1, value_parser = clap::builder::BoolishValueParser::new())]
        test_tool: Option<bool>,
    },
    /// Language, theme, debug mode.
    Prefs {
        #[arg(long)]
        language: Option<String>,
        #[arg(long)]
        theme: Option<String>,
        #[arg(long, num_args = 1, value_parser = clap::builder::BoolishValueParser::new())]
        debug: Option<bool>,
    },
    #[command(subcommand, arg_required_else_help = true)]
    Serial(UiSerialCmd),
    #[command(subcommand, arg_required_else_help = true)]
    Wave(UiWaveCmd),
    #[command(subcommand, arg_required_else_help = true)]
    Calc(UiCalcCmd),
    #[command(subcommand, arg_required_else_help = true)]
    Instrument(UiInstrumentCmd),
}

#[derive(Subcommand, Debug)]
enum UiSerialCmd {
    /// Open a log file as a tab.
    Open {
        #[arg(long)]
        path: PathBuf,
    },
    /// Close a file tab (not the live tab).
    Close {
        #[arg(long)]
        tab_id: u64,
    },
    /// Clear the live log display.
    Clear,
    /// Apply a text filter on a tab pane.
    Filter {
        #[arg(long)]
        query: String,
        #[arg(long, default_value_t = 0)]
        tab_id: u64,
        #[arg(long, default_value_t = 0)]
        pane: u64,
    },
    /// Activate a serial tab.
    Tab {
        #[arg(long)]
        tab_id: u64,
    },
    /// Rename the live log tab.
    Name {
        #[arg(long)]
        name: String,
    },
    /// Set the log file browser directory.
    Browser {
        #[arg(long)]
        dir: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum UiWaveCmd {
    /// Open a waveform file in the GUI analysis page.
    Open {
        #[arg(long)]
        path: PathBuf,
    },
    /// Close all loaded waveforms.
    Close,
    /// Select a loaded channel by index.
    Select {
        #[arg(long)]
        index: u64,
    },
    /// Set the waveform folder browser directory.
    Browser {
        #[arg(long)]
        dir: PathBuf,
    },
    /// Configure bus decode (UART / I2C / SPI / I2S / DDSSS).
    Bus {
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        uart: Option<u64>,
        #[arg(long)]
        signal: Option<u64>,
        #[arg(long)]
        scl: Option<u64>,
        #[arg(long)]
        sda: Option<u64>,
        #[arg(long)]
        clk: Option<u64>,
        #[arg(long)]
        mosi: Option<u64>,
        #[arg(long)]
        miso: Option<u64>,
        #[arg(long)]
        cs: Option<u64>,
        #[arg(long)]
        bclk: Option<u64>,
        #[arg(long)]
        ws: Option<u64>,
        #[arg(long)]
        data: Option<u64>,
        #[arg(long)]
        threshold: Option<f64>,
        #[arg(long)]
        baud: Option<f64>,
        #[arg(long)]
        sequence: Option<String>,
        #[arg(long)]
        extension: Option<String>,
        #[arg(long)]
        fop: Option<f64>,
    },
    Cursor {
        #[arg(long)]
        x1: Option<f64>,
        #[arg(long)]
        x2: Option<f64>,
        #[arg(long)]
        y1: Option<f64>,
        #[arg(long)]
        y2: Option<f64>,
        #[arg(long)]
        clear: bool,
    },
    Fit,
}

#[derive(Subcommand, Debug)]
enum UiCalcCmd {
    Get,
    Set {
        #[arg(long)]
        card: String,
        #[arg(long, default_value = "{}")]
        params: String,
    },
}

#[derive(Subcommand, Debug)]
enum UiInstrumentCmd {
    /// Open a device console, a type card, or the digital-twin overview.
    Select {
        #[arg(long)]
        id: Option<u64>,
        /// Show 设备总览 (digital twin bench).
        #[arg(long)]
        overview: bool,
        /// Instrument kind (`oscilloscope`, `dc_source`, `debug_probe`, …) or `overview`.
        #[arg(long)]
        kind: Option<String>,
    },
    Scan,
    /// Connected sessions + discovered resources (optional `--kind` filter).
    List {
        #[arg(long)]
        kind: Option<String>,
    },
    /// Digital-twin snapshot: LCD/channel live state for every session.
    Overview,
    Connect {
        #[arg(long)]
        resource: String,
        #[arg(long)]
        kind: Option<String>,
    },
    Disconnect {
        #[arg(long)]
        id: u64,
    },
    Measure {
        #[arg(long)]
        id: u64,
    },
    Capture {
        #[arg(long)]
        id: Option<u64>,
    },
    Waveform {
        #[arg(long)]
        id: u64,
        #[arg(long, default_value_t = 1)]
        channel: u8,
        #[arg(long)]
        points: Option<u32>,
    },
    /// Read displayed-channel waveform source (ISF). Same as GUI「读取波形源文件」.
    #[command(name = "waveform-source")]
    WaveformSource {
        #[arg(long)]
        id: Option<u64>,
        #[arg(long)]
        dir: Option<String>,
        #[arg(long)]
        filename: Option<String>,
        #[arg(long)]
        overwrite: bool,
    },
    Command {
        #[arg(long)]
        id: u64,
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        write: Option<String>,
        /// Raw ControlCommand JSON, e.g. `"ScopeRun"` or `{"ScopeChannel":{"channel":1,"enabled":true}}`.
        #[arg(long = "json")]
        command_json: Option<String>,
    },
    /// Connect every discovered instrument that is not already open.
    #[command(name = "connect-all")]
    ConnectAll,
}

fn opts(cli: &Cli) -> OutputOptions {
    OutputOptions {
        pretty: cli.pretty,
        quiet: cli.quiet,
    }
}

fn apply_config(cli: &Cli) {
    if let Some(path) = &cli.config {
        std::env::set_var("WCM_CONFIG", path);
    }
}

fn db_conn() -> Result<rusqlite::Connection, String> {
    let cfg = load_config().map_err(|e| e.to_string())?;
    open_db(wiparse_core::db::default_db_path(&cfg.system.db_name)).map_err(|e| e.to_string())
}

fn main() {
    let cli = Cli::parse();
    let _ = &cli.json;
    apply_config(&cli);
    let o = opts(&cli);

    if let Commands::Api(ApiCmd::Events { since_seq }) = &cli.command {
        let url = cli.url.clone().unwrap_or_else(attach::default_url);
        if let Err(e) = attach::stream_events(&url, *since_seq) {
            let code = emit_error("api.events", &e, &o);
            std::process::exit(code);
        }
        return;
    }

    // NDJSON stream writes its own lines; skip envelope wrapper.
    if let Commands::Serial(SerialCmd::Stream { .. }) = &cli.command {
        let code = match run_stream(&cli, &o) {
            Ok(()) => 0,
            Err(e) => emit_error("serial.stream", &e, &o),
        };
        std::process::exit(code);
    }

    let code = match run(&cli, &o) {
        Ok(cmd) => emit_ok(&cmd.0, cmd.1, &o),
        Err((cmd, err)) => emit_error(&cmd, &err, &o),
    };
    std::process::exit(code);
}

fn attach_url(cli: &Cli) -> Option<String> {
    if cli.local {
        return None;
    }
    if let Some(url) = &cli.url {
        return Some(url.clone());
    }
    if let Ok(url) = std::env::var("WIPARSE_URL") {
        return Some(url);
    }
    Some(attach::default_url())
}

fn map_to_invoke(cli: &Cli) -> Option<(String, serde_json::Value)> {
    match &cli.command {
        Commands::Ports => Some(("serial.ports".into(), json!({}))),
        Commands::Serial(SerialCmd::Start { port, baud }) => Some((
            "serial.monitor.start".into(),
            json!({ "port": port, "baud": baud }),
        )),
        Commands::Serial(SerialCmd::Stop) => Some(("serial.monitor.stop".into(), json!({}))),
        Commands::Serial(SerialCmd::Status) => Some(("serial.monitor.status".into(), json!({}))),
        Commands::Serial(SerialCmd::Select { port, baud }) => Some((
            "serial.select".into(),
            json!({ "port": port, "baud": baud }),
        )),
        Commands::Serial(SerialCmd::Send { hex_data, .. }) => {
            Some(("serial.send".into(), json!({ "hex": hex_data })))
        }
        Commands::Log(LogCmd::Tabs) => Some(("log.tabs.list".into(), json!({}))),
        Commands::Log(LogCmd::Brief { since, detail }) => Some((
            "log.brief".into(),
            json!({ "since_row": since, "detail": detail }),
        )),
        Commands::Test(TestCmd::Status) => Some(("test.status".into(), json!({}))),
        Commands::Test(TestCmd::Abort { reason }) => {
            Some(("test.abort".into(), json!({ "reason": reason })))
        }
        Commands::Test(TestCmd::Pack) => Some(("test.pack".into(), json!({}))),
        Commands::Serial(SerialCmd::Read {
            port,
            baud,
            max_logs,
            ..
        }) => Some((
            "serial.read".into(),
            json!({
                "port": port,
                "baud": baud,
                "max_logs": max_logs.unwrap_or(100),
            }),
        )),
        Commands::Parse(ParseCmd::Line { text }) => {
            Some(("parse.line".into(), json!({ "text": text })))
        }
        Commands::Parse(ParseCmd::Metrics { text }) => {
            Some(("parse.metrics".into(), json!({ "text": text })))
        }
        Commands::Parse(ParseCmd::File { path, limit }) => Some((
            "parse.file".into(),
            json!({ "path": path, "limit": limit }),
        )),
        Commands::Session(SessionCmd::List { limit }) => {
            Some(("session.list".into(), json!({ "limit": limit })))
        }
        Commands::Session(SessionCmd::Show { id }) => {
            Some(("session.show".into(), json!({ "id": id })))
        }
        Commands::Scope(ScopeCmd::List) => Some(("scope.list".into(), json!({}))),
        Commands::Scope(ScopeCmd::Shot { index, out }) => Some((
            "scope.shot".into(),
            json!({ "index": index, "out": out }),
        )),
        Commands::Scope(ScopeCmd::Wave {
            index,
            channel,
            points,
        }) => Some((
            "scope.wave".into(),
            json!({ "index": index, "channel": channel, "points": points }),
        )),
        Commands::Wave(WaveCmd::Session {
            session_id,
            rel_from,
            rel_to,
            channels,
            format,
        }) => Some((
            "wave.session".into(),
            json!({
                "session_id": session_id,
                "from": rel_from,
                "to": rel_to,
                "channels": channels,
                "format": format,
            }),
        )),
        Commands::Wave(WaveCmd::Export {
            session_id,
            rel_from,
            rel_to,
            format,
            out,
        }) => Some((
            "wave.export".into(),
            json!({
                "session_id": session_id,
                "from": rel_from,
                "to": rel_to,
                "format": format,
                "out": out,
            }),
        )),
        Commands::Ui(UiCmd::State) => Some(("ui.state".into(), json!({}))),
        Commands::Ui(UiCmd::Show { tab }) => Some(("ui.show".into(), json!({ "tab": tab }))),
        Commands::Ui(UiCmd::Panels {
            serial,
            calculator,
            instruments,
            waveform,
            data_analysis,
            test_report,
            test_tool,
        }) => {
            let mut p = json!({});
            if let Some(v) = serial {
                p["serial"] = json!(v);
            }
            if let Some(v) = calculator {
                p["calculator"] = json!(v);
            }
            if let Some(v) = instruments {
                p["instruments"] = json!(v);
            }
            if let Some(v) = waveform {
                p["waveform"] = json!(v);
            }
            if let Some(v) = data_analysis {
                p["data_analysis"] = json!(v);
            }
            if let Some(v) = test_report {
                p["test_report"] = json!(v);
            }
            if let Some(v) = test_tool {
                p["test_tool"] = json!(v);
            }
            Some(("ui.panels".into(), p))
        }
        Commands::Ui(UiCmd::Prefs {
            language,
            theme,
            debug,
        }) => {
            let mut p = json!({});
            if let Some(v) = language {
                p["language"] = json!(v);
            }
            if let Some(v) = theme {
                p["theme"] = json!(v);
            }
            if let Some(v) = debug {
                p["debug"] = json!(v);
            }
            Some(("ui.prefs".into(), p))
        }
        Commands::Ui(UiCmd::Serial(UiSerialCmd::Open { path })) => Some((
            "ui.serial.open".into(),
            json!({ "path": path }),
        )),
        Commands::Ui(UiCmd::Serial(UiSerialCmd::Close { tab_id })) => Some((
            "ui.serial.close".into(),
            json!({ "tab_id": tab_id }),
        )),
        Commands::Ui(UiCmd::Serial(UiSerialCmd::Clear)) => {
            Some(("ui.serial.clear".into(), json!({})))
        }
        Commands::Ui(UiCmd::Serial(UiSerialCmd::Filter { query, tab_id, pane })) => Some((
            "ui.serial.filter".into(),
            json!({ "query": query, "tab_id": tab_id, "pane": pane }),
        )),
        Commands::Ui(UiCmd::Serial(UiSerialCmd::Tab { tab_id })) => {
            Some(("ui.serial.tab".into(), json!({ "tab_id": tab_id })))
        }
        Commands::Ui(UiCmd::Serial(UiSerialCmd::Name { name })) => {
            Some(("ui.serial.name".into(), json!({ "name": name })))
        }
        Commands::Ui(UiCmd::Serial(UiSerialCmd::Browser { dir })) => {
            Some(("ui.serial.browser".into(), json!({ "dir": dir })))
        }
        Commands::Ui(UiCmd::Wave(UiWaveCmd::Open { path })) => {
            Some(("ui.wave.open".into(), json!({ "path": path })))
        }
        Commands::Ui(UiCmd::Wave(UiWaveCmd::Close)) => Some(("ui.wave.close".into(), json!({}))),
        Commands::Ui(UiCmd::Wave(UiWaveCmd::Select { index })) => {
            Some(("ui.wave.select".into(), json!({ "index": index })))
        }
        Commands::Ui(UiCmd::Wave(UiWaveCmd::Browser { dir })) => {
            Some(("ui.wave.browser".into(), json!({ "dir": dir })))
        }
        Commands::Ui(UiCmd::Wave(UiWaveCmd::Bus {
            kind,
            uart,
            signal,
            scl,
            sda,
            clk,
            mosi,
            miso,
            cs,
            bclk,
            ws,
            data,
            threshold,
            baud,
            sequence,
            extension,
            fop,
        })) => {
            let mut p = json!({});
            if let Some(v) = kind {
                p["kind"] = json!(v);
            }
            if let Some(v) = uart {
                p["uart"] = json!(v);
            }
            if let Some(v) = signal {
                p["signal"] = json!(v);
            }
            if let Some(v) = scl {
                p["scl"] = json!(v);
            }
            if let Some(v) = sda {
                p["sda"] = json!(v);
            }
            if let Some(v) = clk {
                p["clk"] = json!(v);
            }
            if let Some(v) = mosi {
                p["mosi"] = json!(v);
            }
            if let Some(v) = miso {
                p["miso"] = json!(v);
            }
            if let Some(v) = cs {
                p["cs"] = json!(v);
            }
            if let Some(v) = bclk {
                p["bclk"] = json!(v);
            }
            if let Some(v) = ws {
                p["ws"] = json!(v);
            }
            if let Some(v) = data {
                p["data"] = json!(v);
            }
            if let Some(v) = threshold {
                p["threshold"] = json!(v);
            }
            if let Some(v) = baud {
                p["baud"] = json!(v);
            }
            if let Some(v) = sequence {
                p["sequence"] = json!(v);
            }
            if let Some(v) = extension {
                p["extension"] = json!(v);
            }
            if let Some(v) = fop {
                p["fop"] = json!(v);
            }
            Some(("ui.wave.bus".into(), p))
        }
        Commands::Ui(UiCmd::Wave(UiWaveCmd::Cursor {
            x1,
            x2,
            y1,
            y2,
            clear,
        })) => Some((
            "ui.wave.cursor".into(),
            json!({ "x1": x1, "x2": x2, "y1": y1, "y2": y2, "clear": clear }),
        )),
        Commands::Ui(UiCmd::Wave(UiWaveCmd::Fit)) => Some(("ui.wave.fit".into(), json!({}))),
        Commands::Ui(UiCmd::Calc(UiCalcCmd::Get)) => Some(("ui.calc.get".into(), json!({}))),
        Commands::Ui(UiCmd::Calc(UiCalcCmd::Set { card, params })) => {
            let fields: serde_json::Value = serde_json::from_str(params).unwrap_or_else(|_| json!({}));
            Some(("ui.calc.set".into(), json!({ "card": card, "fields": fields })))
        }
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::Select { id, overview, kind })) => {
            let mut p = json!({});
            if let Some(id) = id {
                p["device_id"] = json!(id);
            }
            if *overview {
                p["overview"] = json!(true);
            }
            if let Some(kind) = kind {
                p["kind"] = json!(kind);
            }
            Some(("ui.instrument.select".into(), p))
        }
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::Scan)) => {
            Some(("instrument.scan".into(), json!({})))
        }
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::List { kind })) => {
            let mut p = json!({});
            if let Some(kind) = kind {
                p["kind"] = json!(kind);
            }
            Some(("instrument.list".into(), p))
        }
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::Overview)) => {
            Some(("instrument.overview".into(), json!({})))
        }
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::Connect { resource, kind })) => Some((
            "instrument.connect".into(),
            json!({ "resource": resource, "kind": kind }),
        )),
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::Disconnect { id })) => Some((
            "instrument.disconnect".into(),
            json!({ "device_id": id }),
        )),
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::Measure { id })) => Some((
            "instrument.measure".into(),
            json!({ "device_id": id }),
        )),
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::Capture { id })) => Some((
            "instrument.capture".into(),
            json!({ "device_id": id }),
        )),
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::Waveform {
            id,
            channel,
            points,
        })) => Some((
            "instrument.waveform".into(),
            json!({ "device_id": id, "channel": channel, "points": points }),
        )),
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::WaveformSource {
            id,
            dir,
            filename,
            overwrite,
        })) => {
            let mut p = json!({ "overwrite": overwrite });
            if let Some(id) = id {
                p["device_id"] = json!(id);
            }
            if let Some(dir) = dir {
                p["dir"] = json!(dir);
            }
            if let Some(filename) = filename {
                p["filename"] = json!(filename);
            }
            Some(("instrument.waveform_source".into(), p))
        },
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::Command {
            id,
            query,
            write,
            command_json,
        })) => {
            let command = if let Some(raw) = command_json {
                serde_json::from_str(raw).unwrap_or_else(|_| json!("Reset"))
            } else if let Some(q) = query {
                json!({ "RawQuery": q })
            } else if let Some(w) = write {
                json!({ "RawWrite": w })
            } else {
                json!("Reset")
            };
            Some((
                "instrument.command".into(),
                json!({ "device_id": id, "command": command }),
            ))
        }
        Commands::Ui(UiCmd::Instrument(UiInstrumentCmd::ConnectAll)) => {
            Some(("instrument.connect_all".into(), json!({})))
        }
        Commands::Probe(cmd) => map_probe_gui(cmd),
        Commands::Bridge(cmd) => map_bridge_gui(cmd),
        _ => None,
    }
}

fn run_stream(cli: &Cli, o: &OutputOptions) -> Result<(), String> {
    let Commands::Serial(SerialCmd::Stream {
        port,
        baud,
        types,
        demo,
    }) = &cli.command
    else {
        return Ok(());
    };
    let want_metrics = types.contains("metrics");
    let want_logs = types.contains("logs");

    if *demo {
        let m = parse_metric_frame("AA55:9000:1500:8500:1400:4000:3000:45:80:EDED")
            .ok_or_else(|| "demo parse failed".to_string())?;
        if want_metrics {
            emit_ndjson(&serde_json::json!({"type":"metrics","data": m}), o.pretty);
        }
        return Ok(());
    }

    let mut s = SerialSession::open(port, *baud).map_err(|e| e.to_string())?;
    loop {
        let events = s.poll_events().map_err(|e| e.to_string())?;
        for ev in events {
            match ev {
                CapturedEvent::Metrics(m) if want_metrics => {
                    emit_ndjson(&serde_json::json!({"type":"metrics","data": m}), o.pretty);
                }
                CapturedEvent::Log { line, qi } if want_logs => {
                    emit_ndjson(
                        &serde_json::json!({"type":"log","data":{"line": line, "qi": qi}}),
                        o.pretty,
                    );
                }
                _ => {}
            }
        }
        let _ = io::stdout().flush();
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run(cli: &Cli, _o: &OutputOptions) -> Result<(String, serde_json::Value), (String, String)> {
    if let Commands::Api(cmd) = &cli.command {
        let url = cli.url.clone().unwrap_or_else(attach::default_url);
        return match cmd {
            ApiCmd::Health => attach::health(&url)
                .and_then(attach::data_or_error)
                .map(|data| ("api.health".into(), data))
                .map_err(|e| ("api.health".into(), e)),
            ApiCmd::Capabilities => attach::capabilities(&url)
                .and_then(attach::data_or_error)
                .map(|data| ("api.capabilities".into(), data))
                .map_err(|e| ("api.capabilities".into(), e)),
            ApiCmd::Invoke {
                method,
                method_pos,
                params,
            } => {
                let method = method
                    .clone()
                    .or_else(|| method_pos.clone())
                    .ok_or_else(|| {
                        (
                            "api.invoke".into(),
                            "missing method (usage: api invoke <method> [--params JSON])".into(),
                        )
                    })?;
                let params: serde_json::Value = serde_json::from_str(params).map_err(|e| {
                    (
                        "api.invoke".into(),
                        format!("invalid --params JSON: {e}"),
                    )
                })?;
                match attach::invoke_data(&url, &method, params) {
                    Ok(data) => Ok((method.clone(), data)),
                    Err(e) => Err((method.clone(), e)),
                }
            }
            ApiCmd::Events { .. } => unreachable!("handled in main"),
        };
    }

    if let Commands::Test(TestCmd::Run { plan, port, baud }) = &cli.command {
        let text = fs::read_to_string(plan).map_err(|e| ("test.start".into(), e.to_string()))?;
        let plan_json: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
            (
                "test.start".into(),
                format!("invalid plan JSON: {e}"),
            )
        })?;
        let url = attach_url(cli).ok_or_else(|| {
            (
                "test.start".into(),
                "test run needs a running WiParse.exe (drop --local)".into(),
            )
        })?;
        let params = json!({ "plan": plan_json, "port": port, "baud": baud });
        return match attach::invoke_data(&url, "test.start", params) {
            Ok(data) => Ok(("test.start".into(), data)),
            Err(e) => Err(("test.start".into(), e)),
        };
    }

    if let Some(url) = attach_url(cli) {
        if let Some((method, params)) = map_to_invoke(cli) {
            match attach::invoke_data(&url, &method, params) {
                Ok(data) => return Ok((method, data)),
                Err(e) => return Err((method, e)),
            }
        }
    }

    match &cli.command {
        Commands::Api(_) => unreachable!("handled above"),
        Commands::Version => Ok((
            "version".into(),
            serde_json::json!({
                "version": VERSION,
                "name": "wiparse",
                "edition": "rust",
            }),
        )),
        Commands::Ports => {
            let ports = list_ports().map_err(|e| ("ports".into(), e.to_string()))?;
            Ok(("ports".into(), serde_json::to_value(ports).unwrap()))
        }
        Commands::Serial(SerialCmd::Send {
            port,
            baud,
            hex_data,
        }) => {
            let mut s = SerialSession::open(port, *baud)
                .map_err(|e| ("serial.send".into(), e.to_string()))?;
            let n = s
                .write_hex(hex_data)
                .map_err(|e| ("serial.send".into(), e.to_string()))?;
            Ok((
                "serial.send".into(),
                serde_json::json!({ "written": n, "port": port }),
            ))
        }
        Commands::Serial(SerialCmd::Stream { .. }) => unreachable!("handled in main"),
        Commands::Serial(SerialCmd::Start { .. })
        | Commands::Serial(SerialCmd::Stop)
        | Commands::Serial(SerialCmd::Status)
        | Commands::Serial(SerialCmd::Select { .. }) => Err((
            "serial.monitor".into(),
            "serial start/stop/status/select need a running WiParse.exe (drop --local)".into(),
        )),
        Commands::Log(_) | Commands::Test(_) | Commands::Ui(_) => Err((
            "ui".into(),
            "ui/log/test commands need a running WiParse.exe (drop --local)".into(),
        )),
        Commands::Serial(SerialCmd::Read {
            port,
            baud,
            duration,
            max_metrics,
            max_logs,
            demo,
            save_db,
        }) => {
            if !*demo
                && duration.is_none()
                && max_metrics.is_none()
                && max_logs.is_none()
            {
                return Err((
                    "serial.read".into(),
                    "specify --duration, --max-metrics, or --max-logs (or --demo)".into(),
                ));
            }
            if *demo {
                let m = parse_metric_frame("AA55:9000:1500:8500:1400:4000:3000:45:80:EDED")
                    .ok_or_else(|| ("serial.read".into(), "demo parse failed".into()))?;
                let mut session_info = None;
                if *save_db {
                    let conn = db_conn().map_err(|e| ("serial.read".into(), e))?;
                    let s = create_session(&conn, port, *baud, true)
                        .map_err(|e| ("serial.read".into(), e.to_string()))?;
                    insert_metric(&conn, s.session_id, 0.0, &m)
                        .map_err(|e| ("serial.read".into(), e.to_string()))?;
                    close_session(&conn, s.session_id)
                        .map_err(|e| ("serial.read".into(), e.to_string()))?;
                    session_info = Some(s);
                }
                return Ok((
                    "serial.read".into(),
                    serde_json::json!({
                        "demo": true,
                        "metrics": [m],
                        "logs": [],
                        "session": session_info,
                    }),
                ));
            }
            let mut s = SerialSession::open(port, *baud)
                .map_err(|e| ("serial.read".into(), e.to_string()))?;
            let deadline = duration.map(|d| Instant::now() + Duration::from_secs_f64(d));
            let mut metrics = Vec::new();
            let mut logs = Vec::new();
            let t0 = Instant::now();
            let mut db = if *save_db {
                let conn = db_conn().map_err(|e| ("serial.read".into(), e))?;
                let sess = create_session(&conn, port, *baud, false)
                    .map_err(|e| ("serial.read".into(), e.to_string()))?;
                Some((conn, sess))
            } else {
                None
            };
            loop {
                if let Some(d) = deadline {
                    if Instant::now() >= d {
                        break;
                    }
                }
                let events = s
                    .poll_events()
                    .map_err(|e| ("serial.read".into(), e.to_string()))?;
                for ev in events {
                    let rel = t0.elapsed().as_secs_f64();
                    match ev {
                        CapturedEvent::Metrics(m) => {
                            if let Some((conn, sess)) = db.as_ref() {
                                let _ = insert_metric(conn, sess.session_id, rel, &m);
                            }
                            metrics.push(m);
                            if max_metrics.is_some_and(|n| metrics.len() >= n) {
                                break;
                            }
                        }
                        CapturedEvent::Log { line, qi } => {
                            if let Some((conn, sess)) = db.as_ref() {
                                let _ = insert_log(conn, sess.session_id, rel, &line);
                            }
                            logs.push(serde_json::json!({ "line": line, "qi": qi }));
                            if max_logs.is_some_and(|n| logs.len() >= n) {
                                break;
                            }
                        }
                    }
                }
                if max_metrics.is_some_and(|n| metrics.len() >= n)
                    || max_logs.is_some_and(|n| logs.len() >= n)
                {
                    break;
                }
                if duration.is_none() && max_metrics.is_none() && max_logs.is_none() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let session = if let Some((conn, sess)) = db.take() {
                let _ = close_session(&conn, sess.session_id);
                Some(sess)
            } else {
                None
            };
            Ok((
                "serial.read".into(),
                serde_json::json!({
                    "port": port,
                    "metrics": metrics,
                    "logs": logs,
                    "session": session,
                }),
            ))
        }
        Commands::Parse(ParseCmd::Line { text }) => {
            let p = parse_qi_line(text);
            Ok(("parse.line".into(), serde_json::to_value(p).unwrap()))
        }
        Commands::Parse(ParseCmd::Metrics { text }) => {
            let m = parse_metric_frame(text)
                .ok_or_else(|| ("parse.metrics".into(), "invalid AA55 frame".into()))?;
            Ok(("parse.metrics".into(), serde_json::to_value(m).unwrap()))
        }
        Commands::Parse(ParseCmd::File { path, limit }) => {
            let text =
                fs::read_to_string(path).map_err(|e| ("parse.file".into(), e.to_string()))?;
            Ok(("parse.file".into(), parse_many(text.lines(), *limit)))
        }
        Commands::Parse(ParseCmd::Stdin { limit }) => {
            let stdin = io::stdin();
            let lines: Vec<String> = stdin.lock().lines().filter_map(Result::ok).collect();
            Ok((
                "parse.stdin".into(),
                parse_many(lines.iter().map(|s| s.as_str()), *limit),
            ))
        }
        Commands::Session(SessionCmd::List { limit }) => {
            let conn = db_conn().map_err(|e| ("session.list".into(), e))?;
            let rows =
                list_sessions(&conn, *limit).map_err(|e| ("session.list".into(), e.to_string()))?;
            Ok(("session.list".into(), serde_json::to_value(rows).unwrap()))
        }
        Commands::Session(SessionCmd::Show { id }) => {
            let conn = db_conn().map_err(|e| ("session.show".into(), e))?;
            let info = get_session(&conn, *id)
                .map_err(|e| ("session.show".into(), e.to_string()))?
                .ok_or_else(|| ("session.show".into(), format!("session {id} not found")))?;
            let metrics = session_metric_count(&conn, *id)
                .map_err(|e| ("session.show".into(), e.to_string()))?;
            let logs = session_log_count(&conn, *id)
                .map_err(|e| ("session.show".into(), e.to_string()))?;
            Ok((
                "session.show".into(),
                serde_json::json!({
                    "session": info,
                    "metrics_count": metrics,
                    "logs_count": logs,
                }),
            ))
        }
        Commands::Wave(WaveCmd::Live {
            port,
            baud,
            duration,
            channels,
            demo,
        }) => {
            let chans = parse_channels(channels);
            let mut rows = Vec::new();
            if *demo {
                let m = parse_metric_frame("AA55:9000:1500:8500:1400:4000:3000:45:80:EDED")
                    .ok_or_else(|| ("wave.live".into(), "demo parse failed".into()))?;
                let mut row = MetricRow::from(&m);
                row.rel_t = 0.0;
                rows.push(row);
                let mut row2 = MetricRow::from(&m);
                row2.rel_t = 1.0;
                row2.v_bat += 0.05;
                rows.push(row2);
            } else {
                let mut s = SerialSession::open(port, *baud)
                    .map_err(|e| ("wave.live".into(), e.to_string()))?;
                let deadline = Instant::now() + Duration::from_secs_f64(*duration);
                let t0 = Instant::now();
                while Instant::now() < deadline {
                    for ev in s
                        .poll_events()
                        .map_err(|e| ("wave.live".into(), e.to_string()))?
                    {
                        if let CapturedEvent::Metrics(m) = ev {
                            let mut row = MetricRow::from(&m);
                            row.rel_t = t0.elapsed().as_secs_f64();
                            rows.push(row);
                        }
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            let mut wave = metrics_to_wave(&rows, &chans);
            if let Some(obj) = wave.as_object_mut() {
                obj.insert("port".into(), serde_json::json!(port));
                obj.insert("baud".into(), serde_json::json!(baud));
                obj.insert("duration_sec".into(), serde_json::json!(duration));
                obj.insert("count".into(), serde_json::json!(rows.len()));
            }
            Ok(("wave.live".into(), wave))
        }
        Commands::Wave(WaveCmd::Session {
            session_id,
            rel_from,
            rel_to,
            channels,
            format,
        }) => {
            let conn = db_conn().map_err(|e| ("wave.session".into(), e))?;
            let _ = get_session(&conn, *session_id)
                .map_err(|e| ("wave.session".into(), e.to_string()))?
                .ok_or_else(|| {
                    (
                        "wave.session".into(),
                        format!("session {session_id} not found"),
                    )
                })?;
            let rows = fetch_session_metrics(&conn, *session_id, *rel_from, *rel_to)
                .map_err(|e| ("wave.session".into(), e.to_string()))?;
            if format == "csv" {
                let out = wiparse_core::paths::project_path(format!(
                    "artifacts/session_{session_id}_wave.csv"
                ));
                if let Some(parent) = out.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let n = export_metrics_csv(&out, &rows)
                    .map_err(|e| ("wave.session".into(), e.to_string()))?;
                return Ok((
                    "wave.session".into(),
                    serde_json::json!({
                        "path": out,
                        "count": n,
                        "format": "csv",
                    }),
                ));
            }
            let chans = parse_channels(channels);
            let mut wave = metrics_to_wave(&rows, &chans);
            if let Some(obj) = wave.as_object_mut() {
                obj.insert("session_id".into(), serde_json::json!(session_id));
                obj.insert("count".into(), serde_json::json!(rows.len()));
            }
            Ok(("wave.session".into(), wave))
        }
        Commands::Wave(WaveCmd::Export {
            session_id,
            rel_from,
            rel_to,
            format,
            out,
        }) => {
            let conn = db_conn().map_err(|e| ("wave.export".into(), e))?;
            let _ = get_session(&conn, *session_id)
                .map_err(|e| ("wave.export".into(), e.to_string()))?
                .ok_or_else(|| {
                    (
                        "wave.export".into(),
                        format!("session {session_id} not found"),
                    )
                })?;
            let rows = fetch_session_metrics(&conn, *session_id, *rel_from, *rel_to)
                .map_err(|e| ("wave.export".into(), e.to_string()))?;
            if let Some(parent) = out.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let n = if format == "json" || format == "jsonl" {
                export_metrics_json(out, &rows)
                    .map_err(|e| ("wave.export".into(), e.to_string()))?
            } else {
                export_metrics_csv(out, &rows).map_err(|e| ("wave.export".into(), e.to_string()))?
            };
            Ok((
                "wave.export".into(),
                serde_json::json!({
                    "path": out,
                    "count": n,
                    "format": format,
                }),
            ))
        }
        Commands::Scope(ScopeCmd::List) => {
            let scopes = scope::list_scopes().map_err(|e| ("scope.list".into(), e.to_string()))?;
            Ok((
                "scope.list".into(),
                serde_json::json!({
                    "scopes": scopes,
                    "capabilities": scope_capabilities(),
                }),
            ))
        }
        Commands::Scope(ScopeCmd::Shot { index, out }) => {
            let data = scope::capture_shot(*index, out.as_deref())
                .map_err(|e| ("scope.shot".into(), e.to_string()))?;
            Ok(("scope.shot".into(), data))
        }
        Commands::Scope(ScopeCmd::Wave {
            index,
            channel,
            points,
        }) => {
            let data = scope::read_waveform(*index, channel, *points)
                .map_err(|e| ("scope.wave".into(), e.to_string()))?;
            Ok(("scope.wave".into(), data))
        }
        Commands::Probe(cmd) => local_probe(cmd),
        Commands::Bridge(cmd) => local_bridge(cmd),
    }
}

fn parse_channels(spec: &str) -> Vec<&str> {
    let parts: Vec<&str> = spec
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        DEFAULT_CHANNELS.to_vec()
    } else {
        parts
    }
}

fn parse_many<'a>(lines: impl Iterator<Item = &'a str>, limit: Option<usize>) -> serde_json::Value {
    let mut out = Vec::new();
    let mut metrics = Vec::new();
    for (i, line) in lines.enumerate() {
        if limit.is_some_and(|n| i >= n) {
            break;
        }
        if let Some(m) = parse_metric_frame(line) {
            metrics.push(m);
        } else if line.contains("ASK ") || line.contains("FSK ") {
            out.push(parse_qi_line(line));
        }
    }
    serde_json::json!({ "qi": out, "metrics": metrics, "qi_count": out.len(), "metrics_count": metrics.len() })
}

fn map_probe_gui(cmd: &ProbeCmd) -> Option<(String, serde_json::Value)> {
    match cmd {
        ProbeCmd::List => Some(("instrument.list".into(), json!({ "kind": "debug_probe" }))),
        ProbeCmd::Connect { resource, chip: _ } => Some((
            "instrument.connect".into(),
            json!({ "resource": resource, "kind": "debug_probe" }),
        )),
        ProbeCmd::Halt { id, .. } => gui_dev_cmd(*id, json!("ProbeHalt")),
        ProbeCmd::Run { id, .. } => gui_dev_cmd(*id, json!("ProbeRun")),
        ProbeCmd::Reset { id, hardware, .. } => gui_dev_cmd(
            *id,
            json!({ "ProbeReset": { "hardware": hardware } }),
        ),
        ProbeCmd::Status { id, .. } => gui_dev_cmd(*id, json!("ProbeStatus")),
        ProbeCmd::Regs { id, .. } => gui_dev_cmd(*id, json!("ProbeRegs")),
        ProbeCmd::Flash {
            id, path, verify, ..
        } => gui_dev_cmd(
            *id,
            json!({ "ProbeFlash": { "path": path, "verify": verify } }),
        ),
        ProbeCmd::MemRead {
            id, address, len, ..
        } => gui_dev_cmd(
            *id,
            json!({ "ProbeMemRead": { "address": address, "len": len } }),
        ),
        ProbeCmd::MemWrite {
            id, address, hex, ..
        } => gui_dev_cmd(
            *id,
            json!({ "ProbeMemWrite": { "address": address, "data_hex": hex } }),
        ),
        ProbeCmd::Erase { id, .. } => gui_dev_cmd(*id, json!("ProbeErase")),
        ProbeCmd::Speed { id, khz, .. } => {
            gui_dev_cmd(*id, json!({ "ProbeSpeed": { "khz": khz } }))
        }
        ProbeCmd::RttStart { id, channel, .. } => gui_dev_cmd(
            *id,
            json!({ "ProbeRttStart": { "up_channel": channel } }),
        ),
        ProbeCmd::RttStop { id, .. } => gui_dev_cmd(*id, json!("ProbeRttStop")),
        ProbeCmd::RttRead { id, .. } => gui_dev_cmd(*id, json!("ProbeRttRead")),
    }
}

fn map_bridge_gui(cmd: &BridgeCmd) -> Option<(String, serde_json::Value)> {
    match cmd {
        BridgeCmd::List => Some(("instrument.list".into(), json!({ "kind": "usb_bridge" }))),
        BridgeCmd::Connect { resource } => Some((
            "instrument.connect".into(),
            json!({ "resource": resource, "kind": "usb_bridge" }),
        )),
        BridgeCmd::Info { id, .. } => gui_dev_cmd(*id, json!("BridgeInfo")),
        BridgeCmd::Spi {
            id,
            mode,
            clock_hz,
            write,
            read_len,
            ..
        } => gui_dev_cmd(
            *id,
            json!({
                "BridgeSpi": {
                    "mode": mode,
                    "clock_hz": clock_hz,
                    "write_hex": write,
                    "read_len": read_len
                }
            }),
        ),
        BridgeCmd::I2c {
            id,
            addr,
            write,
            read_len,
            clock_hz,
            ..
        } => gui_dev_cmd(
            *id,
            json!({
                "BridgeI2c": {
                    "addr": addr,
                    "write_hex": write,
                    "read_len": read_len,
                    "clock_hz": clock_hz
                }
            }),
        ),
        BridgeCmd::Gpio {
            id,
            pin,
            output,
            high,
            ..
        } => gui_dev_cmd(
            *id,
            json!({ "BridgeGpio": { "pin": pin, "dir": output, "value": high } }),
        ),
    }
}

fn gui_dev_cmd(id: Option<u64>, command: serde_json::Value) -> Option<(String, serde_json::Value)> {
    Some((
        "instrument.command".into(),
        json!({ "device_id": id?, "command": command }),
    ))
}

fn local_probe(cmd: &ProbeCmd) -> Result<(String, serde_json::Value), (String, String)> {
    use wiparse_core::instrument::ControlCommand;
    match cmd {
        ProbeCmd::List => Ok(("probe.list".into(), hw::list_probes())),
        ProbeCmd::Connect { .. } => Err((
            "probe.connect".into(),
            "connect keeps a session in WiParse.exe; drop --local".into(),
        )),
        other => {
            let (command, resource, chip, demo, name) = match other {
                ProbeCmd::Halt {
                    resource,
                    chip,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeHalt,
                    resource,
                    chip,
                    *demo,
                    "probe.halt",
                ),
                ProbeCmd::Run {
                    resource,
                    chip,
                    demo,
                    ..
                } => (ControlCommand::ProbeRun, resource, chip, *demo, "probe.run"),
                ProbeCmd::Reset {
                    resource,
                    chip,
                    hardware,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeReset {
                        hardware: *hardware,
                    },
                    resource,
                    chip,
                    *demo,
                    "probe.reset",
                ),
                ProbeCmd::Status {
                    resource,
                    chip,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeStatus,
                    resource,
                    chip,
                    *demo,
                    "probe.status",
                ),
                ProbeCmd::Regs {
                    resource,
                    chip,
                    demo,
                    ..
                } => (ControlCommand::ProbeRegs, resource, chip, *demo, "probe.regs"),
                ProbeCmd::Flash {
                    resource,
                    chip,
                    path,
                    verify,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeFlash {
                        path: path.clone(),
                        verify: *verify,
                        base_address: None,
                    },
                    resource,
                    chip,
                    *demo,
                    "probe.flash",
                ),
                ProbeCmd::MemRead {
                    resource,
                    chip,
                    address,
                    len,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeMemRead {
                        address: address.clone(),
                        len: *len,
                    },
                    resource,
                    chip,
                    *demo,
                    "probe.mem-read",
                ),
                ProbeCmd::MemWrite {
                    resource,
                    chip,
                    address,
                    hex,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeMemWrite {
                        address: address.clone(),
                        data_hex: hex.clone(),
                    },
                    resource,
                    chip,
                    *demo,
                    "probe.mem-write",
                ),
                ProbeCmd::Erase {
                    resource,
                    chip,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeErase,
                    resource,
                    chip,
                    *demo,
                    "probe.erase",
                ),
                ProbeCmd::Speed {
                    resource,
                    chip,
                    khz,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeSpeed { khz: *khz },
                    resource,
                    chip,
                    *demo,
                    "probe.speed",
                ),
                ProbeCmd::RttStart {
                    resource,
                    chip,
                    channel,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeRttStart {
                        up_channel: *channel,
                    },
                    resource,
                    chip,
                    *demo,
                    "probe.rtt-start",
                ),
                ProbeCmd::RttStop {
                    resource,
                    chip,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeRttStop,
                    resource,
                    chip,
                    *demo,
                    "probe.rtt-stop",
                ),
                ProbeCmd::RttRead {
                    resource,
                    chip,
                    demo,
                    ..
                } => (
                    ControlCommand::ProbeRttRead,
                    resource,
                    chip,
                    *demo,
                    "probe.rtt-read",
                ),
                ProbeCmd::List | ProbeCmd::Connect { .. } => unreachable!(),
            };
            let data = if demo {
                hw::demo_probe(command).map_err(|e| (name.into(), e))?
            } else {
                let resource = resource.as_ref().ok_or_else(|| {
                    (
                        name.to_string(),
                        "--local needs --resource (or --demo)".to_string(),
                    )
                })?;
                hw::probe_exec(resource, chip.as_deref(), command)
                    .map_err(|e| (name.into(), e))?
            };
            Ok((name.into(), data))
        }
    }
}

fn local_bridge(cmd: &BridgeCmd) -> Result<(String, serde_json::Value), (String, String)> {
    use wiparse_core::instrument::ControlCommand;
    match cmd {
        BridgeCmd::List => Ok(("bridge.list".into(), hw::list_bridges())),
        BridgeCmd::Connect { .. } => Err((
            "bridge.connect".into(),
            "connect keeps a session in WiParse.exe; drop --local".into(),
        )),
        other => {
            let (command, resource, demo, name) = match other {
                BridgeCmd::Info {
                    resource, demo, ..
                } => (ControlCommand::BridgeInfo, resource, *demo, "bridge.info"),
                BridgeCmd::Spi {
                    resource,
                    mode,
                    clock_hz,
                    write,
                    read_len,
                    demo,
                    ..
                } => (
                    ControlCommand::BridgeSpi {
                        mode: *mode,
                        clock_hz: *clock_hz,
                        cs: 0,
                        write_hex: write.clone(),
                        read_len: *read_len,
                    },
                    resource,
                    *demo,
                    "bridge.spi",
                ),
                BridgeCmd::I2c {
                    resource,
                    addr,
                    write,
                    read_len,
                    clock_hz,
                    demo,
                    ..
                } => (
                    ControlCommand::BridgeI2c {
                        addr: addr.clone(),
                        write_hex: write.clone(),
                        read_len: *read_len,
                        clock_hz: *clock_hz,
                    },
                    resource,
                    *demo,
                    "bridge.i2c",
                ),
                BridgeCmd::Gpio {
                    resource,
                    pin,
                    output,
                    high,
                    demo,
                    ..
                } => (
                    ControlCommand::BridgeGpio {
                        pin: *pin,
                        dir: *output,
                        value: *high,
                    },
                    resource,
                    *demo,
                    "bridge.gpio",
                ),
                BridgeCmd::List | BridgeCmd::Connect { .. } => unreachable!(),
            };
            let data = if demo {
                hw::demo_bridge(command).map_err(|e| (name.into(), e))?
            } else {
                let resource = resource.as_ref().ok_or_else(|| {
                    (
                        name.to_string(),
                        "--local needs --resource (or --demo)".to_string(),
                    )
                })?;
                hw::bridge_exec(resource, command).map_err(|e| (name.into(), e))?
            };
            Ok((name.into(), data))
        }
    }
}
