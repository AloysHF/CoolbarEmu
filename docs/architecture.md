# Architecture

Offline HTTP GET uses `(url, callback, handle_out)`: the third argument receives
a unique request handle. The deferred callback receives `(0, 0, 0, 9)` rather than
a fabricated application data packet. GET errors reach active screens as well as
screenless callers; raw channel responses retain their existing mock behavior.

Native dispatch 0xaf consumes a pointer to one allocation-size word and returns
a scalar request handle. Fetch 0x7d1 writes the allocated pointer to the supplied
output slot. It must not apply the generic object frame+8 writeback to this
one-word argument: that address can hold a guest return address. Scalar allocation
and file results share a transient request-result cache.

GameOld Random returns signed values in the inclusive requested range using the
existing deterministic generator. Fixed-manager callers use the advancing guest
clock. OldLib_064 rectangle text reads dimensions/color from the stack, wraps GBK
glyphs and returns packed line count/text width without overwriting source text.

Native dispatch file operations marshal a three-word argument record. Requests
0x41a (open), 0x41b (write), 0x427 (read) and 0x42a (size) return a request identifier;
the subsequent 0x7d1 request retrieves the most recent scalar for that identifier.
Close (0x41c) returns its status directly. Paths use the guest filesystem's
byte-order-aware decoder, while mode strings remain byte strings. Handle zero is
valid, EOF returns the number of bytes actually read, and invalid operations
return 0xffffffff. The files remain in memory and are not persisted to the host.

Resource-package lookup recognizes the exact extended file-package marker
(`1` at offset 84). Compact in-memory descriptors can place unrelated pointer
storage at that offset; treating every nonzero byte as a file marker incorrectly
routes resident resources through file I/O and prevents dynamic code loading.

Native global-block requests (0x8e), including their marshalled fetch form,
return the shared memory-block descriptor and its allocation/reset/release
methods. Fetching the v3 GameManager also switches image headers to 32-bit
dimensions. Its compact resource-package descriptor occupies 76 bytes; extended
file fields and methods must not overwrite the following guest object.

Old-lib blits and digit drawing read arguments beyond r3 from the guest stack.
Window updates call registered guest logic, and repaint callbacks share the dirty
rectangle with old-lib drawing. Left/top clipping advances source coordinates;
picture-library fills also honor the dirty rectangle.

The v3 billing entries retain the address differences used by guest wrappers:
remaining-days is 3764 bytes after pay-count, and the SMS entry is another 1376
bytes later. These entries use existing offline billing semantics and defer the
local SMS result callback from the sixth argument. No external SMS is sent.

The legacy GameManagerOld text-box constructor (index 71) uses the same bounded
initializer as the direct text-box method path. Its methods end at offset
0x34; applying the generic 0x100-byte constructor filler corrupts neighboring
screen state even when the constructor itself returns without an error.

Legacy picture-library methods maintain a bounded image-index table and resource
ID cache, decode images through the shared stream decoder, and draw through the
RGB565 blitter. The constructor stores its scanline at offset 0, target at 4,
capacity at 8, ID/image arrays at 12/16, count at 20, and release marker at 22.
Release frees the owned allocations and clears these fields; unknown image
indices and exhausted capacities do not allocate fallback objects.

NicaiEmu executes native CBE applications instead of replacing their game logic with a scene preview. The core is platform-independent and exposes a framebuffer plus phone-key input to frontends.

## Boot flow

1. `CbeArchive` scans flat, grouped, and nested resource-package sections and records resource names and ranges.
2. `CbeExecutable` validates the executable header, segment bounds, checksums, and guest byte order.
3. `NicaiMachine` maps code, initialized data, stack, heap, manager tables, and service trampolines into the guest address space.
4. The ARM/Thumb interpreter runs the application initializer and entry point, including interworking branches and compiler-generated PC-relative jump tables.
5. Applications that contain an installer can extract their named resource packages into the sandboxed guest filesystem before launching the installed entry point.
   The network application manager retains entry descriptors and dispatches
   startup callbacks independently of the manager initialization directory.
6. Each 100ms guest tick dispatches logic from the top screen downward until an
   event is consumed, then renders stacked screens from bottom to top.


Pending resource callbacks bound to a new screen run before its initialization,
so initialization can use the objects created by resource loading. Requests
issued during initialization still run before logic and rendering. When there
is no explicit request, a screen's resource callback runs once after its
initialization. Screens registered beneath an overlay also initialize and load
their resources. Closing an overlay resumes the underlying screen without
repeating its initialization or resource loading.

## Guest memory

The machine uses checked sparse regions rather than reserving the entire 32-bit address space. Guest reads and writes honor the executable's byte order. Initialized data, stack, heap, service-manager state, and framebuffer storage are writable. Unmapped accesses are recorded for diagnostics, and an unmapped instruction fetch stops execution with an error.

Memory-block allocations clear their aligned storage, including storage reused
after a block reset. Resource lookup preserves mapped resident payload pointers
even when bytes beyond a compact package descriptor resemble a file marker.

## Guest termination

The ARM C runtimes linked into CBE applications terminate through the Angel semihosting interface: ARM `SWI #0x123456` or Thumb `SVC #0xAB` with operation `0x18` (ReportException). The machine treats such an exit as a normal halt: the frame loop stays idle and error-free, and frontends keep presenting the last framebuffer. Semihosting `WRITEC`/`WRITE0` console output is captured under `CBE_TRACE` and is useful for reading guest-side diagnostics such as C runtime arithmetic-exception messages.

## Service bridge

Native applications receive guest-callable tables whose entries lead to emulator trampolines. Implemented service families include:

- heap and memory-block allocation;
- guest file access and directory queries backed by an in-memory sandbox;
- resource lookup by identifier and name, including file-backed data packages;
- byte- and word-length-prefixed stream reads across both DreamFactory manager table layouts;
- RGB565 screen and image drawing;
- GBK text measurement and rendering;
- phone-key edge and held-state queries, with held keys remaining visible on
  every guest tick until the frontend reports their release;
- screen transitions and resource notifications;
- a bounded subset of C-style formatted strings.

Unsupported service entries currently return a neutral value. Service usage counters and opt-in tracing make missing behavior observable during compatibility work.

The shared I/O table and all I/O initializer entry points use the same method
addresses. The first 24 methods have a 0x100-byte spacing because guest startup
code identifies the NV implementation by subtracting adjacent function pointers
and derives its read/write callbacks from method 17. Private tables retain this
layout while each initializer writes only its declared number of slots.

## Rendering

Fixed GameManager image services route to GameLCD: indices 1-3 draw images,
14 sets the clip, 24 returns it, and 32/33 draw opaque/transparent image targets.
Both tables use the same source offsets and clip state.

Native dispatch sid 82 copies the v3 gameold table into the caller's buffer:
the drawing slots (0x00..0x3c) resolve to a dedicated service table (clipped
image blits, full-screen draws, digits, the UI-skin helpers, text, clip
control), the remaining slots to the shared service entries with the v3
8-byte gap folded in, and slots the guest already filled are left alone.
Building that library also switches image headers to the wide layout the
native guests read: u32 width/height at +4/+8 with 16-byte headers, the
screen header included. Every image-header access goes through the same
pair of helpers, so narrow and wide titles share one code path.

Unknown object method slots are assigned the inert object-method kind, never
the global GameManager kind. Object arguments must not trigger global
constructor heuristics that write through caller stack slots. These inert
methods do not establish support for the object's unimplemented behavior.

Fixed-address picture-library services share the legacy picture implementation
for load/cache, image sizes, drawing targets and owned-resource release. The
constructor initializes the default target and ownership marker explicitly.

Fixed legacy window repaint clips each dirty rectangle, invokes its guest
paint callback synchronously with the configured context, and visits child
and sibling windows. CPU registers are restored after each nested painter;
dirty queues are cleared after drawing. Guest callback errors propagate.

Legacy text boxes retain GBK byte offsets and byte lengths in owned line
tables, expose page counts in the guest object, and render the selected page
with horizontal/vertical alignment and RGB888 colors. Constructors receive
the final geometry arguments on the stack. Drawing respects the configured
line height; releasing a box frees its line tables without freeing guest text.

The guest owns an RGB565 LCD surface managed by the core's `DisplaySurface`.
Startup full-surface clips, rectangle fills, and completed scanline clears
select supported 176×220, 240×320, 240×352, or 240×400 LCD modes, including
landscape variants. Explicit startup clips take precedence over inferred modes.
When those requests are absent, the first three callbacks accumulate background
and tile drawing extents. Glyph cells and narrow vertical strips are excluded;
a supported mode must match the right edge, with at most one 16-pixel text row
left below the artwork. A larger startup backdrop supersedes a smaller splash.
Unresolved geometry falls back to 240×400, and later drawing cannot resize it.
Packed LCD points and rectangle corners follow the guest's byte order across
image placement, clipped blits, fills, outlines, lines, and touch hit tests.

The active width, height, and aligned row stride feed the guest screen header,
image blits, rectangle clipping, panel invalidation, text rendering, screenshot
export, and frontend geometry. The backing allocation retains capacity for a
240×400 surface, and a mode change preserves existing pixels while repacking
rows. Save states retain the active dimensions in the existing guest screen
header, with the hint source retained in its reserved bytes; loading restores
host geometry without adding serialized geometry fields. The machine snapshot
uses format version 2 because this compatibility update adds other serialized
machine fields; version-1 states are rejected before decoding. Presentation
rotation is separate from LCD dimensions:
native landscape surfaces are displayed directly in automatic mode, while
profiled games with pre-rotated artwork use an additional rotation. Explicit
orientation overrides remain available. Pointer coordinates and text use the
same size-aware inverse rotation. The libretro frontend reports geometry after
guest drawing so video dimensions and the submitted pixel buffer agree within
the same frame. The desktop frontend adjusts an automatically sized window
when startup resolves the LCD dimensions, preserving manually resized windows
and fullscreen presentation.

Image resources are reconstructed from the CBE GIF variant or firmware PNG
representation, decoded, and copied into guest image objects. Custom GIF
palettes use big-endian RGB565; firmware PNG palettes use little-endian RGB565
while PNG chunk lengths remain big-endian. Decoded RGB565 colors are preserved
even for transparent pixels. A separate transparency mask is stored after the
pixel data in the same heap allocation, with a host tag; opaque blits copy
original colors and alpha blits consult the mask. Guest-created images without
this metadata retain zero-color transparency. The mask is part of guest-memory
save states, so no serialized field layout change is required.

Some custom GIF headers resemble ICO files, so a failed standard-image decode
falls back to the CBE decoder. The desktop frontend converts the completed
screen to 32-bit RGB for `minifb`. Text is decoded as GBK or guest-endian UCS-2 and rasterized from an
embedded Unicode font, so the core does not depend on host fonts. Text
coordinates live in the presented display space: for rotated games the glyphs
are mapped from the presented display back into the active LCD surface pixel
by pixel, and games that submit `DrawText` with zero coordinates inherit the
pen origin latched from the preceding `GetScreenImage` call.
String APIs support explicit offscreen images and the raw LCD buffer alias.
Clipped strings honor their pixel width; rectangle strings wrap and return their
occupied line count and width. Scalar and packed LCD line APIs include both
endpoints and clip pixels to the active surface. Missing firmware text returns
a valid empty string; built-in phone UI strings are not supplied.

## Headless execution

Screen logic and rendering callbacks are optional while firmware dialogs
suspend the underlying screen. Deferred callbacks continue to run and can
restore the screen; frames without rendering preserve existing pixels.

Firmware list controls own a screen, guest-memory item records and selection
state. Their methods preserve guest callback fields, apply image/color skins,
and route directional and soft-key input through deferred guest callbacks.
Nested controls use the same screen stack as application screens; releasing a
control removes its screen and frees its owned memory.

Native game-manager tables bind pointer held/down/up/drag queries, pointer
coordinates and current key state to the shared input state. Sized rectangle
hit tests use their supplied coordinates and exclude the far edge, so idle
frames cannot accidentally activate touch-driven menus.

The fixed game manager routes its string and font queries to the GameLCD renderer.
Font queries return pixel metrics rather than echoing leftover stub addresses.
Legacy GameLCD uses separate register/stack arguments for tile blits and
bounded GBK strings (text, byte length, x, y, RGB888). Its clip rectangle lives
in reserved screen-descriptor bytes and is therefore preserved by memory
snapshots. Clip adjustment advances source coordinates as well as destination
coordinates; transparent blits preserve destination pixels for zero RGB565.

`cbe_boot` loads the same archive and machine used by the desktop frontend. It can schedule phone-key presses at exact frames, run a fixed number of callbacks, print machine diagnostics, and save a screenshot. This is the preferred path for deterministic runtime regression checks.

The desktop screenshot mode preserves the last framebuffer produced before a guest callback error. It only writes a PNG when guest execution has produced a framebuffer with more than one color; blank frames and startup failures are reported without creating a screenshot.

### Native DF interfaces

Native DF constructors are bound in both registered and queried function tables.
Registered screens take over the frame lifecycle after resource loading, and
native idle logic can poll a press edge within the same frame. Record files use
bounded sections with little-endian headers and values. Window callbacks receive
stack-passed contexts and paint queued dirty rectangles. Animation resources load
image references and cumulative frame timing; mirrored parts and collision
methods remain unsupported.

Native registration stores startup and exit callbacks separately; boot invokes
only startup. Registered screen logic polls key and pointer state once per
frame. The fixed manager's record constructor initializes only the bounded
record object, preserving neighboring resource packages. Its UI-skin helper
reads height and slice size from the AAPCS stack. Native allocation requests
(sid 0xb9) write a cleared heap pointer through the request's output address and
return success in a single status byte.

Native dirty-rectangle requests (0x6b/0x6d/0x6e) initialize an eight-byte pool,
append signed rectangle coordinates through a bounded pointer array and bind an
eight-byte listener. Pool storage comes from the supplied guest memory block.
The guest consumes these rectangles to restore background regions on its next
repaint. The listener callback uses a separate service address outside existing
object-table banks. Fetched GameOld drawing slots use the dedicated old-lib
service bank, preserving the other table bindings.

Native text requests 0x3f8 (bounded GBK width), 0x418 (UCS2 length) and 0x453
(bounded UCS2 width) cache scalar results for the following 0x7d1 fetch.
Request fields remain unchanged; two-byte fetches preserve the adjacent word.
Width uses the standard eight-pixel ASCII / sixteen-pixel full-width metrics.
Unknown object requests retain separate method-table behavior. These scalar
bindings prevent pointer values from reaching text-layout arithmetic; they do
not bypass guest assertions or implement the missing startup interfaces.

Embedded-program loaders use the shared virtual-file seek interface and receive
the resource-section offset with the application path. Resource packages with a
separate file context resolve offsets through bounded file reads rather than
interpreting them as guest pointers. The native old-library table exposes LCD
pixel-buffer access and screen changes through their shared implementations.
Diagnostic format-string callbacks preserve the caller's stack and do not create
objects. Fixed-manager allocation slots use the shared owned heap allocator.

Image-page initialization reads compact or standard resource packages from the
specified guest-memory stream. Image IDs resolve within that selected package
rather than the main archive, preserving each page's own background and skin
assets. Explicit package image creation uses the same resource lookup path.

Native scene initialization loads the first SCE2 background map from its resource
name. Map dimensions, image slots, column-major cells and signed placement are
preserved in guest memory for the application's own rendering callbacks. Tile
sizes come from map metadata; malformed or truncated maps are rejected. Additional
scene layers, script callbacks and complete gameplay remain unverified.
