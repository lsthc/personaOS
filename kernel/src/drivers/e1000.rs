use core::fmt::Write as _;
use core::sync::atomic::{fence, Ordering};

use persona_shared::HHDM_OFFSET;
use spin::Mutex;

use crate::drivers::pci;
use crate::drivers::serial::SerialPort;
use crate::mm::{pmm, vmm, PAGE_SIZE};

const RX_COUNT: usize = 16;
const TX_COUNT: usize = 16;
const TX_BUF_SIZE: usize = 2048;

const REG_CTRL: usize = 0x0000;
const REG_STATUS: usize = 0x0008;
const REG_IMC: usize = 0x00D8;
const REG_RCTL: usize = 0x0100;
const REG_TCTL: usize = 0x0400;
const REG_TIPG: usize = 0x0410;
const REG_RDBAL: usize = 0x2800;
const REG_RDBAH: usize = 0x2804;
const REG_RDLEN: usize = 0x2808;
const REG_RDH: usize = 0x2810;
const REG_RDT: usize = 0x2818;
const REG_TDBAL: usize = 0x3800;
const REG_TDBAH: usize = 0x3804;
const REG_TDLEN: usize = 0x3808;
const REG_TDH: usize = 0x3810;
const REG_TDT: usize = 0x3818;
const REG_RAL0: usize = 0x5400;
const REG_RAH0: usize = 0x5404;

const CTRL_SLU: u32 = 1 << 6;
const CTRL_RST: u32 = 1 << 26;
const STATUS_LU: u32 = 1 << 1;
const RCTL_EN: u32 = 1 << 1;
const RCTL_BAM: u32 = 1 << 15;
const RCTL_SECRC: u32 = 1 << 26;
const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3;
const TX_CMD_EOP: u8 = 1 << 0;
const TX_CMD_IFCS: u8 = 1 << 1;
const TX_CMD_RS: u8 = 1 << 3;
const DESC_STATUS_DD: u8 = 1 << 0;
const RX_STATUS_EOP: u8 = 1 << 1;
const DHCP_XID: u32 = 0x706f_7335;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct NetInfo {
    pub present: u32,
    pub link_up: u32,
    pub configured: u32,
    pub tx_packets: u32,
    pub rx_packets: u32,
    pub mac: [u8; 6],
    pub _pad: [u8; 2],
    pub ipv4: [u8; 4],
    pub router: [u8; 4],
    pub dns: [u8; 4],
}

impl NetInfo {
    pub const fn empty() -> Self {
        Self {
            present: 0,
            link_up: 0,
            configured: 0,
            tx_packets: 0,
            rx_packets: 0,
            mac: [0; 6],
            _pad: [0; 2],
            ipv4: [0; 4],
            router: [0; 4],
            dns: [0; 4],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RxDesc {
    addr: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

impl RxDesc {
    const fn zero() -> Self {
        Self {
            addr: 0,
            length: 0,
            checksum: 0,
            status: 0,
            errors: 0,
            special: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TxDesc {
    addr: u64,
    length: u16,
    cso: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}

impl TxDesc {
    const fn zero() -> Self {
        Self {
            addr: 0,
            length: 0,
            cso: 0,
            cmd: 0,
            status: DESC_STATUS_DD,
            css: 0,
            special: 0,
        }
    }
}

struct Controller {
    mmio: u64,
    mac: [u8; 6],
    rx_ring_phys: u64,
    tx_ring_phys: u64,
    rx_bufs: [u64; RX_COUNT],
    tx_buf_phys: u64,
    rx_head: usize,
    tx_tail: usize,
    tx_packets: u32,
    rx_packets: u32,
}

static NET: Mutex<Option<Controller>> = Mutex::new(None);
static LAST_INFO: Mutex<NetInfo> = Mutex::new(NetInfo::empty());

pub fn init_from_pci() {
    let mut serial = unsafe { SerialPort::new(0x3F8) };
    let dev = pci::all().into_iter().find(|d| {
        d.vendor_id == 0x8086 && matches!(d.device_id, 0x100e | 0x100f | 0x1004 | 0x1008)
    });
    let Some(dev) = dev else {
        let _ = serial.write_str("[net] e1000 not present\n");
        return;
    };
    let Some(bar0) = dev.bars[0] else {
        let _ = serial.write_str("[net] e1000 BAR0 missing\n");
        return;
    };
    if bar0.io {
        let _ = serial.write_str("[net] e1000 BAR0 is not MMIO\n");
        return;
    }
    dev.enable_mmio_bus_master();
    let mmio = match vmm::map_mmio(bar0.base, bar0.size as usize) {
        Ok(v) => v,
        Err(_) => {
            let _ = serial.write_str("[net] e1000 MMIO map failed\n");
            return;
        }
    };
    let mut controller = match Controller::new(mmio) {
        Some(c) => c,
        None => {
            let _ = serial.write_str("[net] e1000 allocation failed\n");
            return;
        }
    };
    controller.init_rings();
    let info = controller.snapshot(false);
    let _ = writeln!(
        serial,
        "[net] e1000 at {:02x}:{:02x}.{} MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} link={}",
        dev.bus,
        dev.device,
        dev.function,
        info.mac[0],
        info.mac[1],
        info.mac[2],
        info.mac[3],
        info.mac[4],
        info.mac[5],
        info.link_up,
    );
    *LAST_INFO.lock() = info;
    *NET.lock() = Some(controller);
}

pub fn configure() -> NetInfo {
    let mut serial = unsafe { SerialPort::new(0x3F8) };
    let mut guard = NET.lock();
    let Some(controller) = guard.as_mut() else {
        return *LAST_INFO.lock();
    };
    let mut info = controller.snapshot(false);
    let mut packet = [0u8; TX_BUF_SIZE];
    let len = build_dhcp_discover(&mut packet, controller.mac);
    if controller.send_packet(&packet[..len]) {
        info.tx_packets = controller.tx_packets;
        let _ = serial.write_str("[net] DHCPDISCOVER tx\n");
    } else {
        let _ = serial.write_str("[net] DHCPDISCOVER tx failed\n");
        *LAST_INFO.lock() = info;
        return info;
    }

    for _ in 0..20_000_000 {
        if let Some(offer) = controller.poll_offer() {
            info = controller.snapshot(true);
            info.ipv4 = offer.yiaddr;
            info.router = offer.router;
            info.dns = offer.dns;
            info.rx_packets = controller.rx_packets;
            let _ = writeln!(
                serial,
                "[net] DHCPOFFER rx ip={}.{}.{}.{} router={}.{}.{}.{} dns={}.{}.{}.{}",
                info.ipv4[0],
                info.ipv4[1],
                info.ipv4[2],
                info.ipv4[3],
                info.router[0],
                info.router[1],
                info.router[2],
                info.router[3],
                info.dns[0],
                info.dns[1],
                info.dns[2],
                info.dns[3],
            );
            *LAST_INFO.lock() = info;
            return info;
        }
        core::hint::spin_loop();
    }

    info.rx_packets = controller.rx_packets;
    let _ = serial.write_str("[net] DHCPOFFER timeout\n");
    *LAST_INFO.lock() = info;
    info
}

pub fn info() -> NetInfo {
    *LAST_INFO.lock()
}

impl Controller {
    fn new(mmio: u64) -> Option<Self> {
        let rx_ring_phys = pmm::alloc_frame()?;
        let tx_ring_phys = pmm::alloc_frame()?;
        unsafe {
            core::ptr::write_bytes((rx_ring_phys + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
            core::ptr::write_bytes((tx_ring_phys + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
        }
        let mut rx_bufs = [0u64; RX_COUNT];
        let mut i = 0;
        while i < RX_COUNT {
            let phys = pmm::alloc_frame()?;
            unsafe {
                core::ptr::write_bytes((phys + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
            }
            rx_bufs[i] = phys;
            i += 1;
        }
        let tx_buf_phys = pmm::alloc_frame()?;
        unsafe {
            core::ptr::write_bytes((tx_buf_phys + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
        }
        let mut controller = Self {
            mmio,
            mac: [0; 6],
            rx_ring_phys,
            tx_ring_phys,
            rx_bufs,
            tx_buf_phys,
            rx_head: 0,
            tx_tail: 0,
            tx_packets: 0,
            rx_packets: 0,
        };
        controller.write32(REG_IMC, u32::MAX);
        controller.write32(REG_CTRL, controller.read32(REG_CTRL) | CTRL_RST);
        for _ in 0..100_000 {
            if controller.read32(REG_CTRL) & CTRL_RST == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        controller.write32(REG_IMC, u32::MAX);
        controller.write32(REG_CTRL, controller.read32(REG_CTRL) | CTRL_SLU);
        for _ in 0..100_000 {
            if controller.read32(REG_STATUS) & STATUS_LU != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        controller.mac = controller.read_mac();
        Some(controller)
    }

    fn init_rings(&mut self) {
        let rx = (self.rx_ring_phys + HHDM_OFFSET) as *mut RxDesc;
        let tx = (self.tx_ring_phys + HHDM_OFFSET) as *mut TxDesc;
        for i in 0..RX_COUNT {
            unsafe {
                let desc = rx.add(i);
                desc.write_volatile(RxDesc::zero());
                (*desc).addr = self.rx_bufs[i];
            }
        }
        for i in 0..TX_COUNT {
            unsafe {
                tx.add(i).write_volatile(TxDesc::zero());
            }
        }

        let ral = u32::from_le_bytes([self.mac[0], self.mac[1], self.mac[2], self.mac[3]]);
        let rah = u32::from_le_bytes([self.mac[4], self.mac[5], 0, 0]) | (1 << 31);
        self.write32(REG_RAL0, ral);
        self.write32(REG_RAH0, rah);

        self.write32(REG_RDBAL, self.rx_ring_phys as u32);
        self.write32(REG_RDBAH, (self.rx_ring_phys >> 32) as u32);
        self.write32(
            REG_RDLEN,
            (RX_COUNT * core::mem::size_of::<RxDesc>()) as u32,
        );
        self.write32(REG_RDH, 0);
        self.write32(REG_RDT, (RX_COUNT - 1) as u32);
        self.write32(REG_RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC);

        self.write32(REG_TDBAL, self.tx_ring_phys as u32);
        self.write32(REG_TDBAH, (self.tx_ring_phys >> 32) as u32);
        self.write32(
            REG_TDLEN,
            (TX_COUNT * core::mem::size_of::<TxDesc>()) as u32,
        );
        self.write32(REG_TDH, 0);
        self.write32(REG_TDT, 0);
        self.write32(REG_TIPG, 10 | (8 << 10) | (6 << 20));
        self.write32(REG_TCTL, TCTL_EN | TCTL_PSP | (15 << 4) | (64 << 12));
    }

    fn snapshot(&self, configured: bool) -> NetInfo {
        NetInfo {
            present: 1,
            link_up: u32::from(self.read32(REG_STATUS) & STATUS_LU != 0),
            configured: u32::from(configured),
            tx_packets: self.tx_packets,
            rx_packets: self.rx_packets,
            mac: self.mac,
            _pad: [0; 2],
            ipv4: [0; 4],
            router: [0; 4],
            dns: [0; 4],
        }
    }

    fn send_packet(&mut self, packet: &[u8]) -> bool {
        if packet.len() > TX_BUF_SIZE {
            return false;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                packet.as_ptr(),
                (self.tx_buf_phys + HHDM_OFFSET) as *mut u8,
                packet.len(),
            );
        }
        let idx = self.tx_tail;
        let desc = unsafe { ((self.tx_ring_phys + HHDM_OFFSET) as *mut TxDesc).add(idx) };
        unsafe {
            (*desc).addr = self.tx_buf_phys;
            (*desc).length = packet.len() as u16;
            (*desc).cso = 0;
            (*desc).cmd = TX_CMD_EOP | TX_CMD_IFCS | TX_CMD_RS;
            (*desc).status = 0;
            (*desc).css = 0;
            (*desc).special = 0;
        }
        let next = (idx + 1) % TX_COUNT;
        fence(Ordering::SeqCst);
        self.write32(REG_TDT, next as u32);
        for _ in 0..100_000 {
            let status = unsafe { core::ptr::addr_of!((*desc).status).read_volatile() };
            if status & DESC_STATUS_DD != 0 {
                self.tx_tail = next;
                self.tx_packets = self.tx_packets.saturating_add(1);
                return true;
            }
            core::hint::spin_loop();
        }
        false
    }

    fn poll_offer(&mut self) -> Option<DhcpOffer> {
        let rx = (self.rx_ring_phys + HHDM_OFFSET) as *mut RxDesc;
        for _ in 0..RX_COUNT {
            let idx = self.rx_head;
            let desc = unsafe { rx.add(idx) };
            let status = unsafe { core::ptr::addr_of!((*desc).status).read_volatile() };
            if status & DESC_STATUS_DD == 0 {
                return None;
            }
            let len = unsafe { core::ptr::addr_of!((*desc).length).read_volatile() as usize };
            let eop = status & RX_STATUS_EOP != 0;
            let buf = unsafe {
                core::slice::from_raw_parts((self.rx_bufs[idx] + HHDM_OFFSET) as *const u8, len)
            };
            let offer = if eop {
                parse_dhcp_offer(buf, self.mac)
            } else {
                None
            };
            unsafe {
                (*desc).length = 0;
                (*desc).status = 0;
            }
            self.write32(REG_RDT, idx as u32);
            self.rx_head = (idx + 1) % RX_COUNT;
            self.rx_packets = self.rx_packets.saturating_add(1);
            if offer.is_some() {
                return offer;
            }
        }
        None
    }

    fn read_mac(&self) -> [u8; 6] {
        let ral = self.read32(REG_RAL0).to_le_bytes();
        let rah = self.read32(REG_RAH0).to_le_bytes();
        let mac = [ral[0], ral[1], ral[2], ral[3], rah[0], rah[1]];
        if mac == [0; 6] {
            [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]
        } else {
            mac
        }
    }

    fn read32(&self, off: usize) -> u32 {
        unsafe { ((self.mmio + off as u64) as *const u32).read_volatile() }
    }

    fn write32(&self, off: usize, value: u32) {
        unsafe { ((self.mmio + off as u64) as *mut u32).write_volatile(value) }
    }
}

struct DhcpOffer {
    yiaddr: [u8; 4],
    router: [u8; 4],
    dns: [u8; 4],
}

fn build_dhcp_discover(buf: &mut [u8; TX_BUF_SIZE], mac: [u8; 6]) -> usize {
    let dhcp_len = 236 + 4 + 13;
    let udp_len = 8 + dhcp_len;
    let ip_len = 20 + udp_len;
    let total = 14 + ip_len;

    buf[..6].fill(0xFF);
    buf[6..12].copy_from_slice(&mac);
    put_be16(buf, 12, 0x0800);

    let ip = 14;
    buf[ip] = 0x45;
    buf[ip + 1] = 0;
    put_be16(buf, ip + 2, ip_len as u16);
    put_be16(buf, ip + 4, 0x1001);
    put_be16(buf, ip + 6, 0);
    buf[ip + 8] = 64;
    buf[ip + 9] = 17;
    buf[ip + 12..ip + 16].copy_from_slice(&[0, 0, 0, 0]);
    buf[ip + 16..ip + 20].copy_from_slice(&[255, 255, 255, 255]);
    let csum = ipv4_checksum(&buf[ip..ip + 20]);
    put_be16(buf, ip + 10, csum);

    let udp = ip + 20;
    put_be16(buf, udp, 68);
    put_be16(buf, udp + 2, 67);
    put_be16(buf, udp + 4, udp_len as u16);
    put_be16(buf, udp + 6, 0);

    let dhcp = udp + 8;
    buf[dhcp] = 1;
    buf[dhcp + 1] = 1;
    buf[dhcp + 2] = 6;
    buf[dhcp + 3] = 0;
    put_be32(buf, dhcp + 4, DHCP_XID);
    put_be16(buf, dhcp + 10, 0x8000);
    buf[dhcp + 28..dhcp + 34].copy_from_slice(&mac);
    let opts = dhcp + 236;
    buf[opts..opts + 4].copy_from_slice(&[99, 130, 83, 99]);
    buf[opts + 4..opts + 16].copy_from_slice(&[53, 1, 1, 55, 3, 1, 3, 6, 57, 2, 2, 64]);
    buf[opts + 16] = 255;
    total
}

fn parse_dhcp_offer(packet: &[u8], mac: [u8; 6]) -> Option<DhcpOffer> {
    if packet.len() < 14 + 20 + 8 + 240 || packet[12] != 0x08 || packet[13] != 0x00 {
        return None;
    }
    let ip = 14;
    let ihl = ((packet[ip] & 0x0F) as usize) * 4;
    if ihl < 20 || packet.len() < ip + ihl + 8 + 240 || packet[ip + 9] != 17 {
        return None;
    }
    let udp = ip + ihl;
    if read_be16(packet, udp) != 67 || read_be16(packet, udp + 2) != 68 {
        return None;
    }
    let dhcp = udp + 8;
    if packet[dhcp] != 2 || packet[dhcp + 1] != 1 || packet[dhcp + 2] != 6 {
        return None;
    }
    if read_be32(packet, dhcp + 4) != DHCP_XID || packet[dhcp + 28..dhcp + 34] != mac {
        return None;
    }
    if packet[dhcp + 236..dhcp + 240] != [99, 130, 83, 99] {
        return None;
    }

    let mut msg_type = 0u8;
    let mut router = [0; 4];
    let mut dns = [0; 4];
    let mut pos = dhcp + 240;
    while pos < packet.len() {
        let opt = packet[pos];
        pos += 1;
        if opt == 255 {
            break;
        }
        if opt == 0 {
            continue;
        }
        if pos >= packet.len() {
            break;
        }
        let len = packet[pos] as usize;
        pos += 1;
        if pos + len > packet.len() {
            break;
        }
        match opt {
            53 if len >= 1 => msg_type = packet[pos],
            3 if len >= 4 => router.copy_from_slice(&packet[pos..pos + 4]),
            6 if len >= 4 => dns.copy_from_slice(&packet[pos..pos + 4]),
            _ => {}
        }
        pos += len;
    }
    if msg_type != 2 {
        return None;
    }
    let mut yiaddr = [0; 4];
    yiaddr.copy_from_slice(&packet[dhcp + 16..dhcp + 20]);
    Some(DhcpOffer {
        yiaddr,
        router,
        dns,
    })
}

fn put_be16(buf: &mut [u8], off: usize, value: u16) {
    buf[off..off + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_be32(buf: &mut [u8], off: usize, value: u32) {
    buf[off..off + 4].copy_from_slice(&value.to_be_bytes());
}

fn read_be16(buf: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([buf[off], buf[off + 1]])
}

fn read_be32(buf: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

fn ipv4_checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 1 < bytes.len() {
        sum += u16::from_be_bytes([bytes[i], bytes[i + 1]]) as u32;
        i += 2;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}
