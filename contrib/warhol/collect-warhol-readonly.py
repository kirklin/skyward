#!/usr/bin/env python3
"""Explicitly run with the phone normally booted and USB debugging authorized.
Reads selected hardware facts only. No su, reboot, flashing, data/user IDs,
Wi-Fi names, account info, screen content or complete getprop dump.
"""
import json
import subprocess
from datetime import datetime, timezone
from pathlib import Path


def shell(*args):
    p = subprocess.run(["adb", "shell", *args], capture_output=True, text=True, timeout=20)
    return {"ok": p.returncode == 0, "stdout": p.stdout.strip(), "stderr": p.stderr.strip()}


def main():
    product = shell("getprop", "ro.product.device")
    if product["stdout"] != "warhol":
        raise SystemExit("No single authorized warhol device; nothing else was queried.")
    result = {"collected_utc": datetime.now(timezone.utc).isoformat(), "product": product}
    for prop in ["ro.build.version.incremental", "ro.build.version.release", "ro.boot.slot_suffix",
                 "ro.board.platform", "ro.sf.lcd_density", "ro.boot.verifiedbootstate", "ro.boot.flash.locked"]:
        result[prop] = shell("getprop", prop)
    for path in ["/proc/version", "/proc/modules", "/proc/bus/input/devices"]:
        result[path] = shell("cat", path)
    for path in ["/sys/class/drm", "/sys/class/backlight", "/dev/block/by-name", "/vendor/lib/modules",
                 "/vendor_dlkm/lib/modules", "/sys/class/touch"]:
        result[path] = shell("ls", "-1", path)
    dest = Path(__file__).resolve().parents[2] / "local/live-readonly"
    dest.mkdir(parents=True, exist_ok=True)
    out = dest / (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + ".json")
    out.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(out)


if __name__ == "__main__":
    main()
