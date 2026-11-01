# EmulatorJS Core (Browser)

CoolbarEmu can run in the browser as a standard
[EmulatorJS](https://emulatorjs.org) core. The build follows EmulatorJS's own
pipeline: the Rust libretro core is compiled for `wasm32-unknown-emscripten`,
linked against the `EmulatorJS/RetroArch` fork, and packaged as the
`<core>-wasm.data` bundles that an EmulatorJS page loads at runtime.

The project site ships a ready-to-use player at
<https://AloysHF.github.io/CoolbarEmu/play.html>: visitors pick a `.CBE`
application from their own machine and it runs from a blob URL, so nothing is
uploaded and the site does not host any game data.

## What you get

Every nightly publishes a single asset:

| File | Contents |
|------|----------|
| `coolbaremu-emulatorjs.zip` | Both core variants, the build report `cores/reports/coolbaremu.json`, `core.json`, `build.json`, `license.txt` and a README |

The two `.data` files stay separate inside the ZIP: EmulatorJS requests
`<core>-wasm.data` or `<core>-legacy-wasm.data` by name depending on WebGL2
support, so one bundle cannot serve both cases.

The bundle is laid out like this:

```
coolbaremu-emulatorjs.zip
├── cores/
│   ├── coolbaremu-wasm.data
│   ├── coolbaremu-legacy-wasm.data
│   └── reports/coolbaremu.json
├── core.json
├── build.json
├── license.txt
└── README.md
```

Each `.data` file is a 7z archive containing `<core>_libretro.js`,
`<core>_libretro.wasm`, `core.json`, `build.json` and `license.txt`.

## Install into a site

EmulatorJS always fetches cores from `<EJS_pathtodata>cores/`, so host the
runtime yourself (or use the pinned CDN files) and drop the core beside it:

```
data/
├── loader.js
├── emulator.min.js
├── emulator.min.css
├── compression/
│   ├── extract7z.js       # extracts the core .data archive
│   └── extractzip.js      # extracts ZIP content
└── cores/
    ├── coolbaremu-wasm.data
    ├── coolbaremu-legacy-wasm.data
    └── reports/coolbaremu.json
```

`emulator.min.zip` only contains `emulator.min.js` and `emulator.min.css`;
the `compression/` files are loaded on demand from the same EmulatorJS data
directory. Without `extract7z.js` the core download ends in a "Network Error".

To upgrade the runtime, replace the runtime files with a newer EmulatorJS
release (≥ 4.2.3) and keep this `cores/` directory.

## Use

```html
<div id="game"></div>
<script>
  EJS_player = "#game";
  EJS_core = "coolbaremu";
  EJS_gameUrl = URL.createObjectURL(gameFile);
  EJS_gameName = gameFile.name;
  EJS_pathtodata = "data/";
  EJS_startOnLoaded = false;
</script>
<script src="data/loader.js"></script>
```

`site/play.html` in this repository is a complete example that loads a single
`.CBE` file.

When the game comes from a blob URL, EmulatorJS writes it into the wasm
filesystem under `EJS_gameName`. Set that to the original file name, extension
included (for example `game.CBE`), so the frontend keeps a recognizable
content path for the core.

### Content and companion files

The built-in player loads a bare `.CBE` application. Some applications also
open files from their own directory, such as save records, payment plugins, or
update data. Because EmulatorJS hands the core one content file, a custom page
can ship those files as a separate ZIP in `EJS_biosUrl`:

```html
EJS_biosUrl = URL.createObjectURL(companionZip);
```

EmulatorJS extracts the archive next to the content file in the wasm
filesystem, and the core indexes that directory read-only as the guest
device filesystem. Keep the companion files at the archive root. Guest writes
stay in memory and are lost when the page is reloaded.

The optional orientation profile is a core support file rather than a
companion resource, so it keeps the fixed layout below the wasm filesystem
root: put it at `coolbaremu/coolbaremu_orientation.csv` inside the same
`EJS_biosUrl` archive. The directory and file names are matched
case-sensitively. Each line is `crc32,length,orientation`:

```
a1b2c3d4,341737,landscape
```

## Rendering

The core renders its framebuffer through libretro's software path as XRGB8888
and reports the application's native LCD size at startup: 176×220, 240×320,
240×352, or 240×400, with landscape titles presented as the transposed
variant. The play stage scales the output to fit the page; pixel-art
applications look best with nearest-neighbor scaling.

## Controls

EmulatorJS maps its RetroPad to the core's shared phone-keypad layout:

| Input | Guest key |
|-------|-----------|
| Arrow keys / WASD | D-pad |
| Enter / F | Confirm |
| Q / E | Left / right soft keys |
| N / M | `*` / `#` |
| Number keys | Digits 0–9 |
| Select + directions/face/Start | Number layer |

Touch controls and physical gamepads are provided by EmulatorJS and arrive
through the libretro pointer and RetroPad devices.

## Saves

Applications do not have battery-backed save RAM in this core, so `core.json`
sets `"save": false` to hide EmulatorJS's save-file buttons. Libretro save
states are implemented by the core and handled by EmulatorJS; they stay in the
browser's local storage.

## Building the bundle

The nightly workflow runs this automatically. To build locally you need an
activated [emsdk](https://emscripten.org/docs/getting_started/downloads.html)
(`emmake` in `PATH`), `p7zip`, `zip`, and the pinned nightly toolchain:

```bash
# Rust's prebuilt emscripten stdlib carries wasm exception handling, which
# Binaryen's Asyncify pass rejects. Rebuild std with panic=abort instead.
cargo +nightly build -p coolbaremu-libretro \
  --target wasm32-unknown-emscripten --profile wasm-release \
  -Zbuild-std=std,panic_abort

scripts/build-emulatorjs-core.sh
```

The script clones the pinned `EmulatorJS/RetroArch` revision, links the
staticlib with `Makefile.emulatorjs`, builds the WebGL2 and legacy variants, and
writes `out/coolbaremu-emulatorjs.zip`. `--work-dir` moves the RetroArch
checkout out of `target/`, which matters on CI because `rust-cache` restores
`target/` without the checkout's `.git`.

The Pages deployment fetches the pinned EmulatorJS runtime, downloads the
`coolbaremu-emulatorjs.zip` asset from the rolling `nightly-build` release, and
redeploys after every successful Nightly Build.
