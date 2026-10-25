# ReMinori ShinkiEMU modifications

Upstream: https://github.com/luoyily/ReMinori
Base upstream revision: `79b00990324404bfbd496e7fa76c690cf040f744`.
Modified by the ShinkiEMU contributors, 2026, under AGPL-3.0-only; see `LICENSE`.

## Changes as of 2026-10-25

- Added `crates/shinki`, a native ShinkiEMU C ABI v3 adapter, configuration schema, session worker, cancellation and host window/modal integration. Plugin ID is `ReMinori`; family ID is `musica`.
- Kept the upstream VM, script IR, GPU composition, CPAL mixer, Media Foundation video, menus and save implementation. Added offscreen frame readback into three immutable CPU lease slots; held leases do not stop native VM/render progress.
- Added explicit private `keys.v2` profile selection, isolated save-directory support, lifecycle/storage handling, and original-mixer output mapping for multichannel hardware. The upstream PAZ key/data tables in `crates/formats/src/paz_keys.rs` remain unchanged.
- Migrated project/adapter integration to ShinkiEMU naming, pinned the shared ABI and build helper to `9cd899babbb54cbc84c3e9fbddfe192c2f9200c8`, and moved only the adapter to Rust edition 2024 / Rust 1.98 minimum.
- Added this modification/rebuild notice to the distributed core's license material. No commercial game assets or user-supplied private key profiles are part of the core package.
- Added macOS and iOS device/simulator targets to the adapter build. `dynamic-plugin-export` enables the unmangled query and private logger for the macOS `cdylib`; static iOS `rlib` registration calls the Rust-path query and reuses the host logger. License assets remain in `core-build.json`.

## Source and rebuild

The corresponding-source archive is distributed alongside the matching ShinkiEMU binary at https://github.com/Project-Shinkirou/ShinkiEMU/releases. Select the source archive for the same release and target. Its source references record the exact product and fork revisions, including this modified fork; `BUILD-FROM-SOURCE.md` gives the vendored Cargo/native dependency configuration and offline rebuild commands. Keep that archive with the binary rather than substituting a moving branch checkout.

For a development checkout with network access, use Windows x64, Rust 1.98 or newer, Visual Studio's x64 MSVC build tools and Windows SDK:

```sh
git clone https://github.com/Project-Shinkirou/ShinkiEMU.git
cd ShinkiEMU
# Check out the product revision recorded in the matching source archive.
cargo xtask deps --target x86_64-pc-windows-msvc --core ReMinori
cargo xtask build --core ReMinori --target x86_64-pc-windows-msvc --release
cargo xtask test --core ReMinori
```

The product's `build/cores.toml` selects the exact modified fork revision. The independent adapter can also be built from that fork with `cargo build --locked --release --target x86_64-pc-windows-msvc -p shinki-emu-ReMinori --features dynamic-plugin-export`. A hardware-capable wgpu backend and real audio output are required for normal gameplay, not for the adapter's unit tests. Full corresponding-source and third-party notices accompany release distribution; this notice does not replace their license texts.
