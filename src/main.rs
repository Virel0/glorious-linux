//! `glorious`: configure Glorious Model O3/D3 mice and check Glorious firmware on Linux.
//! Without a subcommand it opens the GUI.
mod firmware;
mod gmmk3he;
mod gui;
mod hid;
mod kb_gui;
mod keybed;
mod nordic_dfu;
mod settings;
mod theme;
mod v3;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use hidapi::HidApi;
use v3::{lookup, Mouse, BUTTONS, DEFAULT_STAGE_COLORS, EFFECTS, FUNCTIONS};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Print raw HID reports
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    cmd: Option<Top>,
}

#[derive(Subcommand)]
enum Top {
    /// Model O3 / D3 mouse (wired or via the V3 receiver)
    Mouse {
        #[command(subcommand)]
        cmd: MouseCmd,
    },
    /// GMMK 3 HE keyboard
    Keyboard {
        #[command(subcommand)]
        cmd: KeyboardCmd,
    },
}

#[derive(Subcommand)]
enum KeyboardCmd {
    /// Firmware versions (with update check) and the active profile; read-only
    Info,
}

#[derive(Subcommand)]
enum MouseCmd {
    /// Model, colour, firmware, battery and current settings
    Info,
    /// Set 1-6 DPI stages, e.g. `dpi 400,800,1600,3200 --active 3`
    Dpi {
        #[arg(value_delimiter = ',', required = true)]
        stages: Vec<u16>,
        /// 1-based stage to switch to (default: 3rd or last)
        #[arg(long)]
        active: Option<u8>,
        /// Hex colour per stage, e.g. ff0000,00ff00
        #[arg(long, value_delimiter = ',')]
        colors: Vec<String>,
    },
    /// Polling rate in Hz (125-8000)
    Polling {
        rate: u16,
        /// Set the wired rate instead of the wireless one
        #[arg(long)]
        wired: bool,
    },
    /// Lift-off distance in mm
    Liftoff {
        #[arg(value_parser = ["0.7", "1", "2"])]
        mm: String,
    },
    /// Button debounce in ms
    Debounce {
        #[arg(long)]
        mechanical: Option<u8>,
        #[arg(long)]
        optical: Option<u8>,
    },
    /// Motion sync on/off
    MotionSync {
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
    /// Minutes of inactivity before sleeping (0 = never, max 60)
    Sleep { minutes: u32 },
    /// RGB effect: off, rainbow, breathing-single, breathing, seamless-breathing, solid, fairies,
    /// ghost, cascade, prismatic, wave, ripple
    Lighting {
        effect: String,
        /// 0-100
        #[arg(long, default_value_t = 60)]
        brightness: u8,
        /// 0-100, defaults to --brightness
        #[arg(long)]
        wireless_brightness: Option<u8>,
        #[arg(long, default_value_t = 50)]
        speed: u16,
        /// Hex RGB for single-colour effects, e.g. ff8800
        #[arg(long)]
        color: Option<String>,
    },
    /// Remap a button (left, right, middle, forward, back, dpi) to a mouse function or key:<name|hex usage>
    Bind { button: String, action: String },
    /// Print the active DPI stage whenever it changes (Ctrl+C to stop)
    #[command(hide = true)]
    WatchDpi,
    /// Send one raw command and print the reply (protocol debugging)
    #[command(hide = true)]
    Raw {
        /// "mouse" (0xBB) or "receiver" (0xAA)
        target: String,
        /// Command byte, hex
        cmd: String,
        /// Payload bytes, hex
        payload: Vec<String>,
    },
    /// Check for or install mouse/receiver firmware
    Firmware {
        #[command(subcommand)]
        cmd: FirmwareCmd,
    },
}

#[derive(Subcommand)]
enum FirmwareCmd {
    /// Compare installed firmware with the latest Glorious release
    Check,
    /// Flash the latest firmware (asks for confirmation)
    Flash {
        #[arg(long, value_enum, default_value = "all")]
        part: firmware::Part,
        /// Print every step without touching the devices
        #[arg(long)]
        dry_run: bool,
        /// Flash even if already up to date
        #[arg(long)]
        force: bool,
    },
}

fn hex_rgb(s: &str) -> Result<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        bail!("colour must be 6 hex digits, got {s:?}");
    }
    let v = u32::from_str_radix(s, 16)?;
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn mouse_info(m: &Mouse) -> Result<()> {
    let link = if m.via_receiver() { "via wireless receiver" } else { "wired" };
    println!("Device:      {} ({link})", m.path);
    println!("Model:       {}", m.model_name()?);
    println!("Color:       {}", m.color()?);
    println!("Firmware:    {}", m.firmware()?);
    if m.via_receiver() {
        let [a, b] = m.receiver_firmware()?;
        println!("Receiver FW: {a} / {b}");
    }
    let bat = m.battery()?;
    println!(
        "Battery:     {}% {} ({} mV); secondary {}% {}",
        bat.level, bat.status, bat.millivolts, bat.secondary_level, bat.secondary_status
    );
    let l = m.lighting()?;
    let effect = EFFECTS.iter().find(|e| e.1 == l.effect).map_or(l.effect.to_string(), |e| e.0.to_string());
    println!("Lighting:    {effect}, brightness {}/{}, speed {}", l.wired_brightness, l.wireless_brightness, l.speed);
    let s = settings::read(m)?;
    let p = &s.performance;
    let stages: Vec<String> = p
        .stages
        .iter()
        .enumerate()
        .map(|(i, st)| {
            let mark = if i == p.active { "*" } else { "" };
            if st.enabled { format!("{mark}{}", st.dpi) } else { "off".into() }
        })
        .collect();
    println!("DPI stages:  {}   (* = active)", stages.join(" / "));
    println!("Polling:     {} Hz wireless, {} Hz cable", p.polling_wireless, p.polling_wired);
    println!("Lift-off:    {} mm   Motion sync: {}", ["0.7", "1", "2"].get(p.lift_off as usize).unwrap_or(&"?"), if p.motion_sync { "on" } else { "off" });
    println!("Debounce:    {} ms mechanical, {} ms optical", p.debounce_mechanical, p.debounce_optical);
    let buttons: Vec<String> = s.buttons.iter().map(|(b, a)| format!("{b}={a}")).collect();
    println!("Buttons:     {}", buttons.join(", "));
    let (sleep_ms, _) = m.power_saving()?;
    if sleep_ms == 0 {
        println!("Sleep after: disabled");
    } else {
        println!("Sleep after: {} min", sleep_ms / 60_000);
    }
    Ok(())
}

fn mouse(cmd: MouseCmd, verbose: bool) -> Result<()> {
    if let MouseCmd::Firmware { cmd } = cmd {
        return match cmd {
            FirmwareCmd::Check => firmware::cli(false, firmware::Part::All, false, false),
            FirmwareCmd::Flash { part, dry_run, force } => firmware::cli(true, part, dry_run, force),
        };
    }
    let api = HidApi::new()?;
    let m = Mouse::open(&api, verbose)?;
    match cmd {
        MouseCmd::Info => mouse_info(&m)?,
        MouseCmd::Dpi { stages, active, colors } => {
            if !(1..=6).contains(&stages.len()) {
                bail!("give between 1 and 6 DPI stages");
            }
            let mut rgb: Vec<[u8; 3]> = colors.iter().map(|c| hex_rgb(c)).collect::<Result<_>>()?;
            rgb.extend_from_slice(&DEFAULT_STAGE_COLORS[rgb.len().min(6)..]);
            let active = match active {
                Some(a) if a >= 1 && (a as usize) <= stages.len() => a - 1,
                Some(_) => bail!("--active must point at one of the given stages (1-based)"),
                None => (stages.len() as u8 - 1).min(2),
            };
            m.set_dpi_stages(&stages, &rgb, active)?;
            println!("DPI stages set to {stages:?}, active stage {} ({} DPI)", active + 1, stages[active as usize]);
        }
        MouseCmd::Polling { rate, wired } => {
            if ![125, 250, 500, 1000, 2000, 4000, 8000].contains(&rate) {
                bail!("polling rate must be one of 125, 250, 500, 1000, 2000, 4000, 8000");
            }
            m.set_polling(rate, !wired)?;
            println!("{} polling rate set to {rate} Hz", if wired { "Wired" } else { "Wireless" });
        }
        MouseCmd::Liftoff { mm } => {
            m.set_lift_off(match mm.as_str() { "0.7" => 0, "1" => 1, _ => 2 })?;
            println!("Lift-off distance set to {mm} mm");
        }
        MouseCmd::Debounce { mechanical, optical } => {
            if mechanical.is_none() && optical.is_none() {
                bail!("give --mechanical and/or --optical");
            }
            m.set_debounce(mechanical, optical)?;
            println!("Debounce updated");
        }
        MouseCmd::MotionSync { state } => {
            m.set_motion_sync(state == "on")?;
            println!("Motion sync {state}");
        }
        MouseCmd::Sleep { minutes } => {
            if minutes > 60 {
                bail!("sleep timer must be 0-60 minutes (0 disables it)");
            }
            m.set_power_saving(minutes)?;
            println!("Sleep timer set; mouse reports {} min", m.power_saving()?.0 / 60_000);
        }
        MouseCmd::Lighting { effect, brightness, wireless_brightness, speed, color } => {
            let Some(id) = lookup(&EFFECTS, &effect) else {
                bail!("unknown effect; use one of {}", EFFECTS.map(|e| e.0).join(", "));
            };
            let rgb = color.as_deref().map(hex_rgb).transpose()?;
            let b = brightness.min(100);
            m.set_lighting(id, b, wireless_brightness.unwrap_or(b).min(100), speed, rgb)?;
            println!("Lighting set to {effect}");
        }
        MouseCmd::Bind { button, action } => {
            if lookup(&BUTTONS, &button).is_none() {
                bail!("unknown button; use one of {}", BUTTONS.map(|b| b.0).join(", "));
            }
            if !action.starts_with("key:") && !FUNCTIONS.iter().any(|f| f.0 == action) {
                bail!("unknown action; use one of {} or key:<name|hex usage>", FUNCTIONS.map(|f| f.0).join(", "));
            }
            settings::apply_button(&m, &button, &action)?;
            println!("{button} -> {action}");
        }
        MouseCmd::WatchDpi => {
            let mut last = None;
            let start = std::time::Instant::now();
            let (mut polls, mut errors) = (0u32, 0u32);
            loop {
                polls += 1;
                match m.active_stage_fast() {
                    Ok(s) if Some(s) != last => {
                        println!("{:>7.2}s  stage {}  ({polls} polls, {errors} errors)", start.elapsed().as_secs_f32(), s + 1);
                        last = Some(s);
                    }
                    Ok(_) => {}
                    Err(_) => errors += 1,
                }
                if polls % 40 == 0 {
                    eprintln!("{:>7.2}s  {polls} polls, {errors} errors", start.elapsed().as_secs_f32());
                }
            }
        }
        MouseCmd::Raw { target, cmd, payload } => {
            let target = if target == "receiver" { v3::RECEIVER } else { v3::MOUSE };
            let cmd = u8::from_str_radix(cmd.trim_start_matches("0x"), 16)?;
            let payload: Vec<u8> = payload.iter().map(|b| u8::from_str_radix(b.trim_start_matches("0x"), 16)).collect::<Result<_, _>>()?;
            let reply = m.query(target, cmd, &payload)?;
            let end = reply.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
            println!("{:02x}: {}", cmd, reply[..end.max(1)].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
        }
        MouseCmd::Firmware { .. } => unreachable!(),
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        None => gui::run(),
        Some(Top::Mouse { cmd }) => mouse(cmd, cli.verbose),
        Some(Top::Keyboard { cmd: KeyboardCmd::Info }) => gmmk3he::cli(),
    }
}
