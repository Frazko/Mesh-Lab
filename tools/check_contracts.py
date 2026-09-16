#!/usr/bin/env python3
"""Check the registered F0 vectors against the actual host library and source boundaries."""
import ctypes, json, pathlib, re, subprocess, sys
ROOT=pathlib.Path(__file__).resolve().parents[1]
registry=json.loads((ROOT/'schema/api/registry.json').read_text())
for family in ['methods','states','events','errors']:
    values=list(registry[family].values())
    assert len(values)==len(set(values)), f'Collision in {family}'
for crate in ['mesh-types','mesh-codec','mesh-runtime','mesh-object','mesh-replication','mesh-crypto','mesh-protocol','mesh-session']:
    for source in (ROOT/'crates'/crate/'src').rglob('*.rs'):
        assert not re.search(r'\bstd::(?:fs|net|thread|time)\b|\bextern\s+"C"|\bunsafe\s*\{',source.read_text()), source
for source in (ROOT/'app/lib').rglob('*.dart'):
    assert not re.search(r"import ['\"](?:dart:ffi|.*bluetooth|.*geolocator|.*record/)",source.read_text()), source
subprocess.run([str(pathlib.Path.home()/'.cargo/bin/cargo'),'build','--locked','-p','mesh-ffi-c'],cwd=ROOT,check=True)
lib=ctypes.CDLL(str(ROOT/('target/debug/libmesh_ffi_c.dylib' if sys.platform=='darwin' else 'target/debug/libmesh_ffi_c.so')))
class Buffer(ctypes.Structure):
    _fields_=[('ptr',ctypes.c_void_p),('len',ctypes.c_size_t)]
lib.mesh_runtime_create.argtypes=[ctypes.c_uint32,ctypes.POINTER(ctypes.c_uint64)]
lib.mesh_runtime_request.argtypes=[ctypes.c_uint64,ctypes.c_void_p,ctypes.c_size_t,ctypes.POINTER(Buffer)]
lib.mesh_buffer_release.argtypes=[Buffer]
lib.mesh_runtime_release.argtypes=[ctypes.c_uint64]
handle=ctypes.c_uint64()
assert lib.mesh_runtime_create(1,ctypes.byref(handle))==0
try:
    for vector in json.loads((ROOT/'vectors/api/foundation.json').read_text())['vectors']:
        raw=bytes.fromhex(vector['request']); out=Buffer()
        code=lib.mesh_runtime_request(handle.value,raw,len(raw),ctypes.byref(out))
        try:
            assert (code==0)==vector['valid'],vector['name']
            if 'snapshotRuntime1' in vector:
                assert handle.value==1
                assert ctypes.string_at(out.ptr,out.len).hex()==vector['snapshotRuntime1']
        finally: lib.mesh_buffer_release(out)
finally: assert lib.mesh_runtime_release(handle.value)==0
print('PASS: registry uniqueness, Rust/Dart boundaries and executable CBOR vectors')
