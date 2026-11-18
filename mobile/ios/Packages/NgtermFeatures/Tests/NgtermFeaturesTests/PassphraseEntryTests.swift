import SwiftUI
import Testing
import UIKit

@testable import NgtermFeatures

/// The passphrase fields of Import a key and Export private key, as UIKit gets them: secure, no capitals, no
/// autocorrection, and no login content type (``PassphraseEntry``; mobile/android's `ImportKeyScreenTest` and
/// `ExportKeyScreenTest` check that its fields are not autofilled).
@MainActor
@Suite("the passphrase fields")
internal struct PassphraseEntryTests {
    // the content types that tell Password AutoFill a field is a login's
    private static let login: [UITextContentType] = [.username, .password, .newPassword]

    private let test = TestGraph()

    // the text fields under `view`, in the order of the view tree
    private static func textFields(under view: UIView) -> [UITextField] {
        let own = (view as? UITextField).map { [$0] } ?? []
        return own + view.subviews.flatMap { textFields(under: $0) }
    }

    // the secure text fields of `screen`, laid out in a window of the test's own
    private static func secureFields(of screen: some View) throws -> [UITextField] {
        let host = UIHostingController(rootView: NavigationStack { screen })
        // UIView's initializer runs UIWindow's, which has no scene here
        let made = (UIWindow.self as UIView.Type).init(frame: CGRect(origin: .zero, size: ScreenVariant.size))
        let window = try #require(made as? UIWindow)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        window.layoutIfNeeded()
        return textFields(under: window).filter(\.isSecureTextEntry)
    }

    private static func expectPassphraseTraits(_ fields: [UITextField], count: Int) {
        #expect(fields.count == count)
        for field in fields {
            #expect(field.textContentType == PassphraseEntry.contentType)
            #expect(!login.contains { $0 == field.textContentType }, "a login's content type: \(field.textContentType)")
            #expect(field.autocorrectionType == .no)
            #expect(field.autocapitalizationType == UITextAutocapitalizationType.none)
        }
    }

    @Test("Import a key's passphrase field has no login content type")
    internal func importField() throws {
        let work = ImportKeyWork(
            state: { (isBusy: false, problem: nil) },
            importKey: { _, _ in
                // nothing is imported here
            },
            unreadable: {
                // no file is chosen here
            }
        )
        Self.expectPassphraseTraits(try Self.secureFields(of: ImportKeyView(work: work)), count: 1)
    }

    @Test("Export private key's two passphrase fields have no login content type")
    internal func exportFields() async throws {
        defer { test.remove() }
        let key = try await test.seedKey()
        let model = ExportKeyModel(keys: test.graph.keys, services: test.graph.exports)
        await model.start(keyId: key.id)
        let screen = ExportKeyView(model: model, key: key, isPassphraseShown: false)
            .environment(\.scenePhase, .active)
        Self.expectPassphraseTraits(try Self.secureFields(of: screen), count: 2)
    }
}
