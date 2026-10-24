import UIKit

/// One session: virtualized transcript under a glass bottom stack (status
/// pill, queue, composer or question panel) that rides the keyboard.
final class SessionViewController: UIViewController, UIGestureRecognizerDelegate {
    private let app: AppModel
    let chatId: String
    private let source: SessionSource
    private lazy var relay = FrameRelay { [weak self] in self?.applyFrame() }
    private lazy var engine = TranscriptView(text: TextEngine.shared, listener: relay)
    private lazy var list = TranscriptListView(engine: engine)
    private let composer = ComposerBar()
    private let questions = QuestionPanel()
    private let queue = QueuePanel()
    private let pill = StatusPill()
    private let needsYou = NeedsYouPill()
    private var needsYouRow: UIStackView!
    private let agentsPill = AgentsPill()
    private var agentsRow: UIStackView!
    private var agentsShown = false
    private var agentsCard: AgentsCard?
    private let agentsScrim = UIControl()
    private var pendingToken: AnyObject?
    private let still = ComputerStillPreview()
    /// A Cloud chat out of credits says so once, above the composer.
    private let creditsNotice = CloudCreditsNotice()
    private let bottom = UIStackView()
    private var bottomRest: NSLayoutConstraint?
    private let jump = Glass.circleButton(symbol: "arrow.down", size: 40, pointSize: 15)
    private let titleView = SessionTitleView()
    private var shown = SessionChrome()
    private let openedAt = CACurrentMediaTime()
    private var reportedOpen = false
    private var routedPanel = false

    init?(app: AppModel, chatId: String) {
        guard let source = app.sessionSource(chatId) else { return nil }
        self.app = app
        self.chatId = chatId
        self.source = source
        super.init(nibName: nil, bundle: nil)
        hidesBottomBarWhenPushed = true
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        wallpaper.behindChat = true
        view.addSubview(wallpaper)
        // The app can be killed in the background: the draft goes to disk first.
        backgroundObserver = NotificationCenter.default.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { [weak self] _ in
            self?.saveDraft()
        }
        navigationItem.largeTitleDisplayMode = .never
        navigationItem.titleView = titleView
        navigationItem.rightBarButtonItem = UIBarButtonItem(image: UIImage(systemName: "ellipsis"), menu: sessionMenu())
        navigationItem.rightBarButtonItem?.accessibilityIdentifier = "session-menu"
        // The session has its own composer, so the tab bar's voice control
        // steps aside with the accessory; the call stays a tap away up here.
        let voiceBar = VoiceBarItem(app: app, host: self)
        self.voiceBar = voiceBar
        // A custom view reads as horizontal-only; on iPhone Duo it joins ⋯ in
        // the vertical bar.
        #if compiler(>=6.4)
        if #available(iOS 27.1, *) { voiceBar.item.axisBehavior = .verticalPreferred }
        #endif
        let menu = navigationItem.rightBarButtonItem!
        let syncVoice = { [weak self, weak voiceBar] in
            guard let self, let voiceBar else { return }
            let items = voiceBar.visible ? [menu, voiceBar.item] : [menu]
            if self.navigationItem.rightBarButtonItems?.count != items.count {
                self.navigationItem.setRightBarButtonItems(items, animated: self.view.window != nil)
            }
        }
        voiceToken = app.voice.observe(syncVoice)
        hostsToken = app.observe(syncVoice)
        syncVoice()

        list.frame = view.bounds
        list.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        list.accessibilityIdentifier = "transcript"
        list.imageLoader = { [weak self] ref, iv in self?.source.loadImage(ref, into: iv) }
        list.imageFetcher = { [weak self] ref in await self?.source.image(ref) }
        // An error card's "Log in": that provider's sign-in, started at once.
        list.onAction = { [weak self] action in
            guard let self, action == "sign-in", let target = self.source.signInTarget() else { return }
            let accounts = AgentAccountsViewController(app: self.app, target: target, signIn: true)
            let nav = UINavigationController(rootViewController: accounts)
            if let sheet = nav.sheetPresentationController {
                sheet.detents = [.medium(), .large()]
                sheet.prefersGrabberVisible = true
            }
            self.present(nav, animated: true)
        }
        list.htmlLoader = { [weak self] device, path in await self?.source.htmlPage(deviceId: device, path: path) }
        list.textFetcher = { [weak self] ref in await self?.source.attachmentText(ref) }
        // Hidden while following (the runway glide and tail spring travel).
        list.onDistanceFromBottom = { [weak self] d in
            guard let self else { return }
            self.setJumpVisible(d > 140 && !self.list.following)
        }
        list.onCardAction = { [weak self] action in self?.perform(action) }
        list.branchBlocked = { [weak self] rewind in await self?.source.branchBlocked(rewind: rewind) }
        list.onBranch = { [weak self] offer in self?.confirmBranch(offer) }
        list.onSkill = { [weak self] name, path in self?.openSkill(name, path: path) }
        list.onLimit = { [weak self] id in self?.openLimitCard(id) }
        list.onPullRequest = { [weak self] url in
            guard let self else { return }
            (self.view.window?.rootViewController as? AppRouter)?.openPullRequest(url: url, deviceId: self.app.row(self.chatId)?.deviceId)
        }
        list.onQuote = { [weak self] text in
            guard let self else { return }
            self.composer.quotes.append(text)
            self.composer.becomeFirstResponder()
        }
        view.addSubview(list)
        still.translatesAutoresizingMaskIntoConstraints = false
        still.fetch = { [weak self] in await self?.source.computerStill() }
        view.addSubview(still)
        NSLayoutConstraint.activate([
            still.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            still.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -12),
        ])
        missionStrip.configuration?.cornerStyle = .capsule
        missionStrip.configuration?.baseForegroundColor = Palette.text
        missionStrip.configuration?.background.visualEffect = UIBlurEffect(style: .systemThickMaterial)
        missionStrip.configuration?.background.backgroundColor = Palette.elevated.withAlphaComponent(0.85)
        missionStrip.configuration?.background.strokeColor = Palette.hairline
        missionStrip.configuration?.background.strokeWidth = 1
        missionStrip.configuration?.image = TaskGlyph.asset("glyph-mission", side: 14)
        missionStrip.configuration?.imagePadding = 6
        missionStrip.configuration?.titleLineBreakMode = .byTruncatingTail
        missionStrip.configuration?.preferredSymbolConfigurationForImage = UIImage.SymbolConfiguration(pointSize: 12)
        missionStrip.accessibilityIdentifier = "mission-strip"
        missionStrip.isHidden = true
        missionStrip.translatesAutoresizingMaskIntoConstraints = false
        missionStrip.addAction(UIAction { [weak self] _ in self?.openMission() }, for: .primaryActionTriggered)
        view.addSubview(missionStrip)
        NSLayoutConstraint.activate([
            missionStrip.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 8),
            missionStrip.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 12),
            missionStrip.trailingAnchor.constraint(lessThanOrEqualTo: still.leadingAnchor, constant: -8),
        ])
        missionToken = app.observe { [weak self] in self?.syncMission() }
        syncMission()
        // Tapping the transcript puts the composer (and keyboard) away.
        let dismissTap = UITapGestureRecognizer(target: self, action: #selector(dismissComposer))
        dismissTap.cancelsTouchesInView = false
        dismissTap.delegate = self
        list.addGestureRecognizer(dismissTap)
        setContentScrollView(list, for: .top)
        list.topEdgeEffect.style = .soft
        list.bottomEdgeEffect.isHidden = true

        composer.onSend = { [weak self] text, images, mode in
            guard let self else { return .kept }
            if self.editingQueueId != nil {
                self.endEdit(commit: text)
                return .replaced
            }
            if self.mission {
                self.startMission(text: text, images: images)
                return .kept
            }
            // An immediate send (new turn, steer, stop-and-send) gets the
            // desktop runway; a message queued behind a live turn keeps the
            // live turn's runway and just follows.
            if let command = compactCommand(text: text) {
                self.source.compact(focus: command.focus)
                DispatchQueue.main.async { self.composer.resignFirstResponder() }
                return .sent
            }
            if let command = sidechatCommand(text: text) {
                DispatchQueue.main.async { self.composer.resignFirstResponder() }
                self.presentSidechats()?.openSidechat(prompt: command.prompt)
                return .sent
            }
            let queued = self.shown.running && mode == .queue
            // Out of credits, a Cloud send can't wake its machine: say so
            // (with a way to add some) instead of a send that never lands.
            if self.cloudOutOfCredits {
                self.creditsNotice.nudge()
                return .kept
            }
            guard self.source.send(text: text, images: images, mode: mode) else { return .kept }
            // Sending puts the keyboard away (the composer rests as the capsule).
            DispatchQueue.main.async { self.composer.resignFirstResponder() }
            if queued { self.list.expectQueuedTurn() } else { self.list.beginOwnTurn() }
            return .sent
        }
        composer.onStop = { [weak self] in self?.source.stop() }
        composer.onCommentRemove = { [weak self] id in self?.source.removeStagedComment(id) }
        composer.onCommentsClear = { [weak self] in self?.source.clearStagedComments() }
        composer.text = Drafts.load(chatId)
        composer.mentionSearch = { [weak self] q in await self?.source.searchFiles(q) ?? [] }
        composer.threadSearch = { [weak self] q in self?.source.threadMentions(q) ?? [] }
        composer.invocationCatalog = { [weak self] skill in await self?.source.invocationCatalog(skill: skill) }
        composer.onWorkspaceCommand = { [weak self] command in self?.run(command) }
        composer.onHeightChange = { [weak self] in self?.view.setNeedsLayout() }
        composer.onActionChange = { [weak self] in self?.render(animated: true) }
        composer.onChipTap = { [weak self] id, _ in
            guard let self else { return }
            if id == "mission" {
                self.mission.toggle()
                return self.render(animated: true)
            }
            guard let sheet = self.source.chipSheet(id) else { return }
            self.present(sheet, animated: true)
        }
        questions.onSubmit = { [weak self] answers in
            guard let self, let id = self.shown.questions?.requestId else { return }
            self.source.answer(requestId: id, answers: answers)
        }
        questions.onHeightChange = { [weak self] in self?.view.setNeedsLayout() }
        queue.onAction = { [weak self] id, action in
            guard let self else { return }
            if action == .edit { self.beginEdit(id) } else { self.source.queueAction(id, action) }
        }
        queue.loadImage = { [weak self] ref, view in self?.source.loadImage(ref, into: view) }
        pill.onTap = { [weak self] in
            guard let self else { return }
            if self.editingQueueId != nil { self.endEdit(commit: nil) }
            if case .notDelivered = self.shown.banner { self.source.retryDelivery() }
            if case .limit = self.shown.banner { self.limitActions() }
            if case let .offer(title, body) = self.shown.banner { self.offerCompaction(title: title, body: body) }
        }

        // Watched whatever the row says now: a just-created Cloud chat's row
        // may not have arrived yet.
        cloudToken = app.cloud.observe { [weak self] in
            guard let self, self.isCloud else { return }
            self.render(animated: true)
        }

        bottom.axis = .vertical
        bottom.spacing = 8
        bottom.alignment = .fill
        bottom.translatesAutoresizingMaskIntoConstraints = false
        let pillRow = UIStackView(arrangedSubviews: [pill, UIView()])
        pillRow.axis = .horizontal
        needsYouRow = UIStackView(arrangedSubviews: [needsYou, UIView()])
        needsYouRow.axis = .horizontal
        needsYouRow.isHidden = true
        needsYou.addAction(UIAction { [weak self] _ in self?.openNeedsYou() }, for: .touchUpInside)
        agentsRow = UIStackView(arrangedSubviews: [agentsPill, UIView()])
        agentsRow.axis = .horizontal
        agentsRow.isHidden = true
        agentsPill.addAction(UIAction { [weak self] _ in self?.toggleAgentsCard() }, for: .touchUpInside)
        agentsScrim.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        agentsScrim.accessibilityIdentifier = "agents-card-scrim"
        agentsScrim.addAction(UIAction { [weak self] _ in self?.closeAgentsCard(animated: true) }, for: .touchDown)
        creditsNotice.onAddCredits = { [weak self] in
            guard let self else { return }
            CloudRoutes.openBilling(app: self.app, from: self)
        }
        creditsNotice.isHidden = true
        for v in [creditsNotice, needsYouRow!, agentsRow!, pillRow, queue, questions, composer] { bottom.addArrangedSubview(v) }
        questions.isHidden = true
        queue.isHidden = true
        // Hidden panels start dematerialized so their first appearance grows in.
        questions.setGlassVisible(false, animated: false)
        queue.setGlassVisible(false, animated: false)
        pillRow.isHidden = true
        view.addSubview(bottom)
        // Transcript fades into the background behind the composer, like the
        // nav bar's edge effect at the top. A gradient overlay (not a mask on
        // the scroll view) so scrolling never renders offscreen.
        bottomFade.translatesAutoresizingMaskIntoConstraints = false
        view.insertSubview(bottomFade, belowSubview: bottom)
        NSLayoutConstraint.activate([
            bottomFade.topAnchor.constraint(equalTo: bottom.topAnchor, constant: -44),
            bottomFade.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            bottomFade.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            bottomFade.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        jump.addAction(UIAction { [weak self] _ in self?.list.scrollToBottom(animated: true) }, for: .touchUpInside)
        jump.accessibilityIdentifier = "jump-to-latest"
        jump.accessibilityLabel = "Jump to latest"
        jump.alpha = 0
        jump.transform = CGAffineTransform(scaleX: 0.6, y: 0.6)
        view.addSubview(jump)

        NSLayoutConstraint.activate([
            // Full width on phones; a centered reading column on iPad/landscape.
            bottom.centerXAnchor.constraint(equalTo: view.safeAreaLayoutGuide.centerXAnchor),
            bottom.leadingAnchor.constraint(greaterThanOrEqualTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 12),
            bottom.widthAnchor.constraint(lessThanOrEqualToConstant: 768),
            {
                let fill = bottom.widthAnchor.constraint(equalTo: view.safeAreaLayoutGuide.widthAnchor, constant: -24)
                fill.priority = .defaultHigh
                return fill
            }(),
            jump.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
            jump.bottomAnchor.constraint(equalTo: bottom.topAnchor, constant: -12),
        ])
        bottomRest = pinAboveKeyboard(bottom)

        source.onChange = { [weak self] in self?.render(animated: true) }
        source.onPresent = { [weak self] vc in self?.present(vc, animated: true) }
        source.attach(engine)
        render(animated: false)
        pendingToken = app.observeNeedsYou { [weak self] in self?.syncNeedsYou(animated: true) }
        syncNeedsYou(animated: false)
        app.markSeen(chatId)
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        setAccessory(visible: false)
        // Coming back on screen (e.g. after a second copy of this chat was
        // popped and detached it): live again.
        if hasAppeared { source.reattach() }
        composer.preferredDelivery = UserDefaults.standard.string(forKey: "followUpBehavior") == "steer" ? .steer : .queue
        if hasAppeared { render(animated: false) }
    }

    private var hasAppeared = false
    /// Set by the split for the moment it moves this page between its
    /// shells, so the move doesn't read as closing the session.
    var relocating = false
    private var backgroundObserver: NSObjectProtocol?
    private var voiceBar: VoiceBarItem?
    private var voiceToken: AnyObject?
    private var hostsToken: AnyObject?
    private var missionToken: AnyObject?
    private let missionStrip = UIButton(configuration: .plain())
    private var mission = false
    private var missionReview = true
    private var missionPlanDoc = true

    deinit {
        if let backgroundObserver { NotificationCenter.default.removeObserver(backgroundObserver) }
    }

    private func setAccessory(visible: Bool) {
        guard let tabs = tabBarController as? MainTabController else { return }
        // Our bottom glass (composer stack) and the tab bar's glass never
        // overlap: the stack fades opposite to the tab bar, tracking an
        // interactive swipe-back too.
        let bottomStack = bottom
        let jumpButton = jump
        if transitionCoordinator != nil { bottomStack.alpha = visible ? 1 : 0 }
        tabs.transitionAccessory(visible: visible, with: transitionCoordinator, alongside: {
            if visible { jumpButton.alpha = 0 }
            bottomStack.alpha = visible ? 0 : 1
        }, completion: { cancelled in
            if cancelled { bottomStack.alpha = 1 }
        })
    }

    // MARK: Opening (loader, then the transcript)

    /// Shown while a pushed session loads.
    private let loader = StatusGlyph(.spinner)
    /// The transcript waits for the push to land before showing: the soft
    /// edge effect under the bar can't pick up content that appears mid-push
    /// (it read crisp under the title, then the effect snapped on). The page
    /// slides in with a loader; the transcript fades in once the push is
    /// done and its rows are laid out.
    private var holdingTranscript = false
    private var landed = false

    override func viewIsAppearing(_ animated: Bool) {
        super.viewIsAppearing(animated)
        // First appearance by an animated push (the new-session handoff and
        // unanimated column swaps reveal their own way).
        guard animated, !hasAppeared, transitionCoordinator != nil, list.alpha > 0 else { return }
        holdingTranscript = true
        list.alpha = 0
        if loader.superview == nil {
            loader.translatesAutoresizingMaskIntoConstraints = false
            view.insertSubview(loader, aboveSubview: list)
            NSLayoutConstraint.activate([
                loader.centerXAnchor.constraint(equalTo: view.centerXAnchor),
                loader.centerYAnchor.constraint(equalTo: view.safeAreaLayoutGuide.centerYAnchor, constant: -40),
                loader.widthAnchor.constraint(equalToConstant: 12),
                loader.heightAnchor.constraint(equalToConstant: 12),
            ])
            loader.transform = CGAffineTransform(scaleX: 1.75, y: 1.75)
        }
        loader.alpha = 1
        loader.accessibilityIdentifier = "session-loading"
    }

    /// Both conditions met (push landed, rows in): show the transcript.
    private func showTranscriptIfReady() {
        guard holdingTranscript, landed, reportedOpen else { return }
        holdingTranscript = false
        list.settleEdgeEffect()
        let reveal = {
            self.list.alpha = 1
            self.loader.alpha = 0
        }
        if UIAccessibility.isReduceMotionEnabled { return reveal() }
        UIView.animate(withDuration: 0.18, delay: 0, options: [.curveEaseOut, .allowUserInteraction], animations: reveal)
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        hasAppeared = true
        landed = true
        (splitViewController as? SplitRootController)?.sessionDidAppear(chatId)
        list.settleEdgeEffect()
        showTranscriptIfReady()
        openRoutedPanel()
        if holdingTranscript {
            // Nothing to lay out (an empty or unreachable session): stop
            // holding eventually rather than spin forever.
            DispatchQueue.main.asyncAfter(deadline: .now() + 8) { [weak self] in
                guard let self, self.holdingTranscript else { return }
                self.reportedOpen = true
                self.showTranscriptIfReady()
            }
        }
        (tabBarController as? MainTabController)?.syncAccessory()
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        // Leaving mid-edit: give the queued row back and the draft its place.
        if editingQueueId != nil { endEdit(commit: nil) }
        saveDraft()
        closeAgentsCard(animated: false)
        if isMovingFromParent { setAccessory(visible: true) }
    }

    /// Tear down only once the pop has really happened: an interactive
    /// swipe-back reports `isMovingFromParent` in viewWillDisappear too, and
    /// if it's cancelled the session stays on screen — detaching there left it
    /// frozen (no new updates) until you left and came back.
    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        // Carried to the other shell (iPhone Duo folding or unfolding): it
        // left one stack for another and stays live. UIKit may report the
        // move late or not at all, so ask where the page is, not how it left.
        if relocating || navigationController?.viewControllers.contains(self) == true { return }
        if isMovingFromParent || navigationController == nil {
            if mainShell != nil { (splitViewController as? SplitRootController)?.sessionDidClose(chatId) }
            source.detach()
            engine.close()
            app.markSeen(chatId)
        }
    }

    // MARK: Arrival from the new-session draft (DraftHandoffAnimator)

    /// Content hidden until the handoff reveals it; the keyboard goes away
    /// with the draft (sending dismisses it).
    func prepareArrival() {
        loadViewIfNeeded()
        list.beginOwnTurn()
        list.alpha = 0
        bottom.alpha = 0
        view.layoutIfNeeded()
    }

    func arrivalComposerFrame(in window: UIWindow) -> CGRect? {
        view.layoutIfNeeded()
        return composer.convert(composer.bounds, to: window)
    }

    func arrivalBubbleFrame(in window: UIWindow) -> CGRect? {
        list.layoutIfNeeded()
        return list.firstUserBubble(in: window)
    }

    func revealArrivalComposer() { bottom.alpha = 1 }
    func revealArrivalTranscript() { list.alpha = 1 }
    func finishArrival() {
        list.alpha = 1
        bottom.alpha = 1
    }

    private func perform(_ action: CardAction) {
        switch action {
        case let .consent(requestId, questionId, answer, _):
            source.answerConsent(requestId: requestId, questionId: questionId, answer: answer)
        case .enableComputer:
            composer.text = computerUseEnablingDraft(draft: composer.text)
            composer.becomeFirstResponder()
        }
    }

    private func confirmBranch(_ offer: BranchOffer) {
        let sheet = UIAlertController(title: offer.title, message: "\(offer.context)\n\n\(offer.detail)", preferredStyle: .actionSheet)
        sheet.addAction(UIAlertAction(title: offer.option, style: offer.rewind ? .destructive : .default) { [weak self] _ in
            guard let self else { return }
            Task { @MainActor [weak self] in
                guard let self else { return }
                do {
                    if offer.rewind {
                        self.composer.text = try await self.source.rewind(to: offer.entryId)
                        self.composer.becomeFirstResponder()
                    } else {
                        let id = try await self.source.fork(from: offer.entryId)
                        (self.view.window?.rootViewController as? AppRouter)?.openSession(id)
                    }
                } catch {
                    Toast.show(Self.branchFailure(error, rewind: offer.rewind), in: self.view.window)
                }
            }
        })
        sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        sheet.popoverPresentationController?.sourceView = list
        present(sheet, animated: true)
    }

    private func openLimitCard(_ id: String) {
        let open: () -> Void = { [weak self] in (self?.view.window?.rootViewController as? AppRouter)?.openSession(id) }
        guard let card = source.limitCard(chatId: id) else {
            open()
            return
        }
        let sheet = LimitCardViewController(card: card)
        sheet.onResolve = { [weak self] choice in
            do {
                try await self?.source.resolveLimit(chatId: id, choice: choice)
                return true
            } catch {
                Toast.show(Self.hostFailure(error, fallback: "Couldn't reach the host. Try again."), in: self?.view.window)
                return false
            }
        }
        sheet.onOpenChat = open
        let nav = UINavigationController(rootViewController: sheet)
        if let s = nav.sheetPresentationController { s.detents = [.medium(), .large()]; s.prefersGrabberVisible = true }
        present(nav, animated: true)
    }

    private func openInfo() {
        let info = SessionInfoViewController(sources: engine.frame().sources())
        info.loadUsage = { [weak self] in await self?.source.chatUsage() }
        info.loadChanges = { [weak self] in await self?.source.chatChanges() }
        info.openChanges = { [weak self, weak info] title in
            guard let self, let info else { return }
            self.source.openChanges(from: info, title: title)
        }
        info.openSource = { [weak self, weak info] item in
            Task { @MainActor in
                guard let self, let info, let image = await self.source.image(item.reference) else {
                    Toast.show("Couldn't load \(item.name). Try again.", in: self?.view.window)
                    return
                }
                info.present(ImageViewer(image: image, source: info.view), animated: true)
            }
        }
        let nav = UINavigationController(rootViewController: info)
        if let s = nav.sheetPresentationController { s.detents = [.medium(), .large()]; s.prefersGrabberVisible = true }
        present(nav, animated: true)
    }

    private func openTerminal(_ terminalId: String?) {
        let nav = UINavigationController(rootViewController: TerminalViewController(source: source, terminalId: terminalId))
        if let s = nav.sheetPresentationController { s.detents = [.medium(), .large()]; s.selectedDetentIdentifier = .large; s.prefersGrabberVisible = true }
        present(nav, animated: true)
    }

    private func run(_ command: WorkspaceCommand) {
        switch command {
        case .model: composer.openChip("model")
        case .new: router?.presentNewSession(prompt: nil)
        case .resume: router?.showSearch()
        case .settings: router?.showSettings()
        case .diff:
            Task { @MainActor [weak self] in
                guard let self else { return }
                guard let changes = await self.source.chatChanges() else {
                    return Toast.show("No changes in this chat yet", in: self.view.window, above: self.bottom)
                }
                self.source.openChanges(from: self, title: changes.title)
            }
        case .files:
            composer.beginFileMention()
        case .terminal: openTerminal(nil)
        case .rename: renameChat()
        case .stop: source.stop()
        }
    }

    private func renameChat() {
        let alert = UIAlertController(title: "Rename Session", message: nil, preferredStyle: .alert)
        alert.addTextField { [weak self] field in field.text = self.flatMap { $0.app.row($0.chatId)?.title } }
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        alert.addAction(UIAlertAction(title: "Rename", style: .default) { [weak self] _ in
            let title = alert.textFields?.first?.text?.trimmingCharacters(in: .whitespaces) ?? ""
            guard let self, !title.isEmpty else { return }
            self.app.rename(self.chatId, title)
        })
        present(alert, animated: true)
    }

    private func openProjectActions() {
        let actions = ProjectActionsViewController(source: source)
        actions.openTerminal = { [weak actions] id in
            actions?.navigationController?.pushViewController(TerminalViewController(source: self.source, terminalId: id), animated: true)
        }
        let nav = UINavigationController(rootViewController: actions)
        if let s = nav.sheetPresentationController { s.detents = [.medium(), .large()]; s.prefersGrabberVisible = true }
        present(nav, animated: true)
    }

    private func confirmComputerStop() {
        let host = shown.hostName ?? "the Mac"
        let alert = UIAlertController(
            title: "Stop computer use?",
            message: "Zeron stops using apps on \(host) for this chat now. Other chats keep their access.",
            preferredStyle: .alert
        )
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        alert.addAction(UIAlertAction(title: "Stop", style: .destructive) { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                do {
                    let stopped = try await self.source.computerStop()
                    Toast.show(stopped ? "Stopped computer use" : "Computer use had already stopped", in: self.view.window)
                } catch {
                    Toast.show(Self.hostFailure(error, fallback: "Couldn't reach the host. Try again."), in: self.view.window)
                }
            }
        })
        present(alert, animated: true)
    }

    private func openRoutedPanel() {
        let args = ProcessInfo.processInfo.arguments
        guard !routedPanel, let i = args.firstIndex(of: "-route"), i + 1 < args.count else { return }
        let parts = args[i + 1].dropFirst(5).split(separator: "/", maxSplits: 1).map(String.init)
        guard args[i + 1].hasPrefix("chat:"), parts.count == 2, parts[0] == chatId else { return }
        routedPanel = true
        switch parts[1] {
        case "terminal": openTerminal(nil)
        case "actions": openProjectActions()
        case "stop": confirmComputerStop()
        default: break
        }
    }

    private func limitActions() {
        guard let limit = shown.limitPill else { return }
        let sheet = UIAlertController(title: limit.text, message: nil, preferredStyle: .actionSheet)
        let run = { [weak self] (work: @escaping (SessionSource) async throws -> Void) in
            _ = Task { @MainActor in
                guard let self else { return }
                do { try await work(self.source) } catch {
                    Toast.show(Self.hostFailure(error, fallback: "Couldn't reach the host. Try again."), in: self.view.window)
                }
            }
        }
        let id = chatId
        if let model = limit.switchTo {
            sheet.addAction(UIAlertAction(title: "Switch to \(model)", style: .default) { _ in
                run { try await $0.resolveLimit(chatId: id, choice: "switch") }
            })
        }
        if let action = limit.action {
            sheet.addAction(UIAlertAction(title: action, style: .default) { _ in
                run { try await $0.setLimitResume(chatId: id, auto: !limit.auto) }
            })
        }
        sheet.addAction(UIAlertAction(title: "Close", style: .cancel))
        sheet.popoverPresentationController?.sourceView = pill
        present(sheet, animated: true)
    }

    private static func hostFailure(_ error: Error, fallback: String) -> String {
        switch error as? CoreError {
        case let .NotFound(text), let .HostError(text): text
        default: fallback
        }
    }

    private func openSkill(_ name: String, path: String?) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                let file = try await self.source.skillFile(name: name, path: path)
                let sheet = SelectTextViewController(text: file.text, title: name, mono: true)
                self.present(UINavigationController(rootViewController: sheet), animated: true)
            } catch {
                let message: String
                switch error as? CoreError {
                case let .NotFound(text), let .HostError(text): message = text
                default: message = "Couldn't open \(name)'s SKILL.md. Try again."
                }
                Toast.show(message, in: self.view.window)
            }
        }
    }

    private static func branchFailure(_ error: Error, rewind: Bool) -> String {
        if case let .HostError(message) = error as? CoreError { return message }
        return rewind ? "Couldn't rewind. Try again." : "Couldn't fork. Try again."
    }

    @objc private func dismissComposer() {
        if composer.textView.isFirstResponder { composer.resignFirstResponder() }
    }

    /// Controls in the transcript (tool rows, copy, disclosure) keep their taps.
    func gestureRecognizer(_ g: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        var v = touch.view
        while let view = v, view !== list {
            if view is UIControl { return false }
            v = view.superview
        }
        return true
    }

    /// Putting the composer away rides along with the transcript's own taps
    /// (links, images). Tap recognizers are exclusive by default: this one
    /// won and every link tap was dropped.
    func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        other.view.map { $0 === list || $0.isDescendant(of: list) } ?? false
    }

    private let bottomFade = EdgeFadeOverlay()

    override func viewWillLayoutSubviews() {
        super.viewWillLayoutSubviews()
        bottomRest?.constant = -composerRestGap
    }

    private let wallpaper = WallpaperView()

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        wallpaper.frame = view.bounds
        let covered = view.bounds.maxY - bottom.frame.minY + 8
        let inset = max(0, covered - view.safeAreaInsets.bottom)
        if abs(list.contentInset.bottom - inset) > 0.5 {
            let following = list.following
            list.contentInset.bottom = inset
            list.verticalScrollIndicatorInsets.bottom = inset
            if following { list.contentOffset.y = list.maxOffsetY }
        }
        let top = missionStrip.isHidden ? 0 : missionStrip.frame.maxY + 6 - view.safeAreaInsets.top
        if abs(list.contentInset.top - top) > 0.5 {
            let atTop = list.contentOffset.y <= -list.adjustedContentInset.top + 0.5
            list.contentInset.top = top
            list.verticalScrollIndicatorInsets.top = top
            if atTop { list.contentOffset.y = -list.adjustedContentInset.top }
        }
    }

    // MARK: Queue editing (host lease)

    private var editingQueueId: String?
    private var stashedDraft = ""
    private var stashedImages: [StagedImage] = []
    /// The draft is parked in the stash (from the first edit until the last ends).
    private var stashed = false

    /// The user's own draft (never a queued message being edited).
    private func saveDraft() {
        Drafts.save(chatId, stashed ? stashedDraft : composer.text)
    }

    /// An edit lease is being requested (taps on other rows wait).
    private var editPending = false

    private func beginEdit(_ id: String) {
        guard !editPending else { return }
        editPending = true
        let switching = editingQueueId != nil
        if switching {
            // Row A goes back and the user's own draft returns right away, so
            // nothing of A's edit can be sent as a new message meanwhile.
            editingQueueId = nil
            restoreStash()
            composer.placeholder = shown.placeholder
            render(animated: true)
        }
        Task { @MainActor in
            defer { editPending = false }
            if switching { await source.finishEdit(text: nil) }
            guard let text = await source.beginEdit(id) else {
                let alert = UIAlertController(title: "Can't edit right now", message: "Another device is editing this message, or it was just sent.", preferredStyle: .alert)
                alert.addAction(UIAlertAction(title: "OK", style: .default))
                present(alert, animated: true)
                return
            }
            // Left the session while waiting: give the row straight back.
            guard view.window != nil else {
                await source.finishEdit(text: nil)
                return
            }
            if !stashed {
                stashedDraft = composer.text
                stashedImages = composer.images
                stashed = true
            }
            editingQueueId = id
            composer.images = []
            composer.text = text
            composer.placeholder = "Edit queued message"
            composer.prediction = nil
            pill.banner = .editing
            pill.superview?.isHidden = false
            composer.becomeFirstResponder()
        }
    }

    private func restoreStash() {
        guard stashed else { return }
        composer.text = stashedDraft
        composer.images = stashedImages
        stashedImages = []
        stashed = false
    }

    private func endEdit(commit text: String?) {
        editingQueueId = nil
        restoreStash()
        composer.placeholder = shown.placeholder
        Task { await source.finishEdit(text: text) }
        render(animated: true)
    }

    private func applyFrame() {
        let frame = engine.frame()
        list.apply(frame)
        if !reportedOpen, frame.rowCount() > 0 {
            reportedOpen = true
            showTranscriptIfReady()
            // Open latency: push → first measured frame on screen.
            let ms = (CACurrentMediaTime() - openedAt) * 1000
            let json = String(format: "{\"openMs\":%.1f,\"rows\":%d,\"layoutPassMs\":%.2f}", ms, frame.rowCount(), Double(frame.buildMicros()) / 1000)
            NSLog("OPEN %@", json)
            if ProcessInfo.processInfo.arguments.contains("-measureopen") {
                let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
                try? json.write(to: docs.appendingPathComponent("open.json"), atomically: true, encoding: .utf8)
            }
        }
    }

    private func setJumpVisible(_ visible: Bool) {
        guard (jump.alpha > 0.5) != visible else { return }
        UIView.animate(withDuration: 0.35, delay: 0, usingSpringWithDamping: 0.8, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState]) {
            self.jump.alpha = visible ? 1 : 0
            self.jump.transform = visible ? .identity : CGAffineTransform(scaleX: 0.6, y: 0.6)
        }
    }

    private var cloudToken: AnyObject?

    private var isCloud: Bool { app.row(chatId)?.isCloud == true }

    /// Out of credits, the chat's Cloud machine can't start or wake.
    private var cloudOutOfCredits: Bool { isCloud && app.cloud.outOfCredits }

    /// A Cloud chat whose send can't land says why instead of a bare "Not
    /// delivered": out of credits, the credits notice already does (the
    /// pill steps aside); a failed machine, the pill names it.
    private func cloudBanner(_ banner: SessionChrome.Banner) -> SessionChrome.Banner {
        guard case .notDelivered = banner, isCloud else { return banner }
        let cloud = app.cloud
        cloud.refreshSessions()
        if cloud.outOfCredits {
            return .none
        }
        if let error = cloud.sessionError(chat: chatId, device: app.row(chatId)?.deviceId) {
            return .failed("Cloud machine: \(error) · Tap to retry")
        }
        return banner
    }

    private func render(animated: Bool) {
        var c = source.chrome
        if c.banner == .none, let notice = followUpNotice(queueing: composer.isQueueing, steer: composer.preferredDelivery == .steer, canSteer: c.canSteer) {
            c.banner = .notice(notice)
        }
        let old = shown
        shown = c
        if let error = c.error, error != old.error {
            Toast.show(error, in: view.window)
            DispatchQueue.main.async { self.source.clearQueueError() }
        }
        titleView.set(title: c.title, subtitle: c.subtitle)
        list.uploadProgress = c.uploadProgress
        composer.running = c.running
        composer.resumable = c.resumable
        still.working = c.running
        composer.canSteer = c.canSteer
        let labels = missionComposerLabels(fromChat: true)
        let missionAvailable = app.row(chatId)?.sidechatOf == nil
        if !missionAvailable { mission = false }
        var chips: [ComposerChip] = []
        if missionAvailable {
            chips.append(ComposerChip(id: "mission", title: labels.start, symbol: "target", tint: mission ? Palette.accent : nil))
        }
        if mission {
            chips.append(ComposerChip(id: "mission-plan", title: missionPlanDoc ? labels.planDoc : labels.stepsOnly, symbol: missionReview ? "checklist" : "doc.text"))
        }
        chips += c.chips
        composer.placeholder = mission ? labels.placeholder : c.placeholder
        composer.prediction = editingQueueId == nil && !mission ? c.prediction : nil
        composer.chips = chips
        composer.comments = c.comments
        var menus = Dictionary(uniqueKeysWithValues: c.chips.map { chip in
            (chip.id, { [weak self] in self?.source.chipMenu(chip.id) })
        })
        menus["mission-plan"] = { [weak self] in self?.missionPlanMenu() }
        composer.chipMenus = menus
        let changes = {
            let asking = c.questions != nil
            self.questions.isHidden = !asking
            self.composer.isHidden = asking
            if let q = c.questions { self.questions.configure(q.items) }
            self.queue.isHidden = c.queue.isEmpty || asking
            self.queue.configure(c.queue)
            let editing = self.editingQueueId != nil
            let banner = editing ? .editing : self.cloudBanner(c.banner)
            self.pill.superview?.isHidden = banner == .none
            self.pill.banner = banner
            self.creditsNotice.isHidden = !self.cloudOutOfCredits
            self.bottom.layoutIfNeeded()
            self.view.layoutIfNeeded()
        }
        let structural = (old.questions == nil) != (c.questions == nil) || old.queue.count != c.queue.count || (old.banner == .none) != (c.banner == .none)
        // Panels materialize when their content appears; a first render (the
        // session reopened with a queue or an open question) shows them at
        // once — they used to stay dematerialized, so the queue "vanished"
        // after navigating away and back.
        if (old.questions == nil) != (c.questions == nil) || !animated {
            questions.setGlassVisible(c.questions != nil, animated: animated)
        }
        if old.queue.isEmpty != c.queue.isEmpty || !animated {
            queue.setGlassVisible(!c.queue.isEmpty, animated: animated)
        }
        if animated, structural {
            UIView.animate(withDuration: 0.38, delay: 0, usingSpringWithDamping: 0.86, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState], animations: changes)
        } else {
            changes()
        }
        syncAgents(c.agents, animated: animated)
    }

    private func syncAgents(_ agents: SessionChrome.Agents?, animated: Bool) {
        if let agents { agentsPill.configure(agents) } else { closeAgentsCard(animated: animated) }
        let show = agents != nil
        guard agentsShown != show else { return }
        agentsShown = show
        let motion = animated && view.window != nil
        if show {
            agentsRow.isHidden = false
            guard motion else {
                agentsPill.alpha = 1
                agentsPill.transform = .identity
                return
            }
            agentsPill.alpha = 0
            agentsPill.transform = UIAccessibility.isReduceMotionEnabled ? .identity : CGAffineTransform(translationX: 0, y: 4)
            UIView.animate(withDuration: 0.15, delay: 0, options: [.curveEaseOut, .allowUserInteraction, .beginFromCurrentState]) {
                self.agentsPill.alpha = 1
                self.agentsPill.transform = .identity
            }
        } else {
            guard motion else { return agentsRow.isHidden = true }
            UIView.animate(withDuration: 0.1, delay: 0, options: [.curveEaseIn, .beginFromCurrentState], animations: {
                self.agentsPill.alpha = 0
            }, completion: { _ in
                if !self.agentsShown { self.agentsRow.isHidden = true }
            })
        }
    }

    private func toggleAgentsCard() {
        if agentsCard != nil { return closeAgentsCard(animated: true) }
        guard let card = source.agentsCard(open: { [weak self] id in self?.router?.openSession(id) }) else { return }
        card.onClose = { [weak self] in self?.closeAgentsCard(animated: true) }
        agentsCard = card
        agentsPill.isSelected = true
        agentsScrim.frame = view.bounds
        view.addSubview(agentsScrim)
        card.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(card)
        NSLayoutConstraint.activate([
            card.bottomAnchor.constraint(equalTo: agentsRow.topAnchor, constant: -8),
            card.leadingAnchor.constraint(equalTo: bottom.leadingAnchor),
            card.trailingAnchor.constraint(equalTo: bottom.trailingAnchor),
            card.topAnchor.constraint(greaterThanOrEqualTo: view.safeAreaLayoutGuide.topAnchor, constant: 8),
        ])
        view.layoutIfNeeded()
        UIAccessibility.post(notification: .layoutChanged, argument: card)
        card.alpha = 0
        UIView.animate(withDuration: 0.16, delay: 0, options: [.curveEaseOut, .allowUserInteraction]) { card.alpha = 1 }
        guard !UIAccessibility.isReduceMotionEnabled else { return }
        // Desktop SIDECAR_POPOVER_SLIDE: 220ms cubic-bezier(.32,.72,0,1).
        card.transform = CGAffineTransform(translationX: 0, y: 10)
        UIViewPropertyAnimator(duration: 0.22, controlPoint1: CGPoint(x: 0.32, y: 0.72), controlPoint2: CGPoint(x: 0, y: 1)) {
            card.transform = .identity
        }.startAnimation()
    }

    private func closeAgentsCard(animated: Bool) {
        guard let card = agentsCard else { return }
        agentsCard = nil
        agentsPill.isSelected = false
        agentsScrim.removeFromSuperview()
        card.isUserInteractionEnabled = false
        guard animated, view.window != nil else { return card.removeFromSuperview() }
        UIView.animate(withDuration: 0.1, delay: 0, options: [.curveEaseIn, .beginFromCurrentState], animations: {
            card.alpha = 0
            if !UIAccessibility.isReduceMotionEnabled { card.transform = CGAffineTransform(translationX: 0, y: 4) }
        }, completion: { _ in card.removeFromSuperview() })
    }

    private func syncNeedsYou(animated: Bool) {
        let waiting = NeedsYou.waiting(app, excluding: chatId)
        let show = !waiting.isEmpty
        if show { needsYou.configure(waiting, app: app) }
        guard needsYouRow.isHidden == show else { return }
        let apply = {
            self.needsYouRow.isHidden = !show
            self.needsYou.alpha = show ? 1 : 0
            self.needsYou.transform = show ? .identity : CGAffineTransform(translationX: 0, y: 10).scaledBy(x: 0.92, y: 0.92)
            self.bottom.layoutIfNeeded()
            self.view.layoutIfNeeded()
        }
        guard animated, view.window != nil, !UIAccessibility.isReduceMotionEnabled else { return apply() }
        if show {
            needsYou.alpha = 0
            needsYou.transform = CGAffineTransform(translationX: 0, y: 10).scaledBy(x: 0.92, y: 0.92)
        }
        UIView.animate(withDuration: 0.42, delay: 0, usingSpringWithDamping: 0.72, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState], animations: apply)
    }

    private func openNeedsYou() {
        let editing = composer.textView.isFirstResponder
        composer.resignFirstResponder()
        NeedsYou.present(app: app, from: self, excluding: chatId) { [weak self] _ in
            guard let self, editing, self.view.window != nil, !self.composer.isHidden else { return }
            self.composer.becomeFirstResponder()
        }
    }

    private func compactAction() -> UIAction {
        let blocked = source.compactBlocked
        let action = UIAction(title: "Compact Context", subtitle: blocked, image: UIImage(systemName: "arrow.down.right.and.arrow.up.left")) { [weak self] _ in
            self?.source.compact(focus: nil)
        }
        if blocked != nil { action.attributes = .disabled }
        return action
    }

    private func offerCompaction(title: String, body: String) {
        let sheet = UIAlertController(title: title, message: body, preferredStyle: .actionSheet)
        sheet.addAction(UIAlertAction(title: "Compact", style: .default) { [weak self] _ in self?.source.compact(focus: nil) })
        sheet.addAction(UIAlertAction(title: "Keep full history", style: .cancel) { [weak self] _ in self?.source.keepFullHistory() })
        sheet.popoverPresentationController?.sourceView = pill
        present(sheet, animated: true)
    }

    private func sidechatAction() -> UIAction? {
        guard app.row(chatId)?.sidechatOf == nil else { return nil }
        return UIAction(title: newSidechatLabel(), image: UIImage(systemName: "bubble.left.and.bubble.right")) { [weak self] _ in
            self?.presentSidechats()
        }
    }

    @discardableResult
    private func presentSidechats() -> SidechatSheetViewController? {
        guard let client = app.client else { return nil }
        let sheet = SidechatSheetViewController(app: app, client: client, sourceId: chatId, sourceTitle: app.row(chatId)?.title ?? "")
        present(sheet.wrapped, animated: true)
        return sheet
    }

    private func syncMission() {
        guard let strip = app.client?.missionStrip(chatId: chatId) else {
            missionStrip.isHidden = true
            view.setNeedsLayout()
            return
        }
        var title = AttributedString("\(strip.role) · \(strip.title) · \(strip.done)/\(strip.total)")
        title.font = Fonts.ui(.sansMedium, 13)
        missionStrip.configuration?.attributedTitle = title
        missionStrip.isHidden = false
        view.setNeedsLayout()
    }

    private func missionPlanMenu() -> UIMenu {
        let labels = missionComposerLabels(fromChat: true)
        return UIMenu(title: labels.start, children: [
            UIMenu(options: .displayInline, children: [(true, labels.planDoc), (false, labels.stepsOnly)].map { planDoc, title in
                UIAction(title: title, state: planDoc == missionPlanDoc ? .on : .off) { [weak self] _ in
                    self?.missionPlanDoc = planDoc
                    self?.render(animated: true)
                }
            }),
            UIAction(title: labels.reviewPlanFirst, state: missionReview ? .on : .off) { [weak self] _ in
                self?.missionReview.toggle()
                self?.render(animated: true)
            },
        ])
    }

    private func startMission(text: String, images: [StagedImage]) {
        let pictures = images.filter(\.isImage)
        if let error = missionStagedError(images: UInt32(pictures.count), otherFile: nil) {
            return Toast.show(error, in: view.window, above: bottom)
        }
        guard let client = app.client, let host = app.row(chatId)?.deviceId else { return }
        let brief = missionText(text: text, pasted: images.filter { !$0.isImage }.map { String(decoding: $0.data, as: UTF8.self) })
        let options = MissionOptions(approvePlan: missionReview, sameAgents: false, planDoc: missionPlanDoc)
        let chat = chatId
        Task { @MainActor [weak self] in
            do {
                let paths = try await MissionStart.upload(pictures, to: host, client: client)
                let started = try await client.startMissionFromChat(chatId: chat, draft: MissionStart.draft(text: brief, spaceId: nil, images: paths), options: options, lead: ChatMission.lead)
                guard let self else { return }
                if let notice = started.notice { Toast.show(notice, in: self.view.window, above: self.bottom) }
                self.composer.text = ""
                self.composer.images = []
                self.mission = false
                self.render(animated: true)
                (self.tabBarController as? MainTabController)?.showTasks("mission:\(started.taskId)")
            } catch {
                Toast.show(TasksErrors.message(error), in: self?.view.window, above: self?.bottom)
            }
        }
    }

    private func openMission() {
        guard let strip = app.client?.missionStrip(chatId: chatId) else { return }
        navigationController?.pushViewController(MissionControlViewController(app: app, taskId: strip.taskId), animated: true)
    }

    private func missionItems() -> [UIMenuElement] {
        guard let client = app.client else { return [] }
        var items: [UIMenuElement] = []
        let strip = client.missionStrip(chatId: chatId)
        if let strip {
            items.append(UIAction(title: missionLabels().openMissionPage, image: TaskGlyph.asset("glyph-mission")) { [weak self] _ in self?.openMission() })
            items.append(UIAction(title: "Brief", image: UIImage(systemName: "doc.plaintext")) { [weak self] _ in
                guard let self else { return }
                self.navigationController?.pushViewController(BriefViewController(app: self.app, taskId: strip.taskId, stepId: strip.stepId), animated: true)
            })
        }
        if let watch = client.watchState(chatId: chatId), !(watch.mission && strip?.stepId != nil) {
            let chat = chatId
            items.append(UIAction(title: watch.everything ? "Watched (everything is on)" : "Watched by the Supervisor", image: UIImage(systemName: "eye"), attributes: watch.everything ? .disabled : [], state: watch.watched ? .on : .off) { [weak self] _ in
                Task { @MainActor in
                    do { try await client.setWatched(chatId: chat, watched: !watch.watched) } catch {
                        Toast.show(TasksErrors.message(error), in: self?.view.window)
                    }
                }
            })
        }
        return items.isEmpty ? [] : [UIMenu(options: .displayInline, children: items)]
    }

    private func sessionMenu() -> UIMenu {
        UIMenu(children: [UIDeferredMenuElement.uncached { [weak self] done in
            guard let self else { return done([]) }
            let vm = self.app.session(self.chatId)
            let pinned = vm?.pinned ?? false
            let stop = self.still.receiving
                ? [UIAction(title: "Stop Computer Use", image: UIImage(systemName: "stop.circle"), attributes: .destructive) { _ in self.confirmComputerStop() }]
                : []
            let items: [UIMenuElement?] = [
                UIAction(title: pinned ? "Unpin" : "Pin", image: UIImage(systemName: pinned ? "pin.slash" : "pin")) { _ in self.app.setPinned(self.chatId, !pinned) },
                UIAction(title: "Session Info", image: UIImage(systemName: "info.circle")) { _ in self.openInfo() },
                UIAction(title: "Terminal", image: UIImage(systemName: "apple.terminal")) { _ in self.openTerminal(nil) },
                UIAction(title: "Project Actions", image: UIImage(systemName: "play.rectangle")) { _ in self.openProjectActions() },
                UIAction(title: "Copy Transcript", image: UIImage(systemName: "doc.on.doc")) { _ in
                    UIPasteboard.general.string = self.engine.frame().plainText()
                },
                self.source.agentsAction { [weak self] id in self?.router?.openSession(id) },
                self.source.reviewAction(),
                self.sidechatAction(),
                self.source.handoffMenu { [weak self] id in self?.router?.openSession(id) },
                self.compactAction(),
                UIAction(title: "Archive", image: UIImage(systemName: "archivebox")) { _ in
                    self.app.archive(self.chatId)
                    self.leave()
                },
                UIAction(title: "Delete", image: UIImage(systemName: "trash"), attributes: .destructive) { _ in
                    self.confirmDelete(app: self.app, self.chatId) { self.leave() }
                },
            ]
            done(self.missionItems() + stop + items.compactMap { $0 })
        }])
    }

    /// Off this session once it's gone from the list. Beside the iPad
    /// sidebar there's nothing to pop back to.
    private func leave() {
        if let split = splitViewController as? SplitRootController, !split.isCollapsed {
            split.showDraft(prompt: nil, focus: false)
        } else {
            navigationController?.popViewController(animated: true)
        }
    }
}

extension UIViewController {
    /// "Delete session?" — permanent, like the desktop's dialog; a Cloud
    /// session's machine goes with it. `then` runs after the delete.
    func confirmDelete(app: AppModel?, _ id: String, then: @escaping () -> Void) {
        guard let app else { return }
        let row = app.row(id)
        let title = row.map { $0.hasTitle ? $0.title : "New session" } ?? "This session"
        let message = row?.isCloud == true
            ? "\u{201C}\(title)\u{201D} and its Cloud machine will be permanently deleted, with anything not pushed from it. This can\u{2019}t be undone."
            : "\u{201C}\(title)\u{201D} will be permanently deleted. This can\u{2019}t be undone."
        let alert = UIAlertController(title: "Delete session?", message: message, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        alert.addAction(UIAlertAction(title: "Delete", style: .destructive) { _ in
            app.delete(id)
            then()
        })
        present(alert, animated: true)
    }

    /// Pins a composer 8pt above the keyboard; at rest it sits on the safe
    /// area, `composerRestGap` up. Returns the rest constraint.
    func pinAboveKeyboard(_ bar: UIView) -> NSLayoutConstraint {
        view.keyboardLayoutGuide.usesBottomSafeArea = false
        let rest = bar.bottomAnchor.constraint(lessThanOrEqualTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -composerRestGap)
        let down = bar.bottomAnchor.constraint(equalTo: view.bottomAnchor)
        down.priority = .defaultLow
        NSLayoutConstraint.activate([
            bar.bottomAnchor.constraint(lessThanOrEqualTo: view.keyboardLayoutGuide.topAnchor, constant: -8),
            rest,
            down,
        ])
        return rest
    }

    /// Beside the sidebar the composer rests on the safe area, level with
    /// the sidebar's bottom toolbar; on its own (tab shell, sheet) 8pt up.
    var composerRestGap: CGFloat {
        guard mainShell == nil, let split = splitViewController, !split.isCollapsed else { return 8 }
        return 0
    }
}

/// Two-line navigation title: session title over "project @ device".
final class SessionTitleView: UIView {
    private let title = FadingLabel()
    private let subtitle = FadingLabel()

    override init(frame: CGRect) {
        super.init(frame: frame)
        title.font = Fonts.ui(.sansSemibold, 16)
        title.textColor = Palette.text
        title.fitsAlignment = .center
        subtitle.font = Fonts.ui(.sans, 12)
        subtitle.textColor = Palette.secondary
        subtitle.fitsAlignment = .center
        let stack = UIStackView(arrangedSubviews: [title, subtitle])
        stack.axis = .vertical
        stack.alignment = .center
        stack.spacing = 1
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            widthAnchor.constraint(lessThanOrEqualToConstant: 240),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func set(title t: String, subtitle s: String) {
        title.text = t
        subtitle.text = s
        subtitle.isHidden = s.isEmpty
    }
}

/// "Out of Cloud credits" with its one way forward, above a Cloud chat's
/// composer while the balance is spent. A send nudges it.
final class CloudCreditsNotice: UIView {
    var onAddCredits: (() -> Void)?
    private let glass = Glass.surface()

    override init(frame: CGRect) {
        super.init(frame: frame)
        glass.translatesAutoresizingMaskIntoConstraints = false
        addSubview(glass)
        let icon = UIImageView(image: CloudUI.symbol("exclamationmark.triangle.fill", 13, weight: .semibold))
        icon.tintColor = Palette.danger
        icon.contentMode = .center
        let label = UILabel()
        label.text = "Out of Cloud credits"
        label.font = Fonts.ui(.sansMedium, 14)
        label.textColor = Palette.text
        label.setContentHuggingPriority(UILayoutPriority(1), for: .horizontal)
        var c = UIButton.Configuration.filled()
        c.title = "Add credits"
        c.baseBackgroundColor = Palette.text
        c.baseForegroundColor = Palette.background
        c.cornerStyle = .capsule
        c.contentInsets = NSDirectionalEdgeInsets(top: 6, leading: 12, bottom: 6, trailing: 12)
        c.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { a in
            var a = a
            a.font = Fonts.ui(.sansSemibold, 13)
            return a
        }
        let button = UIButton(configuration: c, primaryAction: UIAction { [weak self] _ in self?.onAddCredits?() })
        button.setContentHuggingPriority(.required, for: .horizontal)
        button.setContentCompressionResistancePriority(.required, for: .horizontal)
        button.accessibilityIdentifier = "add-credits"
        let row = UIStackView(arrangedSubviews: [icon, label, button])
        row.alignment = .center
        row.spacing = 8
        row.translatesAutoresizingMaskIntoConstraints = false
        glass.contentView.addSubview(row)
        NSLayoutConstraint.activate([
            glass.topAnchor.constraint(equalTo: topAnchor),
            glass.bottomAnchor.constraint(equalTo: bottomAnchor),
            glass.leadingAnchor.constraint(equalTo: leadingAnchor),
            glass.trailingAnchor.constraint(equalTo: trailingAnchor),
            row.topAnchor.constraint(equalTo: glass.contentView.topAnchor, constant: 6),
            row.bottomAnchor.constraint(equalTo: glass.contentView.bottomAnchor, constant: -6),
            row.leadingAnchor.constraint(equalTo: glass.contentView.leadingAnchor, constant: 14),
            row.trailingAnchor.constraint(equalTo: glass.contentView.trailingAnchor, constant: -6),
        ])
        accessibilityIdentifier = "cloud-credits-notice"
    }

    required init?(coder: NSCoder) { fatalError() }

    /// A send was refused: a short shake and a warning tap point at it.
    func nudge() {
        UINotificationFeedbackGenerator().notificationOccurred(.warning)
        guard !UIAccessibility.isReduceMotionEnabled else { return }
        let shake = CAKeyframeAnimation(keyPath: "transform.translation.x")
        shake.values = [0, -8, 8, -5, 5, -2, 0]
        shake.duration = 0.4
        layer.add(shake, forKey: "nudge")
    }
}

/// Small glass capsule above the composer: working/elapsed, offline, retry.
final class StatusPill: UIControl {
    var onTap: (() -> Void)?
    var banner: SessionChrome.Banner = .none { didSet { if banner != oldValue { update() } } }
    private let glass = Glass.surface()
    private let grid = StatusGlyph(.trailer)
    private let label = UILabel()
    private var timer: Timer?

    override init(frame: CGRect) {
        super.init(frame: frame)
        glass.isUserInteractionEnabled = false
        glass.translatesAutoresizingMaskIntoConstraints = false
        addSubview(glass)
        label.font = Fonts.ui(.sansMedium, 13)
        label.textColor = Palette.secondary
        label.numberOfLines = 2
        for v in [grid, label] {
            v.translatesAutoresizingMaskIntoConstraints = false
            glass.contentView.addSubview(v)
        }
        NSLayoutConstraint.activate([
            glass.topAnchor.constraint(equalTo: topAnchor),
            glass.bottomAnchor.constraint(equalTo: bottomAnchor),
            glass.leadingAnchor.constraint(equalTo: leadingAnchor),
            glass.trailingAnchor.constraint(equalTo: trailingAnchor),
            heightAnchor.constraint(greaterThanOrEqualToConstant: 30),
            grid.leadingAnchor.constraint(equalTo: glass.contentView.leadingAnchor, constant: 11),
            grid.centerYAnchor.constraint(equalTo: glass.contentView.centerYAnchor),
            grid.widthAnchor.constraint(equalToConstant: 12),
            grid.heightAnchor.constraint(equalToConstant: 12),
            label.leadingAnchor.constraint(equalTo: grid.trailingAnchor, constant: 8),
            label.trailingAnchor.constraint(equalTo: glass.contentView.trailingAnchor, constant: -12),
            label.centerYAnchor.constraint(equalTo: glass.contentView.centerYAnchor),
            label.topAnchor.constraint(greaterThanOrEqualTo: glass.contentView.topAnchor, constant: 6),
        ])
        let compact = heightAnchor.constraint(equalToConstant: 30)
        compact.priority = .defaultLow
        compact.isActive = true
        addAction(UIAction { [weak self] _ in self?.onTap?() }, for: .touchUpInside)
    }

    required init?(coder: NSCoder) { fatalError() }

    private func update() {
        timer?.invalidate()
        timer = nil
        switch banner {
        case .none:
            break
        case let .working(since, word):
            grid.kind = .trailer
            let render = { [weak self] in
                let secs = since.map { Int(Date().timeIntervalSince($0)) } ?? 0
                self?.label.text = secs > 0 ? "\(word) · \(Self.elapsed(secs))" : "\(word)…"
            }
            render()
            timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { _ in render() }
        case .sending:
            grid.kind = .trailer
            label.text = "Sending to host…"
        case .offline:
            grid.kind = .dot(StatusTone.idle)
            label.text = "Offline — sends are saved"
        case let .reconnecting(secs):
            grid.kind = .dot(StatusTone.idle)
            label.text = "Reconnecting in \(secs)s"
        case .notDelivered:
            grid.kind = .dot(Palette.danger)
            label.text = "Not delivered · Tap to retry"
        case let .failed(message):
            grid.kind = .dot(Palette.danger)
            label.text = message
        case .editing:
            grid.kind = .dot(StatusTone.input)
            label.text = "Editing queued message · Tap to cancel"
        case let .limit(text):
            grid.kind = .dot(Palette.warning)
            label.text = text
        case let .notice(message):
            grid.kind = .dot(StatusTone.idle)
            label.text = message
        case let .offer(title, body):
            grid.kind = .dot(StatusTone.input)
            label.text = "\(title) · \(body)"
        }
    }

    static func elapsed(_ s: Int) -> String {
        s < 60 ? "\(s)s" : s < 3600 ? "\(s / 60)m \(s % 60)s" : "\(s / 3600)h \(s / 60 % 60)m"
    }

    override func willMove(toWindow newWindow: UIWindow?) {
        super.willMove(toWindow: newWindow)
        if newWindow == nil { timer?.invalidate() } else { update() }
    }
}

/// Unsent composer text per session, kept across navigation and launches.
enum Drafts {
    private static let key = "drafts"

    /// The demo (and the UI tests on it) keep drafts in memory only: one
    /// run's leftovers must not seed the next.
    static let persists = !ProcessInfo.processInfo.arguments.contains("-demo")
    private static var volatile: [String: String] = [:]

    private static var all: [String: String] {
        get { persists ? (UserDefaults.standard.dictionary(forKey: key) as? [String: String]) ?? [:] : volatile }
        set { if persists { UserDefaults.standard.set(newValue, forKey: key) } else { volatile = newValue } }
    }

    static func load(_ chatId: String) -> String {
        all[chatId] ?? ""
    }

    static func save(_ chatId: String, _ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        all[chatId] = trimmed.isEmpty ? nil : text
    }
}
