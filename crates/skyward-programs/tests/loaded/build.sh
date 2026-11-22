# Links the files beside this one anew, with the NDK's compiler:
#   ANDROID_NDK_HOME=... sh build.sh
# Each library holds nothing; what the tests read is what each asks its
# loader for. libgone.so and ld-android.so are linked against and kept out
# of the folder: the one stands for a library no folder has, the other for
# the loader itself.
set -eu
cd "$(dirname "$0")"
cc=$(ls "$ANDROID_NDK_HOME"/toolchains/llvm/prebuilt/*/bin/aarch64-linux-android30-clang)
made=$(mktemp -d)
echo 'void _start(void) {}' > "$made/program.c"
: > "$made/nothing.c"
library() {
    name=$1
    shift
    "$cc" -shared -nostdlib -s -Wl,--build-id=none -Wl,-z,max-page-size=4096 -Wl,--no-as-needed -Wl,-soname,"$name" -L"$made" -o "$made/$name" "$made/nothing.c" "$@"
}
library ld-android.so
library libgone.so
library libmore.so
library libdeep.so
library libbootstrap.so
library libextra.so
library libown.so -l:libmore.so -l:libdeep.so '-Wl,-rpath,$ORIGIN/more'
library libshared.so -l:libbootstrap.so -l:libextra.so -l:ld-android.so '-Wl,-rpath,${ORIGIN}/extra/$LIB'
"$cc" -nostdlib -s -Wl,--build-id=none -Wl,-z,max-page-size=4096 -Wl,--no-as-needed -L"$made" -o "$made/program" "$made/program.c" -l:libown.so -l:libshared.so -l:libgone.so
for file in program libown.so libmore.so libdeep.so libshared.so libbootstrap.so libextra.so; do
    cp "$made/$file" "$file"
done
rm -r "$made"
