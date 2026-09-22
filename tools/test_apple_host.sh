#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
"$HOME/.cargo/bin/cargo" build --locked -p mesh-ffi-c
swiftc platforms/mesh_host/ios/mesh_host/Sources/mesh_host/MeshEnvelope.swift platforms/mesh_host/ios/mesh_host/Sources/mesh_host/BleFrameCodec.swift platforms/mesh_host/ios/mesh_host/Sources/mesh_host/BleWriteQueue.swift platforms/mesh_host/ios/mesh_host/Sources/mesh_host/NativeOutput.swift platforms/apple/tests/main.swift \
  -import-objc-header platforms/mesh_host/ios/Classes/include/mesh_engine.h \
  -L target/debug -lmesh_ffi_c -Xlinker -rpath -Xlinker "$PWD/target/debug" \
  -o target/mesh-apple-contract-tests
./target/mesh-apple-contract-tests
