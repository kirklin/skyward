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
switches. A later aa35835 round passed native MTP upload/readback/file deletion,
volume/power/wake/touch, vibration-strength changes and BootControl reads.
Empty-directory cleanup through Windows WPD and enforcing-mode acceptance
remain incomplete. The same post-delete Windows WPD stall was also observed
under normal HyperOS; this does not establish a shared root cause.

The latest native candidate, eeb2412, passed complete CI and the corrected
recovery-preservation path on the phone. An official OS4.0.6.0 full OTA
completed from b to a; the new recovery then booted natively from a with
matching running hashes. Updated HyperOS booted on a and the owner confirmed
normal unlock, desktop and original files. Skyward was retained in both
recovery partitions before returning to HyperOS. This was a same-Kept-API
retry after the original preservation failure, not a second full OTA using
the corrected callback. Metadata EBUSY/overwrite fallback and premature
100% UI progress remain open; see the final section for the exact scope.

The dated sections below retain the earlier test stages and restorations;
they are historical evidence, not the latest phone state. Compatibility is
tested on OS4.0.6.0.XPMCNXM (framework Android 17/API 37, vendor API 36).
Other firmware versions have not been accepted.

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

## Writable MTP follow-up on the same aa35835 image

A new, separately authorized native boot again passed startup service and
payload checks, one correct PIN entered only on the phone, writable data
and sdcard mounts, and the latest all-users-unlocked log state. The owner
confirmed volume-key focus, short-power screen off/wake, touch after wake,
and a clearly stronger vibration at the high slider setting. A read-only
BootControl query returned current and active slot b; no slot was changed.

One self-generated 65536-byte file was created in an exclusive Download
directory through the uniquely bound native Windows WPD endpoint. Upload,
Commit and GetObjectID completed. The first host checker stopped because
Windows mapped this binary file to WPD_CONTENT_TYPE_UNSPECIFIED rather
than GENERIC_FILE. This was an overly narrow host-tool expectation, not
evidence of a recovery defect. A separate read-only SDK helper verified
the same created object and parent chain, read back exactly 65536 bytes,
and matched the complete SHA256 and marker. Independent phone-side reads
also matched canonical paths, both views' inode, size and hashes. No second
file was uploaded and no PIN or credential retry occurred.

A cleanup-only SDK helper then deleted the verified file successfully and
confirmed the folder was empty. It entered the empty-directory cleanup
stage and timed out before completion. Its journal does not distinguish
the exact blocked COM call or prove whether a bulk response reached USB;
neither a recovery protocol defect nor an event race is established.
After confirming the remaining directory's original canonical path and
inode, a single nonrecursive rmdir removed that empty directory. Both
alias/raw paths were absent before restoration. The complete WPD cleanup
workflow is therefore not accepted, despite successful upload, native
readback and file deletion.

Six RAM diagnostic samples totalled 764553 bytes; no persistent diagnostic
directory was found. Original recovery_b and normal HyperOS were restored,
recovery_a/b and boot_b hashes matched, and metadata ciphertext hashes were
unchanged. The owner confirmed normal unlock, desktop and original files.
The phone remains in HyperOS while the directory-cleanup timeout is
investigated offline.

The follow-up diagnostic image enables DeleteObject phase logging only
through the optional myron diagnostic script. Normal images leave it off.
The log contains the operation, transaction ID, response code and send
stage, with no object handles, paths or datasets. Responder sequence and
inotify regression tests check file deletion, empty enumeration, folder
deletion and the next transaction; they do not establish USB delivery on
this phone. Finer host SDK stages are also needed before assigning a cause.

The af568ba diagnostic image passed complete CI 43245275620 and a separately
authorized native test. Startup verified the actual enabled trace property,
the program/profile/partition hashes and the original security-service
identities. The owner again entered one correct PIN on the phone and saw
the original folders; data/sdcard were verified writable.

The v2 host client completed its single 64 KiB upload and native readback,
then verified the independent canonical/inode/hash cleanup gate. Its file
Delete call, result GetCount/GetAt and result verification all returned.
The phone recorded that DeleteObject command, an OK (0x2001) reply, and
completed bulk response transmission. The host then blocked in the
post-file-delete empty-folder enumeration (EnumObjects/Next), before any
folder-delete stage was entered. This is more precise than the prior
round's broad directory-cleanup label. Enumeration request arrival and
data transmission were not instrumented, so neither a host fault nor a
device transport fault is established.

The remaining original empty directory was checked against its canonical
path and inode and removed with one nonrecursive rmdir. No second upload
or credential attempt occurred. Original recovery_b and normal HyperOS
were restored, with all three partition and metadata ciphertext hashes
unchanged. Live DeleteObject trace and the recovery-log tail were retained;
the six-sample RAM archive was not retrieved in this round.

The next diagnostic revision extends the same opt-in trace to at most
sixteen following transactions and command reads, covering related
enumeration operations, dataset sends and a deduplicated watch-change
notification per pending read. Normal I/O decisions and event handling
are unchanged. The real-inotify test additionally checks that processing
self-delete events drains the readable watch fd before the next command.

That 0df9431 revision passed complete CI 43250797489 and a separate native
test. An early read-only startup probe stopped before credentials; its
evidence was retained, and the later service-ready probe verified the
actual running payloads, service mappings and trace property before PIN.
The owner entered one correct PIN and saw original folders. The single
own-file upload, native readback, path gate and file Delete/results passed.
The finer SDK journal then stopped before EnumObjects returned; Next and
folder deletion were never entered.

The phone completed that DeleteObject response, the subsequent
GetObjectHandles dataset/response and twelve GetObjectPropList
dataset/responses in the bounded window. It advanced past the initial
watch-change notification. This does not establish an early watch spin or
unfinished bulk send, but request scope/depth/counts were not captured,
and later activity outside the bounded window is unknown. The remaining
empty own directory was removed after canonical/inode checks, and original
recovery_b/HyperOS were restored with all three partition and metadata
ciphertext hashes unchanged. The owner confirmed normal unlock and files.
All six RAM samples were preserved (763306 bytes), with no persistent trace
directory found after restoration.

The next diagnostic adds only neutral request-marker categories, protocol
selectors and response entry counts within the existing trace budget.
It does not log object identifiers, paths, property values or file content,
and does not change enumeration, deletion, scope or PUID semantics.

The count diagnostic aa0d84e passed complete CI 43258199965 after correcting
a Rust-edition syntax issue in the diagnostic branch. Its separately
authorized native test again passed PIN/RW, one 64 KiB upload, native
readback and file Delete/results, then stopped before EnumObjects returned.
The trace showed a known-parent GetObjectHandles returning ten handles,
followed by known single-object, depth-zero ObjectFormat (0xDC02) and PUID
(0xDC41) queries returning one entry each. Parent/object identifiers were
not logged. This may be identity resolution through a parent rather than
listing the empty probe directory itself; a wrong-directory response or
PUID/cache root cause has not been established.

The original empty probe directory was checked and removed, six RAM samples
(761729 bytes) were preserved, and the original recovery/HyperOS plus all
three partition and metadata ciphertext hashes were verified. The owner
confirmed normal unlock, desktop and original files. Further code work is
offline; no additional hardware authorization is implied by these records.

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

## Normal HyperOS MTP comparison, 2026-11-13

The owner reports that MTP also hangs during copying/deletion in normal
HyperOS. A bounded read-only Windows WPD check listed 62 internal-storage
top-level objects and the first 32 Download objects in about 0.094 seconds.
That baseline did not exercise writes or exclude Windows caching.

A separately authorized single-file comparison used the same native WPD
transfer/deletion sequence as the recovery fixture. One 64 KiB upload
committed, full native readback and raw/emulated-view payload checks passed,
and the file Delete API plus its GetCount/GetAt results returned success.
The subsequent EnumObjects call on the containing folder then blocked
before returning; Next and the folder-delete operation were never entered.
The PC wrapper stopped its own client at the 75-second cap. A canonical,
inode and emptiness guard allowed nonrecursive ADB rmdir of the remaining
own empty directory. The file and directory are absent; HyperOS remained
normally booted. No reboot, flash or driver change occurred in this round.

An earlier fresh-connection control failed directory creation after about
30 seconds with WPD error E_WPD_DEVICE_IS_HUNG (0x802A0006), before upload.
For the latest file round, the owner confirmed changing USB ports, but the
saved Windows location snapshots were identical. It must not be presented
as an independently verified different-port control.

The normal-system test reproduces the recovery test's host API stall stage.
It does not prove a shared root cause or distinguish Windows/provider
caching, USB transport, phone firmware or PUID handling. Full recovery MTP
acceptance remains open; no recovery-specific fix follows from this alone.
See the sanitized control record in `myron-validation.json`.

## 官方整包安装与 recovery 保留，2026-11-13

实机从 b 槽安装 `OS4.0.6.0.XPMCNXM` 官方完整 OTA，payload 签名验证和
update_engine 执行成功，下一启动槽由更新程序设置为 a，未清除用户数据。
随后已核实更新后的 Android 17／HyperOS 从 a 槽正常启动；机主确认正常
解锁、桌面及原有文件均正常。

本次发现“安装后保留 Skyward”在写回 recovery_a 时返回 EPERM：更新程序
校验后将目标块设备留为只读，实测 BLKROGET 与 sysfs ro 均为 1。
写入修正仅临时解除明确选中的块设备的只读标志，保留原有写入、同步、读回
摘要验证，并在成功或出错时恢复原标志；普通文件不接收块设备 ioctl。
CI 的独立 root 用例只分配临时文件支持的 loop 设备，覆盖实际只读写入失败、
修正后写回、超长／短输入、后续输出打开失败、原本可写以及 Kept 保留路径。
修正后的 eeb2412 已通过完整云端构建和上述真实 loop 回归。实机保留本次
OTA 后 recovery_a 只读标志为 1 的原始条件，先将已校验的新镜像作为 b 槽
源，再用自动保留所用的 Kept 接口重试：正确选择 a、完成写入和读回校验，
并恢复 a 只读／b 可写状态；两个 recovery 摘要均匹配新镜像，其他启动
分区在修复前后未变。随后新版 Skyward 已从目标 a 槽原生启动，程序、配置
及 Weaver 库运行摘要均与同次 CI 产物一致。未为该复测再次执行整包 OTA；
这验证的是修正后的同一回刷接口，并非重新跑过新版的整包后自动触发。
返回 HyperOS 后 ADB 为普通 shell，未另行申请 root 或重启读取原始分区。

供开发者重试失败回刷的 `keep-recovery` 工具默认只报告当前与下次槽及目标；
写入必须显式提供 `--write` 与当前槽 recovery 的预期 SHA-256，且更新程序
必须已退出。它复用自动保留的 Kept 路径，不重新安装 OTA，不改变启动槽。

另外，本次 `/metadata` 的重新挂载遇到 EBUSY，更新程序采用覆盖分区回退路径；
不能将本次记录标为正常 Virtual A/B 快照安装通过。界面第一阶段提前显示
100% 的进度计算问题也仍需修正，判断完成以更新程序最终状态为准。
