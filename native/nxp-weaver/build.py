"""Build only inside the selected output directory; no Android tree is required."""
from pathlib import Path
import argparse, hashlib, json, subprocess, sys
sys.dont_write_bytecode=True
from verify_elf import verify

parser=argparse.ArgumentParser()
parser.add_argument('--toolchain',type=Path,required=True)
parser.add_argument('--libraries',type=Path,required=True)
parser.add_argument('--out',type=Path,required=True)
args=parser.parse_args()
kit=Path(__file__).resolve().parent
tool=args.toolchain.resolve()
libraries=args.libraries.resolve()
out=args.out.resolve()
out.mkdir(parents=True,exist_ok=True)
assert out != kit and not kit.is_relative_to(out), 'Output must not contain the source kit'
for entry in json.loads((kit/'KIT_MANIFEST.json').read_text())['files']:
    path=kit/entry['path']
    assert hashlib.sha256(path.read_bytes()).hexdigest()==entry['sha256'], path
for manifest in ('TOOLCHAIN.json','PLATFORM_HEADERS.json'):
    spec=json.loads((kit/manifest).read_text())
    for entry in spec['files']:
        path=(tool if manifest=='TOOLCHAIN.json' else kit)/entry['path']
        assert hashlib.sha256(path.read_bytes()).hexdigest()==entry['sha256'], path
expected=json.loads((kit/'RECOVERY_LIBRARY_INPUTS.json').read_text())
for entry in expected:
    path=libraries/Path(entry['source_archive_path']).name
    assert hashlib.sha256(path.read_bytes()).hexdigest()==entry['sha256'], path
platform=kit/'platform'
prebuilt=platform/'prebuilts/clang/host/linux-x86/clang-r596125'
headers=[kit/'recovery-patch/inc',kit/'weaver/libese_weaver/inc',kit/'keymint/transport/include',kit/'generated',
         platform/'frameworks/native/libs/binder/ndk/include_ndk',platform/'frameworks/native/libs/binder/ndk/include_cpp',
         platform/'frameworks/native/libs/binder/ndk/include_platform',platform/'system/libbase/include',
         platform/'system/logging/liblog/include',platform/'system/logging/liblog/include_vndk',
         prebuilt/'android_libc++/platform/aarch64/include/c++/v1',prebuilt/'include/c++/v1',
         platform/'bionic/libc/include',platform/'bionic/libc/kernel/uapi/asm-arm64',
         platform/'bionic/libc/kernel/uapi',platform/'bionic/libc/kernel/android/uapi',
         platform/'bionic/libc/kernel/android/scsi']
common=['-target','aarch64-linux-android36','-fPIC','-O2','-g','-nostdlibinc',
        '-ffile-prefix-map='+str(kit)+'=/src','-ffile-prefix-map='+str(out)+'=/out',
        '-fdebug-compilation-dir=/src',
        '-resource-dir='+str(prebuilt/'lib/clang/22'),'-DANDROID','-DNDEBUG','-D__ANDROID_VENDOR__',
        '-D__ANDROID_VNDK__','-D__ANDROID_VENDOR_API__=202604','-D__BIONIC_DEPRECATED_PAGE_SIZE_MACRO']
flags=common+['-std=gnu++23','-nostdinc++','-fno-exceptions','-fno-rtti','-fno-strict-aliasing',
              '-fno-short-enums','-DNXP_EXTNS','-DINTERVAL_TIMER','-D_FILE_OFFSET_BITS=64',
              '-Wall','-Wextra','-Wno-sign-compare','-Wno-unused-parameter','-include','cstdint']
flags+=['-I'+str(h) for h in headers]
commands=[]
def run(command,name):
    commands.append(command)
    result=subprocess.run(command,cwd=kit,text=True,capture_output=True)
    (out/(name+'.log')).write_text(result.stdout+result.stderr)
    (out/'COMMANDS.json').write_text(json.dumps(commands,indent=2)+'\n')
    if result.returncode: raise RuntimeError(name+' failed:\n'+result.stderr[-5000:])
    print(name,'OK',flush=True)
clang=str(tool/'bin/clang-22')
sources=sorted((kit/'weaver/libese_weaver/src').glob('*.cpp'))
sources += [kit/'keymint/transport'/name for name in ('AppletConnection.cpp','HalToHalTransport.cpp',
             'IntervalTimer.cpp','SBAccessController.cpp','SignalHandler.cpp','EseTransportUtils.cpp')]
sources += sorted((kit/'generated').rglob('*.cpp'))
assert len(sources)==12
objects=[]
for source in sources:
    replacement=kit/'recovery-patch/src'/source.name
    if replacement.exists(): source=replacement
    obj=out/(source.stem+'.o')
    run([clang,*flags,'-MD','-MF',str(out/(source.stem+'.d')),'-c',str(source),'-o',str(obj)],source.stem)
    objects.append(obj)
probe=kit/'abi-probe.cpp'
run([clang,*flags,'-c',str(probe),'-o',str(out/'abi-probe.o')],'ABI-layout-probe')
crt=platform/'bionic/libc/arch-common/bionic'
crt_flags=common+['-I'+str(platform/'bionic/libc'),*['-I'+str(h) for h in headers if 'bionic' in str(h)]]
for name in ('crtbegin_so.c','crtend_so.S'):
    run([clang,*crt_flags,'-MD','-MF',str(out/(name+'.d')),'-c',str(crt/name),'-o',str(out/(Path(name).stem+'.o'))],name)
exports=out/'exports.map'
exports.write_text('{ global: _ZN10WeaverImpl*; _ZTV10WeaverImpl; local: *; };\n')
library=out/'ese_weaver.so'
linker=out/'ld.lld'
if not linker.exists(): linker.symlink_to(tool/'bin/lld')
inputs=[libraries/name for name in ('libc++.so','libbase.so','liblog.so','libbinder_ndk.so','libc.so','libm.so','libdl.so')]
builtins=tool/'lib/clang/22/lib/linux/libclang_rt.builtins-aarch64-android.a'
run([clang,'-target','aarch64-linux-android36','--ld-path='+str(linker),'-shared','-nostdlib',
     '-Wl,--no-undefined','-Wl,-z,relro','-Wl,-z,now','-Wl,-z,noexecstack','-Wl,-z,max-page-size=16384',
     '-Wl,--build-id=sha1','-Wl,-soname,ese_weaver.so','-Wl,--version-script='+str(exports),
     str(out/'crtbegin_so.o'),*map(str,objects),*map(str,inputs),str(builtins),str(out/'crtend_so.o'),'-o',str(library)],'strict-link')
run([str(tool/'bin/llvm-objcopy'),'--strip-debug',str(library)],'strip-debug')
# Dependency files must resolve only within the bundled source kit or output.
dependencies=set()
for dep in out.glob('*.d'):
    dependencies.update(Path(p).resolve() for p in dep.read_text().replace('\\\n',' ').split(':',1)[1].split())
assert all(p.is_relative_to(kit) or p.is_relative_to(out) for p in dependencies), sorted(dependencies)
proof=verify(library,libraries)
proof.update({'source_compiled':True,'strict_link':True,'platform_tree_required':False,
              'platform_header_dependencies':len(dependencies),
              'compiler_version':subprocess.check_output([clang,'--version'],text=True).replace(str(tool),'<TOOLCHAIN>'),
              'crt_objects_built_from_bundled_pinned_source':True})
(out/'BUILD_PROOF.json').write_text(json.dumps(proof,indent=2)+'\n')
print('BUILD_COMPLETE',proof['sha256'],proof['bytes'],flush=True)
