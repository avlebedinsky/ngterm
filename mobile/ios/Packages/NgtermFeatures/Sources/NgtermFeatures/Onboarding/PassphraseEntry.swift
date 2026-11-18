import SwiftUI

/// The traits of a field that takes a key's passphrase: Import a key's and Export private key's.
///
/// A key's passphrase is not the password of an account or a website. A login content type (`.password`,
/// `.newPassword`) tells Password AutoFill that the field holds one, so the field has no content type. Apple's
/// documentation gives no content type for a secret that is not a login, and no content type alone stops the offer
/// to keep what was typed: iOS makes that offer when a screen goes while a secure field holds text (`.password`, no
/// content type and `.oneTimeCode` all got it on the iOS 27 simulator). Each screen therefore empties its passphrase
/// fields before it goes.
internal enum PassphraseEntry {
    /// The content type of a passphrase's field: none, as a passphrase is not a login.
    internal static let contentType: UITextContentType? = nil
}

extension View {
    /// Gives this field a key passphrase's traits: no capitals, no autocorrection, and no login content type.
    internal func passphraseEntry() -> some View {
        textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .textContentType(PassphraseEntry.contentType)
    }
}
