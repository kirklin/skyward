"""Optional: regenerate checked source snapshot using the pinned public AIDL compiler."""
from pathlib import Path
import argparse, base64, hashlib, json, subprocess, urllib.request
parser=argparse.ArgumentParser()
parser.add_argument('--compiler',type=Path,required=True,help='Cache path; downloaded if absent')
parser.add_argument('--out',type=Path,required=True)
args=parser.parse_args();kit=Path(__file__).resolve().parent
proof=json.loads((kit/'AIDL_GENERATION.json').read_text());spec=proof['compiler']
if not args.compiler.exists():
    url=spec['repository']+'/+/'+spec['commit']+'/'+spec['source_path']+'?format=TEXT'
    with urllib.request.urlopen(url,timeout=90) as response: data=base64.b64decode(response.read())
    assert hashlib.sha256(data).hexdigest()==spec['sha256']
    args.compiler.parent.mkdir(parents=True,exist_ok=True)
    args.compiler.write_bytes(data);args.compiler.chmod(0o755)
assert hashlib.sha256(args.compiler.read_bytes()).hexdigest()==spec['sha256']
api=kit/'aidl-api-v1'
inputs=[api/'android/hardware/secure_element'/name for name in
        ('LogicalChannelResponse.aidl','ISecureElement.aidl','ISecureElementCallback.aidl')]
args.out.mkdir(parents=True,exist_ok=True)
subprocess.run([str(args.compiler.resolve()),'--lang=ndk','--structured','--stability=vintf','--version=1',
                '--omit_invocation','-I'+str(api),'--out='+str(args.out),'--header_out='+str(args.out),
                *map(str,inputs)],check=True)
for entry in proof['output_files']:
    path=args.out/Path(entry['path']).relative_to('generated')
    assert hashlib.sha256(path.read_bytes()).hexdigest()==entry['sha256'], path
print('REGENERATED_AND_VERIFIED',len(proof['output_files']),'files')
