# NSSM is bundled at build-time by the CI workflow
# (.github/workflows/release.yml). It is not committed to the repo because the
# binary is fetched from https://nssm.cc/release/nssm-2.24.zip during CI.
#
# To build the installer locally:
#   1. Download nssm-2.24.zip from https://nssm.cc/release/nssm-2.24.zip
#   2. Extract win64\nssm.exe to packaging\windows\vendor\nssm.exe
#   3. Run: iscc packaging\windows\simple-photos.iss
