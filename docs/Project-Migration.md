# CoolbarEmu Project Migration

CoolbarEmu targets Coolbar/CBE applications across supported feature-phone
runtime variants. A handset brand or chip vendor does not define the complete
application corpus. Collection labels A, B and C distinguish archive variants;
they are not verified device model identifiers.

This rename deliberately breaks previous project identifiers. There are no
command aliases, configuration fallbacks or save-state converters.

## Current identifiers

| Area | Identifier |
| --- | --- |
| Project | `CoolbarEmu` |
| Repository | `AloysHF/CoolbarEmu` |
| Website | `https://aloyshf.github.io/CoolbarEmu/` |
| Desktop command and crate | `coolbaremu` |
| Core crate / Rust import | `coolbaremu-core` / `coolbaremu_core` |
| Tool crate | `coolbaremu-tools` |
| Libretro crate | `coolbaremu-libretro` |
| Installed libretro core | `coolbaremu_libretro.{dll,so,dylib}` |
| Android core | `coolbaremu_libretro_android.so` |
| Libretro metadata | `coolbaremu_libretro.info` |
| Core display / system ID | `Coolbar (CoolbarEmu)` / `coolbar` |
| Core options | `coolbaremu_volume`, `coolbaremu_touch_input`, `coolbaremu_debug_logging`, `coolbaremu_auto_bgm`, `coolbaremu_screen_orientation` |
| Orientation overrides | `coolbaremu_orientation.csv` |
| Save-state signature | `CBARSTAT` (eight bytes; codec version 3) |
| Optional local corpus environment variable | `COOLBAR_GAME_DIR` |
| Optional MP3 test input | `COOLBAR_MP3_PATH` |
| Browser language preference | `coolbar-site-lang` |
| SonarCloud project key | `AloysHF_CoolbarEmu` |

The headless diagnostic commands (`cbe_boot`, `cbe_analyze`, `cbe_disasm`) retain
their format-oriented names. Release and nightly archives use the `coolbaremu-`
prefix. Desktop preferences and per-game saves follow the paths documented in
the frontend guides; old core option keys and orientation filenames are ignored.
States with the previous signature are rejected, even if their codec version
matches. Start with new states and configure the renamed core afresh.

## External services

These operations require the corresponding service administrators. A repository
change cannot establish that they have happened.

1. **GitHub:** rename `AloysHF/NicaiEmu` to `AloysHF/CoolbarEmu`, update the
   description and homepage, and verify Pages at the new URL. Update local
   `origin` URLs, mirrors, webhooks and external links. Keep branch rules and
   required checks attached to the same repository. GitHub redirects repository
   URLs, but Pages URLs need separate attention. Do not create another repository
   under the former name, which would replace the repository redirect.
2. **SonarCloud:** update the existing project's key from `AloysHF_NicaiEmu`
   to `AloysHF_CoolbarEmu`, rename its display name, and verify its GitHub ALM
   binding after the repository rename. Preserve project history. Verify the
   existing CI token can analyze the new key and run analysis again. Do not
   disable the quality gate to mask an incomplete service migration.
   If a PR analysis has automatically created a separate project at the new key,
   remove that new project before assigning its key to the historical project.
   Keep the historical project until its migration and binding are verified.
3. **Libretro:** coordinate the GitLab buildbot mirror/project name, source URL,
   core build recipe and artifact name with its maintainers. Rename the entry in
   [libretro-core-info](https://github.com/libretro/libretro-core-info) to
   `coolbaremu_libretro.info`, synchronize this repository's metadata, and update
   the core list and documentation. Retire the old entry as part of the same
   rollout. The `database` field is intentionally empty: no matching upstream
   content database is assumed. Existing Core Downloader installations do not
   automatically become a differently named core; manually install the new core
   until the coordinated upstream rollout is complete.
4. **Local workspace:** close tools holding the checkout, rename its directory
   to `CoolbarEmu`, reopen it and update IDE shortcuts and external scripts.
   Download fresh build artifacts and remove obsolete installed cores/info files
   when switching installations. Game content and companion resources retain
   their actual names and layout.

Historical Git commits, tags, published release asset names and archived test
evidence retain their original identities. New builds and current documentation
use CoolbarEmu. The rename does not rewrite guest file formats, firmware ABI
symbols, content filenames or third-party project identities.

## Validation scope

The compatibility table records bounded execution and screenshots for 245 CBE
files, with separate interaction limitations. Rebuilding under the new identity
requires fresh captures and matching binary provenance before publishing new
validation claims. A `Ready` machine or completed capture is not a certification
of full gameplay or online functionality.
