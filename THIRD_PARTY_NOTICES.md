# Third-party notices

This is an independent plugin project, not an official Rustel, Strudel, or Steinberg release.

## Rustel

- Project: https://github.com/tzfm/rustel
- Copyright: Rustel contributors
- License: AGPL-3.0-or-later
- Revision: `c60f974b4bc4f3e1fbb79d7982089aa15451b6fe`
- Attribution and component notices: `licenses/RUSTEL-NOTICE.md` for the plugin, `licenses/RUSTEL-CLI-NOTICE.md` for the CLI package
- The Windows package includes `Rustel-CLI-Source.zip` with the corresponding upstream CLI source.

Rustel began as a port of Strudel, which builds on TidalCycles. Its native sound engine and bundled sample are used by this plugin. The full upstream notice is included in `licenses/RUSTEL-NOTICE.md`.

## NIH-plug and egui integration

- Project: https://github.com/robbert-vdh/nih-plug
- Copyright: Robbert van der Helm and contributors
- Framework and egui adapter license: ISC
- License text: https://github.com/robbert-vdh/nih-plug/blob/de421011f41a6d10fc8c7a6084e4f4dee0143683/LICENSE

The framework's license is also included in `licenses/NIH-PLUG-LICENSE`.

The pinned framework's VST3 bindings and transitive dependencies have their own licenses, including GPL-3.0 licensing in the VST3 bindings. This plugin is distributed under AGPL-3.0-or-later; do not assume the resulting binary is an ISC-only or closed-source product.

## Baseview

- Project: https://github.com/RustAudio/baseview
- Revision: `9a0b42c09d712777b2edb4c5e0cb6baf21e988f0`
- License: MIT OR Apache-2.0
- License texts: `licenses/BASEVIEW-LICENSE-MIT` and `licenses/BASEVIEW-LICENSE-APACHE`

A locally patched copy is included in `vendor/baseview`. Embedded Windows
windows leave the host process's DPI awareness unchanged; see
`vendor/baseview/RUSTEL-PATCH.md`.

## Redistribution

Keep the accompanying license files and notices, provide corresponding source as required by the applicable licenses, and audit dependency notices before a public release. VST is a trademark of Steinberg Media Technologies GmbH.
