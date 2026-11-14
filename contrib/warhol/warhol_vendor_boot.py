#!/usr/bin/env python3
"""Offline warhol vendor_boot v4 builder. Never opens a device or runs adb.

Only the single type-2 recovery ramdisk is rebuilt. Other fragments, DTB,
bootconfig, cmdline and all unmodified table fields are preserved. The output
has an unsigned AVB hash footer, as upstream skyward-pack does; it is NOT
signed by Xiaomi. Requires the verified source image and a bring-up binary.
"""
import argparse
import hashlib
import json
import struct
import subprocess
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

SOURCE_SHA256 = "ffe14d3341165424e1b582fb831d0f37864038433830cec5d7489716a3184faf"
VERSION = "OS3.0.311.0.WPSMIXM"
PARTITION_SIZE = 64 * 1024 * 1024


def align(n, page=4096):
    return (n + page - 1) // page * page


def sha(data):
    return hashlib.sha256(data).hexdigest()


def require(ok, why):
    if not ok:
        raise ValueError(why)


class VendorBoot:
    def __init__(self, data):
        require(len(data) >= 2128 and data[:8] == b"VNDRBOOT", "not a vendor boot image")
        u = lambda o: struct.unpack_from("<I", data, o)[0]
        require(u(8) == 4 and u(12) == 4096 and u(2096) == 2128, "only v4 / 4096-byte pages supported")
        self.header = data[:4096]
        ram_size, dtb_size = u(24), u(2100)
        tab_size, count, entry_size, config_size = [u(o) for o in (2112, 2116, 2120, 2124)]
        require(entry_size >= 108 and 0 < count <= 64 and tab_size == count * entry_size, "invalid ramdisk table")
        dtb_at = 4096 + align(ram_size)
        tab_at = dtb_at + align(dtb_size)
        config_at = tab_at + align(tab_size)
        require(config_at + config_size <= len(data), "vendor_boot sections exceed file")
        self.dtb = data[dtb_at:dtb_at + dtb_size]
        self.bootconfig = data[config_at:config_at + config_size]
        self.entries, self.fragments = [], []
        covered = []
        for i in range(count):
            e = data[tab_at + i * entry_size:tab_at + (i + 1) * entry_size]
            size, offset, kind = struct.unpack_from("<III", e)
            require(size > 0 and offset + size <= ram_size, "ramdisk fragment outside section")
            covered.append((offset, offset + size))
            self.entries.append(e)
            self.fragments.append(data[4096 + offset:4096 + offset + size])
        cursor = 0
        for start, end in sorted(covered):
            require(start == cursor, "ramdisk fragments overlap or contain unexplained gaps")
            cursor = end
        require(cursor == ram_size, "unaccounted ramdisk data")
        self.recovery = [i for i, e in enumerate(self.entries) if struct.unpack_from("<I", e, 8)[0] == 2]
        require(len(self.recovery) == 1, "exactly one recovery fragment is required")
        self.recovery = self.recovery[0]

    def replace_recovery(self, fragment):
        header = bytearray(self.header)
        parts = list(self.fragments)
        parts[self.recovery] = fragment
        table = bytearray()
        offset = 0
        for old, data in zip(self.entries, parts):
            e = bytearray(old)
            struct.pack_into("<II", e, 0, len(data), offset)
            table.extend(e)
            offset += len(data)
        struct.pack_into("<I", header, 24, offset)
        out = bytearray(header)
        for section in (b"".join(parts), self.dtb, bytes(table), self.bootconfig):
            out.extend(section)
            out.extend(bytes(align(len(out)) - len(out)))
        return bytes(out)


@dataclass
class Entry:
    name: str
    fields: list
    data: bytes


def read_cpio(data):
    entries, seen, pos = [], set(), 0
    while True:
        require(pos + 110 <= len(data) and data[pos:pos + 6] == b"070701", "invalid/truncated newc archive")
        f = [int(data[pos + 6 + i * 8:pos + 14 + i * 8], 16) for i in range(13)]
        size, namesize = f[6], f[11]
        require(namesize > 0 and pos + 110 + namesize <= len(data), "invalid cpio name length")
        raw = data[pos + 110:pos + 110 + namesize]
        require(raw[-1:] == b"\0", "unterminated cpio name")
        name = raw[:-1].decode()
        at = align(pos + 110 + namesize, 4)
        require(at + size <= len(data), "truncated cpio file")
        if name == "TRAILER!!!":
            require(size == 0 and not any(data[at:]), "extra archive or data after cpio trailer")
            return entries
        require(name not in seen and not name.startswith("/") and ".." not in PurePosixPath(name).parts, "unsafe/duplicate cpio path")
        seen.add(name)
        entries.append(Entry(name, f, data[at:at + size]))
        pos = align(at + size, 4)


def write_cpio(entries):
    out = bytearray()
    trailer = Entry("TRAILER!!!", [0] * 13, b"")
    for e in [*entries, trailer]:
        name = e.name.encode() + b"\0"
        f = list(e.fields)
        f[6], f[11] = len(e.data), len(name)
        out.extend(b"070701" + b"".join(f"{n:08x}".encode() for n in f))
        out.extend(name)
        out.extend(bytes(align(len(out), 4) - len(out)))
        out.extend(e.data)
        out.extend(bytes(align(len(out), 4) - len(out)))
    return bytes(out)


def lz4(data, decode=False):
    args = ["lz4", "-d", "-c"] if decode else ["lz4", "-l", "-12", "-c"]
    return subprocess.run(args, input=data, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True).stdout


def put(entries, name, data, mode=0o100644):
    for e in entries:
        if e.name == name:
            require(e.fields[1] & 0o170000 == mode & 0o170000, f"unexpected file type: {name}")
            e.data = data
            return
    # Explicit directories: no dependency on cpio auto-creating parents.
    for parent in reversed(PurePosixPath(name).parents):
        if str(parent) != "." and not any(e.name == str(parent) for e in entries):
            put(entries, str(parent), b"", 0o040755)
    inode = max([500000] + [e.fields[0] for e in entries]) + 1
    entries.append(Entry(name, [inode, mode, 0, 0, 1, 0, len(data), 0, 0, 0, 0, 0, 0], data))


def avb_footer(image, partition_size, partition="vendor_boot", salt=None):
    # Same unsigned AVB layout as crates/skyward-pack/src/avb.rs.
    salt = hashlib.sha256(image).digest() if salt is None else salt
    digest = hashlib.sha256(salt + image).digest()
    name = partition.encode()
    desc = bytearray(struct.pack(">QQQ32sIIII60s", 2, 0, len(image), b"sha256", len(name), len(salt), 32, 0, bytes(60)))
    desc.extend(name + salt + digest)
    desc.extend(bytes(align(len(desc), 8) - len(desc)))
    struct.pack_into(">Q", desc, 8, len(desc) - 16)
    n = len(desc)
    vb = bytearray(struct.pack(">4sIIQQI", b"AVB0", 1, 0, 0, align(n, 64), 0))
    vb.extend(struct.pack(">11QII", 0, 0, 0, 0, n, 0, n, 0, 0, n, 0, 0, 0))
    vb.extend(b"skyward-pack")
    vb.extend(bytes(256 - len(vb)))
    vb.extend(desc)
    vb.extend(bytes(256 + align(n, 64) - len(vb)))
    offset = align(len(image))
    require(align(offset + len(vb)) + 4096 <= partition_size, "modified ramdisk does not fit partition")
    out = bytearray(image)
    out.extend(bytes(offset - len(out)))
    out.extend(vb)
    out.extend(bytes(partition_size - 64 - len(out)))
    out.extend(struct.pack(">4sIIQQQ28s", b"AVBf", 1, 0, len(image), offset, len(vb), bytes(28)))
    return bytes(out)


def build(stock, binary, config, fonts):
    require(len(stock) == PARTITION_SIZE and sha(stock) == SOURCE_SHA256, "not the verified warhol Global 311 vendor_boot image")
    require(binary[:5] == b"\x7fELF\x02" and struct.unpack_from("<H", binary, 18)[0] == 183, "binary must be AArch64 ELF64")
    require(b"warhol bring-up: pending recovery requests ignored" in binary, "binary lacks the diagnostic entrypoint")
    require(b'codename = "warhol"' in config, "not the warhol configuration")
    v = VendorBoot(stock)
    rd = read_cpio(lz4(v.fragments[v.recovery], decode=True))
    props = next(e for e in rd if e.name == "prop.default")
    require(VERSION.encode() in props.data, "stock recovery properties do not match")
    require(not any(e.name.startswith("skyward/") for e in rd), "source already modified")
    # Overlay only: stock base recovery binary is left byte-for-byte intact
    # in the type-1 fragment; the type-2 copy shadows it only in recovery.
    put(rd, "system/bin/recovery", binary, 0o100755)
    put(rd, "skyward/bringup", b"warhol-vendor-boot-v4-diagnostic-v1\n")
    put(rd, "skyward/device.toml", config)
    for name, data in fonts.items():
        put(rd, "skyward/" + name, data)
    # Recovery-only adb diagnostics. Do not alter OTA downgrade policy,
    # device identity, SELinux policy or normal-boot property files.
    keys = (b"ro.secure=", b"ro.adb.secure=", b"ro.debuggable=")
    props.data = b"\n".join(line for line in props.data.splitlines() if not line.startswith(keys))
    props.data += b"\nro.secure=0\nro.adb.secure=0\nro.debuggable=1\n"
    packed = lz4(write_cpio(rd))
    image = avb_footer(v.replace_recovery(packed), len(stock))
    check = VendorBoot(image)
    require(check.dtb == v.dtb and check.bootconfig == v.bootconfig, "DTB/bootconfig changed")
    for i in range(len(v.fragments)):
        require(i == v.recovery or v.fragments[i] == check.fragments[i], "normal boot fragment changed")
        require(v.entries[i][8:] == check.entries[i][8:], "ramdisk type/name/board selection changed")
    changes = [(i, a, b) for i, (a, b) in enumerate(zip(v.header, check.header)) if a != b]
    require(all(24 <= i < 28 for i, _, _ in changes), "unexpected vendor boot header change")
    report = {
        "status": "UNTESTED_DIAGNOSTIC_ONLY", "device": "warhol", "stock_version": VERSION,
        "stock_sha256": sha(stock), "image_sha256": sha(image), "image_bytes": len(image),
        "binary_sha256": sha(binary), "config_sha256": sha(config),
        "recovery_fragment_index": v.recovery, "new_recovery_compressed_bytes": len(packed),
        "normal_boot_fragment_unchanged": True, "dtb_unchanged": True, "bootconfig_unchanged": True,
        "avb_signature": "NONE; no manufacturer signature; requires compatible unlocked bootloader",
        "device_flashed": False, "runtime_verified": False,
        "disabled_in_diagnostic_entrypoint": ["install", "OTA", "wipe", "decrypt", "dual_boot", "MTP", "automatic_slot_changes"],
        "root_adb": "enabled; not a read-only interface",
    }
    return image, report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stock", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--config", type=Path, default=Path("devices/experimental/warhol.toml"))
    parser.add_argument("--fonts", type=Path, default=Path("resources/fonts"))
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    require(args.out.resolve() != args.stock.resolve(), "never overwrite the original image")
    require(not args.out.exists(), "output exists; choose a new output path")
    font_names = ["MiSansVF.ttf", "KirkMono-Regular.ttf", "KirkMono-OFL.txt"]
    image, report = build(args.stock.read_bytes(), args.binary.read_bytes(), args.config.read_bytes(),
                          {n: (args.fonts / n).read_bytes() for n in font_names})
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_bytes(image)
    args.out.with_suffix(".json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
