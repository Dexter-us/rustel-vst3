# Rustel VST3

A native Windows x64 VST3 instrument built on [Rustel](https://github.com/tzfm/rustel), the Rust engine for Strudel patterns. Windows packages also include the upstream Rustel command-line app, built from the same pinned Rustel revision. Independent project, not an official Rustel release.

Plugin manufacturer: **Dexter U.S.**

**Status: early prototype.** Source implementation is available. A successful Windows Actions run produces a development bundle; that is not a claim of DAW compatibility or a stable release. Native window and Windows DAW testing are still required.

The source passes `cargo check --workspace --locked` on Linux with Rust 1.98.1. The focused audio check verifies that a prepared four-note synth phrase produces finite, audible stereo samples and that invalid JavaScript is refused.

## What this version does

- Edit Strudel scores inside the native plugin window, and load or save `.strudel` and `.txt` score files.
- Press **Evaluate** or **Ctrl+Enter** to prepare the score without evaluating JavaScript in the audio callback.
- Use built-in synths and the bundled `bd` kick sample.
- Host installed x64 VST3 instruments with `.vsti("name")` and audio effects with `.vst("name", { parameter: value })`.
- Scan installed plugins in the editor and add extra VST3 search folders without changing Windows environment settings.
- Accept DAW MIDI note-on/off and velocity input; MIDI notes use the first basic oscillator sound in the active score.
- Capture note-on/off and velocity from hosted VST3 MIDI generators/processors. Route the output to a Rustel built-in synth, a hosted VSTi, or a named Csound instrument.
- Play stereo audio following DAW play/stop, tempo, song position, and loop jumps.
- Select **1–64 phrase cycles** and **1–16 quarter-note beats per cycle**. Defaults: 16 cycles, four beats per cycle.
- Save the draft, last accepted score, phrase settings, gain, transport options, and window state inside the DAW project.
- Keep the last accepted phrase when an edit fails.
- Turn off **Follow DAW** for a free-running preview, if the DAW continues processing audio while stopped.
- Opt in to **Allow native Csound** to use Rustel's `.csound()`, `.csoundm()`, inline `loadCSound`, and trusted `.orc` files.
- Choose the **Csound triangle** example in the editor's Examples list to insert a ready-to-evaluate Csound score.

## Important: repeating phrases, not an unbounded interpreter

This prototype evaluates the selected number of cycles on a background worker and repeats the resulting note sequence. For example, `<c e g>` varies over three cycles: select a phrase length divisible by three to preserve that progression at the repeat boundary. Random choices also repeat with the prepared phrase.

This design keeps JavaScript, queries, allocation, plugin loading, and score-resource loading out of the plugin callback and allows playback without a just-in-time scheduling worker during an offline bounce. Live code edits replace the phrase when preparation finishes, not on a quantized boundary. Scores that use Csound or hosted VST3 plugins render the selected phrase off-thread before playback; the rendered result repeats exactly until the next evaluation.

Hosted MIDI routing is bounded to the prepared phrase. It currently carries note-on, note-off, and velocity only; MIDI CC, pitch bend, aftertouch, and SysEx are not routed. Built-in Rustel targets include the four oscillator waveforms, supersaw, pulse, SBD, noise colors, bytebeat, ZzFX, and one. Sample, wavetable, and KabelSalat graph targets are not yet available through this route.

Notes and gates follow DAW tempo without pitch shifting. Tempo-dependent effect settings are re-prepared in the background, so those settings can briefly lag a tempo change. DAW transport overrides score `setcps`/`setcpm`. Seeking and loop jumps reset sounding voices; sustained notes that began before the new position are not reconstructed.

## Get a Windows development build

1. Open this repository's **Actions → Windows VST3** page.
2. Open a successful run and download **Rustel-Windows-x64**.
3. For portable use, download `Rustel-Windows-x64-Portable.zip` from the run's artifact and extract the whole ZIP to a folder you want to keep. Add that folder to VSTHost's plugin scan paths and rescan. Keep `Rustel.vst3`, `RustelVstWorker.exe`, and `CsoundRuntime` together; Rustel finds its helper and Csound runtime there. The package also includes `Rustel-Help.html`, `rustel.exe`, and the upstream CLI. Read the guide online at [Rustel Help](https://dexter-us.github.io/rustel-vst3/).
4. Or extract the complete `Rustel-Windows-x64` artifact and run `install-windows.ps1` from PowerShell after reviewing it. It installs Rustel, its VST3 scan helper, and Csound 6.18.1 for your Windows user, configures the paths Rustel needs, and preserves any previous install in a backup folder.
5. Reopen your DAW, rescan VST3 plugins, and load **Rustel** as an instrument on a track.

The installer needs no administrator permission. It puts the plugin in `%LOCALAPPDATA%\Programs\Common\VST3` and Csound in `%LOCALAPPDATA%\Programs\Rustel\Csound-6.18.1`. Existing files are renamed to a timestamped backup before replacement. The installer sets per-user `RUSTEL_CSOUND_LIBRARY`, `OPCODE6DIR64`, and `PATH` values; restart open DAWs before using Csound. Some hosts only scan `C:\Program Files\Common Files\VST3`; that location requires administrator permission and is not used by this script. The portable ZIP does not run an installer or change persistent Windows settings.

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
cargo build --package rustel-vst3@0.1.1 --release --locked
Copy-Item target/release/rustel_vst3.dll 'target/release/rustel_vst3@0.1.1.dll' -Force
cargo xtask bundle rustel-vst3@0.1.1 --release --locked
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

## VST3 instruments and effects

Use **Scan VST3 plugins** in the editor to list plugins from the standard Windows VST3 folders. Enter additional search folders in the VST3 folder field if needed; separate multiple paths with semicolons. These expressions use plugin names exactly as shown in the list:

```javascript
note("c2 e2 g2").vsti("Your instrument")
```

```javascript
note("c2 e2 g2").vsti("Your instrument").vst("Your effect", { depth: 0.5 })
```

`vsti()` sends each score note's pitch, velocity, and duration to a VST3 instrument. `vst()` processes the resulting sound through a VST3 audio effect. VST2 plugins are not supported. Hosted plugin phrases are rendered off the audio callback and then repeat with the selected phrase length; plugin changes take effect after **Evaluate** finishes. The scan runs in the bundled helper process, but actual plugin rendering happens inside the DAW process. Only use plugins you trust; a faulty plugin can still crash or hang the DAW during rendering.

Rustel also accepts DAW MIDI note input. In VSTHost, route a MIDI-generating plugin or MIDI-outputting VSTi into Rustel's MIDI input; Rustel honors note-on, note-off, and velocity. MIDI notes use the first basic oscillator's waveform and envelope in the active score, or sine if the score has no basic oscillator note. Hosted VST3 MIDI output is separate and uses the target directive below.

To route notes generated by a hosted VST3 into another Rustel synth, add one target directive to the score:

```javascript
// MIDI target: rustel "supersaw"
note("c4 ~ e4").vsti("MIDI generator")
```

The built-in Rustel target choices are `sine`, `triangle`, `square`, `sawtooth`, `supersaw`, `pulse`, `sbd`, `white`, `pink`, `brown`, `crackle`, `bytebeat`, `zzfx`, and `one`. If there is no directive, `sine` is used. Hosted MIDI can also target a Rustel sample, wavetable, or KabelSalat graph:

```javascript
// MIDI target: sample "bd"
note("c4 ~ e4").vsti("MIDI generator")
```

```javascript
// MIDI target: wavetable "wt_x"
note("c4 ~ e4").vsti("MIDI generator")
```

```javascript
// MIDI target: graph sine(sFreq).mul(0.2).out()
note("c4 ~ e4").vsti("MIDI generator")
```

The sample or wavetable name must be available to Rustel; uncached library sounds load when you evaluate the score. The graph target is a KabelSalat expression with `sFreq` and `sGate` available, and it must send sound to `.out()`. These routes render notes, velocity, and note-off duration into the prepared phrase off the audio callback. MIDI CC, pitch bend, aftertouch, and SysEx are not routed.

## Csound

The Windows downloads include the official double-precision Csound 6.18.1 runtime and standard opcode DLLs; you do not need a separate Csound installation. In the portable ZIP, keep `CsoundRuntime` beside `Rustel.vst3`; the plugin automatically points the current DAW process at the bundled copy. Scan cache and presets for hosted plugins are kept in `RustelData` beside the package, unless `RUSTEL_CONFIG_DIR` is already set. The package does not change persistent Windows settings. The Microsoft Visual C++ 2015–2022 x64 runtime may also be needed; the installer checks for it and prints Microsoft's download link if it is missing. Rustel currently supports Csound 6.18 or later within the 6.x series, so this bundle does not use Csound 7.

Csound is **off by default**. Enable **Allow native Csound** only for scores you trust: an orchestra can use native opcodes to access files and run processes with the DAW's permissions. For local `.orc` files, enter the full path to a trusted folder in **Trusted .orc folder**, then use Rustel's `await loadOrc("local:voice.orc")` in the score. The file loader confines text reads to that folder; it cannot restrict what the compiled orchestra does. Inline `loadCSound` definitions need no folder.

Example Csound score after enabling permission:

```javascript
note("c3 eb3 g3 bb3").csound("triangle organ").gain(0.45).slow(2)
```

To send notes from a hosted MIDI VST to a Csound instrument available to the score—including Rustel's bundled instruments or one you define with `loadCSound` or load from a trusted `.orc` file—use a target directive:

```javascript
// MIDI target: csound "triangle"
note("c4 ~ e4").vsti("MIDI generator")
```

This route maps MIDI pitch to Csound's fractional MIDI-note `p4` and velocity to `p5`; note-off controls the event duration. Enable **Allow native Csound**. Csound routing supports up to 1,024 generated notes per phrase. Rustel renders Csound phrases off the audio callback. The rendered phrase repeats at its chosen tempo; changing DAW tempo prepares a new render off-thread, and playback stays silent until that render is ready rather than pitch-shifting the orchestra. Rendered phrases are limited to 128 MiB of stereo float audio.

## Current limits

- No user-configurable local sample folders or soundfonts, live audio input, MIDI output from Rustel to the DAW, MIDI CC/pitch-bend/aftertouch input, external hardware, Hydra, or VST2 plugins.
- The bundled Csound runtime is the 6.18.1 Windows x64 double-precision build; Csound 7 and 32-bit hosts are not supported.
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

The plugin engine and bundled CLI are pinned to Rustel revision `c60f974b4bc4f3e1fbb79d7982089aa15451b6fe`; `Cargo.lock` pins the plugin dependency graph. The package includes the matching upstream CLI source archive and notices. It also includes the unmodified official Csound 6.18.1 Windows runtime; see `CSOUND-RUNTIME-NOTICE.md` and `licenses/CSOUND-COPYING`. The plugin does not open a second audio device or require a web server, account, API key, or network access during playback.

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Rustel derives from the Strudel/TidalCycles ecosystem. Preserve upstream licenses and attribution when redistributing source or builds.
