use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use coolbaremu_core::{CbeArchive, CoolbarMachine, DisplayOrientation, DEFAULT_INSTRUCTION_LIMIT};

/// Display orientation requested on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum OrientationArg {
    Auto,
    Portrait,
    Landscape,
}

impl From<OrientationArg> for DisplayOrientation {
    fn from(value: OrientationArg) -> Self {
        match value {
            OrientationArg::Auto => DisplayOrientation::Auto,
            OrientationArg::Portrait => DisplayOrientation::Portrait,
            OrientationArg::Landscape => DisplayOrientation::Landscape,
        }
    }
}

#[derive(Parser)]
#[command(about = "Run a CBE executable through the headless ARM core")]
struct Cli {
    file: PathBuf,
    #[arg(long, default_value_t = DEFAULT_INSTRUCTION_LIMIT)]
    instruction_limit: u64,
    #[arg(long, default_value_t = 0)]
    frames: u32,
    #[arg(long)]
    press_key: Option<u8>,
    #[arg(long, value_delimiter = ',', default_value = "1")]
    press_frame: Vec<u32>,
    #[arg(long, value_parser = parse_key_event)]
    key_event: Vec<(u32, u8)>,
    #[arg(long, value_parser = parse_key_hold)]
    key_hold: Vec<(u32, u32, u8)>,
    #[arg(long, value_parser = parse_pointer_event)]
    pointer_event: Vec<(u32, i32, i32)>,
    #[arg(long, value_enum, default_value_t = OrientationArg::Auto)]
    orientation: OrientationArg,
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Dump a guest memory range as u32 words: START:END in hex.
    #[arg(long, value_parser = parse_range)]
    dump: Vec<(u32, u32)>,
}

fn parse_range(value: &str) -> Result<(u32, u32), String> {
    let (start, end) = value
        .split_once(':')
        .ok_or_else(|| "range must use START:END hex syntax".to_owned())?;
    let start = u32::from_str_radix(start.trim_start_matches("0x"), 16)
        .map_err(|_| "range start must be a hex number".to_owned())?;
    let end = u32::from_str_radix(end.trim_start_matches("0x"), 16)
        .map_err(|_| "range end must be a hex number".to_owned())?;
    Ok((start, end))
}

fn parse_key_event(value: &str) -> Result<(u32, u8), String> {
    let (frame, key) = value
        .split_once(':')
        .ok_or_else(|| "key event must use FRAME:KEY syntax".to_owned())?;
    let frame = frame
        .parse()
        .map_err(|_| "key event frame must be an unsigned integer".to_owned())?;
    let key = key
        .parse()
        .map_err(|_| "key event key must be an unsigned byte".to_owned())?;
    Ok((frame, key))
}

fn parse_key_hold(value: &str) -> Result<(u32, u32, u8), String> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 3 {
        return Err("key hold must use START:END:KEY syntax".to_owned());
    }
    let start = parts[0]
        .parse()
        .map_err(|_| "key hold start must be an unsigned integer".to_owned())?;
    let end = parts[1]
        .parse()
        .map_err(|_| "key hold end must be an unsigned integer".to_owned())?;
    let key = parts[2]
        .parse()
        .map_err(|_| "key hold key must be an unsigned byte".to_owned())?;
    if start >= end {
        return Err("key hold end must be greater than start".to_owned());
    }
    Ok((start, end, key))
}

fn parse_pointer_event(value: &str) -> Result<(u32, i32, i32), String> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 3 {
        return Err("pointer event must use FRAME:X:Y syntax".to_string());
    }
    let frame = parts[0]
        .parse()
        .map_err(|_| "pointer event frame must be an unsigned integer".to_string())?;
    let x = parts[1]
        .parse()
        .map_err(|_| "pointer event x must be an integer".to_string())?;
    let y = parts[2]
        .parse()
        .map_err(|_| "pointer event y must be an integer".to_string())?;
    Ok((frame, x, y))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let archive = CbeArchive::load(&cli.file)?;
    let mut machine = CoolbarMachine::new(&archive)?;
    machine.set_orientation(cli.orientation.into());
    let mut result = machine.boot(cli.instruction_limit);
    for frame in 0..cli.frames {
        if result.is_ok() {
            let event_keys: Vec<u8> = cli
                .key_event
                .iter()
                .filter_map(|(event_frame, key)| (*event_frame == frame).then_some(*key))
                .collect();
            if cli.press_frame.contains(&frame) {
                if let Some(key) = cli.press_key {
                    machine.set_key(key, true);
                }
            }
            for key in &event_keys {
                machine.set_key(*key, true);
            }
            for &(start, end, key) in &cli.key_hold {
                if frame == start {
                    machine.set_key(key, true);
                } else if frame == end {
                    machine.set_key(key, false);
                }
            }
            for &(event_frame, x, y) in &cli.pointer_event {
                if event_frame == frame {
                    machine.set_pointer(x, y, true);
                }
            }
            let before_frame_instructions = machine.instruction_count();
            result = machine.run_frame(cli.instruction_limit);
            if std::env::var_os("CBE_FRAME_STATS").is_some() {
                eprintln!(
                    "frame {frame} instructions={} pc=0x{:08X}",
                    machine.instruction_count() - before_frame_instructions,
                    machine.last_pc()
                );
            }
            // Consume guest audio like a frontend so drained one-shots report
            // stopped and looping BGM restarts through the engine path.
            let _ = machine.take_audio_samples(4410);
            if cli.press_frame.contains(&frame) {
                if let Some(key) = cli.press_key {
                    machine.set_key(key, false);
                }
            }
            for key in event_keys {
                machine.set_key(key, false);
            }
            for &(event_frame, x, y) in &cli.pointer_event {
                if event_frame == frame {
                    machine.set_pointer(x, y, false);
                }
            }
        }
    }
    eprintln!("machine={machine:?}");
    eprintln!(
        "screen active=0x{:08X} pending=0x{:08X}",
        machine.active_screen(),
        machine.pending_screen()
    );
    eprintln!(
        "orientation requested={:?} effective={:?}",
        machine.orientation(),
        machine.effective_orientation()
    );
    let audio = machine.audio_diagnostics();
    eprintln!(
        "audio submitted_bytes={} decoded_frames={} nonzero_samples={} rejected={} underflow={} max_buffered={}",
        audio.submitted_bytes,
        audio.decoded_frames,
        audio.nonzero_samples,
        audio.rejected_writes,
        audio.underflow_frames,
        audio.max_buffered_frames
    );
    let pixels = machine.frame_pixels();
    let nonzero = pixels.iter().filter(|pixel| **pixel != 0).count();
    let colors = pixels
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    eprintln!("frame nonzero={nonzero} colors={}", colors.len());
    // Online titles stop at their login/"network error" page when the
    // platform service is unreachable; naming the URL the guest asked for is
    // the difference between "our stack is broken" and "that server is gone".
    if !machine.network_last_http_url().is_empty() {
        eprintln!(
            "network url={} uplink={} downlink={}",
            machine.network_last_http_url(),
            machine.network_uplink_bytes(),
            machine.network_downlink_bytes()
        );
    }
    if let Some(path) = &cli.screenshot {
        let (display_width, display_height) = machine.display_size();
        let rgb: Vec<u8> = pixels
            .iter()
            .flat_map(|pixel| [(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8])
            .collect();
        image::save_buffer(
            path,
            &rgb,
            display_width,
            display_height,
            image::ColorType::Rgb8,
        )?;
    }
    for register in 0..=14 {
        eprintln!("r{register}=0x{:08X}", machine.register_value(register));
    }
    let r0 = machine.register_value(0);
    for offset in (0..=0x50).step_by(4) {
        eprintln!(
            "[r0+0x{offset:02X}]=0x{:08X}",
            machine.read_u32(r0 + offset)
        );
    }
    let mut calls: Vec<_> = machine.service_calls().iter().collect();
    calls.sort_by_key(|((group, index), _)| (*group, *index));
    for ((group, index), count) in calls {
        eprintln!("service group={group} index={index} count={count}");
    }
    for address in machine.bad_accesses().iter().take(32) {
        eprintln!("unmapped address=0x{address:08X}");
    }
    // Fault-context dump: every mapped region plus the ROM bytes around the
    // last non-sequential branch site so the bad jump can be disassembled.
    // Successful runs stay quiet; only a failure needs the crime scene.
    if result.is_err() {
        eprintln!("=== memory regions ===");
        for region in machine.memory_regions() {
            eprintln!(
                "  base=0x{:08X} size=0x{:X} ro={}",
                region.base, region.size, region.read_only
            );
        }
        let fault = machine.last_pc();
        let branch = machine.last_branch_source().unwrap_or(fault);
        eprintln!("=== fault-context ROM (branch site 0x{branch:08X}) ===");
        for addr in (branch.saturating_sub(0x60)..=branch.wrapping_add(0x1C)).step_by(4) {
            eprintln!("0x{addr:08X}: 0x{:08X}", machine.read_u32(addr));
        }
        // For "unsupported instruction" faults the faulting PC is the real
        // site, so dump the bytes there too (the branch site may be padding).
        eprintln!("=== fault-context ROM (faulting pc 0x{fault:08X}) ===");
        for addr in (fault.saturating_sub(0x20)..=fault.wrapping_add(0x20)).step_by(4) {
            eprintln!("0x{addr:08X}: 0x{:08X}", machine.read_u32(addr));
        }
    }
    for &(start, end) in &cli.dump {
        eprintln!("=== dump 0x{start:08X}..0x{end:08X} ===");
        for addr in (start..end).step_by(4) {
            eprintln!("0x{addr:08X}: 0x{:08X}", machine.read_u32(addr));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_hold_range() {
        assert_eq!(parse_key_hold("20:40:16"), Ok((20, 40, 16)));
    }

    #[test]
    fn rejects_invalid_key_hold_range() {
        assert!(parse_key_hold("20:20:16").is_err());
        assert!(parse_key_hold("40:20:16").is_err());
        assert!(parse_key_hold("20:40").is_err());
    }

    #[test]
    fn uses_shared_default_instruction_limit() {
        let cli = Cli::try_parse_from(["cbe_boot", "game.CBE"]).unwrap();

        assert_eq!(cli.instruction_limit, DEFAULT_INSTRUCTION_LIMIT);
    }
}
