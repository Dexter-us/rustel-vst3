# Rustel VST3

A native Windows x64 VST3 instrument built on [Rustel](https://github.com/tzfm/rustel), the Rust engine for Strudel patterns. Independent project, not an official Rustel release.

**Status: early prototype.** Source implementation is available. A successful Windows Actions run produces a development bundle; that is not a claim of DAW compatibility or a stable release. Native window and Windows DAW testing are still required.

## What this version does

- Edit Strudel scores inside the native plugin window.
- Press **Evaluate** or **Ctrl+Enter** to prepare the score without evaluating JavaScript in the audio callback.
- Use built-in synths and the bundled `bd` kick sample.
- Play stereo audio following DAW play/stop, tempo, song position, and loop jumps.
- Select **1–64 phrase cycles** and **1–16 quarter-note beats per cycle**. Defaults: 16 cycles, four beats per cycle.
- Save the draft, last accepted score, phrase settings, gain, transport options, and window state inside the DAW project.
- Keep the last accepted phrase when an edit fails.
- Turn off **Follow DAW** for a free-running preview, if the DAW continues processing audio while stopped.

## Important: repeating phrases, not an unbounded interpreter

This prototype evaluates the selected number of cycles on a background worker and repeats the resulting note sequence. For example, `<c e g>` varies over three cycles: select a phrase length divisible by three to preserve that progression at the repeat boundary. Random choices also repeat with the prepared phrase.

This design keeps JavaScript, queries, allocation, and score-resource loading out of the plugin callback and allows playback without a just-in-time scheduling worker during an offline bounce. Live code edits replace the phrase when preparation finishes, not on a quantized boundary.

Notes and gates follow DAW tempo without pitch shifting. Tempo-dependent effect settings are re-prepared in the background, so those settings can briefly lag a tempo change. DAW transport overrides score `setcps`/`setcpm`. Seeking and loop jumps reset sounding voices; sustained notes that began before the new position are not reconstructed.

## Get a Windows development build

1. Open this repository's **Actions → Windows VST3** page.
2. Open a successful run and download **Rustel-Windows-x64**.
3. Extract the download. Keep the entire `Rustel.vst3` directory, not just its inner binary.
4. Close your DAW and copy the bundle to `%LOCALAPPDATA%\Programs\Common\VST3`, or to a VST3 folder your DAW scans.
5. Rescan VST3 plugins and load **Rustel** as an instrument on a track.

The included `install-windows.ps1` performs a current-user install. Run it from PowerShell after reviewing it. It refuses to overwrite an existing installation. Some hosts only scan `C:\Program Files\Common Files\VST3`; installing there requires administrator permission.

If Actions is still running or failed, no verified Windows download is available from that run. Workflow artifacts expire after 14 days. No binary has been published as a stable release.

## Build from source

Install Git, Rust **1.98.1**, and Visual Studio Build Tools with the **Desktop development with C++** workload and Windows SDK. Rustel's QuickJS dependency compiles C code, so the native C toolchain is required.

From the repository root:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked -- --test-threads=1
cargo xtask bundle rustel-vst3 --release --locked
```

Output: `target/bundled/Rustel.vst3`. Build on Windows x64 for a Windows x64 plugin. A Linux `.so` cannot be installed in a Windows DAW.

The same workspace can be checked on Linux, with X11/OpenGL development libraries and a C compiler. On hosts whose dynamic loader reports a Rust compiler static-TLS error, set `GLIBC_TUNABLES=glibc.rtld.optional_static_tls=32768` for the build command.

## Example score

```javascript
stack(
  s("bd*4").gain(0.6),
  note("c2 ~ eb2 g1").s("sine").gain(0.3)
)
```

Start DAW playback after the status says **Ready**. Loading a preset only replaces the draft; press **Evaluate** to apply it. Output gain starts at −6 dB. This is not a brick-wall limiter: keep listening volume low while trying scores.

## Current limits

- No external sample library, soundfonts, room/reverb, live audio input, MIDI output, external hardware, Hydra, or nested VST plugins.
- No syntax highlighting, sample browser, or full Rustel terminal Studio.
- No unlimited evolving patterns: use the explicit repeating phrase length.
- Maximum 128 KiB score, 16,384 prepared events, 64-voice target polyphony, 8–192 kHz audio.
- Offline playback uses the last prepared phrase. A bounce started while an edit is still compiling uses the previous phrase.
- No plugin-validator or Windows DAW certification yet. Test scanning, GUI open/close, state reload, multiple instances, tempo changes, seek/loop, sample-rate changes, and export in your target DAW before using it in a session.

## Code

- `plugin/src/compiler.rs`: bounded score evaluation and prepared phrase construction.
- `plugin/src/engine.rs`: audio rendering, beat mapping, worker lifecycle, and off-thread retirement.
- `plugin/src/editor.rs`: native egui score editor and controls.
- `plugin/src/params.rs`: host automation and persisted state.
- `xtask`: NIH-plug bundle generation.
- `.github/workflows/windows.yml`: Windows compilation, focused checks, and download packaging.

Upstream Rustel and NIH-plug are pinned to Git revisions; `Cargo.lock` pins their dependency graph. The plugin does not open a second audio device or require a web server, account, API key, or network access during playback.

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Rustel derives from the Strudel/TidalCycles ecosystem. Preserve upstream licenses and attribution when redistributing source or builds.
