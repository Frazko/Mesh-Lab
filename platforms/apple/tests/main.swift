import Foundation

let input = [UInt8](arrayLiteral: 0x83,1,1,0x86,1,0,0,0,0xf4,0x80)
var decoder = try MeshEnvelope(input)
let initial = try decoder.snapshot(method: 1)
assert(initial.runtime == 1)
for end in 0..<input.count {
  do { var bad = try MeshEnvelope(Array(input.prefix(end))); _ = try bad.snapshot(method: 1); fatalError("Truncation accepted") }
  catch { }
}
for _ in 0..<1000 {
  var handle: UInt64 = 0
  assert(mesh_abi_version() == 1)
  assert(mesh_runtime_create(1, &handle) == 0)
  for (method, arg) in [(UInt8(0),Int64(0)),(1,0),(2,42),(2,42),(1,0)] {
    let request = try MeshEnvelope.request(method: method, argument: arg)
    var buffer = MeshBuffer(ptr: nil, len: 0)
    let code = request.withUnsafeBufferPointer { mesh_runtime_request(handle,$0.baseAddress,$0.count,&buffer) }
    assert(code == 0)
    do {
      defer { mesh_buffer_release(buffer) }
      var reply = try MeshEnvelope(Array(UnsafeBufferPointer(start: buffer.ptr!,count: buffer.len)))
      if method == 0 { let info = try reply.info(); assert(info.phase == "F0") }
      else { let s = try reply.snapshot(method: Int64(method)); assert(s.runtime == handle); assert(s.cursor <= 1) }
    }
  }
  assert(mesh_runtime_release(handle) == 0)
  var buffer = MeshBuffer(ptr: nil, len: 0)
  let req: [UInt8] = [0x83,1,0,0]
  let code = req.withUnsafeBufferPointer { mesh_runtime_request(handle,$0.baseAddress,$0.count,&buffer) }
  assert(code == 3); assert(buffer.ptr == nil)
}
print("PASS: Swift CBOR vectors, native round trip and 1000 runtime lifecycles")

var publicBuffer = MeshBuffer(ptr: nil, len: 0)
let seed = [UInt8](repeating: 0, count: 32)
assert(seed.withUnsafeBufferPointer { mesh_identity_public($0.baseAddress, $0.count, &publicBuffer) } == 0)
assert(publicBuffer.len == 32)
let keyHex = Array(UnsafeBufferPointer(start: publicBuffer.ptr!, count: publicBuffer.len)).map { String(format: "%02x", $0) }.joined()
assert(keyHex == "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29")
mesh_buffer_release(publicBuffer)
print("PASS: Swift native identity public-key vector")

var route = [UInt8](repeating: 0, count: 91)
route[0] = 1
for index in 1...16 { route[index] = 5 }
for index in 17...48 { route[index] = 1 }
for index in 49...80 { route[index] = 2 }
route[81] = 1; route[82] = 4; route[90] = 100
let durable: [UInt8] = [0x6d, 1, 1, 0, 3, 9, 8, 7]
var routed = MeshBuffer(ptr: nil, len: 0)
assert(route.withUnsafeBufferPointer { route in
  durable.withUnsafeBufferPointer { durable in
    mesh_routed_record_encode(route.baseAddress, route.count,
                              durable.baseAddress, durable.count, &routed)
  }
} == 0)
defer { mesh_buffer_release(routed) }
assert(routed.len == 2 + route.count + durable.count)
var unpacked = MeshBuffer(ptr: nil, len: 0)
assert(mesh_routed_record_decode(routed.ptr, routed.len, &unpacked) == 0)
defer { mesh_buffer_release(unpacked) }
assert(Array(UnsafeBufferPointer(start: unpacked.ptr!, count: unpacked.len)) == route + durable)
var invalid = MeshBuffer(ptr: nil, len: 0)
assert(mesh_routed_record_decode(routed.ptr, routed.len - 1, &invalid) == 1)
assert(invalid.ptr == nil)
print("PASS: Swift routed-record FFI canonical round trip")

// Protected frames can be larger than 255 bytes. This must encode the low
// length byte instead of trapping in a release build, then round-trip exactly.
let largeFrame = [UInt8](repeating: 0xa5, count: 4_157)
let fragments = BleFrameCodec.split(largeFrame, maximumWrite: 20)!
let assembler = BleFrameCodec.Assembler()
var rebuilt: [UInt8]?
for fragment in fragments { rebuilt = assembler.accept(fragment) ?? rebuilt }
assert(rebuilt == largeFrame)
print("PASS: Swift BLE large-frame segmentation")
