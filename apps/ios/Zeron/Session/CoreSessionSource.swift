import UIKit

/// A live session backed by the Rust core. The transcript flows Rust→Rust
/// (`TranscriptView.attach`); this maps the composer-facing state.
final class CoreSessionSource: SessionSource {
    private(set) var chrome = SessionChrome()
    var onChange: (() -> Void)?
    var onPresent: ((UIViewController) -> Void)?
    private weak var app: AppModel?
    private let client: CoreClient
    private let handle: SessionHandle
    private let chatId: String
    private var token: AnyObject?
    private var appToken: AnyObject?
    private var hostDevice = "" {
        didSet { if hostDevice != oldValue { connectVisuals() } }
    }
    /// What a chat with no model of its own runs: its host's default (the
    /// catalog's first model), by "device|harness". Seeded from the built-in
    /// list so the chip never shows a bare provider name, then the host's.
    private var defaultModels: [String: ModelPickerRow] = [:]
    private var defaultModelsLoading: Set<String> = []
    private weak var engine: TranscriptView?
    private var models: [ModelInfo] = []
    private var modelsKey = ""
    private var profiles: [ProfileRow] = []
    private var profilesFor: ChatConfig?
    private var invocationCatalogs: [String: InvocationCatalog] = [:]
    private weak var contextSheet: ContextSheetViewController?
    private let reviewSheets = NSHashTable<AnyObject>.weakObjects()

    init(app: AppModel, client: CoreClient, handle: SessionHandle, chatId: String) {
        self.app = app
        self.client = client
        self.handle = handle
        self.chatId = chatId
        token = app.observeSession(chatId) { [weak self] in self?.refresh() }
        appToken = app.observe { [weak self] in self?.refresh() }
        refresh()
    }

    func attach(_ engine: TranscriptView) {
        _ = engine.attach(client: client, chatId: chatId)
        self.engine = engine
        connectVisuals()
        handle.setViewAttached(attached: true)
    }

    /// Agent visuals load their scripts from the chat's host device.
    private func connectVisuals() {
        guard let engine, !hostDevice.isEmpty else { return }
        engine.setVisualSource(client: client, deviceId: hostDevice, pixelScale: Float(max(2, UITraitCollection.current.displayScale)))
    }

    func detach() {
        handle.setViewAttached(attached: false)
    }

    var isUnused: Bool {
        let t = handle.transcriptStatus()
        return t.entryCount == 0 && t.pendingCount == 0 && handle.composer().queue.isEmpty
    }

    func reattach() {
        handle.setViewAttached(attached: true)
    }

    private func refresh() {
        let c = handle.composer()
        let row = app?.row(chatId)
        hostDevice = c.host.deviceId
        var next = SessionChrome()
        next.title = c.title
        let project = row?.project?.name ?? "No project"
        // A Cloud chat's machine is "Cloud", not its own device name.
        let hostName = row?.isCloud == true ? row?.deviceName : c.host.name
        next.hostName = hostName
        next.subtitle = hostName.map { "\(project) @ \($0)" } ?? project
        next.running = c.live.turnRunning
        next.resumable = c.live.resumable
        next.canSteer = c.host.capabilities.midTurnSteering ?? false
        next.placeholder = "Message \(row?.harnessLabel ?? "the agent")"
        var chips: [ComposerChip] = []
        let config = client.sessionConfig(chatId: chatId)
        loadProfiles(config)
        if let model = row?.modelLabel ?? row.flatMap({ defaultModel(for: $0)?.label }) ?? row?.harnessLabel {
            let title = profiles.first(where: \.active).map { "\($0.name)\($0.drift == nil ? "" : " •") · \(model)" } ?? model
            chips.append(ComposerChip(id: "model", title: title, symbol: nil, icon: BrandMarks.image(for: row?.harness ?? "claude-code", side: 13)))
        }
        loadModels(harness: row?.harness ?? "claude-code")
        let model = currentModel
        if let r = row?.reasoning, !r.isEmpty {
            let fast = model.flatMap { m in config.map { fastModeOn(model: m, config: $0) } } ?? false
            chips.append(ComposerChip(id: "effort", title: reasoningLabel(level: r), symbol: fast ? "bolt.fill" : "gauge.with.dots.needle.67percent"))
        }
        let mode = config?.permissionMode ?? .full
        if let info = permissionModes().first(where: { $0.mode == mode }) {
            chips.append(ComposerChip(id: "permission", title: info.shortLabel, symbol: Self.permissionSymbol(mode), tint: mode == .full ? Palette.warning : nil))
        }
        if let pr = row?.pullRequest {
            let state: SessionRowVM.PR = switch pr.state { case .open: .open; case .merged: .merged; case .closed: .closed }
            chips.append(ComposerChip(id: "pr", title: "\(pr.number)", symbol: nil, tint: PRBadgeView.tone(state), icon: PRIcon.image(side: 12)))
        } else if let b = row?.branch, !b.isEmpty {
            chips.append(ComposerChip(id: "branch", title: b, symbol: nil, icon: BranchIcon.sized()))
        }
        if let usage = c.contextUsage, let tokens = usage.tokens, let window = usage.window, window > 0 {
            let fraction = Double(tokens) / Double(window)
            chips.append(ComposerChip(
                id: "context",
                title: "\(Int((fraction * 100).rounded()))% context",
                symbol: fraction >= 0.85 ? "exclamationmark.circle" : "circle.lefthalf.filled",
                tint: fraction >= 0.85 ? Palette.warning : nil
            ))
        } else if contextContent() != .init() {
            chips.append(ComposerChip(id: "context", title: "Context", symbol: "circle.lefthalf.filled"))
        }
        if let review = c.review {
            chips.append(ReviewUI.chip(review))
        }
        next.chips = chips
        if !c.stagedComments.isEmpty {
            let labels = handle.stagedCommentsLabels()
            next.comments = .init(title: labels.title, clear: labels.clear, items: c.stagedComments)
        }
        if let sendFailure {
            next.banner = .failed(sendFailure)
        } else if let compactNotice {
            next.banner = .notice(compactNotice)
        } else if c.compacting {
            next.banner = .working(since: nil, word: "Compacting context")
        } else if c.sendState == .failed {
            next.banner = .notDelivered
        } else if c.sendState == .queued {
            // A Cloud chat's message waits for its machine to start or wake —
            // the transcript's setup step says so, as on the desktop.
            if row?.isCloud != true {
                next.banner = .failed("\(c.host.name ?? "Host") is offline — will send when it's back")
            }
        } else if c.sendState == .sending {
            next.banner = .sending
        } else if app?.connectivity?.state == .offline {
            next.banner = .offline
        } else if !c.room.connected, let retry = c.room.retryAtMs {
            next.banner = .reconnecting(in: max(1, Int((retry - Int64(Date().timeIntervalSince1970 * 1000)) / 1000)))
        } else if let pill = c.limitPill {
            next.banner = .limit(pill.text)
        }
        next.limitPill = c.limitPill
        if next.banner == .none, let offer = c.resumeCompaction {
            next.banner = .offer(title: offer.title, body: offer.body)
        }
        next.uploadProgress = c.transferProgress
        next.prediction = c.prediction
        // Working state is shown at the transcript tail (layout engine), not here.
        if let input = c.openInput {
            next.questions = (input.requestId, input.questions.map {
                SessionChrome.Question(id: $0.id, header: $0.header, text: $0.question, options: $0.options, multiSelect: $0.multiSelect, prefill: $0.prefill, multiline: $0.multiline, secret: $0.secret)
            })
        }
        next.queue = c.queue.map { q in
            let gate: String? = switch q.gate {
            case let .editing(_, _, mine)?: mine ? "Editing" : "Being edited"
            case .reviewRequired?: "Needs review"
            case nil: q.actionPending ? "Updating" : nil
            }
            return SessionChrome.QueuedItem(id: q.id, text: q.visibleText, thumbnail: nil, gate: gate, from: q.fromLabel, action: q.primaryAction, image: q.image)
        }
        next.error = c.queueError
        let roster = handle.backgroundRoster(nowMs: nowMs())
        next.agents = roster.summary.map { SessionChrome.Agents(summary: $0, state: roster.state) }
        if next != chrome {
            chrome = next
            onChange?()
        }
        contextSheet?.reload()
        for case let sheet as ReviewReloading in reviewSheets.allObjects { sheet.reload() }
    }

    private func loadModels(harness: String) {
        let key = "\(hostDevice)|\(harness)"
        guard key != modelsKey, !hostDevice.isEmpty else { return }
        modelsKey = key
        Task { @MainActor [weak self] in
            guard let self else { return }
            let models = await self.client.listModels(deviceId: self.hostDevice, harness: harness)
            guard self.modelsKey == key else { return }
            self.models = models
            self.refresh()
        }
    }

    private func loadProfiles(_ config: ChatConfig?) {
        guard config != profilesFor else { return }
        profilesFor = config
        guard config?.profileId != nil || !profiles.isEmpty else { return }
        Task { @MainActor [weak self] in
            guard let self else { return }
            let rows = await self.client.profileRows(chatId: self.chatId)
            guard self.profilesFor == config, rows != self.profiles else { return }
            self.profiles = rows
            self.refresh()
        }
    }

    private var currentModel: ModelInfo? {
        let id = app?.row(chatId)?.model
        return models.first { $0.id == id } ?? models.first
    }

    private func contextContent() -> ContextSheetViewController.Content {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        let stats = handle.statsView(nowMs: now)
        let compact = handle.compactSection(nowMs: now)
        guard let model = currentModel, let config = client.sessionConfig(chatId: chatId) else { return .init(stats: stats, compact: compact) }
        return .init(card: contextCard(model: model, config: config, tokens: handle.composer().contextUsage?.tokens), stats: stats, compact: compact)
    }

    func chipSheet(_ id: String) -> UIViewController? {
        if id == "review" { return reviewChipSheet() }
        guard id == "context" else { return nil }
        let sheet = ContextSheetViewController(
            content: { [weak self] in self?.contextContent() ?? .init() },
            toggleLong: { [weak self] in
                guard let self, let model = self.currentModel else { return }
                self.setConfig { $0 = toggleLongContext(model: model, config: $0) }
            },
            pick: { [weak self] position in
                guard let self, let model = self.currentModel else { return }
                self.setConfig { $0 = pickContext(model: model, config: $0, position: position) }
            }
        )
        contextSheet = sheet
        return sheet.wrapped
    }

    /// False when the core refused the message (it stays in the composer).
    @discardableResult
    func send(text: String, images: [StagedImage], mode: DeliveryMode) -> Bool {
        do {
            if mode == .interrupt, chrome.running { try handle.interrupt() }
            _ = try handle.send(request: SendRequest(text: text, attachments: images.map(\.outgoing), worktree: nil, busy: mode == .steer ? .steer : .queue))
            app?.forgetFreshSideChat(chatId)
            app?.noteDeliveryActivity()
            sendFailure = nil
            compactNotice = nil
            return true
        } catch {
            // Kept across refreshes (which rebuild the chrome) until the next send.
            sendFailure = "Couldn't send: \(error)"
            refresh()
            return false
        }
    }

    private var sendFailure: String?
    private var compactNotice: String?

    var compactBlocked: String? { handle.composer().compactBlocked }

    func compact(focus: String?) {
        compactNotice = nil
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                self.compactNotice = try await self.handle.compact(focus: focus)
            } catch {
                self.compactNotice = "Compaction failed: \(error)"
            }
            self.refresh()
        }
    }

    func keepFullHistory() {
        handle.keepFullHistory()
    }

    func stop() {
        try? handle.interrupt()
    }

    func answer(requestId: String, answers: [(questionId: String, labels: [String])]) {
        try? handle.respondInput(requestId: requestId, answers: answers.map { UserInputAnswer(questionId: $0.questionId, labels: $0.labels) })
    }

    func answerConsent(requestId: String, questionId: String, answer: String) {
        _ = try? handle.answerConsent(requestId: requestId, questionId: questionId, answer: answer)
    }

    func computerStill() async -> (sequence: UInt64, image: UIImage)? {
        guard let still = try? await handle.computerStillFrame(), let image = UIImage(data: still.image) else { return nil }
        return (still.sequence, image)
    }

    func computerStop() async throws -> Bool { try await handle.computerStop() }
    func terminals() async throws -> [AgentTerminalTab] { try await handle.listTerminals() }
    func terminal(_ id: String, dark: Bool) async throws -> AgentTerminalScreen { try await handle.readTerminal(terminalId: id, dark: dark) }
    func projectActions() async throws -> ProjectActions? { try await handle.listProjectActions() }
    func runProjectAction(_ id: String) async throws -> String { try await handle.runProjectAction(actionId: id) }

    func queueAction(_ id: String, _ action: QueueAction) {
        switch action {
        // Steers text into the live turn (never interrupts it); only
        // attachment rows stop the turn to send.
        case .sendNow: Task { _ = try? await handle.deliverQueuedNow(id: id) }
        case .remove: Task { _ = try? await handle.removeQueued(id: id) }
        case .moveUp: _ = try? handle.moveQueuedBy(id: id, delta: -1)
        case .moveDown: _ = try? handle.moveQueuedBy(id: id, delta: 1)
        case .edit: break // Driven by the view: beginEdit / finishEdit.
        }
    }

    func clearQueueError() { handle.clearQueueError() }

    private var lease: QueueEditLease?
    private var renewal: Task<Void, Never>?
    /// The row as it was when the edit began: the composer only edits its
    /// visible text, the rest (Appshot context, attachment trailer) rides along.
    private var editBase: (raw: String, visible: String)?

    func beginEdit(_ id: String) async -> String? {
        let start = await handle.beginQueuedEdit(id: id, instanceId: UUID().uuidString)
        guard case let .acquired(lease) = start else { return nil }
        self.lease = lease
        renewal?.cancel()
        renewal = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(20))
                guard let self, let lease = self.lease, !Task.isCancelled else { return }
                if await !self.handle.renewQueuedEdit(lease: lease) { return }
            }
        }
        guard let item = handle.composer().queue.first(where: { $0.id == id }) else { return nil }
        editBase = (item.text, item.visibleText)
        return item.visibleText
    }

    func finishEdit(text: String?) async {
        renewal?.cancel()
        let base = editBase
        editBase = nil
        guard let lease else { return }
        self.lease = nil
        var body = text
        if let edited = text, let base {
            // Unchanged: release the row as it was (a commit would drop what
            // the composer never showed).
            body = edited == base.visible ? nil : Self.replacingVisible(in: base.raw, visible: base.visible, with: edited)
        }
        _ = await handle.finishQueuedEdit(lease: lease, action: body == nil ? .cancel : .commit, text: body)
    }

    /// The row's raw text with its visible part replaced, keeping the hidden
    /// context after it. Blank edits stay blank (the host removes the row).
    static func replacingVisible(in raw: String, visible: String, with edited: String) -> String {
        if edited.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || raw == visible { return edited }
        let body = raw.drop(while: \.isWhitespace)
        if body.hasPrefix(visible) { return edited + body.dropFirst(visible.count) }
        // Attachment-only rows show a placeholder: all of the raw text is context.
        return body.isEmpty ? edited : edited + "\n\n" + body
    }

    func retryDelivery() {
        try? handle.retryDelivery()
    }

    func chipMenu(_ id: String) -> UIMenu? {
        if id == "review" {
            guard let review = handle.composer().review, review.busy, review.stoppable else { return nil }
            return UIMenu(title: review.tip, children: [
                UIAction(title: review.stopLabel, subtitle: review.stopTip, image: UIImage(systemName: "stop.circle"), attributes: .destructive) { [weak self, handle] _ in
                    Task { @MainActor in
                        do { try await handle.stopReview() } catch { self?.reviewFailed(error) }
                    }
                },
            ])
        }
        guard let row = app?.row(chatId) else { return nil }
        let harness = row.harness ?? "claude-code"
        switch id {
        case "model":
            return UIMenu(children: [UIDeferredMenuElement.uncached { [weak self] done in
                guard let self else { return done([]) }
                Task { @MainActor in done(await self.modelMenu(row: row, harness: harness)) }
            }])
        case "effort":
            return UIMenu(title: "Effort", children: [UIDeferredMenuElement { [weak self] done in
                guard let self else { return done([]) }
                Task { @MainActor in
                    let models = (try? await self.client.listModels(deviceId: self.hostDevice, harness: harness)) ?? fallbackModels(harness: harness)
                    let model = models.first { $0.id == row.model } ?? models.first
                    var items: [UIMenuElement] = (model?.reasoningLevels ?? []).map { l in
                        UIAction(title: reasoningLabel(level: l), state: l == row.reasoning ? .on : .off) { [weak self] _ in
                            self?.setConfig { $0.reasoning = l }
                        }
                    }
                    if let model, let config = self.client.sessionConfig(chatId: self.chatId), let fast = speedToggle(model: model, config: config) {
                        items.append(UIMenu(options: .displayInline, children: [
                            UIAction(title: "Fast", image: UIImage(systemName: fast ? "bolt.fill" : "bolt"), state: fast ? .on : .off) { [weak self] _ in
                                self?.setConfig { $0 = toggleSpeed(model: model, config: $0) }
                            },
                        ]))
                    }
                    done(items)
                }
            }])
        case "permission":
            let current = client.sessionConfig(chatId: chatId)?.permissionMode ?? .full
            return UIMenu(title: "Approve agent actions", children: permissionModes().map { m in
                UIAction(title: m.label, subtitle: m.description, image: UIImage(systemName: Self.permissionSymbol(m.mode)), state: m.mode == current ? .on : .off) { [weak self] _ in
                    self?.setConfig { $0.permissionMode = m.mode }
                }
            })
        case "pr":
            guard let url = row.pullRequest.flatMap({ URL(string: $0.url) }) else { return nil }
            let device = hostDevice
            return UIMenu(title: row.pullRequest?.title ?? "", children: [
                UIAction(title: "Open Pull Request", image: UIImage(systemName: "arrow.triangle.pull")) { _ in
                    let window = UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first
                    guard pullRequestLink(url: url.absoluteString) != nil,
                          let router = window?.rootViewController as? AppRouter
                    else { return UIApplication.shared.open(url) }
                    router.openPullRequest(url: url.absoluteString, deviceId: device)
                },
                UIAction(title: "Copy Link", image: UIImage(systemName: "link")) { _ in UIPasteboard.general.url = url },
            ])
        default:
            return nil
        }
    }

    /// The chat's provider, where it runs: Cloud's account for a Cloud chat,
    /// else the computer hosting it.
    func signInTarget() -> AgentAccountsViewController.Target? {
        guard let row = app?.row(chatId) else { return nil }
        let harness = row.harness ?? "claude-code"
        if row.isCloud {
            return CloudStore.agents.first { $0.harness == harness }.map { .cloud($0) }
        }
        guard !hostDevice.isEmpty, let app else { return nil }
        return .device(id: hostDevice, name: app.deviceName(hostDevice), harness: harness, label: row.harnessLabel ?? HarnessNames.label(harness))
    }

    /// The model a chat without one runs: the host's default for its
    /// provider. The built-in list answers at once; the host's (or Cloud's)
    /// own list replaces it when it arrives.
    private func defaultModel(for row: SessionRow) -> ModelPickerRow? {
        let harness = row.harness ?? "claude-code"
        let key = "\(hostDevice)|\(harness)"
        if defaultModels[key] == nil, let first = fallbackModels(harness: harness).first {
            defaultModels[key] = ModelPickerRow(id: first.id, label: first.label, detail: first.description)
        }
        if !hostDevice.isEmpty, defaultModelsLoading.insert(key).inserted {
            let device = hostDevice
            Task { @MainActor [weak self] in
                guard let self, let first = await self.client.listModels(deviceId: device, harness: harness).first else { return }
                let row = ModelPickerRow(id: first.id, label: first.label, detail: first.description)
                guard self.defaultModels[key] != row else { return }
                self.defaultModels[key] = row
                self.refresh()
            }
        }
        return defaultModels[key]
    }

    static func permissionSymbol(_ mode: PermissionMode) -> String {
        switch mode {
        case .ask: "checklist"
        case .auto: "wand.and.sparkles"
        case .full: "exclamationmark.triangle"
        }
    }

    private func setConfig(_ change: (inout ChatConfig) -> Void) {
        try? client.setSessionConfig(chatId: chatId, config: changedConfig(change))
    }

    private func changedConfig(_ change: (inout ChatConfig) -> Void) -> ChatConfig {
        var config = client.sessionConfig(chatId: chatId) ?? ChatConfig(harness: app?.row(chatId)?.harness ?? "claude-code", model: nil, reasoning: nil, modelOptions: [:], sandbox: .workspaceWrite)
        change(&config)
        return config
    }

    private struct Provider {
        let harness: String
        let instanceId: String?
        let label: String
    }

    @MainActor
    private func modelMenu(row: SessionRow, harness: String) async -> [UIMenuElement] {
        let device = hostDevice
        async let profileRows = client.profileRows(chatId: chatId)
        async let instances = client.listProviderInstances(deviceId: device)
        let config = client.sessionConfig(chatId: chatId)
        let current = Provider(
            harness: harness,
            instanceId: config?.instanceId,
            label: (await instances).first { $0.id == config?.instanceId }?.label ?? row.harnessLabel ?? harness)
        var providers = [current]
        if current.instanceId != nil {
            providers.append(Provider(harness: harness, instanceId: nil, label: row.harnessLabel ?? harness))
        }
        providers += await instances.filter { $0.id != current.instanceId }.map { Provider(harness: $0.harness, instanceId: $0.id, label: $0.label) }
        let catalogs = await withTaskGroup(of: (Int, InstanceModels).self) { group in
            for (i, provider) in providers.enumerated() {
                group.addTask { [client] in
                    guard let id = provider.instanceId else {
                        return (i, InstanceModels(models: await client.listModels(deviceId: device, harness: provider.harness), error: nil))
                    }
                    return (i, await client.listInstanceModels(deviceId: device, harness: provider.harness, instanceId: id, force: false))
                }
            }
            var out = [Int: InstanceModels]()
            for await (i, catalog) in group { out[i] = catalog }
            return out
        }
        profiles = await profileRows
        var sections: [UIMenuElement] = []
        if !profiles.isEmpty { sections.append(profileSection(profiles)) }
        let labels = profileMenuLabels()
        for (i, provider) in providers.enumerated() {
            guard let catalog = catalogs[i] else { continue }
            let children: [UIMenuElement]
            if let error = catalog.error, let id = provider.instanceId {
                children = [
                    UIAction(title: provider.label, subtitle: error, image: UIImage(systemName: "exclamationmark.triangle"), attributes: .disabled) { _ in },
                    UIAction(title: labels.retry, image: UIImage(systemName: "arrow.clockwise")) { [weak self] _ in self?.retryInstance(provider.label, harness: provider.harness, id: id) },
                ]
            } else if i == 0 {
                children = catalog.models.map { m in
                    UIAction(title: m.label, subtitle: m.description, state: m.id == row.model ? .on : .off) { [weak self] _ in
                        self?.pickModel(m, among: catalog.models)
                    }
                }
            } else {
                children = catalog.models.map { m in
                    UIAction(title: m.label, subtitle: m.description) { [weak self] _ in
                        self?.pickProvider(provider, model: m, from: current, row: row)
                    }
                }
            }
            sections.append(UIMenu(title: provider.label, options: .displayInline, children: children))
        }
        return sections
    }

    private func profileSection(_ rows: [ProfileRow]) -> UIMenu {
        let labels = profileMenuLabels()
        var items: [UIMenuElement] = rows.map { p in
            let image = UIImage(systemName: "person.crop.circle")
            guard p.active, let drift = p.drift else {
                return UIAction(title: p.name, subtitle: p.line, image: image, state: p.active ? .on : .off) { [weak self] _ in
                    self?.applyProfile(p.id)
                }
            }
            var actions = [UIAction(title: labels.revert, image: UIImage(systemName: "arrow.uturn.backward")) { [weak self] _ in self?.applyProfile(p.id) }]
            if p.savable {
                actions.append(UIAction(title: labels.save, image: UIImage(systemName: "square.and.arrow.down")) { [weak self] _ in self?.saveProfile(p.id) })
            }
            return UIMenu(title: "\(p.name) · \(labels.edited)", subtitle: drift, image: image, children: actions)
        }
        let none = !rows.contains(where: \.active)
        items.append(UIAction(title: labels.none, subtitle: labels.noneNote, image: UIImage(systemName: "person.crop.circle.badge.xmark"), state: none ? .on : .off) { [weak self] _ in
            guard let self, !none else { return }
            self.reportFailure { try self.client.detachAgentProfile(chatId: self.chatId) }
        })
        return UIMenu(title: labels.title, options: .displayInline, children: items)
    }

    private func applyProfile(_ id: String) {
        let chat = chatId
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                guard let prompt = try await self.client.profileSwitchPrompt(chatId: chat, profileId: id) else {
                    return try await self.client.applyAgentProfile(chatId: chat, profileId: id, note: nil, hint: nil)
                }
                self.confirm(prompt) { [weak self] in
                    try await self?.client.applyAgentProfile(chatId: chat, profileId: id, note: prompt.note, hint: prompt.noteHint)
                }
            } catch {
                self.alert("\(error)")
            }
        }
    }

    private func saveProfile(_ id: String) {
        let chat = chatId
        Task { @MainActor [weak self] in
            do {
                try await self?.client.saveChatIntoProfile(chatId: chat, profileId: id)
                self?.profilesFor = nil
                self?.refresh()
            } catch {
                self?.alert("\(error)")
            }
        }
    }

    private func retryInstance(_ label: String, harness: String, id: String) {
        let device = hostDevice
        Task { @MainActor [weak self] in
            guard let self else { return }
            let catalog = await self.client.listInstanceModels(deviceId: device, harness: harness, instanceId: id, force: true)
            if let error = catalog.error { self.alert("\(label): \(error)") }
        }
    }

    private func pickProvider(_ provider: Provider, model: ModelInfo, from current: Provider, row: SessionRow) {
        let prompt = client.providerSwitchPrompt(
            chatId: chatId,
            fromProvider: current.label,
            toProvider: provider.label,
            fromModel: row.modelLabel ?? row.model ?? "",
            toModel: model.label)
        confirmSwitch(prompt) {
            if $0.harness != provider.harness {
                $0.reasoning = nil
                $0.modelOptions = [:]
            }
            $0.harness = provider.harness
            $0.instanceId = provider.instanceId
            $0.model = model.id
        }
    }

    private func reportFailure(_ body: () throws -> Void) {
        do { try body() } catch { alert("\(error)") }
    }

    private func alert(_ message: String) {
        let alert = UIAlertController(title: nil, message: message, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        onPresent?(alert)
    }

    private func pickModel(_ model: ModelInfo, among models: [ModelInfo]) {
        let current = client.sessionConfig(chatId: chatId)?.model
        let from = models.first { $0.id == current }?.label ?? current ?? ""
        confirmSwitch(client.modelSwitchPrompt(chatId: chatId, toModel: model.id, fromLabel: from, toLabel: model.label)) { $0.model = model.id }
    }

    private func confirmSwitch(_ prompt: ModelSwitchPrompt?, _ change: @escaping (inout ChatConfig) -> Void) {
        guard let prompt else { return setConfig(change) }
        let chat = chatId
        confirm(prompt) { [weak self] in
            guard let self else { return }
            try await self.client.setSessionConfigNoted(chatId: chat, config: self.changedConfig(change), note: prompt.note, hint: prompt.noteHint)
        }
    }

    private func confirm(_ prompt: ModelSwitchPrompt, then apply: @escaping @MainActor () async throws -> Void) {
        let alert = UIAlertController(title: prompt.title, message: "\(prompt.lead).\n\n\(prompt.body)", preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: prompt.cancel, style: .cancel))
        alert.addAction(UIAlertAction(title: prompt.confirm, style: .default) { _ in
            Task { @MainActor in try? await apply() }
        })
        alert.preferredAction = alert.actions.last
        onPresent?(alert)
    }

    func agentsAction(open: @escaping (String) -> Void) -> UIAction? {
        let roster = handle.backgroundRoster(nowMs: nowMs())
        guard let summary = roster.summary else { return nil }
        return UIAction(title: roster.title, subtitle: summary, image: UIImage(systemName: "person.2")) { [weak self] _ in
            guard let self else { return }
            self.onPresent?(AgentsSheetViewController(handle: self.handle, open: open, findings: { [weak self] taskId in
                self?.findingsSheet(taskId: taskId)
            }).wrapped)
        }
    }

    func agentsCard(open: @escaping (String) -> Void) -> AgentsCard? {
        AgentsCard(
            handle: handle,
            open: open,
            findings: { [weak self] taskId in self?.findingsSheet(taskId: taskId).map { ReviewUI.wrap($0) } },
            present: { [weak self] vc in self?.onPresent?(vc) }
        )
    }

    func reviewAction() -> UIAction? {
        guard let review = handle.composer().review else { return nil }
        return UIAction(title: review.menuLabel, image: UIImage(systemName: "checklist")) { [weak self] _ in
            guard let self else { return }
            let sheet = ReviewOptionsViewController(handle: self.handle)
            self.reviewSheets.add(sheet)
            self.onPresent?(ReviewUI.wrap(sheet))
        }
    }

    private func reviewChipSheet() -> UIViewController? {
        guard let review = handle.composer().review else { return nil }
        switch review.action {
        case .openFindings:
            return findingsSheet(taskId: nil).map { ReviewUI.wrap($0) }
        case .start:
            Task { @MainActor [weak self, handle] in
                do { try await handle.startReview() } catch { self?.reviewFailed(error) }
            }
            return nil
        case .inert:
            return nil
        }
    }

    private func findingsSheet(taskId: String?) -> ReviewFindingsViewController? {
        guard handle.reviewFindings(taskId: taskId) != nil else { return nil }
        let sheet = ReviewFindingsViewController(handle: handle, taskId: taskId)
        reviewSheets.add(sheet)
        return sheet
    }

    private func reviewFailed(_ error: Error) {
        let alert = UIAlertController(title: nil, message: GitText.failure(error, fallback: "\(error)"), preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        onPresent?(alert)
    }

    func chatChanges() async -> ChatChangesView? {
        guard handle.composer().review != nil else { return nil }
        return try? await handle.chatChanges()
    }

    var runsClaudeCode: Bool { (app?.row(chatId)?.harness ?? "claude-code") == "claude-code" }

    func claudeConfigPanel() -> ClaudeConfigPanel? {
        runsClaudeCode ? client.claudeConfigPanel(chatId: chatId) : nil
    }

    func applyChip(_ write: ChipWrite) {
        if let model = write.model {
            let current = client.sessionConfig(chatId: chatId)?.model
            let from = app?.row(chatId)?.modelLabel ?? current ?? ""
            confirmSwitch(client.modelSwitchPrompt(chatId: chatId, toModel: model, fromLabel: from, toLabel: write.modelLabel ?? model)) { $0.model = model }
        }
        if let reasoning = write.reasoning { setConfig { $0.reasoning = reasoning } }
        if let permission = write.permission { setConfig { $0.permissionMode = permission } }
    }

    var skillNames: [String] {
        invocationCatalogs.filter { $0.key.hasSuffix("|true") }.values.flatMap { $0.entries.map(\.name) }
    }

    func runClaudeCard(_ text: String) {
        let skills = skillNames
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                if let write = try await self.client.runClaudeCard(chatId: self.chatId, text: text, skills: skills) {
                    self.applyChip(write)
                }
            } catch {
                self.alert("\(error)")
            }
        }
    }

    func retryClaudeCard(messageId: String, command: String) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                try await self.client.retryClaudeCard(chatId: self.chatId, messageId: messageId, command: command)
            } catch {
                self.alert("\(error)")
            }
        }
    }

    func claudeSettingsFile() async throws -> SkillFile {
        try await client.claudeSettingsFile(chatId: chatId)
    }

    func exportMarkdown() -> ChatExport? {
        handle.exportMarkdown()
    }

    func usagePage() -> UIViewController? {
        guard let app, !hostDevice.isEmpty else { return nil }
        return HostPageViewController(app: app, deviceId: hostDevice, page: "usage", title: "Usage")
    }

    func openChanges(from host: UIViewController, title: String) {
        DiffSheetViewController.presentFindings(handle, from: host, title: title, focus: nil)
    }

    func removeStagedComment(_ id: String) {
        handle.removeStagedComment(id: id)
    }

    func clearStagedComments() {
        handle.clearStagedComments()
    }

    func handoffMenu(open: @escaping (String) -> Void) -> UIMenuElement? {
        let labels = handoffLabels()
        let image = UIImage(systemName: "arrow.left.arrow.right")
        guard !handle.handoffBlocked() else {
            return UIAction(title: labels.title, image: image, attributes: .disabled) { _ in }
        }
        return UIMenu(title: labels.title, image: image, children: [UIDeferredMenuElement.uncached { [weak self] done in
            guard let self else { return done([]) }
            let device = self.hostDevice
            Task { @MainActor [weak self] in
                let targets = await self?.client.handoffTargets(deviceId: device) ?? []
                done(self?.handoffSections(targets, labels: labels, open: open) ?? [])
            }
        }])
    }

    @MainActor private func handoffSections(_ targets: [HandoffTarget], labels: HandoffLabels, open: @escaping (String) -> Void) -> [UIMenuElement] {
        let row = app?.row(chatId)
        let current = client.sessionConfig(chatId: chatId)?.harness ?? row?.harness
        let title = { (t: HandoffTarget) in "\(t.name) · \(t.modelLabel)" }
        var sections: [UIMenuElement] = []
        let inPlace = targets.filter { $0.harness != current }.map { t in
            UIAction(title: title(t)) { [weak self] _ in self?.handOffInPlace(t, from: row) }
        }
        if !inPlace.isEmpty { sections.append(UIMenu(title: labels.thisThread, options: .displayInline, children: inPlace)) }
        let fresh = targets.map { t in
            UIAction(title: title(t)) { [weak self] _ in self?.continueInNewThread(harness: t.harness, model: t.model, spaceId: nil, open: open) }
        }
        if !fresh.isEmpty { sections.append(UIMenu(title: labels.newThread, options: .displayInline, children: fresh)) }
        if let row, row.project == nil {
            let projects = (app?.projectOptions ?? []).filter { $0.device == row.deviceId }.map { p in
                UIAction(title: p.name, image: UIImage(systemName: "folder")) { [weak self] _ in self?.continueInNewThread(harness: nil, model: nil, spaceId: p.id, open: open) }
            }
            if !projects.isEmpty { sections.append(UIMenu(title: labels.project, options: .displayInline, children: projects)) }
        }
        return sections
    }

    private func handOffInPlace(_ target: HandoffTarget, from row: SessionRow?) {
        let prompt = client.providerSwitchPrompt(
            chatId: chatId,
            fromProvider: row?.harnessLabel ?? row?.harness ?? "",
            toProvider: target.name,
            fromModel: row?.modelLabel ?? row?.model ?? "",
            toModel: target.modelLabel)
        confirmSwitch(prompt) {
            $0.harness = target.harness
            $0.model = target.model
            $0.reasoning = nil
            $0.modelOptions = [:]
            $0.profileId = nil
        }
    }

    private func continueInNewThread(harness: String?, model: String?, spaceId: String?, open: @escaping (String) -> Void) {
        let source = chatId
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                open(try await self.client.continueInNewThread(sourceChatId: source, harness: harness, model: model, targetSpaceId: spaceId))
            } catch {
                let alert = UIAlertController(title: nil, message: handoffFailedMessage(err: "\(error)"), preferredStyle: .alert)
                alert.addAction(UIAlertAction(title: "OK", style: .default))
                self.onPresent?(alert)
            }
        }
    }

    func searchFiles(_ query: String) async -> [FileMatch] {
        (try? await client.searchFiles(deviceId: hostDevice, chatId: chatId, spaceId: nil, query: query)) ?? []
    }

    func branchBlocked(rewind: Bool) async -> String? {
        await handle.branchBlocked(rewind: rewind)
    }

    func fork(from entryId: String) async throws -> String {
        try await handle.forkFrom(entryId: entryId)
    }

    func rewind(to entryId: String) async throws -> String {
        try await handle.rewindTo(entryId: entryId)
    }

    func skillFile(name: String, path: String?) async throws -> SkillFile {
        try await handle.skillFile(name: name, path: path)
    }

    func chatUsage() async -> UsageMeter? {
        try? await client.chatUsage(chatId: chatId)
    }

    func limitCard(chatId: String) -> LimitCard? {
        client.limitCard(chatId: chatId)
    }

    func resolveLimit(chatId: String, choice: String) async throws {
        _ = try await client.resolveLimitOffer(chatId: chatId, choice: choice)
    }

    func setLimitResume(chatId: String, auto: Bool) async throws {
        _ = try await client.setLimitResume(chatId: chatId, autoResume: auto)
    }

    func threadMentions(_ query: String) -> [ThreadMention] {
        client.threadMentions(deviceId: hostDevice, spaceId: app?.row(chatId)?.project?.id, exclude: chatId, query: query)
    }

    @MainActor
    func invocationCatalog(skill: Bool) async -> InvocationCatalog? {
        let harness = app?.row(chatId)?.harness ?? "claude-code"
        let key = "\(hostDevice)|\(harness)|\(skill)"
        if let hit = invocationCatalogs[key] { return hit }
        let found = await client.invocationCatalog(deviceId: hostDevice, harness: harness, chatId: chatId, spaceId: nil, skill: skill)
        if found.warning == nil { invocationCatalogs[key] = found }
        return found
    }

    private static let images = NSCache<NSString, UIImage>()
    private static let pages = NSCache<NSString, NSString>()

    func htmlPage(deviceId: String, path: String) async -> String? {
        let device = deviceId.isEmpty ? hostDevice : deviceId
        let key = "\(device)\n\(path)" as NSString
        if let hit = Self.pages.object(forKey: key) { return hit as String }
        guard let data = try? await client.readAttachment(deviceId: device, path: path),
              let html = String(data: data, encoding: .utf8)
        else { return nil }
        Self.pages.setObject(html as NSString, forKey: key)
        return html
    }

    func image(_ reference: String) async -> UIImage? {
        if let hit = Self.images.object(forKey: reference as NSString) { return hit }
        guard let data = try? await client.readAttachment(deviceId: hostDevice, path: reference),
              let image = await UIImage(data: data)?.byPreparingForDisplay()
        else { return nil }
        Self.images.setObject(image, forKey: reference as NSString)
        return image
    }

    func attachmentText(_ reference: String) async -> String? {
        guard let data = try? await client.readAttachment(deviceId: hostDevice, path: reference) else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    func loadImage(_ reference: String, into view: UIImageView) {
        // Claim the view first: a slower load for a row it used to show
        // must not land on top of this one.
        view.accessibilityIdentifier = reference
        if let hit = Self.images.object(forKey: reference as NSString) {
            view.image = hit
            return
        }
        let device = hostDevice
        let client = self.client
        view.accessibilityIdentifier = reference
        Task.detached(priority: .userInitiated) {
            guard let data = try? await client.readAttachment(deviceId: device, path: reference),
                  let image = UIImage(data: data)?.preparingForDisplay()
            else { return }
            Self.images.setObject(image, forKey: reference as NSString)
            await MainActor.run {
                guard view.accessibilityIdentifier == reference else { return }
                UIView.transition(with: view, duration: 0.2, options: .transitionCrossDissolve) { view.image = image }
            }
        }
    }
}
