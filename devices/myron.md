# myron: stock evidence and recovery bring-up

This is an unbooted recovery candidate, not a tested release. No myron PR
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
They are not standalone `fastboot boot` images. CI must still extract the
official partition and check its complete hash before building a candidate.

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
| Weaver | NXP Weaver2/default, UID/GID 2911; stock ese_weaver.so uses OmapiTransport | Java dependency remains a PIN blocker, described below |
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

## Remaining Weaver transport dependency

The stock NXP ese_weaver.so constructs OmapiTransport. Its initialization
looks up `android.se.omapi.ISecureElementService/default`. Stock process
identity confirms this service and `secure_element` both belong to
`com.android.se`, loaded by app_process64. Recovery has no Java framework
to supply that process. A registered Weaver binder alone therefore does
not establish that Weaver reads or PIN decryption can work.

The stock NXP StrongBox uses the separate native AppletConnection /
HalToHalTransport path to QTI's eSE1. AOSP's NXP Weaver source also has a
HAL-to-HAL selection when OMAPI_TRANSPORT is not defined. That source
route is under investigation. Any replacement must match the installed
transport C++ ABI, exported Weaver entry points, applet selection and
error behavior before it is used. No vendor binary was patched, replaced
or executed as part of this read-only inspection.

## Data policy and validation limits

The first candidate retains read_only=true, auto_repair=false and
key_upgrades=false. The stock fstab uses F2FS, fscrypt v2,
inlinecrypt_optimized and wrappedkey_v0. The policy also disables the
packer's automatic metadata-key deletion. It is not a device-wide write
blocker: explicit wipe/repair, vendor HAL state and the inherited persist
mount are separate concerns. No keys, credentials or application data were
read, and no phone mount, daemon start, sysfs write, reboot or flash was
performed during collection. The already-root shell was used as found.

Before temporary boot testing, prepare a matching kernel-bearing test
container and verify a return path to normal HyperOS. Start with display,
USB, touch and service diagnostics; PIN/CE and writable-data acceptance
remain separate real-device tests. Haptics strengths remain inherited and
unmeasured.

The initial profile's Linux/ARM64 CI passed at a188aa5
([run 43063699023](https://github.com/Dingtaiqi/skyward/actions/runs/43063699023)).
For this continuation, the targeted myron data-policy/service contract ran
one test and passed, and config all-target Clippy passed with warnings
denied. Updated Linux/ARM64 CI is still required for the changed profile.
The full path-validation suite is not Windows portable; Linux CI executes
the complete suite. None of these checks is a recovery boot test.
