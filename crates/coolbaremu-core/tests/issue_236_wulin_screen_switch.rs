//! Regression coverage for the game-service screen-switch teardown (issue
//! #236 follow-up): `handle_game_service` index 60 queues the outgoing
//! screen's destroy callback, and that callback frees images through
//! IMG_Destory. The firmware's `IMG_Destory` only releases heap-owned image
//! kinds, and the fixed-manager path must reach that guarded implementation —
//! otherwise the stub fill clobbers the image and `B/武林外传V10.CBE` never
//! leaves its 载入中 screen.
//!
//! The game corpus is not checked in, so the test is ignored by default and
//! expects `COOLBAR_GAME_DIR` (or a checkout that still has `tmp/coolbar_game`):
//!
//! ```text
//! COOLBAR_GAME_DIR=tmp/coolbar_game \
//!   cargo test -p coolbaremu-core --release --test issue_236_wulin_screen_switch -- --ignored
//! ```

use coolbaremu_core::{CbeArchive, CoolbarMachine, MachineState};
use std::path::PathBuf;

/// The title menu frame measured on the fixed build: 36 213 lit pixels in 68
/// colours. The broken boot keeps 619 lit pixels in 3 colours, so the
/// thresholds sit far from either side.
const MENU_LIT_PIXELS: usize = 20_000;
const MENU_COLORS: usize = 40;

#[ignore = "requires the local CBE corpus; set COOLBAR_GAME_DIR"]
#[test]
fn wulin_v10_reaches_its_title_menu_through_the_screen_switch() {
    let root = std::env::var_os("COOLBAR_GAME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/coolbar_game")
        });
    let path = root.join("B").join("武林外传V10.CBE");
    assert!(
        path.is_file(),
        "missing corpus game {}; set COOLBAR_GAME_DIR to the folder holding A/B/C",
        path.display()
    );

    let archive = CbeArchive::load(&path).expect("load CBE archive");
    let mut machine = CoolbarMachine::new(&archive).expect("create machine");
    machine.boot(5_000_000).expect("boot guest application");
    for _ in 0..200 {
        machine.run_frame(100_000_000).expect("run guest frame");
    }

    assert_eq!(machine.state(), MachineState::Ready);
    let pixels = machine.frame_pixels();
    let lit = pixels.iter().filter(|pixel| **pixel != 0).count();
    let colors = pixels
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert!(
        lit > MENU_LIT_PIXELS,
        "the title menu must render through the screen-switch destroy; got {lit} lit pixels"
    );
    assert!(
        colors > MENU_COLORS,
        "the title menu must render in full colour; got {colors} colours"
    );
}
