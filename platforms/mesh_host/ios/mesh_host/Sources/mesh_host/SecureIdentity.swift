import Foundation
import Security
import CryptoKit
import MeshEngine

enum KeyStorageFailure: Error { case unavailable, corrupt }
struct SecureStoreMaterial {
  var databaseKey: Data
  var member: [UInt8]
  mutating func wipe() {
    databaseKey.resetBytes(in: 0..<databaseKey.count)
    member = []
  }
}
struct GroupMaterial {
  var identitySeed: Data
  var deliverySeed: Data
  var sessionSeed: Data
  var member: [UInt8]
  mutating func wipe() {
    identitySeed.resetBytes(in: 0..<identitySeed.count)
    deliverySeed.resetBytes(in: 0..<deliverySeed.count)
    sessionSeed.resetBytes(in: 0..<sessionSeed.count)
    member = []
  }
}
/// Called on the native serial executor. Seeds never cross a Pigeon channel.
final class SecureIdentity {
  private let service = "com.frazko.mesh-lab.keys"
  private let account = "installation-v1"
  private func query() -> [String: Any] {
    [kSecClass as String: kSecClassGenericPassword,
     kSecAttrService as String: service, kSecAttrAccount as String: account,
     kSecAttrSynchronizable as String: false]
  }
  private func marker() throws -> URL {
    let root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
    var directory = root.appendingPathComponent("mesh-keys", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
      attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
    var values = URLResourceValues(); values.isExcludedFromBackup = true
    try directory.setResourceValues(values)
    return directory.appendingPathComponent("initialized-v1")
  }
  private func loadMaterial() throws -> Data {
    let initialized = try marker()
    var q = query(); q[kSecReturnData as String] = true; q[kSecMatchLimit as String] = kSecMatchLimitOne
    var item: CFTypeRef?
    let status = SecItemCopyMatching(q as CFDictionary, &item)
    var material: Data
    if status == errSecSuccess {
      guard let bytes = item as? Data, bytes.count == 128 else { throw KeyStorageFailure.corrupt }
      material = bytes
      item = nil
    } else if status == errSecItemNotFound {
      guard !FileManager.default.fileExists(atPath: initialized.path) else { throw KeyStorageFailure.unavailable }
      material = Data(count: 128)
      let result = material.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 128, $0.baseAddress!) }
      guard result == errSecSuccess else { material.resetBytes(in: 0..<material.count); throw KeyStorageFailure.unavailable }
      var add = query(); add[kSecValueData as String] = material
      add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
      let added = SecItemAdd(add as CFDictionary, nil)
      add.removeValue(forKey: kSecValueData as String)
      guard added == errSecSuccess else { material.resetBytes(in: 0..<material.count); throw KeyStorageFailure.unavailable }
    } else { throw KeyStorageFailure.unavailable }
    // Identity, HPKE delivery, Noise static, SQLCipher: four independent 32-byte slots.
    if !FileManager.default.fileExists(atPath: initialized.path) {
      try Data([1]).write(to: initialized, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }
    return material
  }
  private func publicKey(_ material: Data) throws -> [UInt8] {
    var output = MeshBuffer(ptr: nil, len: 0)
    let code = material.withUnsafeBytes { mesh_identity_public($0.bindMemory(to: UInt8.self).baseAddress, 32, &output) }
    defer { mesh_buffer_release(output) }
    guard code == 0, output.len == 32, let ptr = output.ptr else { throw KeyStorageFailure.unavailable }
    return Array(UnsafeBufferPointer(start: ptr, count: 32))
  }
  func prepare() throws -> String {
    var material = try loadMaterial()
    defer { material.resetBytes(in: 0..<material.count) }
    // The binding must carry the same public Ed25519 member identity that a
    // Field policy certifies. A display hash cannot be verified by peers or a
    // cloud relay, and does not add secrecy because this value is public.
    return try publicKey(material).map { String(format: "%02x", $0) }.joined()
  }
  func storeMaterial() throws -> SecureStoreMaterial {
    var material = try loadMaterial()
    defer { material.resetBytes(in: 0..<material.count) }
    return SecureStoreMaterial(databaseKey: material.subdata(in: 96..<128), member: try publicKey(material))
  }
  func groupMaterial() throws -> GroupMaterial {
    var material = try loadMaterial()
    defer { material.resetBytes(in: 0..<material.count) }
    return GroupMaterial(identitySeed: material.subdata(in: 0..<32),
                         deliverySeed: material.subdata(in: 32..<64),
                         sessionSeed: material.subdata(in: 64..<96),
                         member: try publicKey(material))
  }
}
