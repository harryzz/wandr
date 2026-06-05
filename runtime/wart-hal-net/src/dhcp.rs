//! Minimal pure-Rust DHCPv4 client (task 88, M1).
//!
//! Android does DHCP in NetworkStack (Java, `DhcpClient`), which dies with the
//! framework under `--no-art`; no userspace DHCP binary ships (udhcpc / dhcpcd /
//! dhcptool are all absent). This is the one genuinely-missing native piece, so
//! we bring our own: a DISCOVER → OFFER → REQUEST → ACK exchange over a UDP
//! socket bound to the link (`0.0.0.0:68`, `SO_BINDTODEVICE`, broadcast), parsing
//! out the address + mask + gateway + DNS + lease.
//!
//! Deliberately small: a single 4-packet round, one retry budget, no RENEW/REBIND
//! state machine (the daemon re-runs us on lease expiry / link change). The packet
//! *parse* is split out and unit-tested off-device.

use std::io;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::os::fd::AsRawFd;
use std::time::Duration;

/// The result of a successful lease (what the daemon applies to the link).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DhcpLease {
    pub ip: Ipv4Addr,
    /// Subnet prefix length derived from option 1 (e.g. 24 for 255.255.255.0).
    pub prefix: u8,
    pub gateway: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
    /// Lease time, seconds (option 51); 0 if absent.
    pub lease_secs: u32,
    /// DHCP server identity (option 54) — echoed back in REQUEST.
    pub server_id: Option<Ipv4Addr>,
}

impl Default for DhcpLease {
    fn default() -> Self {
        DhcpLease {
            ip: Ipv4Addr::UNSPECIFIED,
            prefix: 0,
            gateway: None,
            dns: Vec::new(),
            lease_secs: 0,
            server_id: None,
        }
    }
}

const OP_REQUEST: u8 = 1;
const OP_REPLY: u8 = 2;
const HTYPE_ETHER: u8 = 1;
const HLEN_ETHER: u8 = 6;
const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];

// DHCP message types (option 53).
const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;
const DHCP_NAK: u8 = 6;

// Option codes.
const OPT_SUBNET: u8 = 1;
const OPT_ROUTER: u8 = 3;
const OPT_DNS: u8 = 6;
const OPT_LEASE: u8 = 51;
const OPT_MSG_TYPE: u8 = 53;
const OPT_SERVER_ID: u8 = 54;
const OPT_PARAM_LIST: u8 = 55;
const OPT_REQUESTED_IP: u8 = 50;
const OPT_END: u8 = 255;

/// A parsed DHCP reply (OFFER or ACK) — the fields we care about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DhcpReply {
    pub msg_type: u8,
    pub yiaddr: Ipv4Addr,
    pub subnet: Option<Ipv4Addr>,
    pub router: Option<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
    pub lease_secs: u32,
    pub server_id: Option<Ipv4Addr>,
}

impl Default for DhcpReply {
    fn default() -> Self {
        DhcpReply {
            msg_type: 0,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            subnet: None,
            router: None,
            dns: Vec::new(),
            lease_secs: 0,
            server_id: None,
        }
    }
}

fn ipv4(b: &[u8]) -> Option<Ipv4Addr> {
    if b.len() == 4 {
        Some(Ipv4Addr::new(b[0], b[1], b[2], b[3]))
    } else {
        None
    }
}

/// Convert a subnet mask to a prefix length (popcount of the mask bits).
fn mask_to_prefix(mask: Ipv4Addr) -> u8 {
    u32::from(mask).count_ones() as u8
}

/// Parse a raw DHCP reply packet for the fields we use. Returns `None` if it
/// isn't a well-formed BOOTREPLY with the magic cookie and a message-type option.
/// Pure + side-effect-free so it can be unit-tested off-device.
pub fn parse_reply(buf: &[u8], expect_xid: u32) -> Option<DhcpReply> {
    // Fixed BOOTP header is 236 bytes, then 4-byte cookie, then options.
    if buf.len() < 240 {
        return None;
    }
    if buf[0] != OP_REPLY {
        return None;
    }
    let xid = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    if xid != expect_xid {
        return None;
    }
    let yiaddr = ipv4(&buf[16..20])?;
    if buf[236..240] != MAGIC_COOKIE {
        return None;
    }

    let mut out = DhcpReply {
        yiaddr,
        ..Default::default()
    };
    let mut i = 240;
    while i < buf.len() {
        let code = buf[i];
        if code == OPT_END {
            break;
        }
        if code == 0 {
            // pad
            i += 1;
            continue;
        }
        if i + 1 >= buf.len() {
            break;
        }
        let len = buf[i + 1] as usize;
        let val_start = i + 2;
        let val_end = val_start + len;
        if val_end > buf.len() {
            break;
        }
        let val = &buf[val_start..val_end];
        match code {
            OPT_MSG_TYPE => {
                if let Some(&t) = val.first() {
                    out.msg_type = t;
                }
            }
            OPT_SUBNET => out.subnet = ipv4(val),
            OPT_ROUTER => out.router = ipv4(val),
            OPT_DNS => {
                out.dns = val.chunks(4).filter_map(ipv4).collect();
            }
            OPT_LEASE => {
                if val.len() == 4 {
                    out.lease_secs = u32::from_be_bytes([val[0], val[1], val[2], val[3]]);
                }
            }
            OPT_SERVER_ID => out.server_id = ipv4(val),
            _ => {}
        }
        i = val_end;
    }
    if out.msg_type == 0 {
        return None;
    }
    Some(out)
}

/// Build a DHCP request packet (DISCOVER or REQUEST) with the given options.
fn build_packet(
    msg_type: u8,
    xid: u32,
    mac: [u8; 6],
    requested_ip: Option<Ipv4Addr>,
    server_id: Option<Ipv4Addr>,
) -> Vec<u8> {
    let mut p = vec![0u8; 240];
    p[0] = OP_REQUEST;
    p[1] = HTYPE_ETHER;
    p[2] = HLEN_ETHER;
    p[3] = 0; // hops
    p[4..8].copy_from_slice(&xid.to_be_bytes());
    // secs = 0, flags = broadcast (0x8000) so the server replies to broadcast
    // (we have no IP yet to receive a unicast).
    p[10] = 0x80;
    p[11] = 0x00;
    // chaddr (client MAC) at offset 28.
    p[28..34].copy_from_slice(&mac);
    // magic cookie at 236.
    p[236..240].copy_from_slice(&MAGIC_COOKIE);

    // Options.
    let mut opts: Vec<u8> = Vec::new();
    opts.extend_from_slice(&[OPT_MSG_TYPE, 1, msg_type]);
    if let Some(ip) = requested_ip {
        opts.push(OPT_REQUESTED_IP);
        opts.push(4);
        opts.extend_from_slice(&ip.octets());
    }
    if let Some(sid) = server_id {
        opts.push(OPT_SERVER_ID);
        opts.push(4);
        opts.extend_from_slice(&sid.octets());
    }
    opts.extend_from_slice(&[OPT_PARAM_LIST, 4, OPT_SUBNET, OPT_ROUTER, OPT_DNS, OPT_LEASE]);
    opts.push(OPT_END);
    p.extend_from_slice(&opts);
    p
}

/// Read the interface MAC from sysfs (`/sys/class/net/<if>/address`).
fn read_mac(ifname: &str) -> io::Result<[u8; 6]> {
    let s = std::fs::read_to_string(format!("/sys/class/net/{ifname}/address"))?;
    let mut mac = [0u8; 6];
    for (i, part) in s.trim().split(':').enumerate() {
        if i >= 6 {
            break;
        }
        mac[i] = u8::from_str_radix(part, 16)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad MAC byte"))?;
    }
    Ok(mac)
}

/// Bind a UDP socket to `0.0.0.0:68` on `ifname` with broadcast enabled. Needs
/// privilege (port < 1024 + `SO_BINDTODEVICE`); the daemon runs as root/system.
fn open_socket(ifname: &str) -> io::Result<UdpSocket> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 68))?;
    sock.set_broadcast(true)?;
    sock.set_read_timeout(Some(Duration::from_secs(3)))?;
    // SO_BINDTODEVICE so the broadcast goes out the right link (no IP/route yet).
    let cif = std::ffi::CString::new(ifname).unwrap();
    let rc = unsafe {
        libc::setsockopt(
            sock.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            cif.as_ptr() as *const libc::c_void,
            (cif.as_bytes_with_nul().len()) as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(sock)
}

fn xid_now() -> u32 {
    // Cheap unique-ish transaction id from the clock + pid.
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    nanos ^ (std::process::id().wrapping_mul(2654435761))
}

/// Run a full DHCPv4 lease cycle on `ifname` (which must already have carrier).
/// Returns the lease on ACK. `tries` bounds the DISCOVER/REQUEST retransmits.
pub fn acquire(ifname: &str, tries: u32) -> io::Result<DhcpLease> {
    let mac = read_mac(ifname)?;
    let sock = open_socket(ifname)?;
    let bcast = SocketAddrV4::new(Ipv4Addr::BROADCAST, 67);
    let xid = xid_now();

    let mut buf = [0u8; 1500];
    for attempt in 0..tries.max(1) {
        // ── DISCOVER ──
        let discover = build_packet(DHCP_DISCOVER, xid, mac, None, None);
        sock.send_to(&discover, bcast)?;
        log::debug!("dhcp: DISCOVER sent on {ifname} (attempt {})", attempt + 1);

        // ── OFFER ──
        let offer = match recv_reply(&sock, &mut buf, xid, DHCP_OFFER) {
            Some(r) => r,
            None => continue, // timeout → retry DISCOVER
        };
        log::info!(
            "dhcp: OFFER {} from server {:?}",
            offer.yiaddr,
            offer.server_id
        );

        // ── REQUEST ──
        let request = build_packet(DHCP_REQUEST, xid, mac, Some(offer.yiaddr), offer.server_id);
        sock.send_to(&request, bcast)?;

        // ── ACK ──
        match recv_reply(&sock, &mut buf, xid, DHCP_ACK) {
            Some(ack) => {
                let prefix = ack.subnet.map(mask_to_prefix).unwrap_or(24);
                let lease = DhcpLease {
                    ip: ack.yiaddr,
                    prefix,
                    gateway: ack.router,
                    dns: ack.dns,
                    lease_secs: ack.lease_secs,
                    server_id: ack.server_id,
                };
                log::info!(
                    "dhcp: ACK ip={}/{} gw={:?} dns={:?} lease={}s",
                    lease.ip,
                    lease.prefix,
                    lease.gateway,
                    lease.dns,
                    lease.lease_secs
                );
                return Ok(lease);
            }
            None => continue,
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "DHCP: no OFFER/ACK after retries",
    ))
}

/// Receive packets until one parses as the wanted message type (or NAK / timeout).
fn recv_reply(
    sock: &UdpSocket,
    buf: &mut [u8],
    xid: u32,
    want: u8,
) -> Option<DhcpReply> {
    // A few reads so stray broadcasts (other clients' traffic) don't end the wait.
    for _ in 0..4 {
        let n = sock.recv(buf).ok()?;
        if let Some(r) = parse_reply(&buf[..n], xid) {
            if r.msg_type == DHCP_NAK {
                log::warn!("dhcp: NAK");
                return None;
            }
            if r.msg_type == want {
                return Some(r);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic ACK and round-trip it through the parser.
    #[test]
    fn parse_ack_roundtrip() {
        let xid: u32 = 0xdead_beef;
        let mut p = vec![0u8; 240];
        p[0] = OP_REPLY;
        p[4..8].copy_from_slice(&xid.to_be_bytes());
        p[16..20].copy_from_slice(&[192, 168, 1, 50]); // yiaddr
        p[236..240].copy_from_slice(&MAGIC_COOKIE);
        // options: type=ACK, subnet /24, router, two DNS, lease 3600, server id
        let opts: &[u8] = &[
            OPT_MSG_TYPE, 1, DHCP_ACK,
            OPT_SUBNET, 4, 255, 255, 255, 0,
            OPT_ROUTER, 4, 192, 168, 1, 1,
            OPT_DNS, 8, 8, 8, 8, 8, 1, 1, 1, 1,
            OPT_LEASE, 4, 0, 0, 0x0e, 0x10, // 3600
            OPT_SERVER_ID, 4, 192, 168, 1, 1,
            OPT_END,
        ];
        p.extend_from_slice(opts);

        let r = parse_reply(&p, xid).expect("parse");
        assert_eq!(r.msg_type, DHCP_ACK);
        assert_eq!(r.yiaddr, Ipv4Addr::new(192, 168, 1, 50));
        assert_eq!(r.subnet, Some(Ipv4Addr::new(255, 255, 255, 0)));
        assert_eq!(mask_to_prefix(r.subnet.unwrap()), 24);
        assert_eq!(r.router, Some(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(r.dns, vec![Ipv4Addr::new(8, 8, 8, 8), Ipv4Addr::new(1, 1, 1, 1)]);
        assert_eq!(r.lease_secs, 3600);
        assert_eq!(r.server_id, Some(Ipv4Addr::new(192, 168, 1, 1)));
    }

    #[test]
    fn rejects_wrong_xid() {
        let mut p = vec![0u8; 240];
        p[0] = OP_REPLY;
        p[4..8].copy_from_slice(&0x1111_1111u32.to_be_bytes());
        p[236..240].copy_from_slice(&MAGIC_COOKIE);
        p.extend_from_slice(&[OPT_MSG_TYPE, 1, DHCP_ACK, OPT_END]);
        assert!(parse_reply(&p, 0x2222_2222).is_none());
    }

    #[test]
    fn rejects_no_cookie() {
        let mut p = vec![0u8; 240];
        p[0] = OP_REPLY;
        p.extend_from_slice(&[OPT_MSG_TYPE, 1, DHCP_ACK, OPT_END]);
        assert!(parse_reply(&p, 0).is_none());
    }

    #[test]
    fn build_discover_has_cookie_and_type() {
        let pkt = build_packet(DHCP_DISCOVER, 0x1234, [1, 2, 3, 4, 5, 6], None, None);
        assert_eq!(pkt[0], OP_REQUEST);
        assert_eq!(&pkt[236..240], &MAGIC_COOKIE);
        assert_eq!(&pkt[28..34], &[1, 2, 3, 4, 5, 6]);
        // first option is message type = DISCOVER
        assert_eq!(&pkt[240..243], &[OPT_MSG_TYPE, 1, DHCP_DISCOVER]);
    }
}
