//! Nordic nRF5 SDK "secure DFU" over a serial port, replacing `nrfutil dfu serial`.
//!
//! Wire format as in Nordic's pc-nrfutil (dfu_transport_serial.py) and the nRF5 SDK bootloader:
//! SLIP-framed requests `[opcode, args..]`, answered by `[0x60, opcode, result, data..]`.
//! The init packet (.dat) goes into a command object, the firmware (.bin) into data objects of
//! at most the size the bootloader reports; every object is CRC-checked before it is executed.
//! The bootloader validates the signed init packet itself and keeps the old firmware until a
//! complete image has been received, so an interrupted transfer can simply be retried.
use anyhow::{anyhow, bail, Context, Result};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

const OP_CREATE: u8 = 0x01;
const OP_SET_PRN: u8 = 0x02;
const OP_CRC: u8 = 0x03;
const OP_EXECUTE: u8 = 0x04;
const OP_SELECT: u8 = 0x06;
const OP_MTU: u8 = 0x07;
const OP_WRITE: u8 = 0x08;
const OP_PING: u8 = 0x09;
const OP_RESPONSE: u8 = 0x60;
const OBJ_COMMAND: u8 = 0x01;
const OBJ_DATA: u8 = 0x02;
const RES_SUCCESS: u8 = 0x01;
const RES_EXT_ERROR: u8 = 0x0B;

const SLIP_END: u8 = 0xC0;
const SLIP_ESC: u8 = 0xDB;
const SLIP_ESC_END: u8 = 0xDC;
const SLIP_ESC_ESC: u8 = 0xDD;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Packet receipt notification interval; 0 = only check the CRC after each object.
    pub prn: u16,
    /// Wait after opening the port before talking to the bootloader.
    pub connect_delay: Duration,
    /// How long to wait for any single reply.
    pub timeout: Duration,
}

/// The two files of a Nordic DFU package (.zip with manifest.json).
pub struct Package {
    pub init: Vec<u8>,
    pub firmware: Vec<u8>,
}

impl Package {
    pub fn load(path: &Path) -> Result<Package> {
        let file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut read = |name: &str| -> Result<Vec<u8>> {
            let mut buf = Vec::new();
            zip.by_name(name).with_context(|| format!("{name} missing from DFU package"))?.read_to_end(&mut buf)?;
            Ok(buf)
        };
        let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")?)?;
        let app = &manifest["manifest"]["application"];
        let (Some(bin), Some(dat)) = (app["bin_file"].as_str(), app["dat_file"].as_str()) else {
            bail!("DFU package has no application image");
        };
        Ok(Package { init: read(dat)?, firmware: read(bin)? })
    }
}

/// zlib CRC-32, continuing from `crc` (0 to start).
pub fn crc32(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

pub fn slip_encode(packet: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(packet.len() + 2);
    for &b in packet {
        match b {
            SLIP_END => out.extend([SLIP_ESC, SLIP_ESC_END]),
            SLIP_ESC => out.extend([SLIP_ESC, SLIP_ESC_ESC]),
            _ => out.push(b),
        }
    }
    out.push(SLIP_END);
    out
}

pub fn slip_decode(frame: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(frame.len());
    let mut bytes = frame.iter();
    while let Some(&b) = bytes.next() {
        out.push(match b {
            SLIP_ESC => match bytes.next() {
                Some(&SLIP_ESC_END) => SLIP_END,
                Some(&SLIP_ESC_ESC) => SLIP_ESC,
                _ => bail!("bad SLIP escape"),
            },
            _ => b,
        });
    }
    Ok(out)
}

fn result_name(code: u8) -> &'static str {
    match code {
        0x00 => "invalid opcode",
        0x02 => "opcode not supported",
        0x03 => "invalid parameter",
        0x04 => "insufficient resources",
        0x05 => "invalid object",
        0x07 => "unsupported type",
        0x08 => "operation not permitted",
        0x0A => "operation failed",
        _ => "unknown error",
    }
}

fn ext_error_name(code: u8) -> &'static str {
    match code {
        0x02 => "invalid init packet format",
        0x03 => "unknown init packet command",
        0x04 => "init command invalid",
        0x05 => "firmware version too low",
        0x06 => "hardware version mismatch",
        0x07 => "SoftDevice version mismatch",
        0x08 => "init packet signature missing",
        0x09 => "wrong hash type",
        0x0A => "hash verification failed",
        0x0B => "wrong signature type",
        0x0C => "signature verification failed (wrong firmware for this device)",
        0x0D => "image too large",
        _ => "unknown extended error",
    }
}

struct Dfu<'a, P: Read + Write> {
    port: P,
    opts: Options,
    /// Payload bytes per write packet, derived from the bootloader's MTU.
    chunk: usize,
    rx: Vec<u8>,
    log: &'a dyn Fn(String),
    progress: &'a dyn Fn(f32),
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

impl<P: Read + Write> Dfu<'_, P> {
    fn send(&mut self, packet: &[u8]) -> Result<()> {
        self.port.write_all(&slip_encode(packet))?;
        self.port.flush()?;
        Ok(())
    }

    fn read_frame(&mut self) -> Result<Vec<u8>> {
        let deadline = Instant::now() + self.opts.timeout;
        loop {
            if let Some(end) = self.rx.iter().position(|&b| b == SLIP_END) {
                let frame: Vec<u8> = self.rx.drain(..=end).take(end).collect();
                if frame.is_empty() {
                    continue; // stray END between frames
                }
                return slip_decode(&frame);
            }
            if Instant::now() > deadline {
                bail!("no reply from the bootloader within {} s", self.opts.timeout.as_secs());
            }
            let mut buf = [0u8; 256];
            match self.port.read(&mut buf) {
                Ok(n) => self.rx.extend_from_slice(&buf[..n]),
                Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Wait for the reply to `op`; returns its data bytes.
    fn response(&mut self, op: u8) -> Result<Vec<u8>> {
        let r = self.read_frame()?;
        if r.len() < 3 || r[0] != OP_RESPONSE || r[1] != op {
            bail!("unexpected reply {r:02x?} to opcode 0x{op:02x}");
        }
        match r[2] {
            RES_SUCCESS => Ok(r[3..].to_vec()),
            RES_EXT_ERROR => bail!("bootloader refused: {}", ext_error_name(r.get(3).copied().unwrap_or(0))),
            code => bail!("bootloader refused opcode 0x{op:02x}: {}", result_name(code)),
        }
    }

    fn request(&mut self, packet: &[u8]) -> Result<Vec<u8>> {
        self.send(packet)?;
        self.response(packet[0])
    }

    fn ping(&mut self) -> Result<()> {
        let saved = self.opts.timeout;
        self.opts.timeout = Duration::from_secs(1);
        let mut last = anyhow!("no reply");
        for id in 1..=5u8 {
            self.send(&[OP_PING, id])?;
            match self.response(OP_PING) {
                Ok(d) if d.first() == Some(&id) => {
                    self.opts.timeout = saved;
                    return Ok(());
                }
                Ok(_) => last = anyhow!("ping id mismatch"),
                // Bootloaders older than SDK 15 have no ping; they still answer, which is enough.
                Err(e) if e.to_string().contains("not supported") => {
                    self.opts.timeout = saved;
                    return Ok(());
                }
                Err(e) => last = e,
            }
            self.rx.clear();
        }
        Err(last.context("the bootloader did not answer"))
    }

    fn setup(&mut self) -> Result<()> {
        self.ping()?;
        let prn = self.opts.prn.to_le_bytes();
        self.request(&[OP_SET_PRN, prn[0], prn[1]])?;
        let mtu = self.request(&[OP_MTU])?;
        let mtu = u16::from_le_bytes([mtu[0], mtu[1]]) as usize;
        // Worst case every byte is SLIP-escaped, plus the opcode: same sizing as nrfutil.
        self.chunk = (mtu.saturating_sub(1) / 2).saturating_sub(1).max(1);
        (self.log)(format!("    bootloader ready (MTU {mtu}, {} byte packets)", self.chunk));
        Ok(())
    }

    /// (max object size, current offset, current CRC)
    fn select(&mut self, kind: u8) -> Result<(usize, usize, u32)> {
        let d = self.request(&[OP_SELECT, kind])?;
        Ok((le32(&d[0..]) as usize, le32(&d[4..]) as usize, le32(&d[8..])))
    }

    fn create(&mut self, kind: u8, size: usize) -> Result<()> {
        let s = (size as u32).to_le_bytes();
        self.request(&[OP_CREATE, kind, s[0], s[1], s[2], s[3]]).map(|_| ())
    }

    fn check_crc(&mut self, data: Vec<u8>, offset: usize, crc: u32) -> Result<()> {
        let (got_offset, got_crc) = (le32(&data[0..]) as usize, le32(&data[4..]));
        if got_offset != offset || got_crc != crc {
            bail!("transfer check failed (bootloader has {got_offset} bytes / CRC {got_crc:08x}, expected {offset} / {crc:08x})");
        }
        Ok(())
    }

    /// Write `data` into the current object; returns the running CRC over everything sent.
    fn stream(&mut self, data: &[u8], mut offset: usize, mut crc: u32) -> Result<u32> {
        for (i, piece) in data.chunks(self.chunk).enumerate() {
            let mut packet = Vec::with_capacity(piece.len() + 1);
            packet.push(OP_WRITE);
            packet.extend_from_slice(piece);
            self.send(&packet)?;
            crc = crc32(crc, piece);
            offset += piece.len();
            if self.opts.prn > 0 && (i + 1) % self.opts.prn as usize == 0 {
                let d = self.response(OP_CRC)?;
                self.check_crc(d, offset, crc)?;
            }
        }
        Ok(crc)
    }

    fn verify(&mut self, offset: usize, crc: u32) -> Result<()> {
        let d = self.request(&[OP_CRC])?;
        self.check_crc(d, offset, crc)
    }

    fn send_init(&mut self, init: &[u8]) -> Result<()> {
        let (max, _, _) = self.select(OBJ_COMMAND)?;
        if init.len() > max {
            bail!("init packet ({} bytes) is larger than the bootloader accepts ({max})", init.len());
        }
        self.create(OBJ_COMMAND, init.len())?;
        let crc = self.stream(init, 0, 0)?;
        self.verify(init.len(), crc)?;
        self.request(&[OP_EXECUTE]).map(|_| ())
    }

    fn send_firmware(&mut self, image: &[u8]) -> Result<()> {
        let (max, _, _) = self.select(OBJ_DATA)?;
        if max == 0 {
            bail!("bootloader reported a zero object size");
        }
        let (mut offset, mut crc, mut shown) = (0usize, 0u32, 0usize);
        for object in image.chunks(max) {
            self.create(OBJ_DATA, object.len())?;
            crc = self.stream(object, offset, crc)?;
            offset += object.len();
            self.verify(offset, crc)?;
            self.request(&[OP_EXECUTE])?;
            (self.progress)(offset as f32 / image.len() as f32);
            let percent = offset * 100 / image.len();
            if percent >= shown + 10 || offset == image.len() {
                shown = percent - percent % 10;
                (self.log)(format!("    {percent}%  ({offset} / {} bytes)", image.len()));
            }
        }
        Ok(())
    }
}

/// Run a full DFU over an already opened port.
pub fn flash_over<P: Read + Write>(
    port: P,
    pkg: &Package,
    opts: Options,
    log: &dyn Fn(String),
    progress: &dyn Fn(f32),
) -> Result<()> {
    let mut dfu = Dfu { port, opts, chunk: 1, rx: Vec::new(), log, progress };
    std::thread::sleep(opts.connect_delay);
    dfu.setup()?;
    log(format!("    sending init packet ({} bytes)", pkg.init.len()));
    dfu.send_init(&pkg.init)?;
    log(format!("    sending firmware ({} bytes)", pkg.firmware.len()));
    dfu.send_firmware(&pkg.firmware)?;
    log("    image accepted - the device restarts with the new firmware".into());
    Ok(())
}

/// Flash the DFU package at `package` through the serial bootloader at `tty`.
pub fn flash(tty: &str, package: &Path, opts: Options, log: &dyn Fn(String), progress: &dyn Fn(f32)) -> Result<()> {
    let pkg = Package::load(package)?;
    let mut port = serialport::new(tty, 115_200)
        .timeout(Duration::from_millis(200))
        .flow_control(serialport::FlowControl::None)
        .open()
        .with_context(|| format!("cannot open {tty}"))?;
    let _ = port.write_data_terminal_ready(true);
    flash_over(port, &pkg, opts, log, progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn crc32_matches_zlib() {
        assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(crc32(0, b"1234"), b"56789"), 0xCBF4_3926);
    }

    #[test]
    fn slip_round_trip() {
        let data = [0x01, SLIP_END, 0x02, SLIP_ESC, SLIP_ESC_END, 0xFF];
        let enc = slip_encode(&data);
        assert_eq!(*enc.last().unwrap(), SLIP_END);
        assert!(!enc[..enc.len() - 1].contains(&SLIP_END));
        assert_eq!(slip_decode(&enc[..enc.len() - 1]).unwrap(), data);
    }

    /// Enough of the nRF5 SDK bootloader to check the conversation.
    struct FakeBootloader {
        inbox: Vec<u8>,
        outbox: VecDeque<u8>,
        prn: u16,
        since_prn: u16,
        mtu: u16,
        max_data: usize,
        object: Vec<u8>,
        object_size: usize,
        object_kind: u8,
        init: Vec<u8>,
        image: Vec<u8>,
        executed_objects: usize,
        corrupt_after: Option<usize>,
    }

    impl FakeBootloader {
        fn new(mtu: u16, max_data: usize) -> Self {
            FakeBootloader {
                inbox: Vec::new(),
                outbox: VecDeque::new(),
                prn: 0,
                since_prn: 0,
                mtu,
                max_data,
                object: Vec::new(),
                object_size: 0,
                object_kind: 0,
                init: Vec::new(),
                image: Vec::new(),
                executed_objects: 0,
                corrupt_after: None,
            }
        }
        fn reply(&mut self, op: u8, data: &[u8]) {
            let mut r = vec![OP_RESPONSE, op, RES_SUCCESS];
            r.extend_from_slice(data);
            self.outbox.extend(slip_encode(&r));
        }
        fn received(&self) -> Vec<u8> {
            let base = if self.object_kind == OBJ_DATA { self.image.clone() } else { Vec::new() };
            [base, self.object.clone()].concat()
        }
        fn crc_reply(&mut self, op: u8) {
            let all = self.received();
            let mut d = (all.len() as u32).to_le_bytes().to_vec();
            d.extend(crc32(0, &all).to_le_bytes());
            self.reply(op, &d);
        }
        fn handle(&mut self, p: &[u8]) {
            match p[0] {
                OP_PING => self.reply(OP_PING, &[p[1]]),
                OP_SET_PRN => {
                    self.prn = u16::from_le_bytes([p[1], p[2]]);
                    self.reply(OP_SET_PRN, &[]);
                }
                OP_MTU => self.reply(OP_MTU, &self.mtu.to_le_bytes()),
                OP_SELECT => {
                    let max = if p[1] == OBJ_COMMAND { 256 } else { self.max_data as u32 };
                    let mut d = max.to_le_bytes().to_vec();
                    d.extend([0u8; 8]);
                    self.reply(OP_SELECT, &d);
                }
                OP_CREATE => {
                    self.object_kind = p[1];
                    self.object_size = le32(&p[2..]) as usize;
                    self.object.clear();
                    self.since_prn = 0;
                    self.reply(OP_CREATE, &[]);
                }
                OP_WRITE => {
                    assert!(p.len() - 1 <= ((self.mtu as usize - 1) / 2 - 1), "packet larger than the MTU allows");
                    self.object.extend_from_slice(&p[1..]);
                    if let Some(n) = self.corrupt_after {
                        if self.image.len() + self.object.len() > n {
                            *self.object.last_mut().unwrap() ^= 0xFF;
                            self.corrupt_after = None;
                        }
                    }
                    assert!(self.object.len() <= self.object_size, "wrote past the object size");
                    self.since_prn += 1;
                    if self.prn > 0 && self.since_prn.is_multiple_of(self.prn) {
                        self.crc_reply(OP_CRC);
                    }
                }
                OP_CRC => self.crc_reply(OP_CRC),
                OP_EXECUTE => {
                    assert_eq!(self.object.len(), self.object_size, "executed an incomplete object");
                    if self.object_kind == OBJ_COMMAND {
                        self.init = std::mem::take(&mut self.object);
                    } else {
                        assert!(self.object.len() <= self.max_data);
                        self.image.append(&mut self.object);
                        self.executed_objects += 1;
                    }
                    self.reply(OP_EXECUTE, &[]);
                }
                op => panic!("unexpected opcode {op:02x}"),
            }
        }
    }

    impl Write for FakeBootloader {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            for &b in buf {
                if b == SLIP_END {
                    let frame = std::mem::take(&mut self.inbox);
                    let packet = slip_decode(&frame).unwrap();
                    self.handle(&packet);
                } else {
                    self.inbox.push(b);
                }
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Read for FakeBootloader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.outbox.is_empty() {
                return Err(std::io::Error::new(ErrorKind::TimedOut, "nothing to read"));
            }
            let n = buf.len().min(self.outbox.len());
            for slot in buf.iter_mut().take(n) {
                *slot = self.outbox.pop_front().unwrap();
            }
            Ok(n)
        }
    }

    fn package(size: usize) -> Package {
        // Include SLIP special bytes so escaping is exercised.
        let firmware: Vec<u8> = (0..size).map(|i| (i * 7 % 256) as u8).collect();
        Package { init: vec![0x12, SLIP_END, 0x34, SLIP_ESC, 0x56], firmware }
    }

    fn opts(prn: u16) -> Options {
        Options { prn, connect_delay: Duration::ZERO, timeout: Duration::from_millis(50) }
    }

    fn run(boot: &mut FakeBootloader, pkg: &Package, prn: u16) -> Result<()> {
        flash_over(&mut *boot, pkg, opts(prn), &|_| {}, &|_| {})
    }

    #[test]
    fn transfers_whole_image_without_prn() {
        let pkg = package(10_000);
        let mut boot = FakeBootloader::new(131, 4096);
        run(&mut boot, &pkg, 0).unwrap();
        assert_eq!(boot.init, pkg.init);
        assert_eq!(boot.image, pkg.firmware);
        assert_eq!(boot.executed_objects, 3);
    }

    #[test]
    fn transfers_whole_image_with_prn() {
        let pkg = package(9_000);
        let mut boot = FakeBootloader::new(64, 4096);
        run(&mut boot, &pkg, 1).unwrap();
        assert_eq!(boot.image, pkg.firmware);
    }

    #[test]
    fn reports_progress_up_to_one() {
        let pkg = package(10_000);
        let mut boot = FakeBootloader::new(131, 4096);
        let seen = std::cell::RefCell::new(Vec::new());
        flash_over(&mut boot, &pkg, opts(0), &|_| {}, &|p| seen.borrow_mut().push(p)).unwrap();
        let seen = seen.into_inner();
        assert_eq!(seen.len(), 3);
        assert!(seen.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(*seen.last().unwrap(), 1.0);
    }

    #[test]
    fn exact_multiple_of_object_size() {
        let pkg = package(8192);
        let mut boot = FakeBootloader::new(131, 4096);
        run(&mut boot, &pkg, 0).unwrap();
        assert_eq!(boot.image, pkg.firmware);
        assert_eq!(boot.executed_objects, 2);
    }

    #[test]
    fn corrupted_transfer_is_caught_before_execute() {
        let pkg = package(6000);
        let mut boot = FakeBootloader::new(131, 4096);
        boot.corrupt_after = Some(5000);
        let err = run(&mut boot, &pkg, 0).unwrap_err().to_string();
        assert!(err.contains("transfer check failed"), "{err}");
        assert_eq!(boot.executed_objects, 1, "the corrupted object must not be executed");
    }
}
