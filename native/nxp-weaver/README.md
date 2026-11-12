# Native NXP Weaver recovery component for myron

This source kit builds an `ese_weaver.so` prototype for the stock myron
OS4.0.6.0.XPMCNXM recovery userspace. The 4ab myron test verified its actual
child-view hash/inode mappings and eSE startup. Its single getConfig failed
before SELECT because QTI rejected a callback with system Binder stability.
This revision enables the standard stable-AIDL build option; the revised
library has not been device-tested. Successful slot configuration and PIN
use are not established.

It compiles Weaver and a private copy of the direct NXP HAL transport together.
The Binder boundary uses stable AIDL V1 through `libbinder_ndk` C functions.
The vendor wrapper still sees the `WeaverImpl` interface and platform
`std::__1` ABI. No vendor transport C++ object is shared with the rebuilt code.
The library is packed under `/skyward/lib64`. The capability-aware myron
candidate gives only Weaver a read-only bind at the stock SO path in a
private child mount namespace, preserving the original executable/filecaps.
Do not replace libraries in the vendor partition.

## Build on Linux x86_64

Python 3.10 or newer, glibc and zlib are required. The optional fake transport
test also needs a host C++17 compiler with AddressSanitizer and UBSan.

```sh
python3 fetch_toolchain.py --out /tmp/myron-clang
python3 extract_recovery_libraries.py /path/to/official-REC.img --out /tmp/myron-rec-libs
python3 test_transport.py --out /tmp/myron-transport-test
python3 build.py --toolchain /tmp/myron-clang --libraries /tmp/myron-rec-libs --out /tmp/myron-native-out
```

The library is `/tmp/myron-native-out/ese_weaver.so`; `BUILD_PROOF.json`
contains the resulting hash, ELF imports and GNU symbol version checks.
`COMMANDS.json` and individual logs record the actual build commands.
Only the chosen output/cache directories are written. The source kit and the
original Android platform tree are not modified. The full platform tree and
Soong output are not required.

The toolchain downloader retrieves six hash-pinned files, about 252 MB,
from the public AOSP prebuilt Clang repository. The build uses the supplied
platform libc++ headers and `-nostdinc++`, preserving `std::__1`. NDK libc++
headers use a different namespace and are not a substitute. This recipe has
been tested with the pinned AOSP Clang distribution; no NDK build claim is made.

The recovery extractor accepts only the complete image with SHA256
`49622932fb32ab623db336866734ad440a2e15af5f750ceb9b1d3d0f7fdabafc`.
It handles its Android v4 header, legacy LZ4 ramdisk and newc archive, then
checks all seven library hashes. It uses Python's standard library. Obtain
that image from the official OTA before this step; the parent Skyward CI
already performs the official partition extraction and image hash check.
The OTA URL is recorded in `STOCK_RECOVERY.json`.

## Source and ABI records

`SOURCES.json` pins the original NXP sources. `PLATFORM_HEADERS.json` records
738 platform header/CRT source files, upstream repositories, commits, Git
blob IDs and SHA256 hashes. All matched their pinned Git objects. The initial
closure was collected with `clang -M` over 12 production C++ files and a
layout probe, then extended for the CRT sources. The CRT is rebuilt from
bundled bionic source. License notices remain in the source files and
`licenses/`, `weaver/` and `keymint/`.

`AIDL_GENERATION.json` pins three Stable AIDL V1 input files, their hash file,
the public compiler and 12 generated files. The generated snapshot omits
the original machine's command line. It can be regenerated separately:

```sh
python3 regenerate_aidl.py --compiler /tmp/myron-aidl --out /tmp/myron-aidl-generated
```

The ABI probe requires SlotInfo=12 bytes, ReadRespInfo=32 bytes and
WeaverImpl=24 bytes. ELF verification requires the 72-byte vtable with its
first five entries in the live wrapper's order: Init, GetSlots, Read, Write,
DeInit. Only WeaverImpl methods and its vtable are exported. Every import,
including its GNU symbol version, must be supplied by the exact recovery
libraries. The transport and generated Binder classes remain hidden.
The build also requires the generated callback factory's object code to call
`AIBinder_markVintfStability`, and a strong final import of that C API at
`LIBBINDER_NDK30`, supplied by the exact recovery's `libbinder_ndk.so`.

## Recovery-specific changes

The compile option `BINDER_STABILITY_SUPPORT` enables the generated stable
AIDL callback's VINTF marking and vendor-local transaction flags. Without
it, the recovery's system `libbinder_ndk` creates a system-stability callback
that QTI cannot call from its vendor context. `BINDER_STABILITY.json` records
the added pinned header, dependency closure and static gates. Generated code
and its interface definitions remain unchanged; no stability downgrade is used.

`weaver-direct-hal.patch` records all changes against the pinned NXP files:

* Recreate the direct transport and close the old channel when changing AID.
* Use the three live AIDs exactly, including the 13-byte second AID.
* Recognize SELECT-not-found only with a fresh response of at least two bytes
  containing `6A82` and a failed transport operation.
* Send each operation once. Try the next AID only after explicit SELECT
  failure; stop on ambiguous failures, including failures at an alternate AID.
* Include `<ostream>` explicitly for current platform libc++.

The upstream `retry=1; do ... while(retry++ <= 12)` loop could send 13 times.
It is removed, rather than changing a constant that still permits an extra
send. Responses are cleared before each attempt so an old `6A82` cannot
trigger another send. `tests/` links the actual patched transport to a fake
and tests success, failure, stale/short responses, three-AID selection,
wrong-key/throttle-like responses and ambiguous failure after an alternate.
The tests do not contact Binder or send any real APDU.

`AppletConnection::selectApplet` maps the stable AIDL
`NO_SUCH_ELEMENT_ERROR` (3) to `6A82`. Ordinary applet responses remain
transport successes and are parsed by Weaver, including its wrong-key and
throttle status tags. An ambiguous error is returned to the caller without
automatic credential or write replay. Manual retry of a read-only information
query is the intended diagnostic recovery path.

## Runtime validation still required

The static checks do not establish that the native eSE1 service is ready,
that its recovery SELinux policy permits this client, or that a Weaver applet
will accept this recovery process. Service startup and readiness must be
checked first. `AServiceManager_waitForService` may wait for an unavailable
eSE1 service.

Any separately authorized device test should start with initialization and
the read-only slot configuration query (`GetSlots`/Weaver `getConfig`), after
confirming the intended library is actually loaded. First inspect the
Weaver process's `/proc/<pid>/maps`. A private bind may retain the stock
`/odm/lib64/ese_weaver.so` pathname, so compare its mapped device/inode with
`/proc/<pid>/root/odm/lib64/ese_weaver.so` and verify that child-view file's
SHA256 against the build proof. The host and unrelated services must still
see the original SO. Capture UID/GID, auxv, capabilities, file capabilities,
service domain and source-inode SELinux label as part of that check. Normal
system StrongBox and Weaver both have file capabilities and AT_SECURE=1;
they keep CAP_SYS_ADMIN, CAP_SYS_NICE and CAP_WAKE_ALARM. An environment-only
LD_LIBRARY_PATH override is insufficient. A bind keeps the source inode's
label; verify that the actual service can read and executable-map it without
changing enforcing policy. The private-bind mapping gate passed in 4ab; its
single read-only getConfig failed before applet selection. The callback
stability correction still requires runtime validation. An unconfirmed
override must never proceed to a PIN/key attempt.
Do not add a security bypass to force loading. Stop
if service discovery or applet selection fails. Do not probe credentials,
try dummy/wrong PINs or keys, write/erase slots, or loop authentication
attempts. Real authentication and decryption require a complete reviewed
candidate and separate authorization. This kit contains no device-control
commands and performs no device test.
