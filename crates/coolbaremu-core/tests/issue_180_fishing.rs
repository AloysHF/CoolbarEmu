//! Regression coverage for issue #180 — `B/捕鱼猎人.CBE`.
//!
//! The report said that picking a game mode left the title stuck on `LOADING`
//! and that continuing to input could end on a fully black frame. These tests
//! replay the issue's own scripted input and assert the semantic outcome, not
//! a screenshot hash: both modes must reach the underwater scene, the HUD must
//! be fully painted, firing a net must change the coin balance, and the mixed
//! input path must keep rendering.
//!
//! The game corpus is not checked in, so the tests are ignored by default and
//! expect `COOLBAR_GAME_DIR` (or a checkout that still has `tmp/coolbar_game`):
//!
//! ```text
//! COOLBAR_GAME_DIR=tmp/coolbar_game \
//!   cargo test -p coolbaremu-core --release --test issue_180_fishing -- --ignored --nocapture
//! ```
//!
//! Known guest-side limitation, deliberately not asserted here: the 达人 mode
//! HUD clock stays at `01:00` because the guest initialises it to 0x3c and
//! never decrements it, while the 挑战 mode countdown does advance. See
//! `docs/Game-Compatibility.md`.

use std::collections::HashMap;
use std::path::PathBuf;

use coolbaremu_core::{CbeArchive, CoolbarMachine, MachineState};

/// Instruction budget per guest callback used by the issue reproduction.
const FRAME_BUDGET: u64 = 20_000_000;

/// Audio frames a frontend drains after each guest frame.
const AUDIO_FRAMES_PER_FRAME: usize = 4410;

/// A rectangle in presented framebuffer pixels.
#[derive(Clone, Copy)]
struct Rect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

/// Coin balance readout in the bottom-right HUD corner. Only the five digit
/// tiles are compared — fish swim across the panel around it, so a wider box
/// would report animation as billing.
const COIN_HUD: Rect = Rect {
    x0: 285,
    y0: 218,
    x1: 370,
    y1: 240,
};

/// Mode timer readout in the bottom-left HUD corner.
const TIMER_HUD: Rect = Rect {
    x0: 20,
    y0: 205,
    x1: 100,
    y1: 240,
};

/// One captured frame.
struct Frame {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl Frame {
    fn region(&self, rect: Rect) -> Vec<u32> {
        let mut out = Vec::new();
        for y in rect.y0..rect.y1.min(self.height) {
            for x in rect.x0..rect.x1.min(self.width) {
                out.push(self.pixels[(y * self.width + x) as usize]);
            }
        }
        out
    }

    fn nonzero(&self) -> usize {
        self.pixels.iter().filter(|pixel| **pixel != 0).count()
    }

    fn colors(&self) -> usize {
        self.pixels
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    fn total(&self) -> usize {
        (self.width * self.height) as usize
    }
}

/// Scripted key/touch input, replayed with press on the event frame and
/// release on the following frame, exactly like `cbe_boot`.
#[derive(Default, Clone)]
struct Script {
    keys: Vec<(u32, u8)>,
    pointers: Vec<(u32, i32, i32)>,
}

fn game_path() -> PathBuf {
    let root = std::env::var_os("COOLBAR_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/coolbar_game")
        });
    let path = root.join("B").join("捕鱼猎人.CBE");
    assert!(
        path.is_file(),
        "missing corpus game {}; set COOLBAR_GAME_DIR to the folder holding A/B/C",
        path.display()
    );
    path
}

/// Boot the guest, replay `frames` frames, and capture the requested frames.
fn replay(frames: u32, script: &Script, captures: &[u32]) -> (CoolbarMachine, HashMap<u32, Frame>) {
    let archive = CbeArchive::load(&game_path()).expect("load CBE archive");
    let mut machine = CoolbarMachine::new(&archive).expect("create machine");
    machine.boot(FRAME_BUDGET).expect("boot guest application");

    let mut shots = HashMap::new();
    for frame in 0..frames {
        for &(at, key) in &script.keys {
            if at == frame {
                machine.set_key(key, true);
            }
        }
        for &(at, x, y) in &script.pointers {
            if at == frame {
                machine.set_pointer(x, y, true);
            }
        }
        machine.run_frame(FRAME_BUDGET).expect("run guest frame");
        // A real frontend drains audio every frame; keep that behaviour so
        // one-shot sounds report stopped instead of piling up.
        let _ = machine.take_audio_samples(AUDIO_FRAMES_PER_FRAME);
        if captures.contains(&frame) {
            let (width, height) = machine.display_size();
            shots.insert(
                frame,
                Frame {
                    width,
                    height,
                    pixels: machine.frame_pixels(),
                },
            );
        }
        for &(at, key) in &script.keys {
            if at == frame {
                machine.set_key(key, false);
            }
        }
        for &(at, x, y) in &script.pointers {
            if at == frame {
                machine.set_pointer(x, y, false);
            }
        }
    }
    (machine, shots)
}

fn changed_pixels(a: &Frame, b: &Frame, rect: Rect) -> usize {
    let left = a.region(rect);
    let right = b.region(rect);
    left.iter()
        .zip(right.iter())
        .filter(|(left, right)| left != right)
        .count()
}

fn assert_fully_painted(frame: &Frame, label: &str) {
    assert!(
        frame.nonzero() == frame.total(),
        "{label}: expected a fully painted {}x{} frame, got {} non-zero pixels",
        frame.width,
        frame.height,
        frame.nonzero()
    );
    assert!(
        frame.colors() >= 256,
        "{label}: expected a detailed scene, got only {} colours",
        frame.colors()
    );
}

#[test]
#[ignore = "requires the local CBE corpus; set COOLBAR_GAME_DIR"]
fn expert_mode_reaches_gameplay_and_bills_a_net() {
    // 达人 mode: tick 达人 at (74, 200), enter it at (109, 200), then press 5
    // (fire) on frame 950. The control replay shares the script up to the fire
    // key, so any digit-tile difference at the same frame afterwards is caused
    // by the key press — the tiles have a frame-driven shine animation that
    // cancels out in the same-frame comparison.
    let enter = Script {
        keys: vec![(950, 5)],
        pointers: vec![(20, 74, 200), (80, 109, 200)],
    };
    let idle = Script {
        keys: Vec::new(),
        pointers: vec![(20, 74, 200), (80, 109, 200)],
    };
    let (machine, fired) = replay(1_150, &enter, &[949, 1_149]);
    let (_, idle_shots) = replay(1_150, &idle, &[949, 1_149]);

    assert_eq!(
        machine.state(),
        MachineState::Ready,
        "达人 mode must stay running instead of faulting or halting"
    );

    assert_fully_painted(&fired[&949], "达人 frame 949");
    assert_eq!(
        (fired[&949].width, fired[&949].height),
        (400, 240),
        "达人 mode presents a 400x240 landscape screen"
    );

    assert_eq!(
        changed_pixels(&fired[&949], &idle_shots[&949], COIN_HUD),
        0,
        "the two replays must render identical coin digits before the fire key"
    );
    let billed = changed_pixels(&fired[&1_149], &idle_shots[&1_149], COIN_HUD);
    assert!(
        billed > 0,
        "firing a net on frame 950 must update the coin readout, but its digit \
         tiles are byte-identical at frame 1,149"
    );
    // Billing lowers the balance (00200 -> 00199), so the glyphs lose ink.
    let dark = |frame: &Frame| {
        frame
            .region(COIN_HUD)
            .iter()
            .filter(|pixel| {
                let pixel = **pixel;
                let [r, g, b] = [pixel >> 16 & 0xff, pixel >> 8 & 0xff, pixel & 0xff];
                r < 50 && g < 50 && b < 50
            })
            .count()
    };
    assert!(
        dark(&fired[&1_149]) < dark(&idle_shots[&1_149]),
        "the balance must go down after firing, but the digit glyphs did not \
         lose ink"
    );
    eprintln!("达人 mode: {billed} coin-digit pixels changed after firing, state Ready");
}

#[test]
#[ignore = "requires the local CBE corpus; set COOLBAR_GAME_DIR"]
fn challenge_mode_countdown_advances() {
    // 挑战 mode needs the mode tick plus a second confirmation tap.
    let script = Script {
        keys: Vec::new(),
        pointers: vec![(20, 74, 200), (80, 74, 200), (100, 74, 200)],
    };
    let (machine, shots) = replay(450, &script, &[200, 400]);

    assert_eq!(
        machine.state(),
        MachineState::Ready,
        "挑战 mode must stay running instead of faulting or halting"
    );

    let early = &shots[&200];
    assert_fully_painted(early, "挑战 frame 200");

    let later = &shots[&400];
    let ticks = changed_pixels(early, later, TIMER_HUD);
    assert!(
        ticks > 0,
        "挑战 mode countdown must advance between frames 200 and 400, but the \
         timer HUD is byte-identical"
    );
    assert_eq!(
        changed_pixels(early, later, COIN_HUD),
        0,
        "challenge coin readout must stay put when no net is fired"
    );
    eprintln!("挑战 mode: {ticks} timer pixels changed between frames 200 and 400");
}

#[test]
#[ignore = "requires the local CBE corpus; set COOLBAR_GAME_DIR"]
fn mixed_input_path_keeps_rendering() {
    // The exact input sequence from the issue that used to end on a black
    // frame at 1,000 frames.
    let script = Script {
        keys: vec![(400, 14), (500, 12), (600, 5)],
        pointers: vec![
            (20, 74, 200),
            (80, 109, 200),
            (300, 119, 200),
            (700, 119, 200),
        ],
    };
    let (machine, shots) = replay(1_000, &script, &[999]);

    assert_eq!(
        machine.state(),
        MachineState::Ready,
        "the mixed-input path must not fault or halt"
    );
    assert_fully_painted(&shots[&999], "mixed input frame 999");
}
