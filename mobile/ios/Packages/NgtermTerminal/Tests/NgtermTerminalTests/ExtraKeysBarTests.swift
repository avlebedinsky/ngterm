import NgtermCoreFakes
import NgtermTerminal
import SnapshotTesting
import Testing
import UIKit

/// The extra keys' lines by the width they get, how they look, when a touch on them acts, and a composition at the
/// cursor.
///
/// The keys at rest, with Ctrl held for the next key and Alt locked, in light and dark. Recorded at a display scale of
/// 2, one set per simulator. The touches go through each key's control events, as UIKit sends them
/// (mobile/android's `TerminalKeyboardTest`).
@MainActor
@Suite("the extra keys")
internal struct ExtraKeysBarTests {
    private static let phone: CGFloat = 390
    private static let wide: CGFloat = 820
    private static let scale: CGFloat = 2

    // the bar of a keyboard with Ctrl held for the next key and Alt locked, `width` points wide in `style`
    private static func bar(width: CGFloat, style: UIUserInterfaceStyle) -> UIView {
        let rig = TerminalRig(clipboard: nil)
        let keyboard = rig.view.keyboard
        keyboard.target = rig.view
        keyboard.toggle(.control)
        keyboard.toggle(.alt)
        keyboard.toggle(.alt)
        let bar = keyboard.bar
        let lines = CGFloat(ExtraKeysBar.rows(forWidth: width).count)
        // the ground the keyboard's backdrop gives it on a device, which a render outside a window leaves out
        let host = UIView(frame: CGRect(x: 0, y: 0, width: width, height: ExtraKeysBar.keyHeight * lines))
        host.backgroundColor = .secondarySystemBackground
        host.traitOverrides.userInterfaceStyle = style
        host.traitOverrides.displayScale = scale
        // the default text size, whatever another test left the process at
        host.traitOverrides.preferredContentSizeCategory = .large
        bar.frame = host.bounds
        host.addSubview(bar)
        host.updateTraitsIfNeeded()
        bar.updateTraitsIfNeeded()
        host.layoutIfNeeded()
        bar.layoutIfNeeded()
        return host
    }

    // a rig whose view is its keyboard's target, as the first responder would be, its bar laid out on a phone
    private static func rig() -> TerminalRig {
        let rig = TerminalRig(clipboard: nil)
        rig.view.keyboard.target = rig.view
        let bar = rig.view.keyboard.bar
        bar.frame = CGRect(x: 0, y: 0, width: phone, height: ExtraKeysBar.keyHeight * 2)
        bar.layoutIfNeeded()
        return rig
    }

    // the control of `key` on the bar of `rig`
    private static func control(_ key: ExtraKey, of rig: TerminalRig) throws -> UIControl {
        let found = views(in: rig.view.keyboard.bar).first { $0.accessibilityIdentifier == key.identifier }
        return try #require(found as? UIControl)
    }

    // sends `event` from `control` to each of its targets' actions, as UIKit does for a touch, with no UIEvent: a test
    // process has no application, through which `sendActions(for:)` would go and deliver nothing
    private static func send(_ event: UIControl.Event, from control: UIControl) {
        for target in control.allTargets {
            let object = target.base as AnyObject
            for action in control.actions(forTarget: target.base, forControlEvent: event) ?? [] {
                _ = object.perform(Selector(action), with: control, with: nil)
            }
        }
    }

    // `view` and every view under it
    private static func views(in view: UIView) -> [UIView] {
        [view] + view.subviews.flatMap(views)
    }

    // what the pane took: each key's name (presses and repeats, not releases) and each typed text
    private static func sent(_ rig: TerminalRig) -> [String] {
        rig.inputs.compactMap { input in
            switch input {
            case .key(let key):
                key.action == .release ? nil : key.key

            case .typed(let text):
                text

            default:
                nil
            }
        }
    }

    private static func image(of view: UIView) -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        return UIGraphicsImageRenderer(bounds: view.bounds, format: format).image { context in
            view.layer.render(in: context.cgContext)
        }
    }

    private func snapshot(_ view: UIView, named name: String, testName: String = #function, line: UInt = #line) {
        assertSnapshot(
            of: Self.image(of: view),
            as: .image(precision: 1, perceptualPrecision: 0.98),
            named: "\(name)-on-\(SnapshotDevice.current)",
            testName: testName,
            line: line
        )
    }

    @Test("two lines of eight on a phone held upright, one line of sixteen once each key gets 44 points")
    internal func rows() {
        #expect(ExtraKeysBar.rows(forWidth: Self.phone) == ExtraKey.twoRows)
        #expect(ExtraKeysBar.rows(forWidth: 703) == ExtraKey.twoRows)
        #expect(ExtraKeysBar.rows(forWidth: 704) == [ExtraKey.oneRow])
        #expect(Set(ExtraKey.twoRows.joined()) == Set(ExtraKey.allCases))
        #expect(Set(ExtraKey.oneRow) == Set(ExtraKey.allCases))
        #expect(ExtraKey.twoRows.allSatisfy { $0.count == 8 })
    }

    @Test("Settings' layouts: two lines at every width, or no bar, with the focus staying")
    internal func layouts() {
        #expect(ExtraKeysBar.rows(forWidth: Self.wide, layout: .twoLines) == ExtraKey.twoRows)
        #expect(ExtraKeysBar.rows(forWidth: Self.wide, layout: .fit) == [ExtraKey.oneRow])
        let rig = TerminalRig(clipboard: nil)
        let keyboard = rig.view.keyboard
        keyboard.target = rig.view
        #expect(keyboard.showsExtraKeys)
        #expect(rig.view.inputAccessoryView === keyboard.bar)
        let bar = keyboard.bar
        bar.frame = CGRect(x: 0, y: 0, width: Self.wide, height: ExtraKeysBar.keyHeight * 2)
        bar.layoutIfNeeded()
        #expect(bar.rows == [ExtraKey.oneRow])
        keyboard.layout = .twoLines
        bar.layoutIfNeeded()
        #expect(bar.rows == ExtraKey.twoRows)
        keyboard.layout = .hidden
        #expect(!keyboard.showsExtraKeys)
        #expect(rig.view.inputAccessoryView == nil)
        keyboard.layout = .fit
        #expect(rig.view.inputAccessoryView === keyboard.bar)
    }

    @Test("a key acts as the finger lifts on it, not as it goes down")
    internal func actsOnLift() throws {
        let rig = Self.rig()
        let keys: [(ExtraKey, String)] = [
            (.escape, "escape"), (.pipe, "|"), (.pageUp, "page_up"), (.downArrow, "arrow_down"),
        ]
        for (key, name) in keys {
            let control = try Self.control(key, of: rig)
            let before = Self.sent(rig).count
            Self.send(.touchDown, from: control)
            #expect(Self.sent(rig).count == before, "\(key) acted as the finger went down")
            Self.send(.touchUpInside, from: control)
            #expect(Array(Self.sent(rig).dropFirst(before)) == [name], "\(key) acted once as the finger lifted")
        }
        let keyboard = rig.view.keyboard
        let control = try Self.control(.control, of: rig)
        Self.send(.touchDown, from: control)
        #expect(keyboard.control == .off, "Ctrl held as the finger went down")
        Self.send(.touchUpInside, from: control)
        #expect(keyboard.control == .once)
        let softKeyboard = try Self.control(.keyboard, of: rig)
        Self.send(.touchDown, from: softKeyboard)
        #expect(!rig.view.softKeyboardWanted, "the keyboard's key opened it as the finger went down")
        Self.send(.touchUpInside, from: softKeyboard)
        #expect(rig.view.softKeyboardWanted)
    }

    @Test("a touch that slides off its key, or that the system takes for a gesture, sends nothing")
    internal func slidOffOrTaken() async throws {
        let rig = Self.rig()
        for key in [ExtraKey.escape, .tilde, .upArrow, .pageDown, .control, .keyboard] {
            let control = try Self.control(key, of: rig)
            Self.send(.touchDown, from: control)
            Self.send(.touchDragExit, from: control)
            Self.send(.touchUpOutside, from: control)
            Self.send(.touchDown, from: control)
            Self.send(.touchCancel, from: control)
        }
        // past the arrows' and the page keys' hold
        try await Task.sleep(for: .milliseconds(300))
        #expect(Self.sent(rig).isEmpty)
        #expect(rig.view.keyboard.control == .off)
        #expect(!rig.view.softKeyboardWanted)
    }

    @Test("an arrow acts on a quick tap's lift, or once held 100 ms, and then repeats")
    internal func arrowTapAndHold() async throws {
        let rig = Self.rig()
        let left = try Self.control(.leftArrow, of: rig)
        Self.send(.touchDown, from: left)
        #expect(Self.sent(rig).isEmpty, "the arrow acted as the finger went down")
        Self.send(.touchUpInside, from: left)
        #expect(Self.sent(rig) == ["arrow_left"])
        try await Task.sleep(for: .milliseconds(500))
        #expect(Self.sent(rig) == ["arrow_left"], "a tap's hold and repeat stop as it lifts")

        Self.send(.touchDown, from: left)
        try await Task.sleep(for: .milliseconds(250))
        let held = Self.sent(rig).count
        #expect(held >= 2, "the hold acted before the lift: \(Self.sent(rig))")
        try await Task.sleep(for: .milliseconds(450))
        #expect(Self.sent(rig).count >= held + 2, "it repeats while held: \(Self.sent(rig))")
        Self.send(.touchUpInside, from: left)
        let lifted = Self.sent(rig).count
        try await Task.sleep(for: .milliseconds(200))
        #expect(Self.sent(rig).count == lifted, "the lift stops the repeat and sends no more")
        #expect(Set(Self.sent(rig)) == ["arrow_left"])
    }

    @Test("a touch moved past the slop sends nothing; a held arrow repeats on until the touch leaves its key")
    internal func arrowSwipe() async throws {
        let rig = Self.rig()
        let bar = rig.view.keyboard.bar
        bar.touch(.upArrow)
        bar.drag(.upArrow, travel: ExtraKeysBar.slop / 2, onKey: true)
        bar.drag(.upArrow, travel: ExtraKeysBar.slop + 1, onKey: true)
        try await Task.sleep(for: .milliseconds(300))
        bar.lift(.upArrow)
        #expect(Self.sent(rig).isEmpty, "a swipe that started on the arrow sent it")

        // a key that doesn't repeat: a short swipe inside it sends nothing, as does a touch that goes off it; a touch
        // that stays within the slop is a tap
        bar.touch(.escape)
        bar.drag(.escape, travel: ExtraKeysBar.slop + 1, onKey: true)
        bar.lift(.escape)
        bar.touch(.tab)
        bar.drag(.tab, travel: 1, onKey: false)
        bar.lift(.tab)
        #expect(Self.sent(rig).isEmpty, "a swipe on Esc or Tab sent it")
        bar.touch(.escape)
        bar.drag(.escape, travel: ExtraKeysBar.slop, onKey: true)
        bar.lift(.escape)
        #expect(Self.sent(rig) == ["escape"])

        // once its hold acted, the arrow repeats as the finger drifts on it, and stops as it leaves the key
        bar.touch(.downArrow)
        try await Task.sleep(for: .milliseconds(250))
        bar.drag(.downArrow, travel: ExtraKeysBar.slop * 3, onKey: true)
        try await Task.sleep(for: .milliseconds(400))
        let downs = Self.sent(rig).count
        #expect(downs >= 4, "the held arrow acted and repeats: \(Self.sent(rig))")
        bar.drag(.downArrow, travel: ExtraKeysBar.slop * 6, onKey: false)
        try await Task.sleep(for: .milliseconds(200))
        bar.lift(.downArrow)
        #expect(Self.sent(rig).count == downs, "the repeat stopped as the touch left the key")
    }

    @Test("a lift past the slop or off the key sends nothing, though the last move was within the slop")
    internal func liftPastSlop() {
        let rig = Self.rig()
        let bar = rig.view.keyboard.bar
        for key in [ExtraKey.escape, .upArrow, .control, .keyboard] {
            bar.touch(key)
            bar.drag(key, travel: ExtraKeysBar.slop / 2, onKey: true)
            bar.lift(key, travel: ExtraKeysBar.slop + 1, onKey: true)
            bar.touch(key)
            bar.lift(key, travel: 1, onKey: false)
        }
        #expect(Self.sent(rig).isEmpty, "a short swipe that ended past the slop sent its key: \(Self.sent(rig))")
        #expect(rig.view.keyboard.control == .off)
        #expect(!rig.view.softKeyboardWanted)
        bar.touch(.escape)
        bar.lift(.escape, travel: ExtraKeysBar.slop, onKey: true)
        #expect(Self.sent(rig) == ["escape"], "a lift within the slop is a tap")
    }

    @Test("two lines on a phone, light")
    internal func phoneLight() {
        snapshot(Self.bar(width: Self.phone, style: .light), named: "phone-light")
    }

    @Test("two lines on a phone, dark")
    internal func phoneDark() {
        snapshot(Self.bar(width: Self.phone, style: .dark), named: "phone-dark")
    }

    @Test("one line on a wide screen")
    internal func wide() {
        snapshot(Self.bar(width: Self.wide, style: .light), named: "wide")
    }

    @Test("a composition waits at the cursor, in the terminal's face on its ground, underlined in the cursor's color")
    internal func composition() {
        let rig = TerminalRig(clipboard: nil)
        rig.view.setMarkedText("にほん", selectedRange: NSRange(location: 3, length: 0))
        rig.view.displayNow()
        snapshot(rig.view, named: "composition")
    }
}
