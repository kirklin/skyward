#!/system/bin/sh
# Development image only. No mount, repair, key access, reboot or /data writes.
# Persistent output requires separate test authorization; normal builds omit it.
[ "$(getprop ro.product.device)" = myron ] || exit 0
[ "$(getprop ro.boot.slot_suffix)" = _b ] || exit 0
[ -f /skyward/device.toml ] || exit 0
# Log only DeleteObject transaction stages, with no object IDs or paths.
# Normal images omit this script and leave the trace disabled.
setprop debug.skyward.mtp_delete_trace 1
umask 077
mkdir /skyward/myron-trace || exit 0
destination=/skyward/myron-trace

# The observed metadata filesystem is F2FS. Never mount or remount it here.
# Check both mount identity and rw state before creating our new directory.
source=$(awk '$2 == "/metadata" && $3 == "f2fs" && $4 ~ /(^|,)rw(,|$)/ {print $1; exit}' /proc/mounts)
expected=$(readlink -f /dev/block/by-name/metadata)
free_bytes=0
set -- $(stat -f -c '%a %S' /metadata 2>/dev/null)
if [ "$#" = 2 ]; then
    case "$1:$2" in
        *[!0-9:]*|:) ;;
        *) free_bytes=$(($1 * $2)) ;;
    esac
fi
if [ -n "$source" ] && [ -n "$expected" ] && [ "$(readlink -f "$source")" = "$expected" ] && [ "$free_bytes" -ge 4194304 ]; then
    boot_id=$(cat /proc/sys/kernel/random/boot_id)
    case "$boot_id" in
        ''|*[!0-9a-f-]*) boot_id= ;;
    esac
    if [ "${#boot_id}" = 36 ]; then
        candidate=/metadata/skyward-myron-trace-$boot_id
        if mkdir "$candidate"; then
            destination=$candidate
            printf 'SKYWARD_MYRON_TRACE_V1\n' > "$destination/OWNER"
        fi
    fi
fi
printf 'skyward myron diagnostics: %s\n' "$destination" > /dev/kmsg

# Six bounded samples at about 0/15/30/45/75/105 seconds. Cover UI/MTP
# startup and the later USB loss; retain the same total byte bound.
for sample in 0 1 2 3 4 5; do
    case "$sample" in
        1|2|3) sleep 15 ;;
        4|5) sleep 30 ;;
    esac
    {
        date
        for property in ro.product.device ro.boot.slot_suffix ro.boot.mode sys.usb.config sys.usb.state sys.usb.controller sys.usb.configfs sys.usb.config_disable sys.usb.ffs.ready sys.usb.ffs.mtp.ready init.svc.adbd init.svc.recovery; do
            printf '%s=%s\n' "$property" "$(getprop "$property")"
        done
        ps -A
        sha256sum /system/bin/recovery /skyward/device.toml /skyward/lib64/ese_weaver.so /odm/lib64/ese_weaver.so
        cat /sys/fs/selinux/enforce
        for process in android.hardware.weaver-service.nxp-qti android.hardware.security.keymint3-service.strongbox.nxp android.hardware.secure_element-service.qti vendor.xiaomi.hw.touchfeature-service; do
            for pid in $(pidof "$process"); do
                printf 'PROCESS %s PID %s\n' "$process" "$pid"
                grep -E '^(Uid|Gid|Groups|CapPrm|CapEff):' "/proc/$pid/status"
                grep -E 'ese_weaver|libjc_keymint|/skyward/' "/proc/$pid/maps" | tail -c 4096
                # A private bind can retain the stock pathname in maps.
                # Verify bytes/inode in the child's own mount view as well.
                sha256sum "/proc/$pid/root/odm/lib64/ese_weaver.so"
                stat -c '%d %D %i %n' /skyward/lib64/ese_weaver.so "/proc/$pid/root/odm/lib64/ese_weaver.so"
                ls -Z /skyward/lib64/ese_weaver.so "/proc/$pid/root/odm/lib64/ese_weaver.so"
                cat "/proc/$pid/attr/current"
                # Kernel aux-vector metadata includes AT_SECURE, which can
                # explain an ignored library path. It is not process memory.
                od -An -t x8 "/proc/$pid/auxv" | head -c 4096
            done
        done
        ls -l /sys/class/udc /dev/dri /dev/input
        for node in /config/usb_gadget/g1/UDC /config/usb_gadget/g1/idVendor /config/usb_gadget/g1/idProduct; do
            printf '%s: ' "$node"
            cat "$node"
        done
        ls -l /config/usb_gadget/g1/configs/b.1/f1 /config/usb_gadget/g1/configs/b.1/f2
        grep -E ' /dev/usb-ffs/(adb|mtp) ' /proc/mounts
        for node in /sys/class/udc/*/state; do
            printf '%s: ' "$node"
            cat "$node"
        done
        tail -c 16384 /skyward/run/recovery.log
        dmesg | tail -c 65536
        timeout 2 logcat -b all -d -t 200 | tail -c 32768
    } 2>&1 | head -c 131072 > "$destination/sample-$sample.log"
    sync
done
