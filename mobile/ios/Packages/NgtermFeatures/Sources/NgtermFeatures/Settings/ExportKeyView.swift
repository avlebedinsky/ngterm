import NgtermDesign
import NgtermPlatform
import SwiftUI

/// Exporting a private key for another install, which Import a key reads back (mobile/android's `ExportKeyScreen`).
///
/// It shows only once the device's owner has unlocked (``ExportKeyModel/start(keyId:)``).
/// What the key is and who can use it, the key's fingerprint, the passphrase typed twice (what is wrong with it under
/// the field it is about, once an export was tried), the choice to export without one (confirmed again), Save to a
/// file (the system's screen places the file) and Copy to the clipboard, then how it went.
///
/// A passphrase is typed only into a secure field, which iOS keeps from other keyboards; Show writes it out under the
/// field. While the app isn't in front (the app switcher's snapshot) or the screen is recorded or shared, a plain
/// cover hides the page and Show goes off.
internal struct ExportKeyView: View {
    @Environment(\.scenePhase)
    private var scenePhase
    @Environment(\.isSceneCaptured)
    private var isCaptured

    @Bindable internal var model: ExportKeyModel
    internal let key: AppKey
    @State private var isPassphraseShown: Bool
    @State private var isPlacing = false

    // whether the page is hidden: what it shows could be seen where the user doesn't look
    private var isCovered: Bool {
        Self.isCovered(phase: scenePhase, captured: isCaptured)
    }

    internal var body: some View {
        let work = model.work
        // a scroll view, not a Form: every part stays laid out, so each grows with the text size (as the import's)
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.xLarge) {
                Banner(
                    .attention,
                    message: String(localized: "This is the key your machines trust"),
                    note: String(
                        localized: """
                            Whoever has the exported key and its passphrase can sign in to them as you. Keep it where \
                            only you can reach it, and delete it once it's imported.
                            """
                    )
                )
                ValuePlate(String(localized: "Key to export"), value: key.fingerprint)
                protection
                actions(work)
                outcome(work)
            }
            .frame(maxWidth: OnboardingLayout.width, alignment: .leading)
            .padding(.horizontal, Spacing.xLarge)
            .padding(.vertical, Spacing.large)
            .frame(maxWidth: .infinity)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(Color(uiColor: .systemGroupedBackground))
        .disabled(work.isBusy)
        .accessibilityHidden(isCovered)
        .overlay {
            if isCovered {
                cover
            }
        }
        .onChange(of: isCovered, initial: true) { _, covered in
            if covered {
                isPassphraseShown = false
            }
        }
        .accessibilityIdentifier("export-key-page")
        .navigationTitle(String(localized: "Export private key"))
        .navigationBarTitleDisplayMode(.inline)
        .confirmingDestructive(
            String(localized: "Export without a passphrase?"),
            isPresented: isConfirming,
            message: String(
                localized: """
                    The exported key won't be encrypted. Anyone who gets the file or reads the clipboard can sign in \
                    to your machines as you.
                    """
            ),
            action: unprotectedAction
        )
        .onChange(of: work.staged) { _, staged in
            isPlacing = staged != nil
        }
        .fileMover(isPresented: $isPlacing, file: work.staged) { result in
            model.placed(result)
        } onCancellation: {
            model.placingCancelled()
        }
    }

    // a plain ground over the page; a recording or a shared screen is told why it is blank
    private var cover: some View {
        ZStack {
            Color(uiColor: .systemGroupedBackground)
                .ignoresSafeArea()
            if isCaptured {
                VStack(spacing: Spacing.medium) {
                    Image(systemName: "eye.slash")
                        .font(.largeTitle)
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                    Text("Hidden while the screen is recorded or shared")
                        .font(Typography.note)
                        .foregroundStyle(Palette.secondaryText.color)
                        .multilineTextAlignment(.center)
                }
                .frame(maxWidth: Spacing.textWidth)
                .padding(Spacing.xLarge)
            }
        }
        .accessibilityIdentifier("export-key-cover")
    }

    private var isConfirming: Binding<Bool> {
        Binding {
            model.confirming != nil
        } set: { isShown in
            if !isShown {
                model.declineUnprotected()
            }
        }
    }

    // the dialog's own button repeats the switch's words; it keeps the action it was shown for
    private var unprotectedAction: RowAction {
        let action = model.confirming
        return RowAction(String(localized: "Export without a passphrase")) {
            if let action {
                Task {
                    await model.confirmUnprotected(action)
                }
            }
        }
    }

    @ViewBuilder private var protection: some View {
        let problem = model.passphraseProblem
        if !model.draft.isUnprotected {
            VStack(alignment: .leading, spacing: Spacing.small) {
                FormHint(String(localized: "Passphrase"), .header)
                field(String(localized: "Passphrase"), text: $model.draft.passphrase, identifier: "export-passphrase") {
                    Button(isPassphraseShown ? String(localized: "Hide") : String(localized: "Show")) {
                        isPassphraseShown.toggle()
                    }
                    .buttonStyle(.borderless)
                    .frame(minWidth: Spacing.hitTarget, minHeight: Spacing.hitTarget)
                    .accessibilityLabel(
                        isPassphraseShown
                            ? String(localized: "Hide passphrases") : String(localized: "Show passphrases")
                    )
                }
                if case .short = problem, let problem {
                    InlineProblem(problem.message)
                } else {
                    FormHint(
                        String(
                            localized: """
                                At least \(model.passphraseMinimum) characters. It encrypts the exported key, and you \
                                type it again when you import the key.
                                """
                        ),
                        .note
                    )
                }
            }
            VStack(alignment: .leading, spacing: Spacing.small) {
                FormHint(String(localized: "Passphrase again"), .header)
                field(
                    String(localized: "Passphrase again"),
                    text: $model.draft.again,
                    identifier: "export-passphrase-again"
                ) {
                    EmptyView()
                }
                if problem == .differs, let problem {
                    InlineProblem(problem.message)
                }
            }
        }
        VStack(alignment: .leading, spacing: Spacing.small) {
            Toggle(isOn: $model.draft.isUnprotected) {
                VStack(alignment: .leading, spacing: Spacing.xxSmall) {
                    Text("Export without a passphrase")
                    Text("Not recommended")
                        .font(Typography.note)
                        .foregroundStyle(Palette.secondaryText.color)
                }
            }
            .padding(.horizontal, Spacing.medium)
            .padding(.vertical, Spacing.small)
            .frame(minHeight: Spacing.hitTarget)
            .background(
                Color(uiColor: .secondarySystemGroupedBackground),
                in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
            )
            .accessibilityIdentifier("export-unprotected")
            if model.draft.isUnprotected {
                Banner(
                    .failed,
                    message: String(localized: "The exported key won't be encrypted"),
                    note: String(localized: "Whoever reads it signs in to your machines.")
                )
            }
        }
    }

    /// The export of `key` over `model`, its passphrases hidden.
    internal init(model: ExportKeyModel, key: AppKey) {
        self.init(model: model, key: key, isPassphraseShown: false)
    }

    /// The export of `key` over `model`, the passphrases written out under their fields when `isPassphraseShown`.
    internal init(model: ExportKeyModel, key: AppKey, isPassphraseShown: Bool) {
        self.model = model
        self.key = key
        _isPassphraseShown = State(initialValue: isPassphraseShown)
    }

    /// Whether the page is hidden in `phase`, `captured` or not: whenever the app isn't in front, and while the screen
    /// is recorded, mirrored or shared.
    internal static func isCovered(phase: ScenePhase, captured: Bool) -> Bool {
        phase != .active || captured
    }

    private func actions(_ work: ExportWork) -> some View {
        VStack(alignment: .leading, spacing: Spacing.small) {
            BusyButton(
                title: work.isBusy ? String(localized: "Exporting the key") : String(localized: "Save to a file"),
                isBusy: work.isBusy
            ) {
                tap(.save)
            }
            .accessibilityIdentifier("export-save")
            Button {
                tap(.copy)
            } label: {
                Label(String(localized: "Copy to the clipboard"), systemImage: "doc.on.doc")
                    .font(Typography.detail.weight(.semibold))
                    .frame(minHeight: Spacing.hitTarget)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.borderless)
            .accessibilityIdentifier("export-copy")
            FormHint(
                String(
                    localized: """
                        A file is safer: other apps can read the clipboard. ngterm clears the key from it when you \
                        close this screen, and it never leaves this device.
                        """
                ),
                .note
            )
        }
    }

    @ViewBuilder
    private func outcome(_ work: ExportWork) -> some View {
        if let done = work.done {
            Banner(.online, message: done.words.message, note: done.words.note)
                .accessibilityIdentifier("export-done")
        } else if let problem = work.problem {
            InlineProblem(problem.message)
                .accessibilityIdentifier("export-problem")
        }
    }

    // a passphrase's secure field on the raised ground, with `trailing` at its end and, once Show was tapped, what was
    // typed under it: a plain text field would hand each character to another keyboard
    private func field<Trailing: View>(
        _ title: String,
        text: Binding<String>,
        identifier: String,
        @ViewBuilder trailing: () -> Trailing
    ) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                SecureField(title, text: text)
                    .passphraseEntry()
                    .frame(minHeight: Spacing.hitTarget)
                    .accessibilityIdentifier(identifier)
                trailing()
            }
            if isPassphraseShown, !text.wrappedValue.isEmpty {
                Text(verbatim: text.wrappedValue)
                    .font(Typography.plateValue)
                    .textSelection(.disabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.bottom, Spacing.small)
                    .accessibilityLabel(String(localized: "\(title), as typed"))
                    .accessibilityValue(text.wrappedValue)
                    .accessibilityIdentifier("\(identifier)-shown")
            }
        }
        .padding(.horizontal, Spacing.medium)
        .background(
            Color(uiColor: .secondarySystemGroupedBackground),
            in: RoundedRectangle(cornerRadius: Radius.medium, style: .continuous)
        )
    }

    private func tap(_ action: ExportAction) {
        Task {
            await model.tap(action)
        }
    }
}
