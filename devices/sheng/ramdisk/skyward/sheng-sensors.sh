#!/system/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Each service runs this in its own private mount namespace.
set -eu
case "$1" in
    /vendor/bin/pd-mapper)
        # Expose only the sensor domain to the mapper after it drops to system.
        meta=/skyward/run/sensor-firmware
        mkdir -p "$meta/image"
        chmod 755 "$meta" "$meta/image"
        cp /firmware/image/adsps.jsn "$meta/image/adsps.jsn"
        chmod 644 "$meta/image/adsps.jsn"
        mount -o bind "$meta" /vendor/firmware_mnt
        mount -o bind "$meta/image" /vendor/firmware
        ;;
    /vendor/bin/sscrpcd)
        # The DSP may update its registry; give it a RAM copy of calibration.
        # Never replay the real persist journal or let the listener write it.
        state=/skyward/run/sensor-persist
        ro=/skyward/run/sensor-persist-ro
        mkdir -p "$state" "$ro" /mnt/vendor/persist
        if [ ! -f "$state/.ready" ]; then
            mount -t ext4 -o ro,noload /dev/block/by-name/persist "$ro"
            trap 'umount "$ro"' EXIT
            cp -a "$ro/sensors" "$state/"
            umount "$ro"
            trap - EXIT
            touch "$state/.ready"
        fi
        mount -o bind "$state" /mnt/vendor/persist
        export ADSP_LIBRARY_PATH='/vendor/lib/rfsa/adsp;/vendor/dsp;/vendor/firmware_mnt/image'
        ;;
    *) exit 1 ;;
esac
exec "$@"
