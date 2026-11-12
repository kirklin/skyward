#!/system/bin/sh
# Development image only. No mount, repair, key access, reboot or /data writes.
# Persistent output requires separate test authorization; normal builds omit it.
[ "$(getprop ro.product.device)" = myron ] || exit 0
[ "$(getprop ro.boot.slot_suffix)" = _b ] || exit 0
[ -f /skyward/device.toml ] || exit 0
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

# Six bounded samples, no credential input or partition contents.
for sample in 0 1 2 3 4 5; do
    {
        date
        for property in ro.product.device ro.boot.slot_suffix ro.boot.mode sys.usb.config sys.usb.state sys.usb.controller sys.usb.configfs sys.usb.ffs.ready init.svc.adbd init.svc.recovery; do
            printf '%s=%s\n' "$property" "$(getprop "$property")"
        done
        ps -A
        ls -l /sys/class/udc /dev/dri /dev/input
        for node in /sys/class/udc/*/state; do
            printf '%s: ' "$node"
            cat "$node"
        done
        tail -c 16384 /skyward/run/recovery.log
        dmesg | tail -c 65536
        timeout 2 logcat -b all -d -t 200 | tail -c 32768
    } > "$destination/sample-$sample.log" 2>&1
    sync
    [ "$sample" = 5 ] || sleep 5
done
