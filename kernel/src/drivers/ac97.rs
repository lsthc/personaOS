use core::fmt::Write as _;

use persona_shared::HHDM_OFFSET;
use spin::Mutex;

use crate::arch::x86_64::{inb, inl, inw, outb, outl, outw};
use crate::drivers::pci;
use crate::drivers::serial::SerialPort;
use crate::mm::{pmm, PAGE_SIZE};

const NABM_PO_BDBAR: u16 = 0x10;
const NABM_PO_LVI: u16 = 0x15;
const NABM_PO_SR: u16 = 0x16;
const NABM_PO_PICB: u16 = 0x18;
const NABM_PO_CR: u16 = 0x1B;
const NAM_RESET: u16 = 0x00;
const NAM_MASTER_VOLUME: u16 = 0x02;
const NAM_PCM_VOLUME: u16 = 0x18;
const NAM_EXT_AUDIO_ID: u16 = 0x28;
const NAM_EXT_AUDIO_CTRL: u16 = 0x2A;
const NAM_PCM_FRONT_RATE: u16 = 0x2C;
const CR_RPBM: u8 = 1;
const CR_RR: u8 = 1 << 1;
const SR_BCIS: u16 = 1 << 3;
const SR_LVBCI: u16 = 1 << 2;
const TONE_RATE: u32 = 48_000;
const TONE_MS: u32 = 180;
const TONE_FRAMES: usize = (TONE_RATE as usize * TONE_MS as usize) / 1000;
const AUDIO_BYTES: usize = TONE_FRAMES * 4;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AudioInfo {
    pub present: u32,
    pub played: u32,
    pub sample_rate: u32,
    pub frames: u32,
}

impl AudioInfo {
    pub const fn empty() -> Self {
        Self {
            present: 0,
            played: 0,
            sample_rate: 0,
            frames: 0,
        }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct BufferDescriptor {
    addr: u32,
    samples: u16,
    control: u16,
}

struct Controller {
    nam: u16,
    nabm: u16,
    bdl_phys: u64,
    pcm_phys: u64,
}

static AUDIO: Mutex<Option<Controller>> = Mutex::new(None);
static LAST_INFO: Mutex<AudioInfo> = Mutex::new(AudioInfo::empty());

pub fn init_from_pci() {
    let mut serial = unsafe { SerialPort::new(0x3F8) };
    let dev = pci::all()
        .into_iter()
        .find(|d| d.vendor_id == 0x8086 && d.device_id == 0x2415);
    let Some(dev) = dev else {
        let _ = serial.write_str("[audio] AC97 not present\n");
        return;
    };
    let Some(nam_bar) = dev.bars[0] else {
        let _ = serial.write_str("[audio] AC97 NAM BAR missing\n");
        return;
    };
    let Some(nabm_bar) = dev.bars[1] else {
        let _ = serial.write_str("[audio] AC97 NABM BAR missing\n");
        return;
    };
    if !nam_bar.io || !nabm_bar.io {
        let _ = serial.write_str("[audio] AC97 I/O BARs missing\n");
        return;
    }
    dev.enable_io_bus_master();
    let Some(mut controller) = Controller::new(nam_bar.base as u16, nabm_bar.base as u16) else {
        let _ = serial.write_str("[audio] AC97 allocation failed\n");
        return;
    };
    controller.init_codec();
    let info = AudioInfo {
        present: 1,
        played: 0,
        sample_rate: TONE_RATE,
        frames: TONE_FRAMES as u32,
    };
    let _ = writeln!(
        serial,
        "[audio] AC97 at {:02x}:{:02x}.{} nam={:#x} nabm={:#x}",
        dev.bus, dev.device, dev.function, nam_bar.base, nabm_bar.base,
    );
    *LAST_INFO.lock() = info;
    *AUDIO.lock() = Some(controller);
}

pub fn play_tone(freq_hz: u32, duration_ms: u32) -> AudioInfo {
    let mut serial = unsafe { SerialPort::new(0x3F8) };
    let mut guard = AUDIO.lock();
    let Some(controller) = guard.as_mut() else {
        return *LAST_INFO.lock();
    };
    let frames = ((TONE_RATE as u64 * duration_ms.min(500) as u64) / 1000) as usize;
    let frames = frames.clamp(1, TONE_FRAMES);
    controller.fill_tone(freq_hz.clamp(110, 1760), frames);
    let played = controller.play(frames);
    let info = AudioInfo {
        present: 1,
        played: u32::from(played),
        sample_rate: TONE_RATE,
        frames: frames as u32,
    };
    if played {
        let _ = writeln!(
            serial,
            "[audio] AC97 PCM playback complete frames={} rate={}",
            frames, TONE_RATE
        );
    } else {
        let _ = serial.write_str("[audio] AC97 PCM playback timeout\n");
    }
    *LAST_INFO.lock() = info;
    info
}

pub fn info() -> AudioInfo {
    *LAST_INFO.lock()
}

impl Controller {
    fn new(nam: u16, nabm: u16) -> Option<Self> {
        let bdl_phys = pmm::alloc_frame()?;
        let pcm_phys = pmm::alloc_contig(AUDIO_BYTES.div_ceil(PAGE_SIZE))?;
        unsafe {
            core::ptr::write_bytes((bdl_phys + HHDM_OFFSET) as *mut u8, 0, PAGE_SIZE);
            core::ptr::write_bytes((pcm_phys + HHDM_OFFSET) as *mut u8, 0, AUDIO_BYTES);
        }
        Some(Self {
            nam,
            nabm,
            bdl_phys,
            pcm_phys,
        })
    }

    fn init_codec(&mut self) {
        self.nam_write16(NAM_RESET, 0);
        for _ in 0..10_000 {
            core::hint::spin_loop();
        }
        self.nam_write16(NAM_MASTER_VOLUME, 0x0000);
        self.nam_write16(NAM_PCM_VOLUME, 0x0808);
        let ext = self.nam_read16(NAM_EXT_AUDIO_ID);
        if ext & 1 != 0 {
            self.nam_write16(NAM_EXT_AUDIO_CTRL, 1);
            self.nam_write16(NAM_PCM_FRONT_RATE, TONE_RATE as u16);
        }
        self.reset_pcm_out();
    }

    fn reset_pcm_out(&self) {
        self.nabm_write8(NABM_PO_CR, CR_RR);
        for _ in 0..10_000 {
            if self.nabm_read8(NABM_PO_CR) & CR_RR == 0 {
                break;
            }
            core::hint::spin_loop();
        }
        self.nabm_write16(NABM_PO_SR, 0x1C);
    }

    fn fill_tone(&self, freq_hz: u32, frames: usize) {
        let samples = unsafe {
            core::slice::from_raw_parts_mut((self.pcm_phys + HHDM_OFFSET) as *mut i16, frames * 2)
        };
        let period = (TONE_RATE / freq_hz.max(1)).max(2);
        for i in 0..frames {
            let phase = (i as u32 % period) < period / 2;
            let sample = if phase { 6000i16 } else { -6000i16 };
            samples[i * 2] = sample;
            samples[i * 2 + 1] = sample;
        }
    }

    fn play(&self, frames: usize) -> bool {
        self.reset_pcm_out();
        let bdl = (self.bdl_phys + HHDM_OFFSET) as *mut BufferDescriptor;
        unsafe {
            bdl.write_volatile(BufferDescriptor {
                addr: self.pcm_phys as u32,
                samples: (frames * 2) as u16,
                control: 1 << 15,
            });
        }
        self.nabm_write32(NABM_PO_BDBAR, self.bdl_phys as u32);
        self.nabm_write8(NABM_PO_LVI, 0);
        self.nabm_write16(NABM_PO_SR, 0x1C);
        self.nabm_write8(NABM_PO_CR, CR_RPBM);
        for _ in 0..8_000_000 {
            let sr = self.nabm_read16(NABM_PO_SR);
            if sr & (SR_BCIS | SR_LVBCI) != 0 {
                self.nabm_write8(NABM_PO_CR, 0);
                self.nabm_write16(NABM_PO_SR, 0x1C);
                return true;
            }
            let _ = self.nabm_read16(NABM_PO_PICB);
            core::hint::spin_loop();
        }
        self.nabm_write8(NABM_PO_CR, 0);
        false
    }

    fn nam_read16(&self, off: u16) -> u16 {
        unsafe { inw(self.nam + off) }
    }

    fn nam_write16(&self, off: u16, value: u16) {
        unsafe { outw(self.nam + off, value) }
    }

    fn nabm_read8(&self, off: u16) -> u8 {
        unsafe { inb(self.nabm + off) }
    }

    fn nabm_write8(&self, off: u16, value: u8) {
        unsafe { outb(self.nabm + off, value) }
    }

    fn nabm_read16(&self, off: u16) -> u16 {
        unsafe { inw(self.nabm + off) }
    }

    fn nabm_write16(&self, off: u16, value: u16) {
        unsafe { outw(self.nabm + off, value) }
    }

    fn nabm_write32(&self, off: u16, value: u32) {
        unsafe { outl(self.nabm + off, value) }
    }

    #[allow(dead_code)]
    fn nabm_read32(&self, off: u16) -> u32 {
        unsafe { inl(self.nabm + off) }
    }
}
