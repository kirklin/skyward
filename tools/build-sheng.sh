#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")/.."
stock=${1:?Usage: bash tools/build-sheng.sh /read-only/path/recovery.img [output-directory]}
out=${2:-target/sheng}
mkdir -p "$out"
out=$(realpath "$out")
stock=$(realpath "$stock")
target_dir=${CARGO_TARGET_DIR:-target}
target_dir=$(realpath -m "$target_dir")
cargo ndk -t arm64-v8a --platform 30 build --locked --release -p skyward
cargo build --locked --release -p skyward-pack
version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')
name=skyward-v${version}-Pad-sheng-OS3.0.303.0.WNXCNXM-experimental
"$target_dir/release/skyward-pack" recovery \
    --stock "$stock" \
    --binary "$target_dir/aarch64-linux-android/release/skyward" \
    --config devices/sheng.toml --overlay devices/sheng/ramdisk \
    --font resources/fonts/MiSansVF.ttf \
    --mono resources/fonts/KirkMono-Regular.ttf \
    --mono-license resources/fonts/KirkMono-OFL.txt \
    --out "$out/$name.img"
cp README.zh-CN.md "$out/README.zh-CN.md"
cp devices/sheng/provenance.json "$out/source-provenance.json"
python3 - "$out" "$name" <<'PY'
import hashlib
import sys
import zipfile
from pathlib import Path
out, name = Path(sys.argv[1]), sys.argv[2]
image = out / (name + ".img")
digest = hashlib.sha256(image.read_bytes()).hexdigest()
(out / "SHA256SUMS").write_text(f"{digest}  {image.name}\n")
with zipfile.ZipFile(out / (name + ".zip"), "w", zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
    for path in (image, out / "SHA256SUMS", out / "README.zh-CN.md", out / "source-provenance.json"):
        archive.write(path, path.name)
print(f"Built {out / (name + '.zip')}")
print(f"Image SHA-256: {digest}")
PY
