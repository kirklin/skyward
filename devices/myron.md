# myron: stock evidence and recovery bring-up

This is an experimental recovery candidate, not a tested release. Two
temporary-boot attempts have no confirmed Skyward execution. No myron PR
has been submitted. Build success does not establish display, touch,
Wi-Fi, PIN decryption or writable-data support.

Read-only inspection on 2026-11-12 identified REDMI K90 Pro Max
`25102RKBEC`, SM8850/canoe, OS4.0.6.0.XPMCNXM, framework Android 17/API 37
and vendor Android 16/API 36. The phone was normally running HyperOS on
slot b, with DSU off. No slot was changed. This phone has its own slot
history; another device's faulty slot does not establish anything about it.

## Matching stock recovery and rollback copies

The profile now pins the exact OS4.0.6.0 OTA, replacing the older 4.0.4.0
input. Reading only 435,331 bytes of the official 10,247,941,315-byte ZIP
provided its payload manifest and recovery partition hash. A complete
read-only copy of the phone's recovery_b independently matches that hash:

| Original partition | Bytes | SHA256 |
| --- | ---: | --- |
| recovery_b, active stock 4.0.6.0 | 104857600 | `49622932fb32ab623db336866734ad440a2e15af5f750ceb9b1d3d0f7fdabafc` |
| recovery_a, different original contents | 104857600 | `586abf2b0388087cd0f1a8c86244b6ca2ee4ed8e7e72428927d0e93d98964660` |

Both complete original partitions are saved locally with verified hashes,
outside Git. The a copy is not substituted for b or identified as the
4.0.6.0 recovery. Both have boot header v4, no kernel and an AVB footer.
They are not standalone `fastboot boot` images. Updated CI extracted the
official partition and checked its complete hash before building the candidate.
A separate 90,202,112-byte temporary v4 container was then made locally
using the phone's current boot_b kernel and the unchanged CI ramdisk. It
was sent with fastboot boot under explicit temporary-test authorization.
The v4 attempt reached HyperOS with the supplied command-line marker but
without the custom ramdisk files. A header-only v3 variant was also
accepted, but exposed no usable ADB/fastboot path. Windows saw an Android
WinUSB function with problem code 10; the owner reported the combined
black-screen-or-fastboot choice. No printk or program-hash evidence proved
Skyward executed in that attempt. A power-key restart returned to HyperOS.
The phone remains on b with DSU off; complete boot_b and both original
recovery hashes were unchanged. No partition-flash, slot-switch, format or
PIN-attempt commands were issued. See `myron-validation.json`.

Public source metadata is recorded in `myron-stock.json`. This is not a
claim that the whole 10 GB OTA ZIP was downloaded or its full-file MD5
checked during manifest inspection.

## Services and hardware

| Area | Observed evidence | Candidate status |
| --- | --- | --- |
| Display | Live DRM lists 1200x2608 at 120/90/60 Hz; resolution property reports 1200,2608,480 | Size/DPI confirmed; recovery display not tested |
| Touch | FocalTech 3683, focaltech_ts on event6, raw flag 1, resolution factor 100; absolute maxima 119999x260799 | Stock touchfeature process configured; no speculative raw=0 write |
| Touch processing | Actual process maps include libtouchreport.so, libtouchreport_alg.so, libtouchreport_hal.so, libtouchreport_sensor.so and TensorFlow models | Use stock system:system/input, SYS_NICE, AIDL/default; processing in recovery untested |
| TEE helpers | Stock qseecomd and hlosminkdaemon rc files; live listeners-ready property true | Preserve identities/capabilities and wait for listeners |
| Default KeyMint | QTI KeyMint4/default, remote provisioning, secure clock, shared secret/default confirmed in VINTF and live service list | Configured; recovery key operations untested |
| Gatekeeper | QTI Rust AIDL/default, system:system | Configured; recovery credential verification untested |
| Native eSE | eSE1 belongs to the QTI secure-element process, not the NXP NFC process | Start stock QTI service as system:nfc/system before StrongBox |
| StrongBox | NXP StrongBox3 and sharedsecret/strongbox; live UID 2910, primary GID 0; native HalToHalTransport | Configured with its real shared-secret peer; recovery negotiation untested |
| Weaver | NXP Weaver2/default, UID/GID 2911; stock ese_weaver.so uses OmapiTransport | Native source component now bundled; override, applet and PIN operations remain untested |
| Wi-Fi | qca_cld3_peach_v2/rfkill/cfg80211, cnss-peach, peach_v2 configuration | Existing module/supplicant scaffold; no recovery scan/association test |
| Boot control | Stock QTI AIDL BootControl and separate recovery_a/b | Configured early; recovery query/write operations untested |

The touch device is handed to system with its observed mode 0666. Only
sysfs nodes that both exist on this phone and match its stock init rules
are included in the system:system/0664 ownership list. Merely seeing
libtouchreport_alg_fts.so on disk is not used as evidence that it is the
algorithm loaded by the running process.

The QTI eSE program needs libEseUtils from vendor/lib64/hw. Its and NXP's
libhardware dependency needs libvndksupport, which is absent from the
stock recovery ramdisk; the installed system and bootstrap library paths
are explicit fallbacks. This was established from ELF dependencies and
the stock ramdisk inventory, not a successful recovery launch.

## Source-built Weaver transport; runtime still unverified

The stock NXP ese_weaver.so constructs OmapiTransport. Its initialization
looks up `android.se.omapi.ISecureElementService/default`. Stock process
identity confirms this service and `secure_element` both belong to
`com.android.se`, loaded by app_process64. Recovery has no Java framework
to supply that process. A registered Weaver binder alone therefore does
not establish that Weaver reads or PIN decryption can work.

The stock NXP StrongBox uses the separate native AppletConnection /
HalToHalTransport path to QTI's eSE1. `native/nxp-weaver/` now contains a
source-only build kit for the Weaver alternative. It compiles the direct
transport and Stable AIDL V1 clients into the replacement library and
hides their C++ interfaces; it does not reuse the incompatible private
layout of the vendor transport class. StrongBox keeps its original library.

The kit uses pinned public NXP/AOSP source, platform headers and six public
Clang files. It rebuilds its CRT, extracts seven hash-verified libraries
from the official recovery for linking, and needs no full Android tree or
private vendor input. The regular myron workflow builds it from source and
passes `--library .../ese_weaver.so` to the packer. Omitting that library
with the new profile is an error, not an image silently using the old path.
Only Weaver searches `/skyward/lib64` first; vendor partitions are unchanged.

The standalone offline build produced 113360 bytes with SHA256
`09a4095831aae55a4589606da194c0bd7f1369a3195316af643810b4136fc5a2`.
One hundred imports and their GNU versions are supplied by the exact REC
libraries. The public 72-byte vtable order and SlotInfo/ReadResp/WeaverImpl
sizes 12/32/24 match the observed client interface. Thirteen fake-transport
and parser scenarios passed ASan/UBSan. The source removes automatic
operation replay: only a fresh, explicit SELECT-not-found response permits
switching to another of the three exact stock AIDs; an ambiguous transport
failure stops. None of those tests contacts hardware or authenticates a user.

Runtime validation remains mandatory. Check the mapped library path and
hash, process auxv/AT_SECURE and capabilities before the read-only getConfig
query; do not proceed to any PIN/key operation if the override or native
service access is unconfirmed. See the kit README for build and test records.

## Data policy and validation limits

The first candidate retains read_only=true, auto_repair=false and
key_upgrades=false. The stock fstab uses F2FS, fscrypt v2,
inlinecrypt_optimized and wrappedkey_v0. The policy also disables the
packer's automatic metadata-key deletion. It is not a device-wide write
blocker: explicit wipe/repair, vendor HAL state and the inherited persist
mount are separate concerns. No keys, credentials or application data were
read, and no phone mount, daemon start, sysfs write, reboot or flash was
performed during the initial collection. A later, separately authorized
temporary-boot round is recorded above. The already-root shell was used as found.

Before temporary boot testing, prepare a matching kernel-bearing test
container and verify a return path to normal HyperOS. Start with display,
USB, touch and service diagnostics; PIN/CE and writable-data acceptance
remain separate real-device tests. Haptics strengths remain inherited and
unmeasured.

The initial profile's Linux/ARM64 CI passed at a188aa5
([run 43063699023](https://github.com/Dingtaiqi/skyward/actions/runs/43063699023)).
For this continuation, the targeted myron data-policy/service contract ran
one test and passed, and config all-target Clippy passed with warnings
denied. Updated Linux/ARM64 CI at 79dec9e also passed all jobs, including
full workspace tests, ARM64 checks/build and stock recovery extraction/hash
verification ([run 43151155964](https://github.com/Dingtaiqi/skyward/actions/runs/43151155964)).
The full path-validation suite is not Windows portable; Linux CI executes
the complete suite. None of these checks is a recovery boot test.

## Optional development trace image

`skyward-pack recovery --myron-diagnostics` (or the manual workflow's
matching input) is a separate development image, not the default build.
It starts a bounded early trace independently of the recovery UI. Only on
myron/b, with the actual metadata partition already mounted F2FS/rw and
at least 4 MiB free, it creates a new root-only directory named
`/metadata/skyward-myron-trace-<boot UUID>`. Otherwise output stays in RAM.
It does not mount/remount, repair, format, read keys, write /data or reboot.
The six samples include whitelisted USB properties, process/device lists,
Weaver library identities/maps, capability and AT_SECURE metadata, and
bounded recovery/kernel/logd output. Raw traces are private diagnostic
material and must not be published unreviewed.
Each sample file is capped at 128 KiB; all six logs total at most 768 KiB.

Booting this variant requires separate authorization for those metadata
log writes and the chosen boot/installation method. It has not been run
on the phone. Existing authorization for the completed RAM-only test does
not authorize flashing it or starting another phone test.
