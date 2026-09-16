#!/usr/bin/env python3
"""Regenerate all three bindings in lockstep and fail on committed-source drift."""
import hashlib, pathlib, subprocess
ROOT=pathlib.Path(__file__).resolve().parents[1]/'platforms/mesh_host'
paths=['lib/src/mesh_api.g.dart','ios/mesh_host/Sources/mesh_host/MeshApi.g.swift','android/src/main/kotlin/com/frazko/mesh_host/MeshApi.g.kt']
before={p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in paths}
subprocess.run(['dart','run','pigeon','--input','pigeons/mesh_api.dart'],cwd=ROOT,check=True)
assert all(hashlib.sha256((ROOT/p).read_bytes()).hexdigest()==before[p] for p in paths), 'Pigeon bindings changed. Review and include all three generated files.'
print('PASS: Pigeon sources are reproducible')
