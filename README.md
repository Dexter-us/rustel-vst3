# Rustel VST3

A Windows x64 VST3 instrument powered by [Rustel](https://github.com/tzfm/rustel), the native Rust engine for the Strudel pattern language. Independent project; not an official Rustel release.

## Status

Repository initialized. Plugin implementation and Windows DAW validation are not complete. No installable plugin is available yet.

## First version

- Stereo VST3 instrument for Windows x64.
- Editable Strudel score, evaluate action, and diagnostic display.
- Built-in synth voices and bundled kick sample.
- DAW tempo, transport, position, and loop synchronization.
- Score and settings saved in DAW project state.
- Windows build automation and downloadable development bundles after passing checks.

## Architecture

Embed rustel-engine with session support and without device-audio: the DAW owns the audio device. Keep QuickJS evaluation and event scheduling on a dedicated worker; transfer prepared events to the audio callback. Do not allocate, block, access files, or evaluate JavaScript in the real-time callback. Pin upstream and plugin framework revisions for reproducible builds.

## Known limitations to address

The upstream host-owned callback currently lacks public sample PCM and reverb impulse-response installation APIs. Arbitrary sample libraries and .room() reverb require additional integration. Nested VST hosting, MIDI output, and full terminal Studio parity are outside the initial scope. Transport seeks, loop boundaries, sample-rate changes, offline rendering, and callback thread changes require explicit tests.

## License and attribution

AGPL-3.0-or-later. Uses Rustel by tzfm and contributors, derived from the Strudel/TidalCycles ecosystem. Preserve upstream attribution and applicable third-party notices when integrating code. See LICENSE.
