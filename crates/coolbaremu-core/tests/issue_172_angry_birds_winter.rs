//! Regression coverage for issue #172 — `B/小鸟愤怒冬季版.CBE`.
//!
//! The report said the game gets stuck on a SELECT EPISODE page that only
//! shows question-mark tiles and gift boxes, so no level can ever be entered.
//! On top of that blocker, this game's slingshot only aims while the guest
//! keeps receiving touch type 5 drag events with a fresh position every frame,
//! which a real mouse produces naturally and a single-jump probe replay does
//! not. These tests replay the issue's own scripted path and assert the
//! semantic outcome: the episode page resolves into the level grid, a smooth
//! drag launches the bird and clears level 1-1 (the cleared panel's green
//! backdrop fills the screen centre while an idle replay keeps showing snow),
//! and the cleared panel's level-grid button returns to a grid where level 2
//! now wears the red unlocked badge.
//!
//! The game corpus is not checked in, so the tests are ignored by default and
//! expect `COOLBAR_GAME_DIR` (or a checkout that still has `tmp/coolbar_game`):
//!
//! ```text
//! COOLBAR_GAME_DIR=tmp/coolbar_game \
//!   cargo test -p coolbaremu-core --release --test issue_172_angry_birds_winter -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use coolbaremu_core::{CbeArchive, CoolbarMachine, MachineState};

/// Instruction budget per guest callback.
const FRAME_BUDGET: u64 = 20_000_000;

/// Audio frames a frontend drains after each guest frame.
const AUDIO_FRAMES_PER_FRAME: usize = 4410;

/// Smooth drag: press at (x1,y1) on the start frame, interpolate the held
/// position every frame in between, release at (x2,y2) on the end frame.
#[derive(Clone, Copy)]
struct Drag {
    start: u32,
    end: u32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
}

/// Scripted input in raw framebuffer coordinates (x 0..239, y 0..399). Taps
/// press on the event frame and release after that frame's run, exactly like
/// `cbe_boot`; comments give the presented landscape coordinate of each target.
#[derive(Default, Clone)]
struct Script {
    taps: Vec<(u32, i32, i32)>,
    drags: Vec<Drag>,
}

impl Script {
    /// The shared path into level 1-1 with the tutorial dialog dismissed.
    fn into_gameplay() -> Self {
        Self {
            // Title: start (presented 200,109), gift box (125,139),
            // confirm (120,65), episode tap (280,139), episode confirm (285,65).
            taps: vec![
                (20, 109, 200),
                (80, 139, 125),
                (140, 65, 120),
                (200, 139, 280),
                (260, 65, 285),
            ],
            drags: Vec::new(),
        }
        .with_taps(&[
            // Dialog advances (keys), level 1 circle (presented 81,194),
            // tutorial check mark (presented 240,67).
            (320, 14, 0),
            (360, 5, 0),
            (400, 16, 0),
            (440, 14, 0),
            (600, 194, 81),
            (800, 67, 240),
        ])
    }

    fn with_taps(mut self, taps: &[(u32, i32, i32)]) -> Self {
        self.taps.extend_from_slice(taps);
        self
    }

    fn with_drag(mut self, drag: Drag) -> Self {
        self.drags.push(drag);
        self
    }
}

/// A rectangle in presented framebuffer pixels.
#[derive(Clone, Copy)]
struct Rect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

/// Screen-centre area covered by the cleared panel's green backdrop; the
/// playing field shows only sky, houses and snow there.
const PANEL_AREA: Rect = Rect {
    x0: 110,
    y0: 90,
    x1: 300,
    y1: 150,
};

/// The level-2 badge circle on the level grid, red once unlocked.
const BADGE_AREA: Rect = Rect {
    x0: 118,
    y0: 30,
    x1: 158,
    y1: 62,
};

fn game_path() -> PathBuf {
    let root = std::env::var_os("COOLBAR_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/coolbar_game")
        });
    let path = root.join("B").join("小鸟愤怒冬季版.CBE");
    assert!(
        path.is_file(),
        "missing corpus game {}; set COOLBAR_GAME_DIR to the folder holding A/B/C",
        path.display()
    );
    path
}

/// Replay `frames` frames and return the machine at the final frame.
fn replay(frames: u32, script: &Script) -> CoolbarMachine {
    let archive = CbeArchive::load(&game_path()).expect("load CBE archive");
    let mut machine = CoolbarMachine::new(&archive).expect("create machine");
    machine.boot(FRAME_BUDGET).expect("boot guest application");

    for frame in 0..frames {
        for &(at, x, y) in &script.taps {
            if at == frame {
                machine.set_pointer(x, y, true);
            }
        }
        for &drag in &script.drags {
            let (x, y, down) = if frame == drag.start {
                (drag.x1, drag.y1, true)
            } else if frame == drag.end {
                (drag.x2, drag.y2, false)
            } else if drag.start < frame && frame < drag.end {
                let t = f64::from(frame - drag.start) / f64::from(drag.end - drag.start);
                (
                    drag.x1 + ((f64::from(drag.x2 - drag.x1)) * t).round() as i32,
                    drag.y1 + ((f64::from(drag.y2 - drag.y1)) * t).round() as i32,
                    true,
                )
            } else {
                continue;
            };
            machine.set_pointer(x, y, down);
        }
        machine.run_frame(FRAME_BUDGET).expect("run guest frame");
        // A real frontend drains audio every frame; keep that behaviour so
        // one-shot sounds report stopped instead of piling up.
        let _ = machine.take_audio_samples(AUDIO_FRAMES_PER_FRAME);
        for &(at, x, y) in &script.taps {
            if at == frame {
                machine.set_pointer(x, y, false);
            }
        }
    }
    machine
}

/// Pixels whose green channel dominates both others — the cleared panel's
/// backdrop greens, absent from the sky/houses palette of the playing field.
fn green_pixels(machine: &mut CoolbarMachine, area: Rect) -> usize {
    let (width, _) = machine.display_size();
    let pixels = machine.frame_pixels();
    let mut count = 0;
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let pixel = pixels[(y * width + x) as usize];
            let [r, g, b] = [pixel >> 16 & 0xff, pixel >> 8 & 0xff, pixel & 0xff];
            if g > r + 30 && g > b + 30 {
                count += 1;
            }
        }
    }
    count
}

/// Pixels of the red unlocked-level badge drawn over the grey locked circles.
fn red_badge_pixels(machine: &mut CoolbarMachine, area: Rect) -> usize {
    let (width, _) = machine.display_size();
    let pixels = machine.frame_pixels();
    let mut count = 0;
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let pixel = pixels[(y * width + x) as usize];
            let [r, g, b] = [pixel >> 16 & 0xff, pixel >> 8 & 0xff, pixel & 0xff];
            if r > 100 && g < 60 && b < 60 {
                count += 1;
            }
        }
    }
    count
}

#[test]
#[ignore = "requires the local CBE corpus; set COOLBAR_GAME_DIR"]
fn slingshot_launch_clears_level_1() {
    // Pull the bird at raw (91,109) back to raw (44,60) — presented
    // (109,148) to (60,195), the natural down-left slingshot pull — over
    // 100 frames so the guest keeps seeing touch type 5 aim events.
    let launch = Script::into_gameplay().with_drag(Drag {
        start: 900,
        end: 1000,
        x1: 91,
        y1: 109,
        x2: 44,
        y2: 60,
    });
    let idle = Script::into_gameplay();
    let mut played = replay(1_400, &launch);
    let mut untouched = replay(1_400, &idle);

    assert_eq!(
        played.state(),
        MachineState::Ready,
        "the launch path must keep the guest running instead of faulting"
    );
    assert_eq!(
        untouched.state(),
        MachineState::Ready,
        "the idle path must keep the guest running instead of faulting"
    );

    let cleared = green_pixels(&mut played, PANEL_AREA);
    let waiting = green_pixels(&mut untouched, PANEL_AREA);
    assert!(
        cleared > 4_000,
        "the cleared panel must fill the screen centre with its green \
         backdrop, found only {cleared} green pixels"
    );
    assert!(
        waiting < 500,
        "without the drag the level must keep showing the playing field, \
         but {waiting} green pixels appeared"
    );
    eprintln!("cleared panel green={cleared}, idle playing field green={waiting}");
}

#[test]
#[ignore = "requires the local CBE corpus; set COOLBAR_GAME_DIR"]
fn cleared_level_returns_to_grid_with_next_level_unlocked() {
    // Clear the level, then tap the cleared panel's level-grid button at
    // presented (130,195) on frame 1500.
    let script = Script::into_gameplay()
        .with_drag(Drag {
            start: 900,
            end: 1000,
            x1: 91,
            y1: 109,
            x2: 44,
            y2: 60,
        })
        .with_taps(&[(1500, 44, 130)]);
    let mut machine = replay(1_700, &script);

    assert_eq!(
        machine.state(),
        MachineState::Ready,
        "the return path must keep the guest running instead of faulting"
    );

    let badge = red_badge_pixels(&mut machine, BADGE_AREA);
    assert!(
        badge > 150,
        "returning from a cleared level must show the level grid with level 2 \
         wearing the red unlocked badge, found only {badge} badge-red pixels"
    );
    let cleared = green_pixels(&mut machine, PANEL_AREA);
    assert!(
        cleared < 500,
        "the cleared panel must be gone after returning, but {cleared} green \
         pixels remain"
    );
    eprintln!("returned grid badge-red={badge}, panel green={cleared}");
}
