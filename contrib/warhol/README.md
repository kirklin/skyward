# Xiaomi 17T Pro (warhol): experimental bring-up

**Draft port, not tested on hardware. Not a supported recovery release.**

This prototype targets `OS3.0.311.0.WPSMIXM` (Android 16). It was prepared
against a Global311 system on a CN311 firmware base with a modified LK.
That existing system combination had booted before this work; **this Skyward
image has not been flashed or booted**. No bootloader patch or firmware is
included here. Other firmware versions and bootloader configurations are
not validated.

## Why a separate packer

The inspected stock image uses a 64 MiB `vendor_boot` v4 partition, with
4096-byte pages and two vendor ramdisk fragments:

| Index | Type | Name | Compressed size |
| --- | --- | --- | --- |
| 0 | PLATFORM (1) | empty | 36,638,170 bytes |
| 1 | RECOVERY (2) | recovery | 16,036,598 bytes |

There is no independent recovery image in this firmware's image set.
Recovery files are spread across both fragments. The Python packer overlays
Skyward in the type-2 fragment, retaining the type-1 compressed bytes, DTB,
bootconfig, command line and ramdisk selection fields. It recalculates sizes
and offsets and generates an unsigned AVB hash footer using the same layout
as `skyward-pack`. This preserves image structure, not a manufacturer
signature or a guarantee that a bootloader accepts the result.

Only this stock vendor_boot SHA-256 is accepted:

`ffe14d3341165424e1b582fb831d0f37864038433830cec5d7489716a3184faf`

Source firmware filename:
`warhol_global_images_OS3.0.311.0.WPSMIXM_20260920.0000.00_16.0_global_0e3b9b28c9.tgz`.
Supply the unmodified `vendor_boot.img` locally. No firmware is downloaded
automatically or committed. Output files never overwrite the original.

## Diagnostic runtime

The `/skyward/bringup` marker selects a small diagnostic screen before the
normal recovery runtime processes pending misc requests. It exercises KMS
and input events, with physical volume keys selecting system/bootloader
reboot and the power key confirming. It does not start the installer,
decrypt data, mount the system partitions, merge snapshots, persist settings,
run OTA updates, expose MTP or start dual boot. An explicit reboot selection
clears the bootloader control block; startup does not clear or execute it.

If display/input initialization fails, the process stays alive so ADB remains
available instead of repeatedly restarting recovery. The error is kept in
`/tmp/warhol-bringup-error.txt`. Recovery ADB is an unauthenticated root shell,
as in upstream; the diagnostic UI is **not** a read-only security boundary.

The config records a 1280x2772 panel, density 520 and the observed module names
`xiaomi_touch_warhol` / `goodix_core_warhol`. It deliberately has no guessed
Qualcomm services, raw-touch sysfs writes, crypto HALs or OTA endpoint. This
first runtime does not automatically load those touch modules: input may
remain key-only if stock recovery init does not load them.

The experimental config is outside the release workflow's `devices/*.toml`
glob, and these scripts are outside its generated `tools/` artifact directory.
The normal runtime remains selected for images without the marker.

## Build and validation

Requirements: the upstream Rust toolchain and Android target, NDK r29,
Python 3 with `fonttools` (tested with 4.61.1), and `lz4` in PATH. Dependencies
must already be cached for the offline helper. Set `CARGO_HOME`, `RUSTUP_HOME`
and `ANDROID_NDK_HOME` to existing installations, or use the helper's
project-local defaults under `local/tools`.

Place the verified source at `local/stock/vendor_boot.img`, then run:

```sh
python3 -m unittest discover -s contrib/warhol -p 'test_*.py' -v
cargo test --locked -p skyward-config
sh contrib/warhol/build-warhol.sh
```

The helper compiles Skyward for AArch64 Android API 30 and subsets the UI font
to this diagnostic screen to fit the partition. It writes the image and a
JSON verification report under `dist/`. It refuses an existing output path.
It never invokes adb/fastboot or modifies a phone.

Offline results on the development machine:

- Android release build succeeded with the locked dependencies.
- Six packer tests passed, including malformed/overlapping layout rejection,
  partition capacity, CPIO metadata and AVB output against the upstream
  avbtool fixture images.
- All 27 config tests passed, including the experimental warhol config.
- Both original CPIO fragments round-tripped all 1524 entries' content and
  metadata. The output's normal-boot fragment, DTB and bootconfig matched.
- All 204 required global ELF symbols were found in the stock libc/libm/libdl;
  the stock linker64 was present. Runtime linking and SELinux remain untested.

## Remaining work

- Verify modified LK acceptance, recovery fragment selection, KMS, colors,
  page flips, backlight, power behavior and physical keys on hardware.
- Locate/load the correct touch modules and dependencies; validate touch
  coordinate reporting and any MTK/Xiaomi-specific setup.
- Integrate MTK security services and validate data decryption separately.
- Implement vendor_boot-aware recovery identification, preservation and
  self-update before enabling the full installer.
- Adapt dual boot's independent recovery_a/b assumptions; this packaging
  change alone does not make dual boot work on warhol.

Before a hardware trial, verify the current slot, firmware and snapshot
state, back up the actual current vendor_boot and confirm a working recovery
path through the bootloader. Do not use the upstream `flash recovery`
instructions or treat this image as a boot image for `fastboot boot`.
Preserve the existing LK, preloader, calibration and NV partitions. No
flashing script is provided by this prototype.

`collect-warhol-readonly.py` optionally collects selected hardware facts over
an already-authorized ADB connection. It checks `ro.product.device`, uses no
su, and performs no reboot or write; its output stays under ignored `local/`.
