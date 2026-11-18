# Rustel VST3

A native Windows x64 VST3 instrument built on [Rustel](https://github.com/tzfm/rustel), the Rust engine for Strudel patterns. Windows packages also include the upstream Rustel command-line app, built from the same pinned Rustel revision. Independent project, not an official Rustel release.

Plugin manufacturer: **Dexter U.S.**

**Status: early prototype.** Source implementation is available. A successful Windows Actions run produces a development bundle; that is not a claim of DAW compatibility or a stable release. Native window and Windows DAW testing are still required.

The source passes `cargo check --workspace --locked` on Linux with Rust 1.98.1. The focused audio check verifies that a prepared four-note synth phrase produces finite, audible stereo samples and that invalid JavaScript is refused.

## What this version does

- Edit Strudel scores inside the native plugin window, and load or save `.strudel` and `.txt` score files. A separate Csound source editor handles `.orc` and `.csd` text.
- Use the visual **Patch** screen to connect note sources, Rustel sounds, samples, wavetables, KabelSalat, Csound, VST3 instruments, and effects; generate ordinary Rustel score code from the patch.
- Press **Evaluate** or **Ctrl+Enter** to prepare the score without evaluating JavaScript in the audio callback.
- Use built-in synths and the bundled `bd` kick sample.
- Host installed x64 VST3 instruments with `.vsti("name")` and audio effects with `.vst("name", { parameter: value })`.
- Scan the standard Windows VST3 folders and browse to add extra plugin folders without changing Windows environment settings. The bundled scan helper runs without a console window.
- Accept DAW MIDI note-on/off and velocity input; MIDI notes use the first basic oscillator sound in the active score.
- Capture note-on/off and velocity from hosted VST3 MIDI generators/processors and route phrase-rendered output to Rustel synths, samples, wavetables, KabelSalat graphs, Csound instruments, or hosted VSTi plugins.
- Open a free-running MIDI VST3 generator in the separate `RustelMidiHost.exe` companion window and stream its notes live to a Rustel synth node in Patch.
- Play stereo audio following DAW play/stop, tempo, song position, and loop jumps.
- Select **1–64 phrase cycles** and **1–16 quarter-note beats per cycle**. Defaults: 16 cycles, four beats per cycle.
- Save the draft, last accepted score, phrase settings, gain, transport options, and window state inside the DAW project.
- Keep the last accepted phrase when an edit fails.
- Turn off **Follow DAW** for a free-running preview, if the DAW continues processing audio while stopped.
- Opt in to **Allow native Csound** to use Rustel's `.csound()`, `.csoundm()`, inline `loadCSound`, and trusted `.orc` files.
- Choose the **Csound triangle** example or one of the three bundled FM examples in the editor's Examples list.

## Important: repeating phrases, not an unbounded interpreter

This prototype evaluates the selected number of cycles on a background worker and repeats the resulting note sequence. For example, `<c e g>` varies over three cycles: select a phrase length divisible by three to preserve that progression at the repeat boundary. Random choices also repeat with the prepared phrase.

This design keeps JavaScript, queries, allocation, plugin loading, and score-resource loading out of the plugin callback and allows playback without a just-in-time scheduling worker during an offline bounce. Live code edits replace the phrase when preparation finishes, not on a quantized boundary. Scores that use Csound or hosted VST3 plugins render the selected phrase off-thread before playback; the rendered result repeats exactly until the next evaluation.

Phrase-rendered hosted MIDI routing is bounded to the prepared phrase. It currently carries note-on, note-off, and velocity only; MIDI CC, pitch bend, aftertouch, and SysEx are not routed. Built-in Rustel targets include the four oscillator waveforms, supersaw, pulse, SBD, noise colors, bytebeat, ZzFX, and one; sample, wavetable, KabelSalat graph, Csound, and hosted VSTi targets are supported for phrase-rendered MIDI.

Free-running MIDI streaming is a separate mode: leave the MIDI Source input unconnected, connect it to a Rustel synth node, generate the Patch, and press **Evaluate**. Then select **Open MIDI VST editor**. The plugin's own GUI opens in a separate window and starts streaming automatically; closing it stops the stream. Live streaming currently targets Rustel synth nodes only; use a Note Pattern for other instrument families. The companion host runs on its own audio clock and does not follow the DAW's transport or tempo.

Notes and gates follow DAW tempo without pitch shifting. Tempo-dependent effect settings are re-prepared in the background, so those settings can briefly lag a tempo change. DAW transport overrides score `setcps`/`setcpm`. Seeking and loop jumps reset sounding voices; sustained notes that began before the new position are not reconstructed.

## Get a Windows development build

1. Open this repository's **Actions → Windows VST3** page.
2. Open a successful run and download **Rustel-Windows-x64**.
3. For portable use, download `Rustel-Windows-x64-Portable.zip` from the run's artifact and extract the whole ZIP to a folder you want to keep. Add that folder to VSTHost's plugin scan paths and rescan. Keep `Rustel.vst3`, `RustelVstWorker.exe`, `RustelMidiHost.exe`, and `CsoundRuntime` together; Rustel finds its helpers and Csound runtime there. The package also includes `Rustel-Help.html`, `rustel.exe`, and the upstream CLI. Read the guide online at [Rustel Help](https://dexter-us.github.io/rustel-vst3/).
4. Or extract the complete `Rustel-Windows-x64` artifact and run `install-windows.ps1` from PowerShell after reviewing it. It installs Rustel, its VST3 scan helpers, and Csound 6.18.1 for your Windows user, configures the paths Rustel needs, and preserves any previous install in a backup folder.
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
cargo build --package rustel-vst3@0.1.14 --release --locked
Copy-Item target/release/rustel_vst3.dll 'target/release/rustel_vst3@0.1.14.dll' -Force
cargo xtask bundle rustel-vst3@0.1.14 --release --locked
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

Rustel automatically loads saved scan results and checks for changed bundles when the plugin starts. Use **Scan VST3 plugins** to run that check again manually. Rustel searches `%LOCALAPPDATA%\Programs\Common\VST3`, `%CommonProgramFiles%\VST3`, and any folders added with **Browse for a plugin folder**. You can also enter multiple paths in the VST3 folder field, separated by semicolons. Custom folder choices are shared by Rustel instances using the same data directory and saved in `RustelData/vst/folders.txt`. The bundled `RustelVstWorker.exe` checks plugins one at a time in a separate process and should not open a console window. The editor shows the bundle and plugin class currently being checked. Each class has a 30-second limit; other bundles continue if a plugin times out. The overall scan can run for up to two hours. Results, including failures, are saved in `RustelData/vst/scan.json` and reused until a bundle changes. Delete only that file to retry all bundles; saved presets are kept. These expressions use plugin names exactly as shown in the list:

Click **Open plugin list…** to open the searchable, resizable directory popup. It shows the scanned plugin count and groups results under Instruments or Effects / processors, then by VST3 category. Use **All plugins** to browse the full list or **MIDI index** to see plugins you have marked as MIDI sources. On a plugin row, click **Add to MIDI index** or **Remove from MIDI index**; this classification is saved in `RustelData/vst/directory.tsv`. MIDI sources remain visible in **All plugins** with their instrument/effect type. The tag is manual and does not test or change a plugin, so only add plugins that output MIDI. The **MIDI source** menu inserts a MIDI-routing example at the score caret.

```javascript
note("c2 e2 g2").vsti("Your instrument")
```

```javascript
note("c2 e2 g2")
  .vsti("Your instrument")
  .vst("Your effect", { mix: 0.5 })
```

Add an effect by chaining `.vst("Effect Name")` after the sound or instrument. Effect controls use the VST's parameter keys with normalized values from 0 to 1. In the score editor, right-click the effect name inside `.vst("...")` to load its available controls and insert one with its default value. The same editor context menu includes Copy, Cut, and Paste.

`vsti()` sends each score note's pitch, velocity, and duration to a VST3 instrument. `vst()` processes the resulting sound through a VST3 audio effect. VST2 plugins are not supported. Hosted plugin phrases are rendered off the audio callback and then repeat with the selected phrase length; plugin changes take effect after **Evaluate** finishes. The scan runs in the bundled helper process, but actual plugin rendering happens inside the DAW process. Only use plugins you trust; a faulty plugin can still crash or hang the DAW during rendering.

The editor header shows the plugin version, such as **v0.1.14**. Use that label to distinguish this build from older downloads.

## Visual patch editor

Click **Patch** beside **Score** to open the native node canvas. Use the toolbar or right-click the canvas to add a note pattern, a MIDI-outputting VST3, an instrument, a pattern/audio expression, a VST3 effect, and an audio output. Instrument choices include Rustel synths, samples, wavetables, KabelSalat graphs, Csound, and VST3 instruments. The Patch screen shows scan counts and has shortcuts to the plugin list and rescan. The effect picker lists every scanned plugin and labels the scanner's class; choose one that accepts audio input. Empty instrument or MIDI lists explain which scan/tag setting to check.

Click an output port, then a compatible input port to connect the blocks. Amber cables carry MIDI; teal cables carry audio. Drag a block title to move it, drag empty canvas to pan, remove a node with its **×** button, remove cables from **Connections**, or choose **Clear all blocks…** to reset the canvas. A Csound instrument block can browse for an `.orc` file; choosing it sets the trusted folder to the file’s folder and generated code loads the file relative to that folder, not from the DAW’s current directory. Enter the instrument name declared in that ORC, enable **Allow native Csound**, generate code, review it, then press **Evaluate**. The default patch is a note pattern through a Rustel synth to the audio output.

**Generate Code** checks the patch and replaces the current score draft with ordinary Rustel code. Review that code in **Score** and press **Evaluate** to run it. The patch graph is stored in the DAW project state. This is one-way generation: handwritten score code is not converted back into blocks.

A hosted MIDI VST source can feed one compatible instrument target in a score: a Rustel synth, sample, wavetable, KabelSalat graph, Csound instrument, or hosted VSTi. The current renderer does not apply a downstream audio effect to that separately rendered MIDI-target branch. For an ordinary audio-effect chain, connect a note pattern to an instrument, then connect the instrument's audio output through a VST3 effect to Audio output.

Example patch output for an ordinary VSTi/effect chain:

```javascript
(note("c3 e3 g3")).vsti("Your instrument").vst("Your effect", { mix: 0.5 })
```

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

In **Open plugin list → MIDI source**, choose **Rustel sample**, **Rustel wavetable**, or **KabelSalat graph** to insert these directives. Edit the sample/wavetable name or graph expression in the menu before inserting.

## Csound

The Windows downloads include the official double-precision Csound 6.18.1 runtime and standard opcode DLLs; you do not need a separate Csound installation. In the portable ZIP, keep `CsoundRuntime` beside `Rustel.vst3`; the plugin automatically points the current DAW process at the bundled copy. Scan cache and presets for hosted plugins are kept in `RustelData` beside the package, unless `RUSTEL_CONFIG_DIR` is already set. The package does not change persistent Windows settings. The Microsoft Visual C++ 2015–2022 x64 runtime may also be needed; the installer checks for it and prints Microsoft's download link if it is missing. Rustel currently supports Csound 6.18 or later within the 6.x series, so this bundle does not use Csound 7.

Csound is **off by default**. Enable **Allow native Csound** only for scores you trust: an orchestra can use native opcodes to access files and run processes with the DAW's permissions. The portable `Scores` folder is selected automatically as **Trusted .orc folder** when the plugin starts; the ORC picker opens there, and ORCs selected from it are saved as relative references so they follow the portable package when it moves. Old saved references to a previous portable `Scores` location are rebased when the matching file is present. Loading or saving an `.orc` in **Csound files…** also sets this folder. Set **Trusted .orc folder** manually only when using ORCs elsewhere, then use `await loadOrc("local:voice.orc")` in the score. The file loader confines text reads to that folder; it cannot restrict what the compiled orchestra does. Inline `loadCSound` definitions need no folder.

The portable package includes three FM instruments and matching `.strudel` scores in `Scores`: **FM Electric Piano** (`fm_electric_piano`), **FM Bass** (`fm_bass`), and **FM Bell** (`fm_bell`). Turn on **Allow native Csound**, load a matching `.strudel` file from the plugin's **Load…** button, and press **Evaluate**. The portable `Scores` folder is trusted automatically. The editor's Examples list also has one-click score snippets. Each orchestra uses Rustel's `.csoundm()` fields: `p4` is fractional MIDI pitch and `p5` is velocity-scaled level.

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
- No syntax highlighting, sample browser, score-to-patch import, or full Rustel terminal Studio.
- No unlimited evolving patterns: use the explicit repeating phrase length.
- Maximum 128 KiB score, 16,384 prepared events, 64-voice target polyphony, 8–192 kHz audio.
- Offline playback uses the last prepared phrase. A bounce started while an edit is still compiling uses the previous phrase.
- No plugin-validator or Windows DAW certification yet. Test scanning, GUI open/close, state reload, multiple instances, tempo changes, seek/loop, sample-rate changes, and export in your target DAW before using it in a session.

## Code

- `plugin/src/compiler.rs`: bounded score evaluation and prepared phrase construction.
- `plugin/src/engine.rs`: audio rendering, beat mapping, worker lifecycle, and off-thread retirement.
- `plugin/src/editor.rs` and `plugin/src/patch.rs`: native score/patch editors, typed node graph, and code generation.
- `plugin/src/params.rs`: host automation and persisted state.
- `xtask`: NIH-plug bundle generation.
- `.github/workflows/windows.yml`: Windows compilation, focused checks, and download packaging.

The plugin engine and bundled CLI are pinned to Rustel revision `c60f974b4bc4f3e1fbb79d7982089aa15451b6fe`; `Cargo.lock` pins the plugin dependency graph. The package includes the matching upstream CLI source archive and notices. It also includes the unmodified official Csound 6.18.1 Windows runtime; see `CSOUND-RUNTIME-NOTICE.md` and `licenses/CSOUND-COPYING`. The plugin does not open a second audio device or require a web server, account, API key, or network access during playback.

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Rustel derives from the Strudel/TidalCycles ecosystem. Preserve upstream licenses and attribution when redistributing source or builds.
