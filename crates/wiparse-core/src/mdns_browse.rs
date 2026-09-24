//! Background browse of `_uart-bridge._tcp` on every up IPv4 interface.
//! Queries ask for a unicast reply so the socket does not have to bind port 5353.

use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const SERVICE: &str = "_uart-bridge._tcp.local";

#[derive(Clone, Debug)]
pub struct UartBridgeRecord {
    pub instance: String,
    pub host: String,
    pub ipv4: Ipv4Addr,
    pub port: u16,
    pub id: String,
    pub baud: String,
    pub usb_busy: bool,
}

impl UartBridgeRecord {
    pub fn dedup_key(&self) -> String {
        format!(
            "{}|{}|{}",
            self.instance, self.ipv4, self.port
        )
    }

    pub fn device(&self) -> String {
        format!("tcp://{}:{}", self.ipv4, self.port)
    }
}

struct BrowseState {
    rows: Vec<UartBridgeRecord>,
    revision: u64,
    started: bool,
}

static BROWSE: Mutex<BrowseState> = Mutex::new(BrowseState {
    rows: Vec::new(),
    revision: 0,
    started: false,
});

static BROWSE_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

pub fn start_uart_bridge_browse() {
    let mut guard = BROWSE.lock().unwrap_or_else(|e| e.into_inner());
    if guard.started {
        return;
    }
    guard.started = true;
    drop(guard);
    let handle = thread::Builder::new()
        .name("wiparse-mdns-uart".into())
        .spawn(browse_loop)
        .ok();
    if let Ok(mut slot) = BROWSE_THREAD.lock() {
        *slot = handle;
    }
}

pub fn uart_bridge_rows() -> Vec<UartBridgeRecord> {
    start_uart_bridge_browse();
    BROWSE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .rows
        .clone()
}

pub fn uart_bridge_revision() -> u64 {
    BROWSE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .revision
}

pub fn resolve_uart_ipv4(host: &str) -> Option<Ipv4Addr> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Some(ip);
    }
    let rows = uart_bridge_rows();
    rows.iter().find_map(|row| {
        let name = row.host.trim_end_matches('.').to_ascii_lowercase();
        let short = name.split('.').next().unwrap_or("");
        if name == host || short == host || format!("{short}.local") == host {
            Some(row.ipv4)
        } else {
            None
        }
    })
}

fn browse_loop() {
    let mut partial: Vec<Partial> = Vec::new();
    loop {
        let ifaces = local_ipv4s();
        let mut sockets = Vec::new();
        let targets: Vec<Ipv4Addr> = if ifaces.is_empty() {
            vec![Ipv4Addr::UNSPECIFIED]
        } else {
            ifaces
        };
        for ip in targets {
            if let Ok(sock) = UdpSocket::bind(SocketAddrV4::new(ip, 0)) {
                let _ = sock.set_read_timeout(Some(Duration::from_millis(30)));
                let _ = sock.send_to(&ptr_query(), "224.0.0.251:5353");
                sockets.push(sock);
            }
        }
        let deadline = Instant::now() + Duration::from_millis(700);
        while Instant::now() < deadline {
            for sock in &sockets {
                let mut buf = [0u8; 1500];
                if let Ok((n, _)) = sock.recv_from(&mut buf) {
                    ingest(&buf[..n], &mut partial);
                }
            }
        }
        publish(&partial);
        partial.retain(|p| p.seen.elapsed() < Duration::from_secs(12));
        thread::sleep(Duration::from_secs(2));
    }
}

struct Partial {
    instance: String,
    host: String,
    ipv4: Option<Ipv4Addr>,
    port: u16,
    id: String,
    baud: String,
    usb_busy: bool,
    seen: Instant,
}

fn publish(partial: &[Partial]) {
    let mut rows = Vec::new();
    for item in partial {
        let Some(ip) = item.ipv4 else { continue };
        if item.port == 0 {
            continue;
        }
        let rec = UartBridgeRecord {
            instance: item.instance.clone(),
            host: item.host.clone(),
            ipv4: ip,
            port: item.port,
            id: item.id.clone(),
            baud: item.baud.clone(),
            usb_busy: item.usb_busy,
        };
        if rows.iter().any(|r: &UartBridgeRecord| r.dedup_key() == rec.dedup_key()) {
            continue;
        }
        rows.push(rec);
    }
    rows.sort_by(|a, b| a.instance.cmp(&b.instance).then(a.ipv4.cmp(&b.ipv4)));
    let mut state = BROWSE.lock().unwrap_or_else(|e| e.into_inner());
    let same = state.rows.len() == rows.len()
        && state.rows.iter().zip(rows.iter()).all(|(a, b)| {
            a.dedup_key() == b.dedup_key() && a.usb_busy == b.usb_busy && a.baud == b.baud
        });
    if !same {
        state.rows = rows;
        state.revision = state.revision.wrapping_add(1);
    }
}

fn ingest(packet: &[u8], partial: &mut Vec<Partial>) {
    if packet.len() < 12 {
        return;
    }
    let qd = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let an = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let ns = u16::from_be_bytes([packet[8], packet[9]]) as usize;
    let ar = u16::from_be_bytes([packet[10], packet[11]]) as usize;
    let mut i = 12;
    for _ in 0..qd {
        if skip_name(packet, &mut i).is_none() {
            return;
        }
        i = i.saturating_add(4);
    }
    for _ in 0..(an + ns + ar) {
        let Some(name) = read_name(packet, &mut i) else { return };
        if i + 10 > packet.len() {
            return;
        }
        let typ = u16::from_be_bytes([packet[i], packet[i + 1]]);
        let ttl = u32::from_be_bytes([packet[i + 4], packet[i + 5], packet[i + 6], packet[i + 7]]);
        let rdlen = u16::from_be_bytes([packet[i + 8], packet[i + 9]]) as usize;
        i += 10;
        if i + rdlen > packet.len() {
            return;
        }
        let rdata_at = i;
        i += rdlen;
        let _ = ttl;
        apply_rr(packet, &name, typ, rdata_at, rdlen, partial);
    }
}

fn apply_rr(
    packet: &[u8],
    name: &str,
    typ: u16,
    rdata_at: usize,
    rdlen: usize,
    partial: &mut Vec<Partial>,
) {
    let rdata = packet.get(rdata_at..rdata_at + rdlen).unwrap_or(&[]);
    match typ {
        12 => {
            let mut o = rdata_at;
            if let Some(target) = read_name(packet, &mut o) {
                let instance = instance_of(&target);
                touch(partial, &instance).seen = Instant::now();
            }
        }
        33 => {
            if rdlen < 6 || rdata_at + 6 > packet.len() {
                return;
            }
            let port = u16::from_be_bytes([packet[rdata_at + 4], packet[rdata_at + 5]]);
            let mut o = rdata_at + 6;
            let host = read_name(packet, &mut o).unwrap_or_default();
            let instance = instance_of(name);
            let row = touch(partial, &instance);
            row.port = port;
            if !host.is_empty() {
                row.host = host;
            }
            row.seen = Instant::now();
        }
        1 => {
            if rdata.len() < 4 {
                return;
            }
            let ip = Ipv4Addr::new(rdata[0], rdata[1], rdata[2], rdata[3]);
            let host = name.trim_end_matches('.').to_ascii_lowercase();
            for row in partial.iter_mut() {
                let h = row.host.trim_end_matches('.').to_ascii_lowercase();
                if !h.is_empty() && h == host {
                    row.ipv4 = Some(ip);
                    row.seen = Instant::now();
                }
            }
        }
        16 => {
            let instance = instance_of(name);
            let row = touch(partial, &instance);
            for (k, v) in txt_pairs(rdata) {
                match k.as_str() {
                    "id" => row.id = v,
                    "baud" => row.baud = v,
                    "usb" => row.usb_busy = v == "1",
                    _ => {}
                }
            }
            row.seen = Instant::now();
        }
        _ => {}
    }
}

fn touch<'a>(partial: &'a mut Vec<Partial>, instance: &str) -> &'a mut Partial {
    if let Some(i) = partial.iter().position(|p| p.instance == instance) {
        return &mut partial[i];
    }
    partial.push(Partial {
        instance: instance.to_owned(),
        host: String::new(),
        ipv4: None,
        port: 0,
        id: String::new(),
        baud: String::new(),
        usb_busy: false,
        seen: Instant::now(),
    });
    partial.last_mut().unwrap()
}

fn instance_of(name: &str) -> String {
    name.split('.').next().unwrap_or(name).to_owned()
}

fn txt_pairs(rdata: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < rdata.len() {
        let n = rdata[i] as usize;
        i += 1;
        if i + n > rdata.len() {
            break;
        }
        let s = String::from_utf8_lossy(&rdata[i..i + n]).to_string();
        i += n;
        if let Some((k, v)) = s.split_once('=') {
            out.push((k.to_owned(), v.to_owned()));
        }
    }
    out
}

fn ptr_query() -> Vec<u8> {
    let mut pkt = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in ["_uart-bridge", "_tcp", "local"] {
        pkt.push(label.len() as u8);
        pkt.extend(label.as_bytes());
    }
    pkt.push(0);
    pkt.extend_from_slice(&12u16.to_be_bytes());
    pkt.extend_from_slice(&0x8001u16.to_be_bytes());
    let _ = SERVICE;
    pkt
}

fn skip_name(packet: &[u8], i: &mut usize) -> Option<()> {
    read_name(packet, i).map(|_| ())
}

fn read_name(packet: &[u8], i: &mut usize) -> Option<String> {
    let mut out = String::new();
    let mut guard = 0;
    let mut pos = *i;
    let mut jumped = false;
    loop {
        if pos >= packet.len() || guard > 64 {
            return None;
        }
        guard += 1;
        let len = packet[pos];
        if len == 0 {
            if !jumped {
                *i = pos + 1;
            }
            break;
        }
        if len & 0xC0 == 0xC0 {
            if pos + 1 >= packet.len() {
                return None;
            }
            let ptr = (((len as usize) & 0x3F) << 8) | packet[pos + 1] as usize;
            if !jumped {
                *i = pos + 2;
                jumped = true;
            }
            pos = ptr;
            continue;
        }
        pos += 1;
        if pos + len as usize > packet.len() {
            return None;
        }
        if !out.is_empty() {
            out.push('.');
        }
        out.push_str(&String::from_utf8_lossy(&packet[pos..pos + len as usize]));
        pos += len as usize;
        if !jumped {
            *i = pos;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressed_srv_and_ptr_still_publish_ipv4() {
        let packet = [
            0x00, 0x00, 0x84, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x03, 0x0c, 0x5f,
            0x75, 0x61, 0x72, 0x74, 0x2d, 0x62, 0x72, 0x69, 0x64, 0x67, 0x65, 0x04, 0x5f, 0x74,
            0x63, 0x70, 0x05, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x00, 0x00, 0x0c, 0x00, 0x01, 0x00,
            0x00, 0x11, 0x94, 0x00, 0x15, 0x12, 0x57, 0x69, 0x50, 0x61, 0x72, 0x73, 0x65, 0x44,
            0x6f, 0x6e, 0x67, 0x6c, 0x65, 0x5f, 0x32, 0x36, 0x30, 0x30, 0xc0, 0x0c, 0xc0, 0x2f,
            0x00, 0x21, 0x00, 0x01, 0x00, 0x00, 0x00, 0x78, 0x00, 0x1b, 0x00, 0x00, 0x00, 0x00,
            0x04, 0xd2, 0x12, 0x77, 0x69, 0x70, 0x61, 0x72, 0x73, 0x65, 0x64, 0x6f, 0x6e, 0x67,
            0x6c, 0x65, 0x2d, 0x32, 0x36, 0x30, 0x30, 0xc0, 0x1e, 0xc0, 0x2f, 0x00, 0x10, 0x00,
            0x01, 0x00, 0x00, 0x11, 0x94, 0x00, 0x1a, 0x05, 0x75, 0x73, 0x62, 0x3d, 0x30, 0x0b,
            0x62, 0x61, 0x75, 0x64, 0x3d, 0x31, 0x31, 0x35, 0x32, 0x30, 0x30, 0x07, 0x69, 0x64,
            0x3d, 0x32, 0x36, 0x30, 0x30, 0xc0, 0x56, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x78, 0x00, 0x04, 0xc0, 0xa8, 0x01, 0x59,
        ];
        let mut partial = Vec::new();
        ingest(&packet, &mut partial);
        let row = partial
            .iter()
            .find(|p| p.instance == "WiParseDongle_2600")
            .expect("instance");
        assert_eq!(row.port, 1234);
        assert_eq!(row.host, "wiparsedongle-2600.local");
        assert_eq!(row.ipv4, Some(Ipv4Addr::new(192, 168, 1, 89)));
        assert_eq!(row.id, "2600");
        assert!(!row.usb_busy);
    }
}

#[cfg(windows)]
fn local_ipv4s() -> Vec<Ipv4Addr> {
    use std::ptr;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    unsafe {
        let mut size = 16 * 1024u32;
        let mut buf = vec![0u8; size as usize];
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let ret = GetAdaptersAddresses(
            AF_INET as u32,
            flags,
            ptr::null(),
            buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
            &mut size,
        );
        if ret != 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !p.is_null() {
            if (*p).OperStatus == IfOperStatusUp {
                let mut u = (*p).FirstUnicastAddress;
                while !u.is_null() {
                    let sa = (*u).Address.lpSockaddr;
                    if !sa.is_null() && (*sa).sa_family == AF_INET as u16 {
                        let sin = sa as *const SOCKADDR_IN;
                        let addr = (*sin).sin_addr.S_un.S_un_b;
                        let ip = Ipv4Addr::new(addr.s_b1, addr.s_b2, addr.s_b3, addr.s_b4);
                        if !ip.is_loopback() && !ip.is_unspecified() && !out.contains(&ip) {
                            out.push(ip);
                        }
                    }
                    u = (*u).Next;
                }
            }
            p = (*p).Next;
        }
        out
    }
}

#[cfg(not(windows))]
fn local_ipv4s() -> Vec<Ipv4Addr> {
    Vec::new()
}
