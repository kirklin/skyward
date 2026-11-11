#!/system/bin/sh
# Stage the device's secure-element applets where QTI's secure-world loader
# searches for them during recovery.

SRC_DIR="/system/bin/twrp_secure_element_ta"
OUT_ROOT="/tmp/secure_element_fwroot"
OUT_DIR="$OUT_ROOT/image"
FW_MNT="/firmware"
FW_RUNTIME_IMAGE="/vendor/firmware_mnt/image"

log_step() {
    log -t skyward-secelt "$1"
}

fail() {
    log_step "$1"
    exit 1
}

[ -d "$SRC_DIR" ] || fail "secure-element TA directory is missing"
rm -rf "$OUT_ROOT" || fail "cannot clear temporary TA staging directory"
mkdir -p "$OUT_DIR" || fail "cannot create temporary TA staging directory"
chmod 0755 "$OUT_ROOT" "$OUT_DIR" || fail "cannot set TA staging permissions"

if [ -d "$FW_MNT/image" ]; then
    cp -af "$FW_MNT/image"/. "$OUT_DIR"/ || fail "cannot copy modem firmware"
fi
cp -af "$SRC_DIR"/. "$OUT_DIR"/ || fail "cannot stage secure-element applets"

stage_optional_runtime_file() {
    name=$1
    for dir in \
        "$FW_MNT/image" \
        "$FW_MNT" \
        /vendor/firmware/image \
        /vendor/firmware \
        /vendor/firmware_mnt/image \
        /vendor/firmware_mnt \
        /odm/firmware/image \
        /odm/firmware; do
        if [ -f "$dir/$name" ]; then
            cp -af "$dir/$name" "$OUT_DIR/$name" || fail "cannot stage $name"
            return 0
        fi
    done
    log_step "optional runtime file unavailable: $name"
}

stage_optional_runtime_file qwes.mbn
stage_optional_runtime_file haventkn.mbn
stage_optional_runtime_file tzsc.mbn

alias_chain() {
    alias_base=$1
    uuid_base=$2
    for suffix in b00 b01 b02 b03 b04 b05 b06 b07 b08 mdt; do
        source="$OUT_DIR/$uuid_base.$suffix"
        [ -f "$source" ] || fail "secure-element applet file is missing: $uuid_base.$suffix"
        cp -af "$source" "$OUT_DIR/$alias_base.$suffix" || fail "cannot stage $alias_base.$suffix"
    done
}

alias_chain eseservice 05B04A44-BF30-42DF-9E2F-B366B980ED19
alias_chain gpqese 32552B22-89FE-42B4-8A45-A0C4E2DB0326
alias_chain st_eseservice FD719D50-FFFB-11EB-9A03-0242AC130003

for runtime in "$FW_RUNTIME_IMAGE" /firmware/image; do
    mkdir -p "$runtime" || fail "cannot create firmware lookup path: $runtime"
    if ! grep -q " $runtime " /proc/mounts; then
        mount -t tmpfs -o mode=0755 tmpfs "$runtime" || fail "cannot mount firmware overlay: $runtime"
    fi
    cp -af "$OUT_DIR"/. "$runtime"/ || fail "cannot publish secure-element firmware: $runtime"
done

log_step "secure-element firmware staged"
