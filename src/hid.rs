//! Finding and opening the vendor HID interfaces.
use anyhow::{bail, Context, Result};
use hidapi::{HidApi, HidDevice};
use std::{fs, path::Path};

pub struct Opened {
    pub pid: u16,
    pub path: String,
    pub dev: HidDevice,
}

/// Open the first interface matching `vid`, one of `pids` (in preference order) and the usage pair.
pub fn open(api: &HidApi, vid: u16, pids: &[u16], usage_page: u16, usage: u16) -> Result<Option<Opened>> {
    for &pid in pids {
        let found = api.device_list().find(|d| {
            d.vendor_id() == vid && d.product_id() == pid && d.usage_page() == usage_page && d.usage() == usage
        });
        if let Some(info) = found {
            let path = info.path().to_string_lossy().into_owned();
            let dev = info.open_device(api).with_context(|| {
                format!("cannot open {path}; install 70-glorious-linux.rules (instructions at its top) and replug")
            })?;
            return Ok(Some(Opened { pid, path, dev }));
        }
    }
    Ok(None)
}

pub fn send_feature(dev: &HidDevice, report: &[u8]) -> Result<()> {
    dev.send_feature_report(report).context("SET_FEATURE failed")
}

/// GET_FEATURE into a buffer whose first byte is the report ID; returns the whole buffer.
pub fn get_feature(dev: &HidDevice, report_id: u8, len: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    buf[0] = report_id;
    let n = dev.get_feature_report(&mut buf).context("GET_FEATURE failed")?;
    if n < 4 {
        bail!("short feature report ({n} bytes)");
    }
    Ok(buf)
}

/// The /dev/ttyACM* node of a USB serial device (used by the nRF52 DFU bootloaders).
pub fn tty_for(vid: u16, pid: u16) -> Option<String> {
    let mut entries: Vec<_> = fs::read_dir("/sys/class/tty").ok()?.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("ttyACM") {
            continue;
        }
        let Ok(mut dev) = fs::canonicalize(entry.path().join("device")) else { continue };
        while !dev.join("idVendor").exists() {
            if !dev.pop() {
                break;
            }
        }
        if read_hex(&dev.join("idVendor")) == Some(vid) && read_hex(&dev.join("idProduct")) == Some(pid) {
            return Some(format!("/dev/{name}"));
        }
    }
    None
}

fn read_hex(path: &Path) -> Option<u16> {
    u16::from_str_radix(fs::read_to_string(path).ok()?.trim(), 16).ok()
}
