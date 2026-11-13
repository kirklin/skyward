"""Fetch only six pinned AOSP Linux x86_64 compiler files (about 252 MB)."""
from pathlib import Path
import argparse, base64, hashlib, json, os, time, urllib.request

parser=argparse.ArgumentParser()
parser.add_argument('--out',type=Path,required=True)
parser.add_argument('--only',help='Optional one manifest path for download verification')
args=parser.parse_args()
spec=json.loads((Path(__file__).parent/'TOOLCHAIN.json').read_text())
for entry in spec['files']:
    if args.only and entry['path'] != args.only: continue
    target=args.out/entry['path']
    if target.exists() and hashlib.sha256(target.read_bytes()).hexdigest()==entry['sha256']:
        print('VERIFIED_CACHE',entry['path'],flush=True);continue
    url=spec['repository']+'/+/'+spec['commit']+'/'+entry['source_path']+'?format=TEXT'
    print('FETCHING',entry['path'],entry['bytes'],flush=True)
    for attempt in range(3):
        try:
            with urllib.request.urlopen(url,timeout=180) as response:
                data=base64.b64decode(response.read(),validate=False)
            assert len(data)==entry['bytes']
            assert hashlib.sha256(data).hexdigest()==entry['sha256']
            blob=hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()
            assert blob==entry['git_blob_sha1']
            break
        except Exception:
            if attempt==2: raise
            time.sleep(2)
    target.parent.mkdir(parents=True,exist_ok=True)
    temporary=target.with_name(target.name+'.download')
    temporary.write_bytes(data)
    temporary.chmod(0o755 if entry['path'].startswith('bin/') else 0o644)
    os.replace(temporary,target)
    print('VERIFIED_DOWNLOAD',entry['path'],entry['sha256'],flush=True)
