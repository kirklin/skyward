from pathlib import Path
import argparse, json, subprocess
parser=argparse.ArgumentParser()
parser.add_argument('--cxx',default='c++')
parser.add_argument('--out',type=Path,required=True)
args=parser.parse_args();kit=Path(__file__).resolve().parent
args.out.mkdir(parents=True,exist_ok=True)
binary=args.out/'transport-replay-test'
cmd=[args.cxx,'-std=c++17','-O1','-g','-include','cstdint','-fsanitize=address,undefined','-fno-omit-frame-pointer',
     '-I'+str(kit/'tests/fakes'),'-I'+str(kit/'weaver/libese_weaver/inc'),
     str(kit/'tests/transport_replay_test.cpp'),str(kit/'recovery-patch/src/weaver-transport-impl.cpp'),
     str(kit/'recovery-patch/src/weaver-parser-impl.cpp'),'-o',str(binary)]
subprocess.run(cmd,check=True)
result=subprocess.run([str(binary)],text=True,capture_output=True)
(args.out/'TRANSPORT_TEST.json').write_text(json.dumps({'command':cmd,'returncode':result.returncode,
    'stdout':result.stdout,'stderr':result.stderr,'real_device_or_APDU_used':False},indent=2)+'\n')
print(result.stdout,end='');print(result.stderr,end='')
assert result.returncode==0
