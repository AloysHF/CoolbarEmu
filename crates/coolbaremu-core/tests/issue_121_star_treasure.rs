//! Regression coverage for issue #121 — `A/星际宝藏(新品).CBE`.
//!
//! Normal mode and LEVEL 1 both load, but the playfield rendered only the grey
//! level background: the eight-by-eight gem board never appeared and only the
//! tutorial text and part of the HUD updated. The guest builds the board actor
//! table out of recycled heap that still held title-screen graphics, read that
//! garbage as per-slot actor state, and never drew the cells. Host allocations
//! now come back zeroed, the way the firmware returns them.
//!
//! The game corpus is not checked in, so the tests are ignored by default and
//! expect `COOLBAR_GAME_DIR` (or a checkout that still has `tmp/coolbar_game`):
//!
//! ```text
//! COOLBAR_GAME_DIR=tmp/coolbar_game \
//!   cargo test -p coolbaremu-core --release --test issue_121_star_treasure -- --ignored --nocapture
//! ```

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use coolbaremu_core::{CbeArchive, CoolbarMachine, MachineState};

/// Instruction budget per guest callback used by the issue reproduction.
const FRAME_BUDGET: u64 = 20_000_000;

/// Audio frames a frontend drains after each guest frame.
const AUDIO_FRAMES_PER_FRAME: usize = 4410;

/// The gem board and its surrounding level panel.
const PLAYFIELD: Rect = Rect {
    x0: 16,
    y0: 60,
    x1: 224,
    y1: 290,
};

/// Score readout in the top-right HUD corner. The tutorial cursor arrow can
/// reach into it, so comparisons only use frames that share the cursor cell.
const SCORE_HUD: Rect = Rect {
    x0: 194,
    y0: 51,
    x1: 215,
    y1: 70,
};

/// A rectangle in presented framebuffer pixels.
#[derive(Clone, Copy)]
struct Rect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

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

    /// Pixels with a strongly chromatic colour. The level background is a
    /// grey industrial scene with a few cyan trim lines, so a populated gem
    /// board adds thousands of saturated pixels.
    fn saturated(&self, rect: Rect) -> usize {
        self.region(rect)
            .into_iter()
            .filter(|pixel| {
                let [r, g, b] = [pixel >> 16 & 0xff, pixel >> 8 & 0xff, pixel & 0xff];
                let high = r.max(g).max(b);
                let low = r.min(g).min(b);
                high >= 90 && high - low >= 60
            })
            .count()
    }

    fn colors(&self, rect: Rect) -> usize {
        self.region(rect).into_iter().collect::<HashSet<_>>().len()
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
    let path = root.join("A").join("星际宝藏(新品).CBE");
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
    a.region(rect)
        .into_iter()
        .zip(b.region(rect))
        .filter(|(left, right)| left != right)
        .count()
}

/// The issue's own reproduction: confirm the title, pick the normal mode and
/// enter LEVEL 1. The key codes are guest key bit indices; 14 confirms.
fn level_entry_keys() -> Vec<(u32, u8)> {
    vec![(20, 14), (50, 14), (80, 12), (110, 14)]
}

#[test]
#[ignore = "requires the local CBE corpus; set COOLBAR_GAME_DIR"]
fn level_one_renders_the_gem_board_and_accepts_swaps() {
    // Tap the selected cell to enter board input, move the cursor to column
    // 6 row 0, confirm, then tap the right neighbour to swap. The swap
    // completes a horizontal match and refills the cleared cells.
    let swap = Script {
        keys: level_entry_keys(),
        pointers: vec![
            (261, 36, 84),
            (281, 180, 84),
            (301, 180, 84),
            (321, 204, 84),
        ],
    };
    // Identical replay without the final tap: the cursor ends on the same
    // cell, so any remaining pixel difference comes from the board itself.
    let play = Script {
        keys: level_entry_keys(),
        pointers: vec![(261, 36, 84), (281, 180, 84), (301, 180, 84)],
    };

    let (machine, swap_shots) = replay(701, &swap, &[700]);
    let (_, play_shots) = replay(701, &play, &[700]);

    assert_eq!(
        machine.state(),
        MachineState::Ready,
        "LEVEL 1 must stay running instead of faulting or halting"
    );

    let swap_frame = &swap_shots[&700];
    let play_frame = &play_shots[&700];
    assert_eq!(
        (swap_frame.width, swap_frame.height),
        (240, 320),
        "LEVEL 1 presents a 240x320 portrait screen"
    );

    // The reported symptom: only the grey background was drawn where the
    // gem board belongs.
    let saturated = swap_frame.saturated(PLAYFIELD);
    assert!(
        saturated > 10_000,
        "the gem board must be drawn, but the playfield only has {saturated} \
         saturated pixels"
    );
    let colors = swap_frame.colors(PLAYFIELD);
    assert!(
        colors >= 50,
        "the playfield must show the gem palette, but it only has {colors} colours"
    );

    // The reported second half: the board must react to input, not just look
    // painted. A completed swap clears and refills cells, which changes both
    // the board pixels and the HUD score readout.
    let board_changed = changed_pixels(play_frame, swap_frame, PLAYFIELD);
    assert!(
        board_changed > 500,
        "swapping two cells must change the board, but only {board_changed} \
         playfield pixels differ from the replay without the swap"
    );
    let score_changed = changed_pixels(play_frame, swap_frame, SCORE_HUD);
    assert!(
        score_changed > 0,
        "a completed match must update the score readout, but its pixels are \
         identical to the replay without the swap"
    );
}
