# Bundled binaries (fetched by CI, not committed)

The Windows installer bundles two third-party binaries that the CI workflow
(`.github/workflows/pipeline.yml`, `windows` job) downloads at build time. They
are **not** committed to the repo.

## `nssm.exe`

Service wrapper (Simple Photos is a console app; Windows Services need a host).

- CI: pulled via Chocolatey (`nssm 2.24.101.20180116`).
- Local build: download <https://nssm.cc/release/nssm-2.24.zip>, extract
  `win64\nssm.exe` here.

## `vc_redist.x64.exe`

Microsoft Visual C++ 2015-2022 Redistributable (x64). **Required** — the
MSVC-built `simple-photos-server.exe` and `onnxruntime.dll` both link
`VCRUNTIME140.dll` / `MSVCP140.dll`. Without it the service fails to start on a
clean Windows install ("VCRUNTIME140.dll was not found") and never comes up on
reboot. The installer runs it silently before registering the service.

- CI: downloaded from <https://aka.ms/vs/17/release/vc_redist.x64.exe>.
- Local build: download the same URL and save it here as `vc_redist.x64.exe`.

## Building the installer locally

```powershell
# After dropping nssm.exe and vc_redist.x64.exe into this folder:
iscc /DSP_VERSION=1.3.44 packaging\windows\simple-photos.iss
```
