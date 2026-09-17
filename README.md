# ReMinori
Rust-compatible implementation of the Minori engine

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