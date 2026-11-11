#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Build a local, device-specific overlay for a decrypt-capable Popsicle recovery.

Usage:
  extract-overlay.sh --system DIR --vendor DIR --odm DIR --modem DIR \
    --se-omapi FILE --out DIR

DIR is the root of an extracted logical partition. FILE is a locally built
AOSP external/se_omapi arm64 executable. The output directory must not exist.
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

system_root=
vendor_root=
odm_root=
modem_root=
se_omapi=
out=
while (($#)); do
  case "$1" in
    --system|--vendor|--odm|--modem|--se-omapi|--out)
      (($# >= 2)) || { usage >&2; die "missing value for $1"; }
      key=$1
      value=$2
      case "$key" in
        --system) system_root=$value ;;
        --vendor) vendor_root=$value ;;
        --odm) odm_root=$value ;;
        --modem) modem_root=$value ;;
        --se-omapi) se_omapi=$value ;;
        --out) out=$value ;;
      esac
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "unknown argument: $1" ;;
  esac
done

for pair in "system:$system_root" "vendor:$vendor_root" "odm:$odm_root" "modem:$modem_root"; do
  name=${pair%%:*}
  path=${pair#*:}
  [[ -d "$path" ]] || die "--$name must name an extracted partition directory"
done
[[ -f "$se_omapi" ]] || die "--se-omapi must name a locally built executable"
[[ -n "$out" ]] || { usage >&2; die "--out is required"; }
[[ ! -e "$out" ]] || die "output already exists: $out"

mkdir -p "$out"
script_dir=$(cd "$(dirname "$0")" && pwd)

copy_file() {
  local source=$1 destination=$2 mode=${3:-0644}
  [[ -f "$source" ]] || die "required ROM file is missing: $source"
  mkdir -p "$(dirname "$destination")"
  install -m "$mode" "$source" "$destination"
}

copy_first() {
  local destination=$1
  shift
  local source
  for source in "$@"; do
    if [[ -f "$source" ]]; then
      copy_file "$source" "$out/$destination"
      return
    fi
  done
  die "required file is missing; looked for: $*"
}

copy_file "$script_dir/secure-element-ta-setup.sh" \
  "$out/system/bin/secure-element-ta-setup.sh" 0755

ta_dir="$out/system/bin/twrp_secure_element_ta"
for uuid in \
  05B04A44-BF30-42DF-9E2F-B366B980ED19 \
  32552B22-89FE-42B4-8A45-A0C4E2DB0326 \
  FD719D50-FFFB-11EB-9A03-0242AC130003; do
  for suffix in b00 b01 b02 b03 b04 b05 b06 b07 b08 mdt; do
    name="$uuid.$suffix"
    matches=()
    while IFS= read -r -d '' match; do
      matches+=("$match")
    done < <(find "$modem_root" -type f -name "$name" -print0)
    ((${#matches[@]} == 1)) || die "expected exactly one $name under modem, found ${#matches[@]}"
    copy_file "${matches[0]}" "$ta_dir/$name"
  done
done

for name in cs40l26.wmfw cs40l26.bin cs40l26-calib.wmfw cs40l26-calib.bin \
  cs40l26-a2h.bin cs40l26-a2h1.bin cs40l26-dbc.bin cs40l26-dvl.bin \
  cs40l26-lf0t.bin cs40l26-svc.bin; do
  copy_first "lib/firmware/$name" \
    "$odm_root/firmware/$name" "$odm_root/firmware/image/$name" \
    "$odm_root/etc/firmware/$name" "$vendor_root/firmware/$name" \
    "$vendor_root/firmware/image/$name" "$vendor_root/etc/firmware/$name"
done

omapi_dir="$out/skyward/omapi"
copy_file "$se_omapi" "$omapi_dir/se_omapi" 0755
for name in android.se.omapi-V1-ndk.so libbase.so libbinder_ndk.so libc++.so \
  libcutils.so liblog.so libutils.so libvndksupport.so; do
  copy_first "skyward/omapi/$name" \
    "$system_root/lib64/$name" "$vendor_root/lib64/$name" "$odm_root/lib64/$name"
done

cat > "$omapi_dir/start.sh" <<'EOF'
#!/system/bin/sh
exec /system/bin/linker64 /skyward/omapi/se_omapi
EOF
chmod 0755 "$omapi_dir/start.sh"

cat > "$omapi_dir/secure_element-service.xml" <<'EOF'
<manifest version="1.0" type="framework"><hal format="aidl"><name>android.se.omapi</name><fqname>ISecureElementService/default</fqname></hal></manifest>
EOF

# Keep a local-only provenance record; it contains hashes and paths, never ROM data.
{
  printf 'device=popsicle\n'
  printf 'system=%s\nvendor=%s\nodm=%s\nmodem=%s\n' \
    "$(cd "$system_root" && pwd)" "$(cd "$vendor_root" && pwd)" \
    "$(cd "$odm_root" && pwd)" "$(cd "$modem_root" && pwd)"
  find "$out" -type f -exec shasum -a 256 {} \; | sort
} > "$out/LOCAL-SOURCES.txt"

printf 'Created local Popsicle overlay: %s\n' "$out"
printf 'Do not publish this directory: it contains proprietary ROM/TEE files.\n'
