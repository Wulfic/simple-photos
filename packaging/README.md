# Simple Photos — Release Packaging

This directory holds the assets and scripts used by `.github/workflows/release.yml`
to produce the official Linux `.deb` and Windows `.exe` installers attached to
each GitHub release.

```
packaging/
├── debian/                       # cargo-deb inputs
│   ├── config.toml               # bundled config (placeholder JWT secret)
│   ├── postinst                  # creates simple-photos user, randomises secret
│   ├── prerm
│   ├── postrm                    # purges config but PRESERVES /var/lib/simple-photos
│   ├── simple-photos.service     # systemd unit (hardened sandbox)
│   ├── fetch-assets.sh           # post-install ONNX + GeoNames fetcher
│   └── README.Debian
└── windows/
    ├── simple-photos.iss         # Inno Setup 6 installer script
    ├── fetch-assets.ps1          # config generator + asset fetcher
    └── vendor/                   # NSSM is downloaded by CI, not committed
```

## Building locally

### Linux `.deb`

```bash
# 1. Build the web frontend (output goes to web/dist/)
cd web && npm ci && npm run build && cd ..

# 2. Build the server in release mode (default `cuda` feature → GPU-capable;
#    ort's copy-dylibs drops libonnxruntime_providers_*.so into target/release,
#    which cargo-deb bundles into /usr/lib/simple-photos)
cd server && cargo build --release --locked

# 3. (CI only) stamp the release version so the APK download URL resolves:
#    sed -i "s/@SP_VERSION@/<version>/" ../packaging/debian/fetch-assets.sh

# 4. Build the .deb via cargo-deb metadata in server/Cargo.toml
cargo install cargo-deb --version ^2 --locked
cargo deb --no-build --no-strip
# → server/target/debian/simple-photos_<version>_amd64.deb

# 5. Install + smoke test
sudo apt install ./server/target/debian/simple-photos_*_amd64.deb
sudo systemctl status simple-photos
```

The `.deb` does **not** bundle the ~225 MB of ONNX models, the GeoNames
dataset, or the Android APK. They are fetched **automatically** on first boot
by the `simple-photos-setup.service` oneshot (which then restarts the server).
To trigger the fetch immediately instead of waiting for a reboot:

```bash
sudo systemctl start simple-photos-setup.service   # background, idempotent
# or run the fetcher directly:
sudo -u simple-photos /usr/share/simple-photos/fetch-assets.sh
sudo systemctl restart simple-photos
```

> A CPU-only build (`cargo build --release --no-default-features`) does **not**
> produce the `libonnxruntime_providers_*.so` files — remove those two asset
> lines from `server/Cargo.toml` if you build without the `cuda` feature.

### Windows `.exe`

```powershell
# 1. Web build
cd web; npm ci; npm run build; cd ..

# 2. Server release build
cd server; cargo build --release --locked; cd ..

# 3. Drop the two bundled binaries into packaging\windows\vendor\ :
#       nssm.exe          (https://nssm.cc/release/nssm-2.24.zip → win64\nssm.exe)
#       vc_redist.x64.exe (https://aka.ms/vs/17/release/vc_redist.x64.exe)

# 4. Build with Inno Setup 6
iscc /DSP_VERSION=1.3.47 packaging\windows\simple-photos.iss
# → dist\simple-photos-1.3.47-windows-x64-setup.exe
```

The installer first installs the **Microsoft Visual C++ Redistributable**
(required — `server.exe` and `onnxruntime.dll` link `VCRUNTIME140.dll`;
without it the service won't start), then asks for an install location,
registers the server as a Windows Service via NSSM, opens TCP 8080 in the
firewall, and downloads the AI models + GeoNames dataset + ffmpeg.exe +
Android APK (~355 MB) — all required for full functionality with no working
fallback.

#### NVIDIA GPU acceleration (optional)

The Windows build is compiled with the `cuda` ONNX Runtime execution
provider. The CUDA libraries are loaded lazily at runtime (`dlopen`), so
the installer runs unmodified on machines without an NVIDIA card and
falls back to CPU.

To enable GPU acceleration, install on the host:

1. **NVIDIA GPU driver** (latest Game Ready / Studio driver from
   [nvidia.com/Download](https://www.nvidia.com/Download/index.aspx)).
2. **CUDA Toolkit 12.x** (12.2 or newer) — the installer ships
   `cudart64_12.dll` and `cublas64_12.dll`, which ORT loads on demand.
   <https://developer.nvidia.com/cuda-downloads>
3. **cuDNN 9.x for CUDA 12** (extract into the CUDA Toolkit `bin` dir,
   or anywhere on `PATH`). <https://developer.nvidia.com/cudnn>

Restart the `SimplePhotos` service after installing CUDA. The startup
log line `AI engine: GPU available (CUDA detected)` confirms acceleration
is active; without it the engine quietly uses CPU.

## CI/CD

Pushing a tag matching `v*.*.*` triggers `.github/workflows/release.yml`:

1. Resolve version from the tag (e.g. `v0.7.0` → `0.7.0`).
2. Build `web/dist` once and share it across both server-side jobs.
3. Build the `.deb` on `ubuntu-22.04` (oldest glibc we support).
4. Build the `.exe` on `windows-latest`.
5. Build the Android `.apk` on `ubuntu-latest` (JDK 17 + Android SDK 34).
6. Compute `SHA256SUMS.txt` and create a **draft** GitHub Release with all
   artefacts attached.

A maintainer must manually publish the draft release after smoke-testing.
Manual `workflow_dispatch` runs build artefacts but do not create a release.

### Android signing

The Android job uses a release keystore when these repository secrets are set:

| Secret | Description |
|--------|-------------|
| `ANDROID_KEYSTORE_BASE64`   | `base64 release.jks` (one line, no wrap) |
| `ANDROID_KEYSTORE_PASSWORD` | keystore password |
| `ANDROID_KEY_ALIAS`         | key alias inside the keystore |
| `ANDROID_KEY_PASSWORD`      | key password |

Generate a keystore once with:

```bash
keytool -genkeypair -v -keystore release.jks -keyalg RSA -keysize 4096 \
        -validity 10000 -alias simple-photos
base64 -w0 release.jks   # paste the output into ANDROID_KEYSTORE_BASE64
```

If the secrets are absent the workflow still produces an installable APK, but
it is signed with the AGP debug keystore (handy for previews — unsuitable for
playground/Play distribution because subsequent releases must keep the same
signing key).

## Pinned versions

Reproducibility is enforced by lockfiles:

| Component | Lockfile                  | Notes                                              |
|-----------|---------------------------|----------------------------------------------------|
| Rust      | `server/Cargo.lock`       | committed; CI uses `--locked`                      |
| Web       | `web/package-lock.json`   | committed; CI uses `npm ci`                        |
| Python    | `tests/requirements.txt`  | exact `==` pins; CI rejects `>=`/`~=` in `ci.yml`  |
| Toolchain | `RUST_TOOLCHAIN`/`NODE_VERSION` env vars in workflows | bumped via PR |

Bumping any pin should be a deliberate, reviewed change.
