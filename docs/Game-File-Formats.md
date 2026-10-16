# Game File Format — CBE (Cool Bar Engine)

Coolbar games use the **CBE** container format. This document describes the
file structure, section layout, and resource types.

## Overview

Each game is a single `.CBE` file containing:

1. Section markers (`FE FE FE FE FE FE FE FE`) separating resource groups
2. Resource sections with headers, offset tables, name tables, and data
3. Executable code (ARM/Thumb) and packaged resources (scenes, maps, actors, scripts, images, audio)

## File Structure

```
┌─────────────────────────────────────┐
│ Section 0: System Resources         │
│  ├─ Header                          │
│  ├─ Offset Table                    │
│  ├─ Name Table                      │
│  └─ Resource Data                   │
├─────────────────────────────────────┤
│ Section Marker: FE FE FE FE FE FE FE FE │
├─────────────────────────────────────┤
│ Section 1: Game Resources           │
│  ├─ Header                          │
│  ├─ Offset Table                    │
│  ├─ Name Table                      │
│  └─ Resource Data                   │
├─────────────────────────────────────┤
│ ...                                 │
└─────────────────────────────────────┘
```

## Section Header

Each section contains a header with metadata:

```
Offset   Size   Field
─────────────────────────────────────────────────
0x00     4      Resource count
0x04     4      Section flags
0x08     4      Name table offset
0x0C     4      Data offset
0x10     4      Data size
...
```

## Resource Types

| Extension | Type | Description |
|-----------|------|-------------|
| `.sce` | Scene | Game screen definition with actors and scripts |
| `.map` | Map | Tile-based background data |
| `.actor` | Actor | Interactive game object with behavior |
| `.xse` | Script | XSE (eXecutable Script Engine) bytecode |
| `.gif` | Image | CBE-encoded GIF image (RGB565) |
| `.wav` | Audio | WAV audio file |
| `.mid` | Audio | MIDI music file |

## Executable Format

CBE executables contain ARM/Thumb code designed for Coolbar phones:

- **Display**: 240×400 (WQVGA)
- **Pixel format**: RGB565 (16-bit, 5-6-5 bit layout)
- **CPU**: ARM with Thumb instruction set
- **Byte order**: Little-endian or big-endian (detected from header)

### Memory Map

```
Address Range              Size    Description
──────────────────────────────────────────────────
0x00000000 – 0x00FFFFFF   16 MB   RAM (code + data)
0x01000000 – 0x01FFFFFF   16 MB   Video RAM (VRAM)
```

## XSE Virtual Machine

XSE scripts run on a bytecode VM with:

- **Group Dispatcher**: Routes commands to handlers
- **Operand Stack**: Manages script parameters
- **Writeback Mechanism**: Updates game state
- **Provider Services**: Host-side services for resource loading

## Graphics Format

Custom GIF resources store palette entries as big-endian RGB565. Firmware PNG
resources use the `89 50 4E 47 47 41 4D 45` signature and little-endian RGB565
palette entries; their PNG chunk lengths remain big-endian. A firmware `PLTE`
chunk declares the RGB888 length but stores two bytes per palette entry.
Palette colors and transparency are decoded separately: opaque drawing copies
the original color, while alpha drawing skips pixels marked transparent.

- **Pixel format**: RGB565 (16-bit, 5-6-5 bit layout)
- **Resolution**: Active LCD dimensions; supported portrait modes are 176×220,
  240×320, 240×352 and 240×400, with matching landscape variants. Unknown
  modes fall back to 240×400 (WQVGA).
- **Framebuffer**: Located in guest RAM, converted to RGB888 for display

The renderer reads RGB565 pixels from the framebuffer address in emulated memory and
converts them to RGB888 (24-bit) for output:

```
RGB565 pixel → R = (pixel >> 11) & 0x1F  → scale to 8-bit
                G = (pixel >> 5)  & 0x3F  → scale to 8-bit
                B = pixel & 0x1F          → scale to 8-bit
```

## System Services

The emulator implements firmware-style services for:

- Memory allocation and management
- Resource lookup by identifier and name
- Stream reads and compressed data
- RGB565 screen and image drawing
- GBK text measurement and rendering
- Phone-key input queries
- Screen transitions and resource notifications
- Formatted string output
