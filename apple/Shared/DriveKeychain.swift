import Foundation
import Security

enum DriveKeychain {
    /// A key rotation keeps the new secret until the database moves to it,
    /// then the old one until it is retired.
    enum Role: String {
        case current = ""
        case next = ".next"
        case previous = ".previous"
    }

    private static let service = "ovh.uid.lupine.drive"

    static func store(secret: String, drive: String, role: Role = .current) throws {
        let query = baseQuery(drive: drive, role: role)
        SecItemDelete(query as CFDictionary)
        var item = query
        item[kSecValueData as String] = Data(secret.utf8)
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        try check(SecItemAdd(item as CFDictionary, nil))
    }

    static func delete(drive: String, role: Role = .current) throws {
        let status = SecItemDelete(baseQuery(drive: drive, role: role) as CFDictionary)
        if status != errSecItemNotFound {
            try check(status)
        }
    }

    static func secret(drive: String, role: Role = .current) throws -> String? {
        var query = baseQuery(drive: drive, role: role)
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

    private static func baseQuery(drive: String, role: Role) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: drive + role.rawValue,
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
