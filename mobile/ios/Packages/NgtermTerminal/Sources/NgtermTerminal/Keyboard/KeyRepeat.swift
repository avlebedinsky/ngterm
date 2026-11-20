import Foundation

/// A held key's repeat: its action runs again after a moment, then steadily, until the key is let go (mobile/android's
/// extra keys: after 400 ms, every 50 ms).
///
/// An extra key, which acts only once it is held a moment, starts it with its first action still to come
/// (``startHeld(_:)``).
@preconcurrency
@MainActor
internal final class KeyRepeat {
    /// How long a key is held before it repeats.
    internal static let delay: TimeInterval = 0.4
    /// How often it repeats then.
    internal static let interval: TimeInterval = 0.05
    /// How long an extra key is held before it acts, if it is not let go first (Android's tap timeout).
    internal static let hold: TimeInterval = 0.1

    private var timer: Timer?
    private var action: (@MainActor () -> Void)?
    // the waits before the steady interval: those still to come
    private var waits: [TimeInterval] = []

    /// A repeat with no key held.
    internal init() {
        // a key starts it
    }

    /// A key went down: `action` runs again after ``delay``, then every ``interval``.
    ///
    /// A key held before is let go.
    @preconcurrency
    internal func start(_ action: @escaping @MainActor () -> Void) {
        begin(action, waits: [Self.delay])
    }

    /// A key went down that has not acted yet: `action` runs after ``hold``, again ``delay`` after the touch, then
    /// every ``interval``.
    ///
    /// A key held before is let go.
    @preconcurrency
    internal func startHeld(_ action: @escaping @MainActor () -> Void) {
        begin(action, waits: [Self.hold, Self.delay - Self.hold])
    }

    /// The key was let go.
    internal func stop() {
        timer?.invalidate()
        timer = nil
        action = nil
        waits = []
    }

    private func begin(_ action: @escaping @MainActor () -> Void, waits: [TimeInterval]) {
        stop()
        self.action = action
        self.waits = waits
        next()
    }

    // the timer of the next run: the next wait to come, or the interval once they are done
    private func next() {
        schedule(after: waits.isEmpty ? Self.interval : waits.removeFirst())
    }

    // one run, as the timer makes it: the action, and the next one
    private func fire() {
        guard let action else {
            return
        }
        action()
        // the action may have let the key go
        if self.action != nil {
            next()
        }
    }

    private func schedule(after delay: TimeInterval) {
        timer?.invalidate()
        let made = Timer(timeInterval: delay, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.fire()
            }
        }
        RunLoop.main.add(made, forMode: .common)
        timer = made
    }

    isolated deinit {
        timer?.invalidate()
    }
}
