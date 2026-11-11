# myron: initial stock evidence

This is an offline bring-up candidate, not a tested recovery release.
Read-only inspection on 2026-11-12 identified REDMI K90 Pro Max
`25102RKBEC`, SM8850/canoe, OS4.0.6.0.XPMCNXM, framework Android 17/API 37
and vendor Android 16/API 36. The connected phone was using slot b; no
slot was changed. Restrictions learned from another device's faulty slot
do not establish anything about this phone's slots.

Only selected system/vendor/odm files, kernel interface descriptions and
properties were read. There was no reboot, flash, mount, daemon start,
credential/key read or application-data inspection. Raw logs and the
device identifier remain outside Git in local private evidence.

| Area | Observed stock evidence | Candidate status |
| --- | --- | --- |
| Default KeyMint | `vendor/etc/init/android.hardware.security.onekeymint-service-qti.rc`; matching VINTF fragment declares KeyMint v4 and remote provisioning v3/default | QTI process, stock system:drmrpc identity; runtime/library closure untested |
| Secure clock/shared secret | Secure-clock/default VINTF contents read; `android.hardware.security.sharedsecret-service-qti.xml` filename observed | Shared-secret default interface still requires fragment-content and runtime confirmation |
| Gatekeeper | `android.hardware.gatekeeper-service-qti.rc` starts `android.hardware.gatekeeper-rust-service-qti`; matching AIDL VINTF/default | Stock system:system identity; PIN verification untested |
| Weaver | ODM `android.hardware.weaver-service.nxp.rc` and VINTF v2/default | NXP binary and vendor_nxp_weaver user/group (2911), not nezha's Goodix path |
| StrongBox | ODM NXP StrongBox v3/strongbox manifest and executable present | Not started by this minimal candidate; dependencies and shared-secret participation remain to be checked |
| TEE helpers | This phone's qseecomd and hlosminkdaemon rc files | Paths, users/groups and Mink capabilities copied from those files; TEE readiness untested |
| Touch | Loaded focaltech_touch_3683/xiaomi_touch; input focaltech_ts; raw flag read as 1; libtouchreport_alg_fts.so present | Removed inherited, unverified raw=0 write; no claim that module loading supplies recovery touch |
| Wi-Fi | Loaded qca_cld3_peach_v2/rfkill/cfg80211; cnss symlink targets qcom,cnss-peach; peach_v2 config directory | Existing module/supplicant scaffold retained; no scan/association test or speculative daemon additions |
| Data | Stock fstab: F2FS, fscrypt v2, inlinecrypt_optimized and wrappedkey_v0 metadata/content keys | read_only=true, auto_repair=false, key_upgrades=false |
| Boot control | Stock AIDL boot-service.qti rc/manifest; separate recovery_a/recovery_b links | Configured root:root before the security services; query and write operations remain untested |

The captured `vendor.xiaomi.hw.touchfeature-service.rc` starts
`/odm/bin/hw/vendor.xiaomi.hw.touchfeature-service` as system with system
and input groups, SYS_NICE, and the AIDL interface
`vendor.xiaomi.hw.touchfeature.ITouchFeature/default`. The ODM library
directory contains libtouchreport.so, libtouchreport_alg.so,
libtouchreport_alg_fts.so, libtouchreport_hal.so and libtouchreport_sensor.so.
No process maps/ELF dependency inspection or complete live sysfs inventory
was captured, so the actual algorithm-loading path and required ownership
changes remain unproven. This candidate does not start that HAL yet.

The read-only decrypt policy governs Skyward's data mount and automatic
fsck/key-upgrade paths; it also makes the packer disable automatic metadata
key deletion. It is not a device-wide write blocker. Explicit wipe/repair,
partition operations, vendor HAL state and the inherited persist mount
remain separate concerns requiring their own authorization and validation.

Before boot testing, obtain and verify the OS4.0.6.0 stock recovery/header
and rollback material. The profile still pins the upstream OS4.0.4.0 OTA
and SHA because no exact OS4.0.6.0 recovery hash was captured. Display
dimensions/density and haptics strengths remain inherited, not newly
measured. The connected device was removed before the next identity-guarded
batch, so no partition headers were downloaded.

Next checks are the exact shared-secret fragments and NXP dependencies,
touch processing/native position mode, display/USB behavior, metadata-only
read-only opening, then user-authorized PIN/CE testing. Successful profile
parsing or CI must not be reported as any of those device tests passing.

Local validation: the targeted
`myron_bringup_preserves_data_and_uses_its_observed_aidl_services` contract
test passed (one test actually executed), and skyward-config all-target
clippy passed with warnings denied. Its Unix build also calls the normal
DeviceConfig parser. The full configuration suite is not Windows portable:
Android absolute paths are rejected by Windows Path validation. Full
Linux/ARM64 CI remains required; no recovery image was built or booted in
this inspection task.
