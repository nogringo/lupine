import Foundation
import Security

enum DriveKeychain {
    private static let service = "ovh.uid.lupine.drive"

    static func store(secret: String, drive: String) throws {
        let query = baseQuery(drive: drive)
        SecItemDelete(query as CFDictionary)
        var item = query
        item[kSecValueData as String] = Data(secret.utf8)
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        try check(SecItemAdd(item as CFDictionary, nil))
    }

    static func secret(drive: String) throws -> String? {
        var query = baseQuery(drive: drive)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        try check(status)
        return (result as? Data).map { String(decoding: $0, as: UTF8.self) }
    }

    private static func baseQuery(drive: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: drive,
            kSecAttrAccessGroup as String: SharedContainer.keychainGroup,
            kSecUseDataProtectionKeychain as String: true,
        ]
    }

    private static func check(_ status: OSStatus) throws {
        guard status == errSecSuccess else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
        }
    }
}
