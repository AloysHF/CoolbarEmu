# CoolbarEmu — A Coolbar/CBE game emulator written in Rust

<p align="center">
  <img src="res/logo-banner.png" alt="CoolbarEmu" width="600">
</p>

<p align="center">
  <a href="https://aloyshf.github.io/CoolbarEmu/"><img src="https://img.shields.io/badge/Website-CoolbarEmu-E8553A?logo=githubpages&logoColor=white" alt="Website"></a>
  <a href="https://github.com/AloysHF/CoolbarEmu/actions/workflows/ci.yml"><img src="https://github.com/AloysHF/CoolbarEmu/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://git.libretro.com/libretro/coolbaremu/-/pipelines"><img src="https://img.shields.io/gitlab/pipeline-status/coolbaremu?gitlab_url=https%3A%2F%2Fgit.libretro.com%2Flibretro&branch=master&logo=gitlab&label=Pipeline%20Status" alt="Gitlab Pipeline Status" ></a>
  <a href="https://github.com/AloysHF/CoolbarEmu/releases/latest"><img src="https://img.shields.io/github/v/release/AloysHF/CoolbarEmu" alt="Release"></a>
  <a href="https://github.com/AloysHF/CoolbarEmu/releases"><img src="https://img.shields.io/github/downloads/AloysHF/CoolbarEmu/total" alt="Downloads"></a>
  <a href="https://sonarcloud.io/dashboard?id=AloysHF_CoolbarEmu"><img src="https://sonarcloud.io/api/project_badges/measure?project=AloysHF_CoolbarEmu&metric=alert_status" alt="Quality Gate Status"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-BSD%203--Clause-blue.svg" alt="License: BSD 3-Clause"></a>
  <a href="https://discord.gg/7XDdSrYD"><img src="https://img.shields.io/badge/Discord-Join%20Us-5865F2?logo=discord&logoColor=white" alt="Discord"></a>
  <a href="https://qm.qq.com/q/LAO7DKAWUC"><img src="https://img.shields.io/badge/QQ%E7%BE%A4-Join%20Us-12B7F5?logo=tencent-qq&logoColor=white" alt="QQ Group"></a>
</p>

CoolbarEmu is a Rust emulator for Coolbar/CBE applications on feature phones.
It loads the executable and packaged resources directly, runs
guest code through a pure-Rust ARM core, and bridges the phone services needed
by supported games.

CoolbarEmu uses the `coolbaremu` command, crates and libretro core identifiers.
Save states use the `CBARSTAT` signature. Previous project names, save-state
signatures and configuration keys are not supported.

## Features

- **CBE format support** — section parsing, installed-package extraction,
  resource lookup, image decoding
- **ARM/Thumb CPU emulation** — little- and big-endian execution, interworking branches
- **Service bridge** — firmware-style API for memory, resources, display,
  input, text, little-endian game-data reads, DF panel invalidation,
  fixed-point game math, and
  packed-rectangle collision detection
- **Guest filesystem** — sandboxed in-memory files used by CBE installers and
  file-backed resource packages
- **Offline startup** — HTTP errors reach loading screens so supported embedded
  applications can enter their local menus; online content and paid gameplay
  remain subject to the limitations in [Game Compatibility](docs/Game-Compatibility.md)
- **Login fields** — basic numeric account/password controls with focus,
  password masking and save-state preservation; online authentication is not implemented
- **Graphics rendering** — RGB565 framebuffer with GIF and PNG image reconstruction with format-specific RGB565 palette byte order
- **Text rendering** — GBK and guest-endian UCS-2 strings, clipped and wrapped
  text, and offscreen drawing with an embedded Unicode bitmap font
- **Screen composition** — resource initialization, stacked backgrounds and
  modal overlays, event propagation, and resuming screens without reinitialization
- **Application LCD dimensions** — startup screen clips, clears, and background layout resolve
  176×220, 240×320, 240×352, or 240×400 surfaces and their landscape variants;
  240×400 is the fallback. Screenshots and frontends share the active geometry;
  standalone supports integer `--scale` (1–8)
- **Automatic landscape rotation** — games packaged for the original phone's
  rotated landscape LCD are rotated using the active surface dimensions, with a
  landscape default window matching the frame, a `--orientation` override
  for manual control, and a `--orientation-profile` file for titles outside the
  built-in profile
- **Display scaling** — nearest, bilinear, bicubic, and xbrz filters with
  aspect-ratio-preserving centering (`--filter`)
- **Key remapping** — rebind any guest key to any host key (`--remap`)
- **Physical gamepad input** — first connected pad drives guest keys via gilrs,
  RetroPad-compatible mapping, keyboard and pad combine as a logical OR
  (`--no-gamepad` disables the pad)
- **Virtual gamepad overlay** — translucent gamepad-layout dock (numeric row,
  large D-pad, centered OK, face-key diamond) highlighting held keys
  (`--show-gamepad`)
- **Fullscreen and volume** — borderless fullscreen and 0–100 playback volume
  (`--fullscreen`, `--volume`)
- **Window backend selection** — on Linux/BSD the default prefers X11
  (XWayland) so the desktop supplies standard window buttons such as maximize
  and close; XWayland HiDPI sessions keep the window's on-screen size by
  compensating for the denser X11 coordinate space; `--window-backend` forces
  `x11` or `wayland`
- **Headless mode** — run N frames without a window for testing and batch
  processing (`--headless --frames`)
- **Screenshot capture** — automated PNG screenshot generation
- **Save states** — versioned, checksummed machine snapshots
  through the libretro API and the standalone `--save-state` / `--load-state`
  options; format version 3 rejects version-1 and version-2 states from older builds
- **Reset** — rebuilds the emulator runtime state from the loaded archive
  (standalone `R` key and libretro `retro_reset`)
- **Guest memory exposure** — libretro exposes the guest heap and screen
  framebuffer to frontend memory tools
- **Audio** — WAV/MP3 decoding and MIDI synthesis, stereo mixing, volume
  control, and 44.1 kHz output through the libretro sample callback and the
  standalone device sink; `--auto-bgm` plays the first packaged MIDI resource
  when a game never calls the audio manager on its own (file-based MP3 control
  is a planned follow-up)
- **Touch input** — the guest touchscreen responds to mouse clicks in the
  standalone frontend and to pointer devices (mouse or touchscreen) in
  RetroArch
- **Libretro integration** — playable libretro core with RGB888 video output,
  RetroPad input, content loading, save states, reset, and memory exposure
  (core options cover volume, touch input, auto BGM, screen orientation, and
  debug logging); landscape titles use the resolved LCD dimensions, matching
  the standalone frontend

## Usage

### Standalone Mode

Download the latest binary from the
[Releases](https://github.com/AloysHF/CoolbarEmu/releases) page and run:

```bash
coolbaremu path/to/game.CBE
```

See the [Standalone Emulator](docs/Standalone-Emulator.md) guide for
installation, keyboard and gamepad controls, headless mode, screenshots, and
all command-line options.

### RetroArch Mode

Install the core and load a game through RetroArch's **Load Content** menu.

See the [RetroArch Core](docs/RetroArch-Core.md) guide for installation,
supported platforms, RetroPad mapping, and features.

## Building

Requires [Rust](https://www.rust-lang.org/tools/install) (stable).

### Standalone Mode

```bash
cargo build -p coolbaremu --release
cargo run -p coolbaremu --release -- path/to/game.CBE
```

### Libretro Core (for RetroArch)

```bash
cargo build -p coolbaremu-libretro --release
```

The binary is produced at `target/release/coolbaremu.dll`
(`libcoolbaremu.so` on Linux, `libcoolbaremu.dylib` on macOS). Rename it to
`coolbaremu_libretro.<ext>` before placing it in RetroArch's `cores/`
directory.

For Android cross-compilation, see [Android Libretro Core](docs/Android-Libretro-Core.md).
For iOS, see [iOS Libretro Core](docs/iOS-Libretro-Core.md).

## Architecture

```
crates/
├── coolbaremu-core/         # Platform-independent emulator engine (library)
│   └── src/
│       ├── lib.rs            # Crate root and public re-exports
│       ├── cbe/              # CBE container parsing
│       │   ├── mod.rs        # Archive and resource-type definitions
│       │   ├── archive.rs    # Section/resource scanning and loading
│       │   ├── sce.rs        # Scene resource decoder
│       │   ├── map.rs        # Map resource decoder
│       │   ├── actor.rs      # Actor resource decoder
│       │   └── resource.rs   # Resource entry helpers
│       ├── machine/          # Guest machine (CoolbarMachine)
│       │   ├── mod.rs        # Executable parsing, boot, frame loop, input
│       │   ├── memory.rs     # Sparse guest memory regions
│       │   ├── packages.rs   # Guest resource package parsing
│       │   ├── virtual_fs.rs # Sandboxed guest filesystem
│       │   ├── cpu_bridge.rs # Execution loop and service dispatch
│       │   ├── drawing.rs    # Framebuffer drawing, blits, and text
│       │   └── services/     # Firmware service handlers by manager
│       ├── audio_engine.rs   # WAV/MP3/MIDI decoding and mixing
│       ├── image_decoder.rs  # CBE GIF and firmware PNG decoding
│       ├── save_state.rs     # Versioned, checksummed save-state codec
│       └── runtime.rs        # Scene-level HLE (crate-internal, experimental)
├── coolbaremu/              # Standalone binary (→ coolbaremu)
│   └── src/
│       ├── main.rs           # Window loop, CLI, input, audio output
│       └── standalone/       # Display scalers, gamepad overlay, key/gamepad mappers, window backend
├── coolbaremu-tools/        # Archive analysis and headless diagnostics
│   └── src/
│       ├── bin/
│       │   ├── cbe_boot.rs   # Headless boot tool
│       │   ├── cbe_analyze.rs # Archive analysis tool
│       │   └── cbe_disasm.rs # ARM/Thumb disassembly tool
└── coolbaremu-libretro/     # Libretro cdylib (→ coolbaremu_libretro.{dll,so,dylib})
    ├── coolbaremu_libretro.info   # RetroArch core metadata
    └── src/
        ├── lib.rs               # cdylib crate root
        └── libretro/
            ├── api.rs           # Exported libretro functions
            ├── callbacks.rs     # Callback management
            ├── constants.rs     # libretro constants
            ├── logger.rs        # Bridges the `log` crate to the frontend
            └── types.rs         # libretro type definitions
```

See [Architecture](docs/architecture.md) for implementation details.

## Controls

The games expect a phone keypad and a touchscreen. CoolbarEmu offers three
input modes, all supported by both the standalone app and the RetroArch
core:

| Input mode | Standalone | RetroArch core |
| --- | --- | --- |
| Touch | ✅ mouse clicks act as touch (touchscreen taps arrive as clicks) | ✅ touchscreen and mouse via the pointer device |
| Keyboard + mouse | ✅ full phone keypad | ✅ full phone keypad (enable Game Focus, Scroll Lock) |
| Gamepad | ✅ | ✅ identical RetroPad mapping |

What playing feels like:

- **Touch** — the natural fit: these titles are designed for touchscreens,
  so menus and in-game controls just work wherever you tap.
- **Keyboard + mouse** — mouse clicks are touch input on both frontends,
  and the keyboard presses the full phone keypad in both. In RetroArch,
  turn on **Game Focus** (Scroll Lock) first so keys reach the core instead
  of RetroArch's hotkeys.
- **Gamepad** — one RetroPad-compatible mapping shared by both frontends.
  Hold **Select** for a number layer: directions 1–4, face buttons 5–8,
  Start 9, shoulders 0.

Notes and limits:

- A few titles are not playable for unrelated reasons (for example the
  network-dependent ones). See [Game Compatibility](docs/Game-Compatibility.md).
- The standalone `--show-gamepad` overlay is a read-only debug view of the
  merged key state, not a playable virtual keyboard.

Detailed references by input mode:

- **Touch** —
  [Standalone: mouse as touch](docs/Standalone-Emulator.md#default-key-mappings) ·
  [RetroArch: Touch Input](docs/RetroArch-Core.md#touch-input)
- **Keyboard + mouse** —
  [Standalone: Default Key Mappings](docs/Standalone-Emulator.md#default-key-mappings) ·
  [RetroArch: Keyboard Input](docs/RetroArch-Core.md#keyboard-input)
- **Gamepad** —
  [Standalone: Physical Gamepads](docs/Standalone-Emulator.md#physical-gamepads) ·
  [RetroArch: RetroPad Button Mapping](docs/RetroArch-Core.md#retropad-button-mapping)
- **Debug overlay** —
  [Standalone: Virtual Gamepad Overlay](docs/Standalone-Emulator.md#virtual-gamepad-overlay)

## Game Compatibility

See [Game Compatibility](docs/Game-Compatibility.md) for tested applications,
screenshots, and known limitations.

## Testing

Run the unit tests:

```bash
cargo test --workspace --release
```

Game files are not included. Supply legally obtained CBE applications separately.

## Contributing

Contributions are welcome! Whether you're interested in fixing bugs, adding
features, improving documentation, or testing game compatibility, we'd love your
help. See [CONTRIBUTING.md](docs/CONTRIBUTING.md) for details.

## License

This project is licensed under the [BSD 3-Clause License](LICENSE).
