# Xiaomi 17 Pro Max (`popsicle`)

Popsicle support is validated on a Xiaomi 17 Pro Max running
`OS4.0.6.0.XPBCNXM`. Other models and ROM builds may work, but have not been
validated. Data decryption depends on the matching ROM's KeyMint, Gatekeeper,
Weaver, secure-element services, trusted applications, and CS40L26 firmware.
Those proprietary files are not distributed by this project.

## Build a local decrypt-capable overlay

Use partition images extracted from the exact ROM build installed on the
device. The extracted roots must have their normal partition layout, such as
`system/build.prop`, `vendor/bin/qseecomd`, `odm/bin/hw/...`, and
`firmware/image/<TA files>` under the modem root. Keep these files local.

Build `external/se_omapi` from an Android source tree with the matching Android
platform version and an arm64 target. Pass its resulting `se_omapi` executable
to the script. The service binary is source-built; no prebuilt executable is
included here.

```sh
tools/popsicle/extract-overlay.sh \
  --system /path/to/extracted/system \
  --vendor /path/to/extracted/vendor \
  --odm /path/to/extracted/odm \
  --modem /path/to/extracted/modem \
  --se-omapi /path/to/android/out/target/product/PRODUCT/system/bin/se_omapi \
  --out /path/to/private/popsicle-overlay
```

The script checks that every required TA segment and firmware file exists and
records local source paths and output hashes in `LOCAL-SOURCES.txt`. That file
and the overlay can identify a user's ROM layout, so do not publish either.

Build the normal Skyward tools, then pass the local overlay to the packer:

```sh
cargo build --locked --release -p skyward-pack
target/release/skyward-pack recovery \
  --stock target/stock/popsicle/recovery.img \
  --binary target/aarch64-linux-android/release/skyward \
  --system-init target/system-init/aarch64-linux-android/release/system-init \
  --config devices/popsicle.toml \
  --font resources/fonts/MiSansVF.ttf \
  --mono resources/fonts/KirkMono-Regular.ttf \
  --mono-license resources/fonts/KirkMono-OFL.txt \
  --overlay /path/to/private/popsicle-overlay \
  --out target/skyward-popsicle.img
```

Without the local overlay, Skyward can still start on Popsicle, but hardware-
backed credential verification and data decryption are not provided. Never
flash an image built from files taken from a different ROM build.
