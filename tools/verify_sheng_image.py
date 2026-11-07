#!/usr/bin/env python3
"""Independently audit the packed sheng image and its stock-derived ramdisk."""
import argparse
import hashlib
import json
import struct
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("--stock", required=True, type=Path)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--packer", required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    data, stock = args.image.read_bytes(), args.stock.read_bytes()
    assert len(data) == len(stock) == 104857600, "must fill the 100 MiB recovery partition"
    assert data[:8] == b"ANDROID!"
    kernel, ramdisk = struct.unpack_from("<II", data, 8)
    version = struct.unpack_from("<I", data, 40)[0]
    assert (kernel, version) == (0, 4), "stock v4 ramdisk-only layout must remain"
    assert data[44:1580] == stock[44:1580], "boot command line changed"
    assert data[-64:-60] == b"AVBf"
    original, offset, size = struct.unpack_from(">QQQ", data, len(data) - 64 + 12)
    vbmeta = data[offset:offset + size]
    assert vbmeta[:4] == b"AVB0"
    assert struct.unpack_from(">I", vbmeta, 28)[0] == 0, "expected unsigned AVB metadata"
    auth_size = struct.unpack_from(">Q", vbmeta, 12)[0]
    descriptor_offset, descriptor_size = struct.unpack_from(">QQ", vbmeta, 96)
    descriptor = vbmeta[256 + auth_size + descriptor_offset:256 + auth_size + descriptor_offset + descriptor_size]
    assert struct.unpack_from(">Q", descriptor)[0] == 2, "expected hash descriptor"
    image_size = struct.unpack_from(">Q", descriptor, 16)[0]
    name_size, salt_size, digest_size = struct.unpack_from(">III", descriptor, 56)
    assert descriptor[24:56].rstrip(b"\0") == b"sha256"
    name = descriptor[132:132 + name_size]
    salt = descriptor[132 + name_size:132 + name_size + salt_size]
    digest = descriptor[132 + name_size + salt_size:132 + name_size + salt_size + digest_size]
    assert name == b"recovery" and image_size == original
    assert hashlib.sha256(salt + data[:image_size]).digest() == digest, "AVB payload digest differs"
    assert 4096 + ((ramdisk + 4095) // 4096) * 4096 == original

    def cat(path):
        return subprocess.check_output([str(args.packer), "cat", str(args.image), path])

    assert cat("skyward/device.toml") == (root / "devices/sheng.toml").read_bytes()
    binary = cat("system/bin/recovery")
    assert binary == args.binary.read_bytes()
    assert binary[:4] == b"\x7fELF" and struct.unpack_from("<H", binary, 18)[0] == 183
    for path in (root / "devices/sheng/ramdisk").rglob("*"):
        if path.is_file():
            relative = path.relative_to(root / "devices/sheng/ramdisk").as_posix()
            assert cat(relative) == path.read_bytes(), f"overlay differs: {relative}"
    props = cat("prop.default").decode()
    assert "ro.secure=0\n" in props and "ro.adb.secure=0\n" in props
    assert not any(line.startswith("ro.build.version.security_patch=") for line in props.splitlines())
    # Init, USB services and the kernel module payload are inherited from stock.
    for path in ("init", "system/bin/adbd", "system/bin/fastbootd", "system/etc/init/hw/init.rc"):
        original_file = subprocess.check_output([str(args.packer), "cat", str(args.stock), path])
        assert cat(path) == original_file, f"stock runtime changed: {path}"
    report = {
        "image": args.image.name, "size": len(data), "header_version": version,
        "kernel_bytes": kernel, "ramdisk_bytes": ramdisk,
        "sha256": hashlib.sha256(data).hexdigest(), "avb_digest_verified": True,
        "config_binary_overlay_verified": True, "stock_init_adbd_fastbootd_verified": True,
        "hardware_validation": "not assessed by this static audit",
    }
    args.image.with_suffix(".verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
