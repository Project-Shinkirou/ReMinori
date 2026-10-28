# ReMinori
Rust-compatible implementation of the Minori engine

## ShinkiEMU fork

The `shinki` branch adapts upstream commit `79b00990324404bfbd496e7fa76c690cf040f744`
to ShinkiEMU's native C ABI v3. Adapter sources are in `crates/shinki`; the plugin
ID is exactly `ReMinori`, and the original engine family is `musica`.
The upstream VM, script IR, renderer, CPAL mixer, Media Foundation video decoder,
menus and save implementation remain the gameplay implementation.

### Build and integration

On Windows x64, use Rust 1.92 or newer, the MSVC build tools and Windows
SDK. A hardware-capable wgpu backend and a working native audio output device are
required for normal sessions. No second game window or Present loop is created.

```sh
cargo build --locked --release --target x86_64-pc-windows-msvc -p shinki-emu-ReMinori
```

The output is `target/x86_64-pc-windows-msvc/release/shinki_emu_ReMinori.dll`. Load
`shinki_emu_query(3, ...)`, initialize diagnostics, then use the returned `CoreApi`.
The worker owns and paces the original Engine at 60 Hz; host input/state/frame
polls never advance the VM. The complete native GPU composition is read back as
immutable, top-down BGRA8 CPU leases at the original 1024x640 raster. The Manager
owns presentation. There is no render-extent capability or public headless CLI.
Frame leases retain only pixel storage and can be released after session close.
The CPU side uses three fixed preallocated frame slots. A slot is writable only
when the worker is its sole owner. If the host retains every slot, gameplay and
native GPU rendering continue without CPU readback or a new display generation;
leased pixels are never overwritten. GPU targets and map-completion channels are
also reused.
Cancellation invalidates pending host platform requests; close joins the Engine,
audio loader and movie decoder before returning. The dynamic library stays
resident across closes and reopens.

`build.rs` supports Windows x64 MSVC and writes target/profile plus the root
AGPL-3.0 `LICENSE` to `OUT_DIR/core-build.json`. No separate core Resources are
required: shader sources are compiled into the library; scripts, archives and
optional `fonts/default.ttf` / `fonts/default.otf` overrides belong to the user's
game tree. Without an override, the engine reads installed Windows Japanese
fonts through GDI or the Windows Fonts directory. These fonts are not redistributed.
Video requires Windows Media Foundation (including the Media Feature Pack on
editions without it), audio uses CPAL/WASAPI, and wgpu uses the Windows graphics
stack and installed driver. The release DLL imports `VCRUNTIME140.dll` and the
Windows Universal CRT; install the matching Microsoft Visual C++ redistributable
where needed. No Python, FFmpeg or separately shipped codec runtime is required.

Binary publication still requires an exact corresponding-source archive,
including Cargo.lock and build instructions, and the license/copyright notices
for the resolved Rust dependency closure. The root `LICENSE` covers ReMinori,
not all dependencies. No commercial game assets, private key profiles or
Windows fonts belong in that source or binary package.

The strictly typed configuration fields are:

| Field | Type | Meaning |
| --- | --- | --- |
| `profile_path` | string/path | Empty uses upstream retail CD-ROM parameters. An explicit relative path is resolved against the game directory; `key.toml` reads a private `shinki.emu.musica.keys.v2` profile in place. |
| `diagnostic_null_audio` | boolean | Defaults to false. True explicitly selects the upstream silent backend for diagnostics; it is not normal audio acceptance. |

The profile reader implements PAZ versions 0/1/2, per-role Blowfish keys, header
XOR, type passwords, version-2 RC4 CRC skipping, movie transforms and zlib entries.
It uses ReMinori's archive and resource APIs; it does not transfer execution to
the owned Musica core. Private profile parser failures do not quote key material.
Profile texture replacements remain confined to the game directory.
Native scripts use strict per-file CP932/GBK selection: CP932 stays primary on
ties, GBK is chosen only when it has fewer invalid source lines, and remaining
invalid bytes fail decoding rather than becoming replacement characters. This
matches the translated-edition source policy without adding secrets or encoding
fields to keys.v2. Games that load GBK scripts use ReMinori's existing UTF-8 port
save container so dialogue and comments survive native saves and reopens; pure
retail CP932 saves retain their original format.
Standalone ReMinori uses `userdata/` beside its executable. The embedded adapter
runs inside the ShinkiEMU Manager process, so it passes the game directory in
place of the executable directory and uses `<game>/userdata/`. If that
directory cannot be created or written, the upstream fallback
`Documents/eden/userdata/` applies unchanged. ReMinori stores `edenNNNN.sav`
and `edenNNNN.png` thumbnails, `edenPMNNNN.sav` quick saves,
encrypted/compressed `system.dat`, `settings.tsv`, `readflags.bin` and
`cgseen.tsv` there. Decoded movies use upstream's cache,
`<OS cache dir>/eden/movies/`; movies decoded with a private profile go into a
per-profile subdirectory of it. The save root is per Engine thread instead of
upstream's process-wide `OnceLock`, because one resident library opens
sessions for different games.

The original Windows retail eden executable uses `<game>/save/`, not
`userdata/`. To use original saves in ReMinori, copy the contents of `save/`
to the active `userdata/` directory. To move compatible retail-format saves
back, copy them from `userdata/` to `save/`. Back up both directories and do
not overwrite existing files. ReMinori's UTF-8 port saves for GBK scripts are
not retail-format saves and should not be assumed compatible with original eden.
ShinkiEMU beta.1/beta.2 required a user-chosen `save_directory`; copy the files
from that directory into the active `userdata/` directory by hand, without
overwriting existing files.
Ambient screenshot-dump environment variables are ignored by offscreen sessions.

Original in-game menus are rendered into the frame. Native context menus and
save-comment editing use the host's typed modal service. A missing or failed
modal service produces an explicit session failure, not an invented user
cancellation; only a real successful Cancel button produces a cancel response.
F5/F9 do not require a modal and retain the original quick-save/load path.
Best-effort window restoration, screen-mode failure, pointer capture and external
help/URL requests retain warning-and-continue semantics with operation, status and
error diagnostics. The host owns pointer capture and output-window geometry.

Native audio prefers the existing stereo configurations on the same default
device. A multichannel-only device uses its actual default format: the original
stereo mixer feeds front left/right and clears the remaining channels. Callback
scratch is fixed-size and reused, with no per-callback vector allocation.
Enumeration, format, stream creation and stream start errors retain their native
cause; the adapter never silently replaces failed audio with NullAudio.

### Acceptance procedure

The intended Steam sample is an owned `eden` installation with its existing
private `keys.v2` `key.toml`. Pass `profile_path=key.toml` and leave
`diagnostic_null_audio=false`. Never copy private keys or commercial assets
into this repository. Other games, including ef first/latter, are not added
to the supported-games list by this adapter.

Exercise the C ABI with real input: Enter starts a new game and advances dialogue,
ControlLeft held enables the original fast-forward, F5 saves `eden0001.sav`, and
F9 loads that file. A title NewGame click is `(937,114)`; title LoadGame is
`(937,142)`. All coordinates are raster pixels. The bottom-right corner
`(1000,620)` opens the native drawn menu; after its animation, QuickSave is
`(970,500)`, Load `(934,600)` and Save `(890,575)`. Save/load slots also accept
arrow keys and Enter. Windows double-click timing is preserved.

Check title rendering, continuous story advancement, native audio and movies,
save-file creation, restored VM position/frame, close/reopen with F9, cancellation
and a frame lease held across close. Structured event `reminori.native_state`
contains numeric `phase`, `input_count`, `pc`, `wait_kind`, `wait_frames` and
`wait_voice`, without script names or dialogue. Phase codes are 1=Title, 2=Game,
3=SaveLoad, 4=Memories, 5=CgGallery, 6=MusicMenu, 7=Config. Wait codes are 0=no VM,
1=Running, 2=WaitFrames, 3=AwaitInput, 4=AwaitAutoMessage, 5=WaitShake,
6=WaitScroll, 7=Ended. A changing frame counter alone is not evidence of playability.

Boundary regressions are available with:

```sh
cargo test --locked -p shinki-emu-ReMinori
cargo test --locked -p eden-formats
cargo test --locked -p eden-engine
```

### Attribution and modifications

Upstream ReMinori is by luoyily and its contributors. The ShinkiEMU adaptation,
profile-driven PAZ support, offscreen GPU capture and lifecycle/storage changes
are by the ShinkiEMU contributors (2026), under the same GNU Affero General Public
License version 3 as this repository; see `LICENSE`. Existing notices are
retained. The ABI crate and host-side build helper are pinned to ShinkiEMU commit
`cb8d9dd22d7d0ab129085b324c94d6dc81fa19e3` and retain their own licenses.
The Release profile uses unwinding so native panics are contained before reaching
the C boundary. No game assets, private keys or decrypted commercial data are
part of the fork.


## Features

- Runs games built on the Minori Engine directly from their original game archives — encrypted `.paz` containers are opened in place, no pre-extraction needed
- One codebase, two backends: native desktop (winit + wgpu) and WebAssembly (wasm32) for in-browser play
- Drop-in file overrides — same-name replacement files (scripts, images, sprite-sheet animations, ...) are layered on top of the original archives, enabling fan translations and HD/upscaled asset packs
- Script IR layer — game scripts are decoded into a typed intermediate representation with round-trip tooling, enabling analysis and text replacement
- Save compatibility with the original engine — original `.sav` files are parsed, and a headless VM trace tool verifies interpreter state field-by-field against them
- Resource tooling shipped in-tree: `eden-paz` (list / verify / extract), `eden-ani` (export / repack), `eden-script` (stats / round-trip), `eden-vm-trace` (headless run + save diff)

## Tested Platforms

| Platform | Native | WASM |
| --- | --- | --- |
| Windows | ✅ | ✅ |
| macOS | — | ❓ |
| Linux (Ubuntu) | — | ❓ |
| Android | — | ❓ |

✅ = tested · ❓ = theoretically supported through the same WASM backend, but untested · — = not implemented / not tested

## Tested Games

- 『eden*』 (*eden* — minori)

Testing was done against the retail CD-ROM edition of eden*. Other editions, and other games built on the Minori Engine, ship with different PAZ keys, XOR keys, etc. and will not work out of the box — adapting them is up to you.

## Getting Started

Prerequisites: a stable [Rust](https://rustup.rs) toolchain. The WASM target additionally needs `rustup target add wasm32-unknown-unknown` and `wasm-bindgen-cli` pinned to `0.2.127` (the version the glue ABI is generated against).

### Native (Windows)

```sh
cargo build --release
```

This builds the `eden` game binary as well as the resource tools (`eden-paz`, `eden-ani`, `eden-script`, `eden-vm-trace`).

Run the game either from the game's install directory (the folder containing the original `.paz` archives is auto-detected next to the executable) or point it at the data directory explicitly:

```sh
eden --game-root "C:\Games\eden*"
```

`--override DIR` layers a drop-in override directory on top of the archives, and a `.sav` positional argument resumes an original save directly (`eden save01.sav`). The resource tools always take explicit paths, e.g. `eden-paz list 0 scr.paz`.

### Overrides

An override directory mirrors archive addressing with plain files: any file placed at `override/<group>/<entry-name>` (e.g. `override/bg/ev001.png`) fully replaces the same-named entry in `<group>.paz` — addressed by name alone, no offsets or hashes, case-insensitive. Upscaled images inherit the logical size of the original they replace. Whole-file replacement works for images, `.ani` clips (repack with `eden-ani pack`) and scripts (UTF-8). An `override/` folder next to the game data is picked up automatically, or pass one explicitly with `--override DIR`; an optional `override.toml` inside it pins `format_version = 1` and `script_encoding = "utf-8"`.

### WebAssembly

```sh
cargo build --release --target wasm32-unknown-unknown -p eden-web
wasm-bindgen target/wasm32-unknown-unknown/release/eden_web.wasm --out-dir target/wasm-web/pkg --target web
```

The browser front-end shell lives in `crates/web/static` (`index.html`, `bootstrap.js`, `app.css`); the `wasm-bindgen` output above lands in `target/wasm-web/pkg/`.

Serve both with the bundled development server:

```sh
pip install fastapi uvicorn
python python_scripts/dev_server.py --open
```

The shell only accepts the fixed origin `http://127.0.0.1:4173` that this server binds to.

## Legal Disclaimer

**Non-affiliation.** This project is an independent open-source reimplementation. It is not affiliated with, endorsed by, or sponsored by the original publishers or rights holders in any way.

**Trademarks.** All trademarks, logos, and brand names are the property of their respective owners. The names used here are referenced solely for compatibility identification and educational purposes, and fall under descriptive fair use.

**No copyrighted assets included.** This repository contains only source code written from scratch. It does not include, and must not be distributed with, any copyrighted game assets (audio, images, scripts, or proprietary binaries).

**Requirements to run.** To use this engine to play a game, you must possess a legally obtained copy of the original game (e.g. original physical media or an official digital download).

**Purpose.** This project is intended for technical research, software preservation, and educational purposes only. It must not be used for commercial purposes. The developers assume no responsibility for any actions taken by users of this software.

---

> 「おやすみ、シオン――」
>
> ―― *eden*, final episode