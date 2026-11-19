import NgtermFeatures
import NgtermPlatform
import UIKit
import UserNotifications

/// The app's delegate (`@UIApplicationDelegateAdaptor`), made as the app launches.
///
/// It makes the core and its graph, registers the background refresh (before launching ends, as the system asks),
/// follows the scenes for the app's lifecycle, and takes the notifications' responses. A notification shows as a
/// banner in the foreground too (an ask out of sight, an agent's notice), but a status or an ask of a session on
/// screen. A response goes to the graph's ``Alerts``: a tap opens its target, an ask's action answers through the core
/// (in the grace too).
@MainActor
internal final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    /// The launch argument that turns off UIKit's view animations in a Debug build, for the UI tests.
    ///
    /// Before each action, XCTest waits until the app's view animations have stopped. On the iPad simulator a
    /// system animation sometimes never reports its end (UIKit's in-process animation manager gets no more
    /// frames), and then each action waits 2 x 60 s. Without view animations, XCTest finds none to wait for.
    internal static let stillArgument = "-ngterm-no-animations"

    /// The launch argument that makes a Debug build lay its windows out as soon as the text size changes, for the UI
    /// tests' Dynamic Type audits.
    ///
    /// The audit sets each text size in turn, and it reads the fonts when the app's main thread is idle, about 0.1 s
    /// after each change. Without help, the app takes a new size at a later frame (35-230 ms after the change on the
    /// simulator). On a loaded host, a late frame left a page at its last size when the audit read it, and the audit
    /// then reported each text as "partially unsupported". With this argument, every window has the new size before
    /// the main thread is idle again.
    internal static let textSizeArgument = "-ngterm-text-size-at-once"

    /// Which core the app runs on, and the graph over it.
    internal let core: CoreChoice
    private let life: AppLifecycle
    // follows the scenes for as long as the app runs
    private let scenes: SceneWatcher

    #if DEBUG
        // lays the windows out at each text size change (textSizeArgument)
        private var textSizeWatch: NSObjectProtocol?
    #endif

    override internal init() {
        #if DEBUG
            if ProcessInfo.processInfo.arguments.contains(Self.stillArgument) {
                UIView.setAnimationsEnabled(false)
            }
        #endif
        let choice = CoreChoice(arguments: ProcessInfo.processInfo.arguments)
        let refresh = BackgroundRefresh()
        let made = AppLifecycle(graph: choice.graph, grace: SystemGrace(), scheduler: refresh)
        core = choice
        life = made
        scenes = SceneWatcher(lifecycle: made.lifecycle)
        super.init()
        refresh.register {
            await made.refresh()
        }
        UNUserNotificationCenter.current().delegate = self
        #if DEBUG
            if ProcessInfo.processInfo.arguments.contains(Self.textSizeArgument) {
                textSizeWatch = Self.layOutAtTextSizeChanges()
            }
        #endif
    }

    #if DEBUG
        // lays every window out in the notification of a text size change. With no queue, the observer runs in the
        // notification's own turn of the main thread, where the windows' traits already have the new size.
        private static func layOutAtTextSizeChanges() -> NSObjectProtocol {
            NotificationCenter.default.addObserver(
                forName: UIContentSizeCategory.didChangeNotification,
                object: nil,
                queue: nil
            ) { _ in
                MainActor.assumeIsolated {
                    for case let scene as UIWindowScene in UIApplication.shared.connectedScenes {
                        for window in scene.windows {
                            window.layoutIfNeeded()
                        }
                    }
                }
            }
        }
    #endif

    nonisolated internal func userNotificationCenter(
        _: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        let info = SystemNotices.info(of: notification)
        return await presentation(of: notification.request.identifier, info: info)
    }

    nonisolated internal func userNotificationCenter(
        _: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let answer = SystemNotices.response(response)
        await respond(to: answer)
    }

    private func respond(to response: NoticeResponse) {
        core.graph.alerts.respond(to: response)
    }

    // a notice shows in the foreground too, but a status or an ask of a session on screen already (it came just before
    // it was)
    private func presentation(of id: String, info: [String: String]) -> UNNotificationPresentationOptions {
        var key: SessionKey?
        if let machineId = info[Alert.machineInfo], let sessionId = info[Alert.sessionInfo] {
            key = SessionKey(machineId: machineId, sessionId: sessionId)
        }
        let shows = Alert.showsInForeground(id: id, key: key, onScreen: core.graph.asks.onScreen)
        return shows ? [.banner, .list, .sound] : []
    }

    deinit {
        // nothing to release: the delegate lives as long as the app
    }
}
