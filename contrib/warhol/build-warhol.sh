#!/bin/sh
# Offline build only. No adb, fastboot, phone writes, or automatic download.
set -eu
cd "$(dirname "$0")/../.."
root=$(pwd)
export CARGO_HOME="${CARGO_HOME:-$root/local/tools/cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$root/local/tools/rustup}"
export PATH="$CARGO_HOME/bin:$PATH"
ndk="${ANDROID_NDK_HOME:-$root/local/tools/android-ndk-r29}"
case "$(uname -s)" in
  Darwin) host=darwin-x86_64 ;;
  Linux) host=linux-x86_64 ;;
  *) echo 'Build on macOS or Linux.' >&2; exit 1 ;;
esac
bin="$ndk/toolchains/llvm/prebuilt/$host/bin"
test -x "$bin/aarch64-linux-android30-clang"
export CC_aarch64_linux_android="$bin/aarch64-linux-android30-clang"
export CXX_aarch64_linux_android="$bin/aarch64-linux-android30-clang++"
export AR_aarch64_linux_android="$bin/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_aarch64_linux_android"
cargo build --locked --offline --release --target aarch64-linux-android -p skyward
python3 contrib/warhol/warhol_fonts.py
python3 contrib/warhol/warhol_vendor_boot.py \
  --stock local/stock/vendor_boot.img \
  --binary target/aarch64-linux-android/release/skyward \
  --fonts local/bringup-fonts \
  --out dist/skyward-warhol-Global311-bringup-v1-vendor_boot.img
