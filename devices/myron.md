# myron: stock evidence and recovery bring-up

This is an experimental recovery port with incomplete release acceptance.
Native recovery_b tests verified display/touch, exact running payloads,
StrongBox startup, both shared-secret peers and Weaver getConfig (64/16/16).
A separately authorized 759d4d2 test also converted an inner wrapped storage
blob after one temporary renewal, yielding 100 bytes. Original disk-blob
hashes were unchanged, and normal HyperOS unlock/files were confirmed after
restoration. Integrated read-only mounting and correct-PIN/CE access passed
in 34fe498, with original folders visible. The 2a586d1 round verified writable
data/sdcard mounts, and aa35835 passed all three available logical mount
switches. MTP payload transfer and enforcing-mode acceptance remain incomplete.
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
build.prop selection had set both to 2026-10-01. This mismatch remains
recorded evidence. The later wrapped-storage investigation below established
the separate temporary-conversion path without guessing patch dates or
enabling general/credential key upgrades.

## Services and hardware

| Area | Observed evidence | Candidate status |
| --- | --- | --- |
| Display | Live DRM lists 1200x2608 at 120/90/60 Hz; resolution property reports 1200,2608,480 | Size/DPI and native recovery display confirmed by owner |
| Touch | FocalTech 3683, focaltech_ts on event6, raw flag 1, resolution factor 100; absolute maxima 119999x260799 | Native recovery touch confirmed by owner; no speculative raw=0 write |
| Touch processing | Actual process maps include libtouchreport.so, libtouchreport_alg.so, libtouchreport_hal.so, libtouchreport_sensor.so and TensorFlow models | Stock service ran as configured; owner confirmed responsive touch |
| TEE helpers | Stock qseecomd and hlosminkdaemon rc files; live listeners-ready property true | Preserve identities/capabilities and wait for listeners |
| Default KeyMint | QTI/default, remote provisioning, secure clock and shared secret/default confirmed; hardware info says OneKeymint version 1, TEE, Qualcomm | Inner conversion, PIN/CE and writable mounts passed; general upgrades remain disabled; MTP payload follow-up pending |
| Gatekeeper | QTI Rust AIDL/default, system:system | Configured; recovery credential verification untested |
| Native eSE | eSE1 belongs to the QTI secure-element process, not the NXP NFC process | Stock driver/node and QTI callback delivery passed |
| StrongBox | NXP StrongBox3 and sharedsecret/strongbox; live UID 2910, primary GID 0; native HalToHalTransport | 9f RAM dependencies fixed loading and passed startup ready; UID/GID/caps/AT_SECURE confirmed |
| Weaver | NXP Weaver2/default, UID/GID 2911; original executable uses private native library bind | f32 actual mapping/getConfig passed; 34fe498 correct-PIN recovery unlock and original-file access passed |
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

## d923 native mapping acceptance and next NFC dependency

The d923 image passed full CI (run 43183345626) and a fresh native test.
Program, config, scripts and partition hashes matched. The proper RAM CLI
alias was verified with --help. Weaver, native eSE1 and StrongBox were each
described and registered from cold startup, without a RAM manifest edit.

The Weaver mapping gate passed: original executable, UID/GID 2911/2911,
capabilities 0000000800a00000 and AT_SECURE=1 were preserved. The private
child view's SO hash was 09a409..., and all four mappings matched its
actual device/inode and the bundled RAM file. Host, StrongBox and eSE
continued to see original 556b6e... . SELinux was permissive as found;
enforcing operation was not tested. With the corrected CLI invocation,
USB remained available at uptime 210 seconds without an operator toggle.

One actual read-only getConfig call reached GetSlots and transport Send,
then returned service error 1, Failed to retrieve slots info. It was not
retried. No slot read/write, PIN or credential command was issued. Startup
shared-secret parameter collection separately returned -49 without naming
the peer. A trace shows QTI repeatedly missing /dev/nq-nci and the STM node;
service registration is not chip readiness. The exact original recovery
was restored, HyperOS boot completed, and recovery_a/b plus boot_b full
hashes matched again. No persistent trace directory was found.

Subsequent normal-system reads confirmed /dev/nq-nci is 0660 nfc:nfc by
/vendor/etc/ueventd.rc:411. The stock 85288-byte nxp-nci.ko has SHA256
`ae9a9fcf262e29ca5be19c5db733174baec47d5978acb2d864bbf05dc2de6735`.
Its build-id matches the loaded nxp_nci module. It is the NXP I2C driver;
the separately loaded nfc protocol module is not the same file. The module
loader resolves nxp-nci by filename through modules.dep, which lists
smcinvoke_dlkm. This read-only collection did not load a module, change
permissions, restart a daemon or touch credentials.

The profile now also requests that exact stock driver and observed node
ownership before eSE startup. The 4ab round below verified module insertion, node ownership and working
shared-secret agreement. Weaver callback initialization remained a separate
failure. Do not change daemon identity, guess a backend property,
retry authentication or enable key upgrades to bypass the present error.
See myron-nfc-evidence.json for the source metadata without driver bytes.

## 4ab native backend acceptance and callback stability failure

The 4ab image passed CI run 43188698396 and a separately authorized native
test. The exact stock nxp-nci module loaded and /dev/nq-nci appeared as
1027:1027/0660. The private Weaver mapping gate passed again. Startup now
completed shared-secret agreement between both peers, resolving the prior
-49 for this configuration. Weaver and eSE1 were declared and registered.

One actual getConfig still failed. Full private logs identify the leaf:
QTI rejected ISecureElementCallback as a system-stability binder used in a
vendor-stability context, and AppletConnection initialization failed. No
Weaver SELECT/GETSLOTS APDU was sent. GetSlots Exit106 is a common return
path; its adjacent generic parse-error text does not prove ParseSlotInfo
was called. No PIN or slot operation was attempted and no query was retried.

The generated callback already contains the standard VINTF mark under
BINDER_STABILITY_SUPPORT, but the original recipe omitted that macro. The
next source change enables the generator's normal path with the matching
pinned binder_stability.h and verifies its C-ABI import from exact REC
libbinder_ndk. It does not alter generated method definitions, daemon
identity or Binder enforcement. The component must pass strict ABI/source
checks and independent-output reproducibility before another device test.

The original recovery was restored again, normal HyperOS verified, and
full recovery_a/b and boot_b hashes matched pre-test. Storage-key -62 and
the no-upgrade policy remain unchanged. The callback fix is not yet an
accepted device result; see the structured validation record.

## f32 native Weaver configuration acceptance

The f32dee0 callback-stability revision passed complete CI run 43194221841,
13 sanitizer-backed transport tests, strict ABI/import checks and two
byte-identical independent-output native builds. The checked image contains
the same library as that run's native artifact. Its callback factory really
calls AIBinder_markVintfStability, provided strongly by exact REC
libbinder_ndk at LIBBINDER_NDK30.

A separately authorized recovery_b-only native test then passed program,
configuration, module/node, executable identity, capabilities, AT_SECURE and
actual child-view hash/inode mapping gates. Exactly one read-only getConfig
succeeded: **64 slots, 16-byte keys and 16-byte values**. Logs show QTI's
callback connected, transmit completed and ParseSlotInfo returned 64. No
credential or slot read/write was issued. Both shared-secret peers agreed.

This establishes callback delivery and configuration retrieval, not PIN
decryption. Data stayed closed because its existing storage key still
returned -62 with upgrades forbidden. Keep that preservation policy; do not
guess patch dates, upgrade keys or infer credential readiness from getConfig.
The observed SELinux state was permissive as found; enforcing-mode operation
has not been accepted.

Six RAM diagnostic samples totalled 764850 bytes, within the 768 KiB cap.
MTP/ADB remained available through the observed interval. ADB can report
the valid transport state `recovery`; waiting exclusively for `device` can
mistakenly report a connected recovery as unavailable.

The test ended with original recovery_b restored and normal HyperOS on slot
b, DSU off. Full recovery_a/b and boot_b hashes match the pre-test values.
No persistent trace directory was found. The one-round authorization ended;
further boot, credential or data-write tests need their own agreed scope.

## Wrapped storage-key conversion follow-up

The f32 failure occurs after StoredKey.unwrap has already decrypted the
on-disk envelope. The failing input is its inner hardware-wrapped storage
blob, not the outer keymaster_key_blob or a PIN operation. The existing
transaction code 13 and single byte-array argument match IKeyMintDevice.

Android keystore2 handles a conversion's KEY_REQUIRES_UPGRADE with one
upgradeKey using empty parameters, then one conversion of the result.
Vold explicitly ignores the returned upgraded inner blob and keeps the
original; repeated temporary renewal is expected for implementations that
require it. A normal Android data mount therefore does not prove that
direct conversion with upgrades forbidden should succeed.

Pinned upstream references:

- [keystore2 conversion fallback](https://android.googlesource.com/platform/system/security/+/d43c1b9db3510dd5c25d35a34b926389bfeb2d33/keystore2/src/security_level.rs)
- [vold retains the inner storage blob](https://android.googlesource.com/platform/system/vold.git/+/03a059177ec42321e2c54f9fbe2b5c6706752217/Keystore.cpp)
- [IKeyMintDevice contract](https://android.googlesource.com/platform/hardware/interfaces/+/a742007dfaf46b6e23022ff1ff92c0983b2c2b98/security/keymint/aidl/android/hardware/security/keymint/IKeyMintDevice.aidl)

The manual `skyward-cli storage-key` diagnostic requires the installed
fstab's wrapped-storage flag, unwraps only that metadata key directory
with general upgrades disabled, and attempts conversion. Only an explicit
`--renew-inner` permits the single -62 -> upgradeKey -> convert fallback.
It reports length and whether renewal occurred, never the blob contents.
There is no mount, device-mapper creation, credential input, blob file
write or deleteKey call. Empty results and further errors stop the path;
an error on the second conversion is not retried.

In 759d4d2 the automatic UI/data policy remained unchanged. A separately
authorized single native diagnostic succeeded: the inner renewal occurred
and conversion returned 100 bytes. Original disk-blob hashes matched normal
HyperOS preflight, both recovery checks and post-restoration HyperOS. The
three full partition hashes also matched; HyperOS mounted data and the
owner confirmed normal unlock and original files. No data mount, PIN or
Weaver slot operation was issued in recovery.

The next integration adds `decrypt.storage_key_renewal`, default false and
explicitly true only for myron. General/credential upgrades remain false,
filesystem repair stays off, and data access remains read-only. The packer
still disables automatic metadata key deletion. The data path may opt into
only the tested inner conversion fallback; this integrated mount/PIN path
has not been hardware-accepted. The permission applies to each conversion
of an inner storage blob, including metadata and DE/shared storage, and CE
storage after credential verification. Each conversion may renew once on
-62; multiple data-open requests can perform new conversions. Outer and
credential KeyMint blobs still cannot be upgraded under this policy.

An upgradeKey call is a secure-world key operation, **not read-only evidence**.
The general AIDL version-binding contract includes old-version invalidation;
keeping a copy in RAM does not guarantee a vendor implementation has no
secure-world side effects. Any test needs separate authorization covering
that operation, exact candidate/identity gates and subsequent original
recovery/HyperOS restoration. Do not represent a partition restore as a
rollback of secure-world state.

The integrated 34fe498 image subsequently passed native read-only mounting
and one owner-entered correct PIN. Both data and sdcard were verified F2FS
read-only/norecovery; the recovery log's closed-user list became empty and
the owner saw original folders. The first evidence probe had run while the
UI was still idle; the owner then pressed Decrypt once. No decrypt CLI retry
or host-supplied credential was used. The metadata ciphertext hashes stayed
unchanged, six RAM samples totalled 764469 bytes, and the original recovery
and three full partition hashes were restored and verified. Normal HyperOS
mounted data, and the owner confirmed normal unlock/files again.

The next profile permits writable access while retaining auto_repair=false,
key_upgrades=false and storage_key_renewal=true. Automatic metadata-key
deletion remains disabled. This writable configuration is not yet hardware
accepted; its next test must restrict writes to an explicitly owned small
payload and verify cleanup. MTP payload transfer and other untested recovery
operations must not be inferred from PIN success.

A fresh normal-HyperOS read-only inspection confirmed OS/vendor properties
2026-10-05, whereas the actual f32 startup log used 2026-10-01 for both.
This difference remains evidence to investigate; no patch date was changed,
and matching those strings is not established as a fix for the inner blob.

## Logical mount toggle correction and native acceptance

The 2a586d1 RW round verified data/sdcard writable and the recovery log
reported all users unlocked. The owner confirmed brightness changes and
Wi-Fi hotspot visibility. The MTP payload test was not performed: PC
PnP/WPD identity matched, but Shell device identity properties were absent.
No fixture directory/file was created. The owner clarified that the reported
return to HyperOS was the controller's restoration, with no confirmed key
fault. Power/wake and touch after wake still need a separate acceptance check.

The owner found every available logical mount switch reported failure.
Read-only evidence showed vendor_dlkm absent in the parent namespace while
Weaver's private namespace still mounted the original RO dm-6, named
vendor_dlkm_b. The prior unmount_one removed the parent filesystem, then
tried to delete a DM device still held by the service namespace. The normal
mount path then unconditionally created a same-name DM, which already
existed. This left the displayed failure after a partial state change.

The source correction makes the UI's single-partition unmount remove only
its filesystem view. A normal remount reuses an existing DM only after
verifying RO and every linear/zero extent against this slot's LP metadata;
wrong type, backing device, offset, lengths, gaps and overflow are refused
without logging target parameters. A failed remount does not delete an
existing shared DM. Strict full unmap/OTA cleanup, snapshot handling and
the already-shown mount path keep their prior behavior. Eight boundary
tests accompany this change. Complete CI 43228377535 passed for aa35835.

During evidence collection the original mi_ext_b, mi_product_b and
vendor_dlkm_b EROFS RO views were restored using their original still-mapped
devices. No table was reloaded or service identity changed. The round then
ended with original recovery_b, normal HyperOS, all three partition hashes
and metadata ciphertext hashes verified. No test file or persistent trace
was left. The owner subsequently entered FASTBOOT manually; no new flash
authorization has been inferred from that action.

In a separately authorized aa35835 short test, vendor_dlkm was turned off
and back on. Kernel evidence confirmed the parent filesystem view vanished,
the original RO DM device remained, and remount reused that same device as
EROFS RO. Weaver kept its private mount, native-library hash and process
identity throughout. The owner also cycled mi_ext and mi_product off/on;
both reported success, and all three final RO views used their original DM
devices. No data mount, PIN or user-file operation occurred in this round.

A native Windows WPD identity query uniquely bound an endpoint to the
physical phone through its USB parent and container. No WPD content session
was opened and no file was transferred; this establishes identity binding
only. Six RAM diagnostic samples totalled 761812 bytes, within 768 KiB,
with no persistent diagnostic directory found. Original recovery_b and
normal HyperOS were restored, still on b with DSU off. Full recovery_a,
recovery_b and boot_b hashes matched, as did the metadata ciphertext hashes.

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
