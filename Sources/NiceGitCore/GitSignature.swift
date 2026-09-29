import Foundation

/// A commit's GPG, SSH, or X.509 signature as verified by the local Git configuration.
public struct GitSignature: Equatable, Sendable {
    public enum Status: Equatable, Sendable {
        /// Valid and from a trusted key.
        case verified
        /// Valid, but the key's trust or validity is unknown, expired, or revoked.
        case untrusted
        /// The signature does not match the commit.
        case bad
        /// Signed, but this computer cannot check it (for example, a missing key or tool).
        case unverifiable
    }
    public let status: Status
    public let signer: String
    public let key: String
    /// Why the signature could not be checked, when Git reported a problem.
    public var problem: String?
}

extension GitClient {
    /// The signature on `commit`, or nil when it is unsigned. Verification is local; no key
    /// servers are contacted.
    public func signature(of commit: String, in url: URL) throws -> GitSignature? {
        let id = try run(["rev-parse", "--verify", "--end-of-options", commit + "^{commit}"], in: url).trimmingCharacters(in: .whitespacesAndNewlines)
        // Read the commit object's headers first so unsigned commits never start a verifier.
        let object = try run(["cat-file", "commit", id], in: url)
        let headers = object.components(separatedBy: "\n\n").first ?? ""
        guard headers.split(separator: "\n").contains(where: { $0.hasPrefix("gpgsig ") || $0.hasPrefix("gpgsig-sha256 ") }) else { return nil }
        let fields: [String]
        do {
            fields = try run(["log", "-1", "--no-color", "--format=%G?%x1f%GS%x1f%GK", id, "--"], in: url)
                .trimmingCharacters(in: .newlines).components(separatedBy: "\u{1f}")
        } catch {
            // A broken signing setup (such as an invalid gpg.format) leaves the signature unchecked.
            return GitSignature(status: .unverifiable, signer: "", key: "", problem: error.localizedDescription)
        }
        guard let code = fields.first?.first else {
            return GitSignature(status: .unverifiable, signer: "", key: "", problem: nil)
        }
        let status: GitSignature.Status
        switch code {
        case "N": status = .unverifiable
        case "G": status = .verified
        case "U", "X", "Y", "R": status = .untrusted
        case "B": status = .bad
        default: status = .unverifiable
        }
        return GitSignature(status: status, signer: fields.count > 1 ? fields[1] : "", key: fields.count > 2 ? fields[2] : "")
    }
}
