# Rustel MIDI VST3 Host Helper

A small Windows JUCE host helper for MIDI-producing VST3 plugins. In standalone
capture mode, it can feed the plugin a source MIDI file and write captured
events to a Standard MIDI File. In Rustel stream mode, it opens the plugin
editor and forwards note-on, note-off, and velocity events to Rustel over
loopback UDP.

This is an early prototype, not a general-purpose DAW or a security sandbox.
Only use VST3 plugins you trust. A plugin runs in-process and can crash the
helper.

## Build

The Windows GitHub Actions workflow builds a 64-bit Release executable and
uploads it as an artifact. For a local build, install CMake 3.25+ and Visual
Studio 2022 C++ tools, then run:

```powershell
cmake -S . -B build -A x64
cmake --build build --config Release
```

JUCE 8.0.6 is fetched by CMake at configure time.

## Launch configuration

The helper accepts the path to a JSON configuration file as its only argument:

```json
{
  "plugin_path": "C:/Program Files/Common Files/VST3/Example.vst3",
  "input_midi_path": "C:/temp/source.mid",
  "output_midi_path": "C:/temp/captured.mid",
  "status_path": "C:/temp/capture-status.json",
  "duration_seconds": 15,
  "ticks_per_beat": 480,
  "tempo_microseconds": 500000,
  "stream_host": "",
  "stream_port": 0,
  "stream_token": ""
}
```

`input_midi_path` may be an empty string when the plugin generates notes
without MIDI input. The plugin editor opens first; configure it, then click
**Start capture**. The helper writes a success or failure result to
`status_path`. It will not write a successful MIDI file unless at least one
note-on event was captured.

For live Rustel streaming, set `stream_host` to `127.0.0.1`, provide the
Rustel-assigned `stream_port` and `stream_token`, set `duration_seconds` to
`0`, and omit the output MIDI file. The plugin editor opens as the visible
window and streaming begins automatically; closing that editor stops it. The
helper sends note events and heartbeat packets while its audio callback runs.
The separate helper host does not follow the DAW's transport or tempo and does
not isolate plugin crashes.