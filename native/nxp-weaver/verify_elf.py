"""Check Android ELF64 imports, including GNU symbol versions, without pyelftools."""
from pathlib import Path
import argparse, hashlib, json, struct

class Elf:
    def __init__(self, path):
        self.path = Path(path)
        self.data = self.path.read_bytes()
        assert self.data[:6] == b'\x7fELF\x02\x01', self.path
        h = struct.unpack_from('<16sHHIQQQIHHHHHH', self.data)
        assert h[2] == 183, 'Expected AArch64'
        self.sections = [struct.unpack_from('<IIQQQQIIQQ', self.data, h[6] + i*h[11]) for i in range(h[12])]
        names = self.section_data(self.sections[h[13]])
        self.named = {self.string(names, s[0]): s for s in self.sections}

    @staticmethod
    def string(data, offset):
        return data[offset:data.index(b'\0', offset)].decode()

    def section_data(self, section):
        return self.data[section[4]:section[4]+section[5]]

    def symbols(self):
        section = self.named['.dynsym']
        strings = self.section_data(self.sections[section[6]])
        versions = {}
        if '.gnu.version_r' in self.named:
            data = self.section_data(self.named['.gnu.version_r'])
            offset = 0
            while offset < len(data):
                _, count, _, aux, nxt = struct.unpack_from('<HHIII', data, offset)
                pos = offset + aux
                for _ in range(count):
                    _, _, index, name, step = struct.unpack_from('<IHHII', data, pos)
                    versions[index] = self.string(strings, name)
                    pos += step
                if not nxt: break
                offset += nxt
        if '.gnu.version_d' in self.named:
            data = self.section_data(self.named['.gnu.version_d'])
            offset = 0
            while offset < len(data):
                _, _, index, _, _, aux, nxt = struct.unpack_from('<HHHHIII', data, offset)
                name, _ = struct.unpack_from('<II', data, offset + aux)
                versions[index] = self.string(strings, name)
                if not nxt: break
                offset += nxt
        versym = self.section_data(self.named['.gnu.version']) if '.gnu.version' in self.named else None
        out = []
        for i, offset in enumerate(range(section[4], section[4]+section[5], section[9])):
            name, info, visibility, shndx, value, size = struct.unpack_from('<IBBHQQ', self.data, offset)
            if not name: continue
            version_index = struct.unpack_from('<H', versym, 2*i)[0] if versym else 1
            out.append({'name':self.string(strings,name), 'version':versions.get(version_index & 0x7fff),
                        'version_hidden':bool(version_index & 0x8000), 'defined':bool(shndx),
                        'binding':info >> 4, 'visibility':visibility & 3, 'size':size,'address':value})
        return out

    def needed(self):
        section = self.named['.dynamic']
        strings = self.section_data(self.sections[section[6]])
        return [self.string(strings,value) for tag,value in struct.iter_unpack('<qQ',self.section_data(section)) if tag == 1]

    def public_vtable(self):
        symbols=self.symbols()
        table=next(s for s in symbols if s['name']=='_ZTV10WeaverImpl')
        dynsym=self.named['.dynsym']
        relocations={}
        for section in self.sections:
            if section[1] != 4: continue
            for offset,info,addend in struct.iter_unpack('<QQq',self.section_data(section)):
                kind,index=info & 0xffffffff,info >> 32
                if kind == 1027: relocations[offset]=addend
                elif kind == 257:
                    value=struct.unpack_from('<Q',self.data,dynsym[4]+index*24+8)[0]
                    relocations[offset]=value+addend
        methods=[]
        for index in range(5):
            value=relocations[table['address']+16+8*index]
            methods.append([s['name'] for s in symbols if s['defined'] and s['address']==value])
        return methods

def verify(library, library_dir):
    elf = Elf(library)
    providers = {}
    inputs = {}
    for path in sorted(Path(library_dir).glob('*.so')):
        target = Elf(path)
        inputs[path.name] = hashlib.sha256(target.data).hexdigest()
        for symbol in target.symbols():
            if symbol['defined'] and symbol['binding'] in (1,2) and symbol['visibility'] in (0,3):
                providers.setdefault(symbol['name'], []).append((path.name,symbol))
    imports = []
    exports = []
    unresolved = []
    for symbol in elf.symbols():
        if symbol['defined']:
            exports.append(symbol)
            continue
        matches = [(name,s) for name,s in providers.get(symbol['name'],[]) if
                   (s['version'] == symbol['version'] if symbol['version'] else not s['version_hidden'])]
        record = {**symbol, 'providers':[name for name,_ in matches]}
        imports.append(record)
        if not matches and symbol['binding'] != 2: unresolved.append(record)
    expected = {'libc++.so','libbase.so','liblog.so','libbinder_ndk.so','libc.so','libm.so','libdl.so'}
    assert set(elf.needed()) == expected, elf.needed()
    assert not unresolved, unresolved
    assert all(s['name'].startswith(('_ZN10WeaverImpl','_ZTV10WeaverImpl')) for s in exports), exports
    assert any(s['name'] == '_ZN10WeaverImpl11getInstanceEv' for s in exports)
    assert any(s['name'] == '_ZTV10WeaverImpl' and s['size'] == 72 for s in exports)
    expected_methods=[
        '_ZN10WeaverImpl4InitEv','_ZN10WeaverImpl8GetSlotsER8SlotInfo',
        '_ZN10WeaverImpl4ReadEjRKNSt3__16vectorIhNS0_9allocatorIhEEEER12ReadRespInfo',
        '_ZN10WeaverImpl5WriteEjRKNSt3__16vectorIhNS0_9allocatorIhEEEES6_',
        '_ZN10WeaverImpl6DeInitEv']
    actual_methods=elf.public_vtable()
    assert all(wanted in actual for wanted,actual in zip(expected_methods,actual_methods)),actual_methods
    assert not any('__ndk1' in s['name'] for s in imports)
    assert not any('keymint' in s['name'] or 'Omapi' in s['name'] for s in imports)
    return {'bytes':len(elf.data),'sha256':hashlib.sha256(elf.data).hexdigest(), 'needed':elf.needed(),
            'all_strong_import_names_and_versions_provided':True, 'unresolved_strong_imports':unresolved,
            'weak_unresolved_imports':[s for s in imports if not s['providers']],
            'exports':exports,'imports':imports,'library_inputs':inputs,'device_executed':False,
            'public_vtable_bytes':72,'public_vtable_first_five_methods':expected_methods}

if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('library',type=Path)
    parser.add_argument('library_dir',type=Path)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    proof=verify(args.library,args.library_dir)
    args.output.write_text(json.dumps(proof,indent=2)+'\n')
    print('VERIFIED',proof['sha256'],len(proof['imports']),'imports',len(proof['exports']),'exports')
