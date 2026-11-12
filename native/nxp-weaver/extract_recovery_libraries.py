"""Extract exactly seven hash-pinned libraries from the public myron 4.0.6 REC image."""
from pathlib import Path
import argparse, hashlib, json, struct

RECOVERY_SHA256='49622932fb32ab623db336866734ad440a2e15af5f750ceb9b1d3d0f7fdabafc'

def lz4_block(data):
    output=bytearray(); pos=0
    def length(base):
        nonlocal pos
        value=base
        if base == 15:
            while True:
                extra=data[pos];pos+=1;value+=extra
                if extra != 255: break
        return value
    while pos < len(data):
        token=data[pos];pos+=1
        literals=length(token >> 4)
        output.extend(data[pos:pos+literals]);pos+=literals
        if pos == len(data): break
        distance=struct.unpack_from('<H',data,pos)[0];pos+=2
        count=length(token & 15)+4
        assert 0 < distance <= len(output)
        pattern=output[-distance:]
        output.extend((pattern*((count+distance-1)//distance))[:count])
        assert len(output) <= 8*1024*1024
    return output

def unpack_ramdisk(image):
    assert image[:8] == b'ANDROID!'
    kernel, size = struct.unpack_from('<II',image,8)
    assert struct.unpack_from('<I',image,40)[0] == 4
    assert kernel == 0
    data=image[4096:4096+size]
    assert data[:4] == b'\x02\x21\x4c\x18', 'Expected legacy LZ4'
    pos=4;output=bytearray()
    while pos < len(data):
        count=struct.unpack_from('<I',data,pos)[0];pos+=4
        if count == 0: break
        output.extend(lz4_block(data[pos:pos+count]));pos+=count
        assert len(output) <= 256*1024*1024
    return output

def extract(image, out):
    data=Path(image).read_bytes()
    assert hashlib.sha256(data).hexdigest()==RECOVERY_SHA256, 'REC image SHA256 mismatch'
    ramdisk=unpack_ramdisk(data)
    spec=json.loads((Path(__file__).parent/'RECOVERY_LIBRARY_INPUTS.json').read_text())
    wanted={x['source_archive_path']:x for x in spec}
    results={};pos=0
    while pos+110 <= len(ramdisk):
        header=ramdisk[pos:pos+110]
        assert header[:6] in (b'070701',b'070702')
        fields=[int(header[i:i+8],16) for i in range(6,110,8)]
        name=bytes(ramdisk[pos+110:pos+110+fields[11]-1]).decode()
        pos=(pos+110+fields[11]+3)&~3
        contents=ramdisk[pos:pos+fields[6]]
        pos=(pos+fields[6]+3)&~3
        if name == 'TRAILER!!!': break
        name=name.removeprefix('./')
        if name in wanted:
            assert hashlib.sha256(contents).hexdigest()==wanted[name]['sha256'], name
            results[Path(name).name]=contents
    assert len(results)==7, list(results)
    out=Path(out);out.mkdir(parents=True,exist_ok=True)
    for name,contents in results.items(): (out/name).write_bytes(contents)
    print('EXTRACTED',len(results),'hash-verified recovery libraries')

if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('image',type=Path)
    parser.add_argument('--out',type=Path,required=True)
    args=parser.parse_args()
    extract(args.image,args.out)
