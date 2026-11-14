# Rustel VST3

A native Windows x64 VST3 instrument built on [Rustel](https://github.com/tzfm/rustel), the Rust engine for Strudel patterns. Windows packages also include the upstream Rustel command-line app, built from the same pinned Rustel revision. Independent project, not an official Rustel release.

Plugin manufacturer: **Dexter U.S.**

**Status: early prototype.** Source implementation is available. A successful Windows Actions run produces a development bundle; that is not a claim of DAW compatibility or a stable release. Native window and Windows DAW testing are still required.

The source passes `cargo check --workspace --locked` on Linux with Rust 1.98.1. The focused audio check verifies that a prepared four-note synth phrase produces finite, audible stereo samples and that invalid JavaScript is refused.

## What this version does

- Edit Strudel scores inside the native plugin window.
- Press **Evaluate** or **Ctrl+Enter** to prepare the score without evaluating JavaScript in the audio callback.
- Use built-in synths and the bundled `bd` kick sample.
- Play stereo audio following DAW play/stop, tempo, song position, and loop jumps.
- Select **1–64 phrase cycles** and **1–16 quarter-note beats per cycle**. Defaults: 16 cycles, four beats per cycle.
- Save the draft, last accepted score, phrase settings, gain, transport options, and window state inside the DAW project.
- Keep the last accepted phrase when an edit fails.
- Turn off **Follow DAW** for a free-running preview, if the DAW continues processing audio while stopped.
- Opt in to **Allow native Csound** to use Rustel's `.csound()`, `.csoundm()`, inline `loadCSound`, and trusted `.orc` files.

## Important: repeating phrases, not an unbounded interpreter

This prototype evaluates the selected number of cycles on a background worker and repeats the resulting note sequence. For example, `<c e g>` varies over three cycles: select a phrase length divisible by three to preserve that progression at the repeat boundary. Random choices also repeat with the prepared phrase.

This design keeps JavaScript, queries, allocation, and score-resource loading out of the plugin callback and allows playback without a just-in-time scheduling worker during an offline bounce. Live code edits replace the phrase when preparation finishes, not on a quantized boundary.

Notes and gates follow DAW tempo without pitch shifting. Tempo-dependent effect settings are re-prepared in the background, so those settings can briefly lag a tempo change. DAW transport overrides score `setcps`/`setcpm`. Seeking and loop jumps reset sounding voices; sustained notes that began before the new position are not reconstructed.

## Get a Windows development build

1. Open this repository's **Actions → Windows VST3** page.
2. Open a successful run and download **Rustel-Windows-x64**.
3. Extract the download. Keep the entire `Rustel.vst3` directory, not just its inner binary. The same folder also contains `rustel.exe`, the full upstream CLI, and `Rustel-CLI-Source.zip`.
4. Close your DAW and copy the bundle to `%LOCALAPPDATA%\Programs\Common\VST3`, or to a VST3 folder your DAW scans.
5. Rescan VST3 plugins and load **Rustel** as an instrument on a track.

The included `install-windows.ps1` performs a current-user install. Run it from PowerShell after reviewing it. It refuses to overwrite an existing installation. Some hosts only scan `C:\Program Files\Common Files\VST3`; installing there requires administrator permission.

To use the command-line app from PowerShell, run it in the extracted package folder:

```powershell
.\rustel.exe --help
.\rustel.exe play song.strudel
.\rustel.exe export song.strudel -o take.wav --duration 30s
```

It is the upstream CLI, not a reduced plugin control panel. See the [Rustel CLI guide](https://github.com/tzfm/rustel/blob/main/docs/cli.md) for its commands. The CLI and VST3 are separate programs; CLI features do not automatically appear in the plugin.

If Actions is still running or failed, no verified Windows download is available from that run. Workflow artifacts expire after 14 days. No binary has been published as a stable release.

## Build from source

Install Git, Rust **1.98.1**, and Visual Studio Build Tools with the **Desktop development with C++** workload and Windows SDK. Rustel's QuickJS dependency compiles C code, so the native C toolchain is required.

From the repository root:

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo test --workspace --locked -- --test-threads=1
cargo xtask bundle rustel-vst3 --release --locked
git clone https://github.com/tzfm/rustel.git rustel-cli-source
git -C rustel-cli-source checkout $(Get-Content rustel-cli-revision.txt)
git -C rustel-cli-source apply ..\scripts\windows-cli-pulseaudio.patch
cargo update --manifest-path rustel-cli-source/Cargo.toml --package pulseaudio --precise 0.3.2
cargo build --manifest-path rustel-cli-source/Cargo.toml --package rustel --release --locked
```

The Windows-only patch removes an upstream PulseAudio git fork that is not publicly fetchable. The CLI still builds with all upstream default features; PulseAudio is a Linux-only backend and is not used by the Windows executable. The lockfile is updated to the matching public `pulseaudio` 0.3.2 crate before the locked build.

Outputs: `target/bundled/Rustel.vst3` and `rustel-cli-source/target/release/rustel.exe`. Build on Windows x64 for Windows binaries. A Linux `.so` cannot be installed in a Windows DAW.

The same workspace can be checked on Linux, with X11/OpenGL development libraries and a C compiler. On hosts whose dynamic loader reports a Rust compiler static-TLS error, set `GLIBC_TUNABLES=glibc.rtld.optional_static_tls=32768` for the build command.

## Example score

```javascript
stack(
  s("bd*4").gain(0.6),
  note("c2 ~ eb2 g1").s("sine").gain(0.3)
)
```

Start DAW playback after the status says **Ready**. Loading a preset only replaces the draft; press **Evaluate** to apply it. Output gain starts at −6 dB. This is not a brick-wall limiter: keep listening volume low while trying scores.

## Csound

Install a double-precision Csound 6.18 (or later 6.x) shared library separately; this plugin bundle does not include Csound. Rustel looks for `csound64.dll`/`libcsound64.dll`, or use the host process environment variable `RUSTEL_CSOUND_LIBRARY` to point to the shared library. Restart the DAW after changing environment variables.

Csound is **off by default**. Enable **Allow native Csound** only for scores you trust: an orchestra can use native opcodes to access files and run processes with the DAW's permissions. For local `.orc` files, enter the full path to a trusted folder in **Trusted .orc folder**, then use Rustel's `await loadOrc("local:voice.orc")` in the score. The file loader confines text reads to that folder; it cannot restrict what the compiled orchestra does. Inline `loadCSound` definitions need no folder.

Example Csound score after enabling permission and installing Csound:

```javascript
note("c3 eb3 g3 bb3").csound("triangle organ").gain(0.45).slow(2)
```

Rustel renders Csound phrases off the audio callback. The rendered phrase repeats at its chosen tempo; changing DAW tempo prepares a new render off-thread, and playback stays silent until that render is ready rather than pitch-shifting the orchestra. Rendered phrases are limited to 128 MiB of stereo float audio.

## Current limits

- No external sample library, soundfonts, room/reverb, live audio input, MIDI output, external hardware, Hydra, or nested VST plugins.
- Csound needs a separate compatible shared-library install on each Windows machine; the VST3 does not bundle it.
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

The plugin engine and bundled CLI are pinned to Rustel revision `c60f974b4bc4f3e1fbb79d7982089aa15451b6fe`; `Cargo.lock` pins the plugin dependency graph. The package includes the matching upstream CLI source archive and notices. The plugin does not open a second audio device or require a web server, account, API key, or network access during playback.

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Rustel derives from the Strudel/TidalCycles ecosystem. Preserve upstream licenses and attribution when redistributing source or builds.
