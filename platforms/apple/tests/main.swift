import Foundation

let stalledHealth = BleLinkHealth(now: 0)
stalledHealth.received(now: 11, authenticated: false)
assert(!stalledHealth.probeDue(now: 11))
assert(!stalledHealth.expired(now: 11.999))
assert(stalledHealth.expired(now: 12))
let healthyBle = BleLinkHealth(now: 0)
healthyBle.received(now: 1, authenticated: true)
assert(!healthyBle.probeDue(now: 2.999))
for time in stride(from: 3.0, through: 60.0, by: 3.0) {
  assert(healthyBle.probeDue(now: time))
  assert(!healthyBle.probeDue(now: time + 0.001))
  healthyBle.received(now: time + 0.1, authenticated: true)
  assert(!healthyBle.expired(now: time + 11.999))
}
assert(healthyBle.expired(now: 72.101))
assert(!healthyBle.probeDue(now: 72.101))
let replacementBle = BleLinkHealth(now: 72)
assert(!replacementBle.probeDue(now: 75))
replacementBle.received(now: 75, authenticated: true)
assert(replacementBle.probeDue(now: 75))
assert(stalledHealth.expired(now: 75))
assert(!BleLinkHealth().expired(now: ProcessInfo.processInfo.systemUptime))
print("PASS: BLE liveness deadlines, authenticated progress, probes and independent reconnection")

let probeScheduler = DurableRecordHistory()
let probeWrites = BleWriteQueue()
let probeReceiver = BleFrameCodec.Assembler()
let probeVoice = (0...2).map { index in (0..<4096).map { UInt8(truncatingIfNeeded: $0 + index) } }
let healthProbe: [UInt8] = [0x7d, 0, 0, 0, 1]
probeScheduler.enqueue(probeVoice)
probeWrites.append(BleFrameCodec.split(probeScheduler.next()!, maximumWrite: 185)!)
assert(probeReceiver.accept(probeWrites.begin()!) == nil)
probeScheduler.enqueue([healthProbe])
assert(probeWrites.begin() == nil)
probeWrites.complete()
var probeReceived = [[UInt8]]()
while true {
  if probeWrites.idle {
    guard let record = probeScheduler.next() else { break }
    probeWrites.append(BleFrameCodec.split(record, maximumWrite: 185)!)
  }
  if let record = probeReceiver.accept(probeWrites.begin()!) { probeReceived.append(record) }
  probeWrites.complete()
}
assert(probeReceived == [probeVoice[0], healthProbe, probeVoice[1], probeVoice[2]])
print("PASS: BLE health probes preserve the in-flight frame and overtake queued voice records")

// Exercise the actual Swift FFI copy boundary, not merely fragmentation.
// Before the fix Rust committed these notes, then Swift threw them away at
// the 4163-byte link limit and the durable store would not emit them again.
for size in [0, 4163, 4164, 19 * 1024, 48 * 1024 + 1102, NativeOutput.durableDeliveryLimit] {
  var payload = (0..<size).map { UInt8(truncatingIfNeeded: $0) }
  try payload.withUnsafeMutableBufferPointer { pointer in
    let buffer = MeshBuffer(ptr: pointer.baseAddress, len: size)
    let result = try NativeOutput.copy(buffer, status: 0, maximumBytes: NativeOutput.durableDeliveryLimit)
    assert(result.count == size && result.enumerated().allSatisfy { $0.element == UInt8(truncatingIfNeeded: $0.offset) })
    if size > NativeOutput.linkRecordLimit {
      do {
        _ = try NativeOutput.copy(buffer, status: 0, maximumBytes: NativeOutput.linkRecordLimit)
        assertionFailure("Wire limits must remain bounded")
      } catch NativeFailure.status(let code) { assert(code == 1) }
    }
  }
}
for (size, status) in [(NativeOutput.durableDeliveryLimit + 1, Int32(0)), (1, 0), (0, 3)] {
  do {
    _ = try NativeOutput.copy(MeshBuffer(ptr: nil, len: size), status: status, maximumBytes: NativeOutput.durableDeliveryLimit)
    assertionFailure("Invalid native output accepted")
  } catch NativeFailure.status(let code) { assert(code != 0) }
}
let emptyNativeOutput = try NativeOutput.copy(MeshBuffer(ptr: nil, len: 0), status: 0, maximumBytes: NativeOutput.durableDeliveryLimit)
assert(emptyNativeOutput.isEmpty)
print("PASS: Swift full durable voice delivery boundary, strict link cap and invalid buffers")

// Exercise the same write queue used by CoreBluetooth with a delayed ATT
// completion and a new product action arriving while the radio is busy.
let attQueue = BleWriteQueue()
assert(attQueue.idle)
attQueue.complete() // A stale completion cannot consume a new record.
attQueue.append([[1], [2]])
assert(!attQueue.idle)
assert(attQueue.begin() == [1])
attQueue.append([[3]])
for _ in 0..<10 { assert(attQueue.begin() == nil) }
attQueue.complete()
assert(attQueue.begin() == [2])
attQueue.complete()
assert(attQueue.begin() == [3])
attQueue.complete()
assert(attQueue.begin() == nil)
assert(attQueue.idle)

let radioQueue = BleWriteQueue()
let voiceFrames = (0..<48).map { index in (0..<1024).map { UInt8(truncatingIfNeeded: index + $0) } } + [[9, 8, 7]]
for frame in voiceFrames { radioQueue.append(BleFrameCodec.split(frame, maximumWrite: 185)!) }
let radioReceiver = BleFrameCodec.Assembler()
var receivedFrames = [[UInt8]]()
var fragmentCount = 0
while let fragment = radioQueue.begin() {
  if fragmentCount % 7 == 0 {
    radioQueue.refused()
    assert(radioQueue.begin() == fragment)
  }
  assert(radioQueue.begin() == nil)
  if let frame = radioReceiver.accept(fragment) { receivedFrames.append(frame) }
  radioQueue.complete()
  fragmentCount += 1
}
assert(receivedFrames == voiceFrames)
let history = DurableRecordHistory()
let routedRecords = (0..<520).map { index -> [UInt8] in
  var bytes = [UInt8](repeating: 0, count: 100)
  bytes[0] = 0x72; bytes[1] = 1
  bytes[98] = UInt8(index >> 8); bytes[99] = UInt8(truncatingIfNeeded: index)
  return bytes
}
assert(routedRecords.filter { history.admit($0) }.count == 520)
for _ in 0..<20 { assert(routedRecords.filter { history.admit($0) }.isEmpty) }
assert(DurableRecordHistory().admit(routedRecords[0]))
let scheduler = DurableRecordHistory()
scheduler.enqueue(Array(routedRecords.prefix(100)))
assert(scheduler.next() == routedRecords[0])
scheduler.enqueue([routedRecords[101]] + Array(routedRecords.prefix(100)))
assert(scheduler.next() == routedRecords[101])
for item in routedRecords[1..<100] { assert(scheduler.next() == item) }
assert(scheduler.next() == nil)
let boundedHistory = DurableRecordHistory()
for index in 0...8192 {
  var record = routedRecords[0]
  record[98] = UInt8(index >> 8); record[99] = UInt8(truncatingIfNeeded: index)
  assert(boundedHistory.admit(record))
}
assert(boundedHistory.admit(routedRecords[0]))
for _ in 0..<2 { assert(history.admit([0x7d])) }
print("PASS: Swift ATT serialization, voice fragmentation, backpressure and outbox replay suppression")

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
