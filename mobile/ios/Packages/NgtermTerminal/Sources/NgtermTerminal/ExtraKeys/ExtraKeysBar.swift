import NgtermDesign
import UIKit

/// The keys a soft keyboard lacks, over it: the terminal's `inputAccessoryView` (mobile/android's `ExtraKeysRow`).
///
/// Esc, Tab, a sticky Ctrl and Alt, `/`, `|` and `~`, Home and End, Page Up and Down, the arrows, and the soft
/// keyboard's own key, going to the ``TerminalKeyboard``'s target. Two lines of eight on a phone held upright, one
/// line of sixteen once the width gives each key 44 points, or two lines at every width when Settings asks for them
/// (``TerminalKeyboard/layout``). A key acts on a tap, not on a touch: the keyboard's click, a light tap and the key's
/// highlight come as the finger goes down, the key as it lifts on the key. A touch that moves past ``slop`` (a swipe),
/// that slides off the key, or that the system takes for a gesture sends nothing. The arrows and the page keys also act
/// once held 100 ms, and repeat while held. Ctrl and Alt hold for the next key (outlined in the tint), lock on a second
/// tap (filled), and let go on a third, with a selection tick. The keyboard's key closes the soft keyboard and opens it
/// again, the focus and the bar staying. It isn't shown while a hardware keyboard is attached, nor when Settings hides
/// it.
@preconcurrency
@MainActor
public final class ExtraKeysBar: UIInputView, UIInputViewAudioFeedback {
    /// A key's height, and the least width a key gets before the bar takes one line.
    public static let keyHeight = CGFloat(Spacing.hitTarget)
    /// How far a touch on a key may move before the key acts: past it, the touch is a swipe, however short, and sends
    /// nothing (Android's touch slop; a scroll view's pan starts at about as far).
    public static let slop: CGFloat = 10

    // the share of a key's room the widest word takes, leaving its glyphs' overhang room
    private static let fit: CGFloat = 0.9
    // the room a word keeps from its key's edges, besides the face's inset
    private static let legendInset: CGFloat = 6
    // a symbol is one character: a size up from the words, as the terminal draws it
    private static let symbolSize: CGFloat = 18
    private static let largestSymbol: CGFloat = 26
    // a key's two sides
    private static let sides: CGFloat = 2

    // the keyboard owns the bar, but UIKit may keep it a moment longer, while it goes off screen
    private weak var keyboard: TerminalKeyboard?
    private let content = UIView()
    private let repeats = KeyRepeat()
    // the key held whose repeat runs, or whose hold waits to act
    private var repeatingKey: ExtraKey?
    // the keys touched whose touch is still a tap: not slid off, not taken by the system, and, for a repeating key,
    // not acted on by its hold yet
    private var armed: Set<ExtraKey> = []
    // where each key's touch went down, in the key's points
    private var starts: [ExtraKey: CGPoint] = [:]
    private var buttons: [ExtraKey: ExtraKeyButton] = [:]
    private var height: NSLayoutConstraint?

    /// The lines of keys shown now.
    public private(set) var rows = ExtraKey.twoRows

    /// The keyboard's click sounds as the user set them.
    public var enableInputClicksWhenVisible: Bool {
        true
    }

    /// The bar of `keyboard`.
    internal init(keyboard: TerminalKeyboard) {
        self.keyboard = keyboard
        let size = CGSize(width: 0, height: Self.keyHeight * CGFloat(ExtraKey.twoRows.count))
        super.init(frame: CGRect(origin: .zero, size: size), inputViewStyle: .keyboard)
        allowsSelfSizing = true
        accessibilityLabel = String(localized: "Extra keys")
        accessibilityIdentifier = "extra-keys"
        content.translatesAutoresizingMaskIntoConstraints = false
        addSubview(content)
        let keysHeight = content.heightAnchor.constraint(equalToConstant: size.height)
        // the keys' height decides the bar's: its foot gives way while the bar takes a new height
        let foot = content.bottomAnchor.constraint(equalTo: safeAreaLayoutGuide.bottomAnchor)
        foot.priority = .required - 1
        NSLayoutConstraint.activate([
            content.topAnchor.constraint(equalTo: topAnchor),
            content.leadingAnchor.constraint(equalTo: safeAreaLayoutGuide.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: safeAreaLayoutGuide.trailingAnchor),
            foot,
            keysHeight,
        ])
        height = keysHeight
        for key in ExtraKey.allCases {
            let button = ExtraKeyButton(key)
            button.addTarget(self, action: #selector(down(_:event:)), for: .touchDown)
            button.addTarget(self, action: #selector(moved(_:event:)), for: [.touchDragInside, .touchDragOutside])
            button.addTarget(self, action: #selector(up(_:event:)), for: .touchUpInside)
            button.addTarget(self, action: #selector(lost(_:)), for: [.touchDragExit, .touchUpOutside, .touchCancel])
            button.activate = { [weak self] in
                self?.press(key)
            }
            content.addSubview(button)
            buttons[key] = button
        }
        registerForTraitChanges([UITraitPreferredContentSizeCategory.self]) { (bar: Self, _: UITraitCollection) in
            bar.setNeedsLayout()
        }
        refresh()
    }

    @available(*, unavailable, message: "the extra keys are made in code")
    internal required init?(coder _: NSCoder) {
        nil
    }

    /// The lines of keys for `width` points in `layout`: two lines when it asks for them, else one line once every key
    /// gets ``keyHeight``, and two until then.
    public static func rows(forWidth width: CGFloat, layout: ExtraKeysLayout = .fit) -> [[ExtraKey]] {
        if layout == .twoLines {
            return ExtraKey.twoRows
        }
        return width >= keyHeight * CGFloat(ExtraKey.oneRow.count) ? [ExtraKey.oneRow] : ExtraKey.twoRows
    }

    // where `event`'s touch on `button` is, in its points: nil for an action sent with no touch
    private static func location(of event: UIEvent?, in button: UIView) -> CGPoint? {
        event?.touches(for: button)?.first?.location(in: button)
    }

    /// The finger goes down on `key`: its click and tap as it lights up, and no action yet.
    ///
    /// The tap is a selection tick for Ctrl and Alt. The key does not act yet, since a swipe or a system gesture can
    /// still take the touch. A repeating key acts once held 100 ms, then repeats while held.
    public func touch(_ key: ExtraKey) {
        guard keyboard != nil else {
            return
        }
        feedback(for: key)
        armed.insert(key)
        if case .press(let name, repeats: true) = key.action {
            repeatingKey = key
            repeats.startHeld { [weak self] in
                self?.armed.remove(key)
                self?.keyboard?.press(name)
            }
        }
    }

    /// The finger lifts on `key`, `travel` points from where it went down, on the key or off it (`onKey`): the key
    /// acts if its touch is still a tap, and its repeat stops.
    ///
    /// The lift's point counts as a last ``drag(_:travel:onKey:)``: a touch can end past ``slop`` or off the key with
    /// no move reported before it.
    public func lift(_ key: ExtraKey, travel: CGFloat = 0, onKey: Bool = true) {
        drag(key, travel: travel, onKey: onKey)
        stopRepeat(of: key)
        if armed.remove(key) != nil {
            act(key)
        }
    }

    /// The touch on `key` moved `travel` points from where it went down, on the key or off it (`onKey`).
    ///
    /// Off the key, the touch sends nothing more. Before the key acts, a move past ``slop`` makes the touch a swipe,
    /// which sends nothing; a repeating key whose hold acted may drift on the key.
    public func drag(_ key: ExtraKey, travel: CGFloat, onKey: Bool) {
        let swipe = travel > Self.slop && armed.contains(key)
        if !onKey || swipe {
            cancel(key)
        }
    }

    /// The touch on `key` is not a tap: it slid off the key, or the system took it for a gesture.
    ///
    /// The key sends nothing, and its repeat stops.
    public func cancel(_ key: ExtraKey) {
        stopRepeat(of: key)
        armed.remove(key)
    }

    /// `key` acts at once, with its click and tap and no repeat: VoiceOver's activation, which has no touch.
    public func press(_ key: ExtraKey) {
        guard keyboard != nil else {
            return
        }
        feedback(for: key)
        act(key)
    }

    /// Shows each key as things stand: the sticky modifiers, and whether the soft keyboard is open.
    public func refresh() {
        guard let keyboard else {
            return
        }
        let open = keyboard.target?.softKeyboardWanted ?? false
        let fonts = legendFonts()
        for (key, button) in buttons {
            let sticky = if case .hold(let modifier) = key.action { keyboard.sticky(modifier) } else { Sticky.off }
            button.show(sticky: sticky, keyboardOpen: open, wordFont: fonts.word, symbolFont: fonts.symbol)
        }
    }

    /// Lays the keys out in their lines, each its whole cell with no gaps, taking one line once the width allows
    /// unless the keyboard's layout keeps two.
    override public func layoutSubviews() {
        super.layoutSubviews()
        let fitting = Self.rows(forWidth: content.bounds.width, layout: keyboard?.layout ?? .fit)
        if fitting != rows {
            rows = fitting
            height?.constant = Self.keyHeight * CGFloat(rows.count)
        }
        refresh()
        let lineHeight = content.bounds.height / CGFloat(max(rows.count, 1))
        for (line, keys) in rows.enumerated() {
            let width = content.bounds.width / CGFloat(max(keys.count, 1))
            for (column, key) in keys.enumerated() {
                buttons[key]?.frame = CGRect(
                    x: CGFloat(column) * width,
                    y: CGFloat(line) * lineHeight,
                    width: width,
                    height: lineHeight
                )
            }
        }
        let shown = Set(rows.joined())
        for (key, button) in buttons {
            button.isHidden = !shown.contains(key)
        }
    }

    // the words at one size, the widest's ("PgUp") shrunk only as far as its key needs; the symbols in the
    // terminal's face, a size up
    private func legendFonts() -> (word: UIFont, symbol: UIFont) {
        let base = UIFont.preferredFont(forTextStyle: .subheadline, compatibleWith: traitCollection)
        let word = UIFont.systemFont(ofSize: base.pointSize, weight: .semibold)
        let keys = rows.first?.count ?? 1
        let room = content.bounds.width / CGFloat(keys) - Self.sides * Self.legendInset
        let widths = ExtraKey.allCases.compactMap { key -> CGFloat? in
            guard case .word(let text) = key.legend(keyboardOpen: false) else {
                return nil
            }
            return NSAttributedString(string: text, attributes: [.font: word]).size().width
        }
        let widest = widths.max() ?? 1
        let scale = room > 0 ? min(1, room * Self.fit / widest) : 1
        let metrics = UIFontMetrics(forTextStyle: .body)
        let scaled = metrics.scaledValue(for: Self.symbolSize, compatibleWith: traitCollection)
        let size = min(scaled, Self.largestSymbol)
        let symbol =
            UIFont(name: Fonts.terminalFaces.regular, size: size)
            ?? .monospacedSystemFont(ofSize: size, weight: .regular)
        return (word.withSize(word.pointSize * scale), symbol)
    }

    // what a key does, once it is a tap
    private func act(_ key: ExtraKey) {
        guard let keyboard else {
            return
        }
        switch key.action {
        case .hold(let modifier):
            keyboard.toggle(modifier)

        case .press(let name, _):
            keyboard.press(name)

        case .type(let text):
            keyboard.type(text)

        case .softKeyboard:
            if let target = keyboard.target {
                if target.softKeyboardWanted {
                    target.hideSoftKeyboard()
                } else {
                    target.focus(showingKeyboard: true)
                }
            }
        }
    }

    // a key's click and tap as the finger goes down, as a keyboard's; a selection tick for a modifier
    private func feedback(for key: ExtraKey) {
        UIDevice.current.playInputClick()
        if case .hold = key.action {
            UISelectionFeedbackGenerator(view: self).selectionChanged()
        } else {
            UIImpactFeedbackGenerator(style: .light, view: self).impactOccurred()
        }
    }

    // `key`'s repeat or hold stops, if it is the key repeating; a key touched since repeats on
    private func stopRepeat(of key: ExtraKey) {
        if repeatingKey == key {
            repeats.stop()
            repeatingKey = nil
        }
    }

    @objc
    private func down(_ button: ExtraKeyButton, event: UIEvent?) {
        starts[button.key] = Self.location(of: event, in: button)
        touch(button.key)
    }

    // how far `event`'s touch on `button` is from where it went down, and whether it is on the key: UIKit counts a
    // touch as inside a control up to some way off it, so the key counts its own bounds. Nil for an action sent with
    // no touch.
    private func track(of event: UIEvent?, on button: ExtraKeyButton) -> (travel: CGFloat, onKey: Bool)? {
        guard let point = Self.location(of: event, in: button) else {
            return nil
        }
        let start = starts[button.key] ?? point
        return (hypot(point.x - start.x, point.y - start.y), button.bounds.contains(point))
    }

    @objc
    private func moved(_ button: ExtraKeyButton, event: UIEvent?) {
        if let motion = track(of: event, on: button) {
            drag(button.key, travel: motion.travel, onKey: motion.onKey)
        }
    }

    @objc
    private func up(_ button: ExtraKeyButton, event: UIEvent?) {
        let motion = track(of: event, on: button) ?? (travel: 0, onKey: true)
        starts[button.key] = nil
        lift(button.key, travel: motion.travel, onKey: motion.onKey)
    }

    @objc
    private func lost(_ button: ExtraKeyButton) {
        starts[button.key] = nil
        cancel(button.key)
    }

    deinit {
        // the repeat stops as it goes
    }
}
