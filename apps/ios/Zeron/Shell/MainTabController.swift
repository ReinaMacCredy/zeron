import UIKit

/// Root: native tab bar (Liquid Glass comes from the system, so tab switches,
/// minimize-on-scroll and the search morph are render-server animations), with
/// the "Ask anything" composer as the tab bar's bottom accessory.
final class MainTabController: UITabBarController, UITabBarControllerDelegate, AppRouter {
    private let app: AppModel

    init(app: AppModel) {
        self.app = app
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        // The selected tab reads through the glass pill + label weight; the
        // violet accent made it shout.
        tabBar.tintColor = Palette.text
        tabBarMinimizeBehavior = .onScrollDown

        let sessions = UITab(title: "Sessions", image: UIImage(named: "tab-chat"), identifier: "sessions") { [app] _ in
            Self.nav(SessionsViewController(app: app))
        }
        let tasks = UITab(title: "Tasks", image: UIImage(named: "tab-tasks"), identifier: "tasks") { [app] _ in
            Self.nav(TasksViewController(app: app))
        }
        let more = UITab(title: "Settings", image: UIImage(named: "tab-settings"), identifier: "more") { [app] _ in
            Self.nav(MoreViewController(app: app))
        }
        let search = UISearchTab { [app] _ in
            Self.nav(SearchViewController(app: app))
        }
        search.automaticallyActivatesSearch = true
        tabs = [sessions, tasks, more, search]
        updateTasksBadge()
        selectedTab = sessions

        bottomAccessory = accessory
        delegate = self
        accessoryContent.onNeedsYou = { [weak self] in
            guard let self else { return }
            NeedsYou.present(app: self.app, from: self, excluding: nil)
        }
        accessoryContent.update(app.live)
        liveToken = app.observe { [weak self] in
            guard let self else { return }
            self.accessoryContent.update(self.app.live)
            // Hosts come and go with the workspace (online, capabilities).
            self.accessoryContent.updateVoice(self.app.voice)
            self.updateTasksBadge()
        }
        let voice = accessoryContent.voice
        voice.onStart = { [weak self] source in
            guard let self else { return }
            self.openVoice(app: self.app, source: source)
        }
        voice.onOpen = { [weak self] source in
            guard let self else { return }
            self.presentVoiceStage(app: self.app, source: source)
        }
        voice.onMute = { [weak self] in self?.app.voice.toggleMute() }
        voice.onEnd = { [weak self] in self?.app.voice.stop() }
        voice.startButton.menu = voiceMenu(app: app)
        accessoryContent.updateVoice(app.voice)
        voiceToken = app.voice.observe { [weak self] in
            guard let self else { return }
            self.accessoryContent.updateVoice(self.app.voice)
        }
    }

    /// The accessory stays attached across tab switches, search included —
    /// the system carries it through the search morph (above the field, then
    /// under the keyboard). Taking it off for search and adding it back on
    /// the way out made the tab bar reflow mid-morph (pill and icons out of
    /// step for several frames). Leaving search, the shrinking field crosses
    /// the accessory for a couple of frames, so its content fades back in
    /// just after.
    func tabBarController(_ tabBarController: UITabBarController, didSelectTab selectedTab: UITab, previousTab: UITab?) {
        syncAccessory()
        guard previousTab is UISearchTab, !(selectedTab is UISearchTab), bottomAccessory != nil,
              !UIAccessibility.isReduceMotionEnabled else { return }
        accessoryContent.alpha = 0
        UIView.animate(withDuration: 0.25, delay: 0.12, options: [.curveEaseOut, .allowUserInteraction]) {
            self.accessoryContent.alpha = 1
        }
    }

    private func updateTasksBadge() {
        let count = app.tasks.needsYouCount
        tabs.first { $0.identifier == "tasks" }?.badgeValue = count > 0 ? "\(count)" : nil
    }

    private lazy var accessoryContent = AskAnythingAccessory { [weak self] in self?.presentNewSession() }
    private lazy var accessory = UITabAccessory(contentView: accessoryContent)
    private var liveToken: AnyObject?
    private var voiceToken: AnyObject?

    /// Accessory state from what's on screen: hidden over a session or a PR
    /// page (each has its own composer).
    func syncAccessory(animated: Bool = false) {
        setAccessoryVisible(accessoryWanted, animated: animated)
    }

    private var accessoryWanted: Bool {
        let top = (selectedTab?.viewController as? UINavigationController)?.topViewController
        return !(top is SessionViewController || top is PullRequestViewController)
    }

    /// Pushed sessions carry their own composer; the accessory steps aside.
    /// Swipe-back / pop: put the accessory in place *before* the transition
    /// animation (inside it, its first layout animates from a zero frame — it
    /// flew in from the top of the screen), content transparent so it can
    /// fade in with the gesture.
    func prepareAccessoryForReveal() {
        guard bottomAccessory == nil else { return }
        UIView.performWithoutAnimation {
            setBottomAccessory(accessory, animated: false)
            view.layoutIfNeeded()
            accessoryContent.alpha = 0
        }
    }

    /// The accessory's frame when it's on screen (toast placement).
    func accessoryFrame(in window: UIWindow) -> CGRect? {
        guard bottomAccessory != nil, accessoryContent.window != nil, !accessoryContent.isHidden else { return nil }
        // The content view sits inside the glass capsule; its bounds are the capsule's.
        return accessoryContent.convert(accessoryContent.bounds, to: window)
    }

    func setAccessoryContentAlpha(_ alpha: CGFloat) {
        accessoryContent.alpha = alpha
    }

    /// Swap the accessory *inside* a push/pop animation so it travels with
    /// the tab bar instead of snapping ahead of it (no empty glass pill, no
    /// composer ghosting over the tab bar). `alongside` and `completion`
    /// let the page fade its own bottom glass opposite to it.
    func transitionAccessory(visible: Bool, with coordinator: UIViewControllerTransitionCoordinator?,
                             alongside: @escaping () -> Void = {}, completion: @escaping (_ cancelled: Bool) -> Void = { _ in }) {
        let visible = visible && accessoryWanted
        guard let coordinator else { return setAccessoryVisible(visible, animated: false) }
        if visible { prepareAccessoryForReveal() }
        coordinator.animate(alongsideTransition: { _ in
            if visible {
                self.setAccessoryContentAlpha(1)
            } else {
                self.setAccessoryVisible(false, animated: false)
            }
            alongside()
        }, completion: { context in
            completion(context.isCancelled)
            // A cancelled interactive pop also re-runs viewWillAppear; two
            // flips left "New session" showing over the page, so re-derive.
            self.syncAccessory()
            self.setAccessoryContentAlpha(1)
        })
    }

    func setAccessoryVisible(_ visible: Bool, animated: Bool) {
        let target = visible ? accessory : nil
        guard bottomAccessory !== target else { return }
        setBottomAccessory(target, animated: animated)
    }

    static func nav(_ root: UIViewController) -> UINavigationController {
        let nav = UINavigationController(rootViewController: root)
        nav.navigationBar.prefersLargeTitles = true
        // Setting an appearance object drops iOS 26's scroll-edge effect;
        // style titles through the bar's own attributes instead.
        nav.navigationBar.largeTitleTextAttributes = [.font: Fonts.ui(.sansSemibold, 30), .foregroundColor: Palette.text]
        nav.navigationBar.titleTextAttributes = [.font: Fonts.ui(.sansSemibold, 17), .foregroundColor: Palette.text]
        nav.navigationBar.tintColor = Palette.text
        return nav
    }

    func presentNewSession(prompt: String? = nil) {
        let vc = NewSessionViewController(app: app, prompt: prompt) { [weak self] chatId, handoff in
            guard let self else { return }
            self.openSession(chatId, handoff: handoff)
        }
        let nav = UINavigationController(rootViewController: vc)
        nav.modalPresentationStyle = .pageSheet
        if let sheet = nav.sheetPresentationController {
            sheet.detents = [.large()]
            sheet.prefersGrabberVisible = true
        }
        present(nav, animated: true)
    }

    func showSettings() {
        selectedTab = tabs.first { $0.identifier == "more" }
    }

    func showSearch() {
        selectedTab = tabs.first { $0 is UISearchTab }
    }

    func showPullRequests(git: Bool, history: Bool) {
        guard let nav = sessionsNav() else { return }
        let board = PullRequestsViewController(app: app, segment: git ? .git : .pulls)
        nav.popToRootViewController(animated: false)
        nav.pushViewController(board, animated: true)
        if history { board.showGit(history: true) }
    }

    func openPullRequest(url: String, deviceId: String?) {
        guard let device = app.pullRequestDevice(deviceId) else { return Toast.show("No Mac serves pull requests.", in: view.window) }
        guard let nav = sessionsNav() else { return }
        let page = PullRequestViewController(app: app, deviceId: device, url: url)
        if nav.topViewController is PullRequestsViewController || nav.topViewController is PullRequestViewController {
            nav.pushViewController(page, animated: true)
        } else {
            nav.setViewControllers([nav.viewControllers[0], PullRequestsViewController(app: app), page], animated: true)
        }
    }

    private func sessionsNav() -> UINavigationController? {
        if presentedViewController != nil { dismiss(animated: true) }
        guard let tab = tabs.first(where: { $0.identifier == "sessions" }) else { return nil }
        selectedTab = tab
        syncAccessory()
        return tab.viewController as? UINavigationController
    }

    func showTasks(_ path: String) {
        selectedTab = tabs.first { $0.identifier == "tasks" }
        guard let nav = selectedTab?.viewController as? UINavigationController,
              let list = nav.viewControllers.first as? TasksViewController else { return }
        nav.popToRootViewController(animated: false)
        if path == "supervisor" {
            nav.pushViewController(SupervisorViewController(app: app), animated: false)
        } else if path.hasPrefix("mission:") {
            list.openMission(String(path.dropFirst("mission:".count)))
        } else if path.hasPrefix("task:") {
            list.presentTask(String(path.dropFirst("task:".count)))
        }
    }

    /// From the new-session sheet: the chat goes in under the sheet at once,
    /// the sheet leaves without its slide, and the handoff animation carries
    /// the draft (page, composer, message) into the chat.
    func openSession(_ chatId: String, handoff: DraftHandoff?) {
        guard let handoff, let window = view.window,
              let tab = tabs.first(where: { $0.identifier == "sessions" })
        else { return openSession(chatId) }
        selectedTab = tab
        guard let nav = tab.viewController as? UINavigationController else { return openSession(chatId) }
        guard let session = SessionViewController(app: app, chatId: chatId) else {
            return showSessionOpenError()
        }
        UIView.performWithoutAnimation {
            nav.popToRootViewController(animated: false)
            nav.pushViewController(session, animated: false)
            setAccessoryVisible(false, animated: false)
            view.layoutIfNeeded()
            session.prepareArrival()
            // Sending puts the keyboard away; it slides down with the handoff.
            presentedViewController?.view.endEditing(true)
            // Hide the sheet now; tear it down only once the motion is done.
            // Dismissal (plus the keyboard re-hosting with it) is a heavy
            // compositor frame — mid-animation it swallowed the motion.
            if let sheet = presentedViewController?.presentationController as? UISheetPresentationController {
                // Drop the sheet's background dimming with it (it would sit over
                // the chat until the deferred dismissal, then snap off).
                sheet.largestUndimmedDetentIdentifier = .large
            }
            presentedViewController?.presentationController?.containerView?.alpha = 0
        }
        DraftHandoffAnimator.run(handoff, into: session, window: window) { [weak self] in
            if self?.presentedViewController != nil { self?.dismiss(animated: false) }
        }
    }

    /// Back to the Sessions list, releasing any pushed session (the iPad
    /// split takes the open session back into its own column).
    func popToFrontPage() {
        sessionsStack?.popToRootViewController(animated: false)
    }

    private var sessionsStack: UINavigationController? {
        tabs.first(where: { $0.identifier == "sessions" })?.viewController as? UINavigationController
    }

    /// The split's open session, handed over as it is when the window
    /// narrows (iPhone Duo folding): same page, scroll position and draft.
    func adoptSession(_ session: SessionViewController) {
        guard let nav = sessionsStack else { return }
        selectedTab = tabs.first { $0.identifier == "sessions" }
        nav.popToRootViewController(animated: false)
        nav.pushViewController(session, animated: false)
        syncAccessory()
    }

    /// The pushed session, taken off the stack for the split's main column
    /// when the window widens; nil when none is open. It comes back marked
    /// `relocating`, for the split to clear once it's placed.
    func releaseSession() -> SessionViewController? {
        guard let nav = sessionsStack,
              let session = nav.viewControllers.last(where: { $0 is SessionViewController }) as? SessionViewController else {
            return nil
        }
        session.relocating = true
        nav.popToRootViewController(animated: false)
        syncAccessory()
        return session
    }

    func showSessionOpenError() {
        if presentedViewController != nil { dismiss(animated: false) }
        view.endEditing(true)
        selectedTab = tabs.first { $0.identifier == "sessions" }
        popToFrontPage()
        syncAccessory()
        view.layoutIfNeeded()
        Toast.show("Couldn't open session. Try again.", in: view.window)
    }

    /// Push a session on the Sessions tab (from new-session, deep links, search).
    func openSession(_ chatId: String) {
        guard let tab = tabs.first(where: { $0.identifier == "sessions" }) else { return }
        selectedTab = tab
        guard let nav = tab.viewController as? UINavigationController else { return }
        guard let session = SessionViewController(app: app, chatId: chatId) else {
            return showSessionOpenError()
        }
        if presentedViewController != nil { dismiss(animated: true) }
        nav.popToRootViewController(animated: false)
        nav.pushViewController(session, animated: true)
    }
}

/// The capsule above the tab bar: a plus, "New session", and a live
/// summary of what's running ("2 working · 1 needs you"), with voice at its
/// trailing end. Tapping the plus side opens the new-session composer; the
/// waveform calls Codex, and during a call the strip shows it (orb, clock,
/// mute, hang-up) and opens the call stage.
final class AskAnythingAccessory: UIView {
    private let newSession = PressableControl()
    private let label = UILabel()
    private let summary = UILabel()
    private let cells = StatusGlyph()
    private let mark = UIImageView(image: UIImage(systemName: "plus", withConfiguration: UIImage.SymbolConfiguration(pointSize: 15, weight: .semibold)))
    private let plate = UIView()
    private let needsChip = NeedsYouChip()
    private var summaryEnd: NSLayoutConstraint!
    var onNeedsYou: (() -> Void)?
    let voice = VoiceAccessoryView()
    private var voiceShown = false
    private var voiceLive = false
    private var live = AppModel.LiveCounts()
    private var restConstraints: [NSLayoutConstraint] = []
    private var idleVoiceConstraints: [NSLayoutConstraint] = []
    private var liveVoiceConstraints: [NSLayoutConstraint] = []

    init(onTap: @escaping () -> Void) {
        super.init(frame: .zero)
        newSession.accessibilityIdentifier = "new-session"
        newSession.accessibilityTraits = .button
        newSession.isAccessibilityElement = true
        newSession.addAction(UIAction { _ in onTap() }, for: .touchUpInside)

        mark.tintColor = Palette.accent
        mark.contentMode = .center
        plate.backgroundColor = Palette.accentSoft
        // A circle, concentric with the accessory capsule (7pt inset).
        plate.layer.cornerRadius = 17
        plate.layer.cornerCurve = .continuous
        plate.isUserInteractionEnabled = false
        plate.addSubview(mark)
        label.text = "New session"
        label.font = Fonts.ui(.sansMedium, 16)
        label.textColor = Palette.text
        summary.font = Fonts.ui(.sansMedium, 13)
        summary.textColor = Palette.secondary
        summary.textAlignment = .right
        cells.isHidden = true
        for v in [newSession, plate, label, mark, summary, cells, voice] as [UIView] { v.translatesAutoresizingMaskIntoConstraints = false }
        for v in [plate, label, summary, cells] as [UIView] { newSession.addSubview(v) }
        addSubview(newSession)
        needsChip.translatesAutoresizingMaskIntoConstraints = false
        needsChip.isHidden = true
        needsChip.addAction(UIAction { [weak self] _ in self?.onNeedsYou?() }, for: .touchUpInside)
        addSubview(needsChip)
        addSubview(voice)
        summaryEnd = summary.trailingAnchor.constraint(equalTo: newSession.trailingAnchor, constant: -16)
        voice.isHidden = true
        summary.lineBreakMode = .byTruncatingTail
        NSLayoutConstraint.activate([
            newSession.leadingAnchor.constraint(equalTo: leadingAnchor),
            newSession.topAnchor.constraint(equalTo: topAnchor),
            newSession.bottomAnchor.constraint(equalTo: bottomAnchor),
            plate.leadingAnchor.constraint(equalTo: newSession.leadingAnchor, constant: 7),
            plate.centerYAnchor.constraint(equalTo: newSession.centerYAnchor),
            plate.widthAnchor.constraint(equalToConstant: 34),
            plate.heightAnchor.constraint(equalToConstant: 34),
            mark.centerXAnchor.constraint(equalTo: plate.centerXAnchor),
            mark.centerYAnchor.constraint(equalTo: plate.centerYAnchor),
            label.leadingAnchor.constraint(equalTo: plate.trailingAnchor, constant: 11),
            label.centerYAnchor.constraint(equalTo: newSession.centerYAnchor),
            summaryEnd,
            needsChip.trailingAnchor.constraint(equalTo: newSession.trailingAnchor, constant: -8),
            needsChip.centerYAnchor.constraint(equalTo: newSession.centerYAnchor),
            summary.centerYAnchor.constraint(equalTo: newSession.centerYAnchor),
            cells.trailingAnchor.constraint(equalTo: summary.leadingAnchor, constant: -7),
            cells.leadingAnchor.constraint(greaterThanOrEqualTo: plate.trailingAnchor, constant: 11),
            cells.centerYAnchor.constraint(equalTo: newSession.centerYAnchor),
            cells.widthAnchor.constraint(equalToConstant: 12),
            cells.heightAnchor.constraint(equalToConstant: 12),
            voice.topAnchor.constraint(equalTo: topAnchor),
            voice.bottomAnchor.constraint(equalTo: bottomAnchor),
            voice.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -7),
        ])
        // Yields while the call strip narrows this side to the plus.
        let labelEnd = label.trailingAnchor.constraint(lessThanOrEqualTo: cells.leadingAnchor, constant: -10)
        labelEnd.priority = .defaultHigh
        labelEnd.isActive = true
        // No voice: "New session" spans the capsule.
        restConstraints = [newSession.trailingAnchor.constraint(equalTo: trailingAnchor)]
        // Idle voice: a 34pt waveform at the trailing end.
        idleVoiceConstraints = [
            voice.widthAnchor.constraint(equalToConstant: 34),
            newSession.trailingAnchor.constraint(equalTo: voice.leadingAnchor, constant: -2),
        ]
        // Live: the call strip takes everything after the plus.
        liveVoiceConstraints = [
            newSession.trailingAnchor.constraint(equalTo: plate.trailingAnchor, constant: 4),
            voice.leadingAnchor.constraint(equalTo: plate.trailingAnchor, constant: 12),
        ]
        NSLayoutConstraint.activate(restConstraints)
        registerForTraitChanges([UITraitTabAccessoryEnvironment.self]) { (self: AskAnythingAccessory, _) in
            self.voice.setInline(self.traitCollection.tabAccessoryEnvironment == .inline)
            self.setNeedsLayout()
        }
        update(AppModel.LiveCounts())
    }

    required init?(coder: NSCoder) { fatalError() }

    func update(_ live: AppModel.LiveCounts) {
        self.live = live
        let parts = summaryParts(live)
        summary.isHidden = voiceLive
        newSession.accessibilityLabel = parts.isEmpty ? "New session" : "New session, " + parts.joined(separator: ", ")
        setNeedsLayout()
    }

    private func summaryParts(_ live: AppModel.LiveCounts) -> [String] {
        var parts: [String] = []
        if live.working > 0 { parts.append("\(live.working) working") }
        if live.awaiting > 0 { parts.append("\(live.awaiting) need\(live.awaiting == 1 ? "s" : "") you") }
        return parts
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        fit()
    }

    /// Drops content by priority instead of overlapping: first the
    /// "New session" title, then the working count; "N needs you" is a
    /// chip of its own that opens the Needs you sheet.
    private func fit() {
        let working = live.working > 0 ? "\(live.working) working" : ""
        let needs = live.awaiting > 0 && !voiceLive
        if needs { needsChip.count = live.awaiting }
        needsChip.isHidden = !needs
        func width(_ s: String, _ f: UIFont) -> CGFloat { ceil((s as NSString).size(withAttributes: [.font: f]).width) }
        let chip = needs ? needsChip.intrinsicContentSize.width + 8 : 0
        let room = newSession.bounds.width - 41 - 16
        let workingRow = working.isEmpty ? 0 : 11 + 12 + 7 + width(working, summary.font)
        let showWorking = !working.isEmpty && workingRow + chip <= max(room, workingRow)
        let row = (showWorking ? workingRow : 0) + chip
        let title = 11 + width(label.text ?? "", label.font) + 10
        let inline = traitCollection.tabAccessoryEnvironment == .inline
        label.alpha = !inline && !voiceLive && title + row <= room ? 1 : 0
        let text = showWorking ? working : ""
        if summary.text != text { summary.text = text }
        summaryEnd.constant = needs ? -(8 + chip) : -16
        cells.isHidden = !showWorking || voiceLive
        cells.kind = .spinner
    }

    /// Show voice when a host can run it, and the call strip while one runs.
    func updateVoice(_ controller: RemoteVoiceController) {
        let shown = controller.live || controller.available
        let live = controller.live
        voice.update(controller)
        guard shown != voiceShown || live != voiceLive else { return }
        voiceShown = shown
        voiceLive = live
        NSLayoutConstraint.deactivate(restConstraints + idleVoiceConstraints + liveVoiceConstraints)
        NSLayoutConstraint.activate(!shown ? restConstraints : live ? liveVoiceConstraints : idleVoiceConstraints)
        voice.isHidden = !shown
        let apply = {
            self.summary.isHidden = live
            self.layoutIfNeeded()
            self.fit()
        }
        if window == nil || UIAccessibility.isReduceMotionEnabled {
            apply()
        } else {
            UIView.animate(withDuration: 0.35, delay: 0, usingSpringWithDamping: 0.9, initialSpringVelocity: 0, options: [.beginFromCurrentState, .allowUserInteraction], animations: apply)
        }
    }
}

extension UIViewController {
    /// The iPhone shell this screen sits in, if any (not beside the iPad
    /// sidebar, not a standalone sheet).
    var mainShell: MainTabController? {
        var ancestor = parent
        while let current = ancestor {
            if let shell = current as? MainTabController { return shell }
            ancestor = current.parent
        }
        return nil
    }
}
