# Bundled Csound runtime

The Windows package includes the unmodified official Csound 6.18.1 x64
double-precision runtime DLLs and standard opcode DLLs.

- Binary archive: https://github.com/csound/csound/releases/download/6.18.1/Csound-6.18.1-windows-x64-binaries.zip
- SHA-256 of the upstream binary archive: `bd499ac6f476d98c6ae951a8fbfda365e594ef34bcf480c5a273281df940c220`
- Corresponding upstream source: https://github.com/csound/csound/archive/refs/tags/6.18.1.zip
- License: GNU Lesser General Public License, version 2.1; see `licenses/CSOUND-COPYING` in the download.

Csound is a separate library included alongside Rustel, not linked into the
Rustel plugin binary. This project does not modify the Csound runtime.
