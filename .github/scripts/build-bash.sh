#!/usr/bin/env bash
# Build on a native AArch64 Linux runner with musl-tools installed.
# GNU source and every patch are pinned; no phone or stock library is used.
set -euo pipefail
[[ $(uname -m) == aarch64 ]]
scripts=$(cd -- "$(dirname -- "$0")" && pwd)
out=$(realpath -m -- "${1:?give an output directory}")
work=$(mktemp -d)
mkdir -p "$out" "$work/source"
cd "$work/source"
curl --fail --location --retry 2 https://ftp.gnu.org/gnu/bash/bash-5.3.tar.gz -o bash-5.3.tar.gz
for number in $(seq -f '%03g' 20); do
    curl --fail --location --retry 2 "https://ftp.gnu.org/gnu/bash/bash-5.3-patches/bash53-$number" -o "bash53-$number"
done
sha256sum --check "$scripts/bash-sources.sha256"
cp "$scripts/bash-sources.sha256" .
cp "$scripts/build-bash.sh" "$scripts/bash-smoke.sh" .
# Keep exactly the corresponding source and build instructions with binaries.
tar -czf "$out/bash-source.tar.gz" .
tar -xzf bash-5.3.tar.gz
cd bash-5.3
for number in $(seq -f '%03g' 20); do patch -p0 -i "../bash53-$number"; done
CC=musl-gcc CFLAGS='-Os' LDFLAGS='-static' bash_cv_dev_fd=whacky \
    ./configure --prefix=/system --without-bash-malloc --disable-nls --disable-readline --enable-static-link
make -j"$(nproc)"
strip bash
readelf -h bash | grep -q 'Machine:.*AArch64'
if readelf -l bash | grep -q INTERP || readelf -d bash | grep -q NEEDED; then
    echo 'Bash must have no dynamic interpreter or library dependency' >&2
    exit 1
fi
./bash --noprofile --norc "$scripts/bash-smoke.sh" 3 3 synthetic.zip 3>"$work/requests"
grep -q '^ui_print bash-smoke-ok$' "$work/requests"
cp bash "$out/bash"
cp COPYING "$out/COPYING"
cp /usr/share/doc/musl/copyright "$out/musl-COPYRIGHT"
{
    echo 'GNU Bash 5.3.20, GPL-3.0-or-later; musl libc, MIT.'
    echo 'Built by .github/scripts/build-bash.sh, statically for AArch64 Linux.'
    echo 'Source: https://ftp.gnu.org/gnu/bash/; exact archive and patches below.'
    echo 'Corresponding source/build script: bash-source.tar.gz in the same release.'
    echo 'Readline and NLS disabled; /proc/self/fd used for process substitution.'
    dpkg-query -W musl musl-dev musl-tools gcc
    cat "$scripts/bash-sources.sha256"
} > "$out/SOURCE"
sha256sum "$out/bash"
