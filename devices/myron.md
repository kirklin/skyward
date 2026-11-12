# myron: stock evidence and recovery bring-up

This is an experimental recovery candidate, not a tested release. After two
inconclusive temporary-boot attempts, a separately authorized native
recovery_b boot reached Skyward. The owner confirmed display and touch;
ADB and exact running payload hashes were verified. The later 9f round
also passed StrongBox startup. USB reliability, Weaver acceptance, PIN
decryption and writable-data acceptance remain incomplete.
The original recovery was restored and verified. No myron PR is submitted.

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

## Native recovery test on 2026-11-12

The diagnostic candidate at `de63f37` passed the complete public-source CI
([run 43161796967](https://github.com/Dingtaiqi/skyward/actions/runs/43161796967)).
Its 104857600-byte native image SHA256 is
`37eb708f04eaab1ad76e31d23cac22345a64199e0c747deedc2e88d3564a88c0`.
It has header v4 and no kernel; it is not a standalone fastboot-boot image.

The owner separately authorized writing only recovery_b, bounded metadata
trace logs and restoration. Product, b slot, no pending snapshot merge,
BCB, candidate and rollback hashes were checked first. The running recovery,
profile, library file and flashed partition hashes matched the CI artifact.
The owner confirmed the Skyward home and responsive touch. ADB was available
with state `recovery`, and mtp,adb configured with an opened host MTP session.
No file-transfer acceptance test was performed. The recovery log also recorded
a Wi-Fi lease and successful external reachability probe.

StrongBox repeatedly exited because vendor libhardware could not load
libvndksupport.so. Its readiness timeout prevented the later Weaver service
from starting; the bundled replacement file has not been proven mapped or
executed. No Weaver getConfig, slot access or PIN command was issued. The
data partition stayed closed: the attempted storage-key setup returned -62
and the diagnostic policy refused the requested key upgrade. This is not
PIN decryption success and not a device-wide write blockade.

A later USB disconnect left no device visible to the host. The owner used
the working UI to enter bootloader, then the exact original recovery_b was
flashed back. HyperOS boot_completed, b slot and DSU off were verified;
full hashes of restored recovery_b, unchanged recovery_a and boot_b matched
the pre-test values. Six bounded diagnostic samples had stayed in RAM; no
persistent trace directory was found. The full RAM trace pull failed after
the disconnect. Reviewed log excerpts were collected before restoration.

Read-only follow-up on normal HyperOS found security.capability attributes
on both stock StrongBox and Weaver executables and AT_SECURE=1 for each.
Thus an environment-only LD_LIBRARY_PATH override is insufficient; do not
claim that the replacement works merely because its file exists. Normal
system/vendor patch properties were 2026-10-05, while recovery's installed
build.prop selection had set both to 2026-10-01. Record and resolve this
mismatch before another storage-key attempt; do not enable key upgrades
merely to move past the refusal.

## Services and hardware

| Area | Observed evidence | Candidate status |
| --- | --- | --- |
| Display | Live DRM lists 1200x2608 at 120/90/60 Hz; resolution property reports 1200,2608,480 | Size/DPI and native recovery display confirmed by owner |
| Touch | FocalTech 3683, focaltech_ts on event6, raw flag 1, resolution factor 100; absolute maxima 119999x260799 | Native recovery touch confirmed by owner; no speculative raw=0 write |
| Touch processing | Actual process maps include libtouchreport.so, libtouchreport_alg.so, libtouchreport_hal.so, libtouchreport_sensor.so and TensorFlow models | Stock service ran as configured; owner confirmed responsive touch |
| TEE helpers | Stock qseecomd and hlosminkdaemon rc files; live listeners-ready property true | Preserve identities/capabilities and wait for listeners |
| Default KeyMint | QTI KeyMint4/default, remote provisioning, secure clock, shared secret/default confirmed in VINTF and live service list | Service ran; storage-key setup refused -62; key upgrades remain disabled |
| Gatekeeper | QTI Rust AIDL/default, system:system | Configured; recovery credential verification untested |
| Native eSE | eSE1 belongs to the QTI secure-element process, not the NXP NFC process | Start stock QTI service as system:nfc/system before StrongBox |
| StrongBox | NXP StrongBox3 and sharedsecret/strongbox; live UID 2910, primary GID 0; native HalToHalTransport | 9f RAM dependencies fixed loading and passed startup ready; UID/GID/caps/AT_SECURE confirmed |
| Weaver | NXP Weaver2/default, UID/GID 2911; stock ese_weaver.so uses OmapiTransport | 9f reached service but missed VINTF fragment; RAM correction reached thread loop; mapping/getConfig/PIN still pending |
| Wi-Fi | qca_cld3_peach_v2/rfkill/cfg80211, cnss-peach, peach_v2 configuration | Recovery log records DHCP lease and external reachability; repeatability untested |
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
Only Weaver receives the replacement in its private mount view; vendor
partitions are unchanged. AT_SECURE prevents the earlier environment-only
search order from being sufficient.

The standalone offline build produced 113360 bytes with SHA256
`09a4095831aae55a4589606da194c0bd7f1369a3195316af643810b4136fc5a2`.
One hundred imports and their GNU versions are supplied by the exact REC
libraries. The public 72-byte vtable order and SlotInfo/ReadResp/WeaverImpl
sizes 12/32/24 match the observed client interface. Thirteen fake-transport
and parser scenarios passed ASan/UBSan. The source removes automatic
operation replay: only a fresh, explicit SELECT-not-found response permits
switching to another of the three exact stock AIDs; an ambiguous transport
failure stops. None of those tests contacts hardware or authenticates a user.

Runtime validation remains mandatory. Check the mapped device/inode and
child-view hash, process auxv/AT_SECURE and capabilities before the read-only getConfig
query; do not proceed to any PIN/key operation if the override or native
service access is unconfirmed. See the kit README for build and test records.

## Follow-up capability-aware loading candidate

The next candidate keeps the stock executables, UIDs/GIDs, file capabilities
and exec paths. StrongBox requests only two missing, SHA256-pinned installed
libraries: libvndksupport.so and bootstrap/libdl_android.so. The recovery
supplies them under RAM /system/lib64 before starting that service, reuses
only identical existing files and refuses conflicting files. The nineteen
strong imports across these two files match the exact REC libraries/linker,
including their GNU versions; see `myron-recovery-libraries.json`. No libc,
libc++, Binder or installed-partition library is replaced.

Weaver requests a read-only bind of the bundled native SO onto the stock
SO path in its own private mount namespace, before the original executable
runs as its normal account. Other processes retain their original view.
The environment library path is no longer the replacement mechanism. A
same-executable process from an earlier UI instance must also match the
requested library inode in its own mount view before adoption. Pinning uses
the process's namespace rather than opening a possibly different host path.

The follow-up at `9f0adfb` passed full workspace tests, the actual sudo
child-namespace test, ARM64 lint/build and image packaging
([run 43171699583](https://github.com/Dingtaiqi/skyward/actions/runs/43171699583)).
The 104857600-byte image SHA256 is
`884f9e57fb9c113efa6a4dca396dad66ca33c7b91512f9aba2becd5e16d513d7`.
Its program exactly matches the same run's tools artifact; profile and
trace scripts match the source commit, and the native SO remains `09a409...`.
This follow-up was boot-tested in a separately authorized round, described
below. A further test requires fresh myron identity/slot checks and agreed
scope; never use it on socrates.
Preserve SELinux enforcing policy;
check the source inode's label, the actual service domain and denied mmap
operations. A bind retains the source inode's label. Before getConfig,
verify process identity/caps/AT_SECURE, the library mapping's device/inode,
and SHA256 through /proc/PID/root/odm/lib64/ese_weaver.so. The host view must
still contain the original library, including for StrongBox and QTI eSE.
Neither a binder registration nor a matching filename establishes this.

## 9f native round and explicit Weaver fragment fix

The owner confirmed display and touch. Program, profile, bundled SO and
partition hashes matched. StrongBox passed startup readiness with its
original executable, UID/GID 2910/0, capabilities 0000000800a00000 and
AT_SECURE=1. Its two missing RAM dependencies matched the pinned hashes.
The host, StrongBox and QTI eSE retained the original Weaver SO hash.
SELinux was observed permissive; this round did not change its mode or
policy, and enforcing operation has not been validated.

Weaver then aborted on service registration. Its stock v2 manifest spells
IWeaver and default in separate interface/instance elements; the generic
fqname text filter missed it. A diagnostic RAM-only copy converted that
actual fragment to framework type, followed by servicemanager restart and
a supervised restart of QTI eSE. Weaver was then observed waiting in its
Binder thread loop. Subsequent USB loss prevented the full child-view
mapping/auxv gate, so no getConfig, slot access or PIN command was issued.
The profile now explicitly names that existing stock fragment via
service.description. This follow-up configuration needs a fresh boot test;
the RAM adjustment is not proof that the previously archived image works
without intervention.

USB was initially adb and later mtp,adb. Repeated loss left no host USB
node, without an owner cable/switch action at the first loss. The owner
confirmed that toggling File Transfer in recovery restored connectivity;
this is a workaround and not proof that reboot is required. Subsequent
source audit identified an operator diagnostic error: invoking
/system/bin/recovery service ... does not run a subcommand. The basename
recovery unconditionally enters the recovery startup path, which resets
non-adb USB through none before requesting adb. These calls are a concrete
trigger sufficient to explain the observed losses; independent USB
instability has not been established. A private tar of six bounded RAM
samples and the recovery log was saved. The owner returned to HyperOS before the remaining
query. The candidate was still in recovery_b, so it was subsequently
restored with the exact original and normal HyperOS was verified again.
Full recovery_a, restored recovery_b and boot_b hashes matched pre-test.
No persistent trace directory was found.

Read-only normal-system provenance collected twelve static property files.
The raw replay and last writers still gave 2026-10-01, unlike the live
2026-10-05 properties. Absolute REC import results remain unknown; do not
attribute the discrepancy to imports without evidence. No property date
was changed and key upgrades remain disabled. UI-triggered storage-key
setup again stopped at -62 with data closed.

## Diagnostic executable name

After verifying product, slot and payload, create a RAM symlink to the
same binary with a non-recovery basename, such as /skyward/skyward-cli.
Confirm its --help lists service and weaver, then use that alias for CLI
queries. Do not append diagnostic arguments to /system/bin/recovery:
its unconditional basename dispatch runs UI startup and can reset USB,
read/clear BCB and redo bring-up. No CLI query result can be claimed from
those mistaken invocations. The failed mapping/getConfig attempt in this
round did not issue a getConfig call.

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
The next diagnostic variant samples at about 0/15/30/45/75/105 seconds and
includes the gadget UDC binding, function links and FFS readiness, covering
UI/MTP startup that the earlier 0..25-second trace missed.

Booting this variant requires separate authorization for those metadata
log writes and the chosen boot/installation method. The separately authorized native test above ran this variant. That test
has ended and the original recovery is restored; further reboot/flash or
credential tests require their own agreed scope.
