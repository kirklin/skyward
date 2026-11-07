#!/system/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Hardware sequence derived from stock 303 and the supplied sheng TWRP tree.
mkdir -p /skyward/run
exec >> /skyward/run/sheng-hardware.log 2>&1
set -eu
slot=$(getprop ro.boot.slot_suffix)
case "$slot" in _a|_b) ;; *) echo "Invalid slot: $slot"; exit 1 ;; esac
mkdir -p /firmware /vendor/firmware_mnt /vendor/firmware
# Keep firmware outside /vendor so mounting the installed vendor over the
# ramdisk never makes a split ADSP firmware segment disappear.
mount -t vfat -o ro "/dev/block/by-name/modem$slot" /firmware
mount -o bind /firmware /vendor/firmware_mnt
mount -o bind /firmware/image /vendor/firmware
echo /firmware/image > /sys/module/firmware_class/parameters/path
for segment in adsp.mdt adsp_dtb.mdt adsp.b01; do
    test -r "/firmware/image/$segment"
done
for remote in /sys/class/remoteproc/remoteproc*; do
    test -r "$remote/name" || continue
    case "$(cat "$remote/name")" in
        *remoteproc-adsp*)
            # Never retry a partial boot: GLINK may already be registered.
            if [ "$(cat "$remote/state")" = offline ]; then
                echo start > "$remote/state"
            fi
            echo "ADSP: $(cat "$remote/state")"
            ;;
    esac
done
attempt=0
while [ ! -r /sys/class/power_supply/battery/capacity ] && [ "$attempt" -lt 100 ]; do
    sleep 0.1
    attempt=$((attempt + 1))
done
test -r /sys/class/power_supply/battery/capacity
echo "Battery: $(cat /sys/class/power_supply/battery/capacity)"
setprop sys.skyward.sheng.hardware_ready 1
