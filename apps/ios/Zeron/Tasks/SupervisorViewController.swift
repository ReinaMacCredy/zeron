import UIKit

final class SupervisorViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<String, Row>!
    private var appToken: AnyObject?
    private var filter = Filter.all

    enum Filter: String, CaseIterable {
        case all = "All"
        case questions = "Questions"
        case decisions = "Decisions"
        case plans = "Plans"

        func matches(_ entry: NeedsYouEntry) -> Bool {
            switch (self, entry.kind) {
            case (.all, _), (.questions, .input), (.decisions, .item), (.plans, .plan): true
            default: false
            }
        }
    }

    struct Row: Hashable {
        let id: String
        var title: String
        var subtitle: String?
        var glyph: StatusGlyph.Kind?
        var symbol: String?
        var tint: UIColor = Palette.text
        var tappable = true

        static func == (a: Row, b: Row) -> Bool {
            a.id == b.id && a.title == b.title && a.subtitle == b.subtitle && a.glyph == b.glyph
        }

        func hash(into hasher: inout Hasher) {
            hasher.combine(id)
        }
    }

    init(app: AppModel) {
        self.app = app
        super.init(nibName: nil, bundle: nil)
        title = "Supervisor"
        navigationItem.largeTitleDisplayMode = .never
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        collectionView.accessibilityIdentifier = "supervisor"
        view.addSubview(collectionView)
        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, Row> { cell, _, row in
            var c = UIListContentConfiguration.subtitleCell()
            c.text = row.title
            c.textProperties.font = Fonts.ui(.sansMedium, 16)
            c.textProperties.color = row.tint
            c.textProperties.numberOfLines = 0
            c.secondaryText = row.subtitle
            c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
            c.secondaryTextProperties.color = Palette.secondary
            c.secondaryTextProperties.numberOfLines = 0
            c.textToSecondaryTextVerticalPadding = 3
            var accessories: [UICellAccessory] = []
            if let glyph = row.glyph { accessories.append(TaskGlyph.leading(glyph)) }
            if let symbol = row.symbol {
                c.image = UIImage(systemName: symbol)
                c.imageProperties.tintColor = Palette.secondary
            }
            if row.tappable { accessories.append(.disclosureIndicator()) }
            cell.accessories = accessories
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = "sv-\(row.id)"
            var bg = UIBackgroundConfiguration.listCell()
            bg.backgroundColor = Palette.elevated
            cell.backgroundConfiguration = bg
        }
        let header = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionHeader) { [weak self] view, _, path in
            var c = UIListContentConfiguration.header()
            c.text = self?.dataSource.snapshot().sectionIdentifiers[path.section]
            view.contentConfiguration = c
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { cv, path, row in
            cv.dequeueConfiguredReusableCell(using: cell, for: path, item: row)
        }
        dataSource.supplementaryViewProvider = { cv, _, path in
            cv.dequeueConfiguredReusableSupplementary(using: header, for: path)
        }
        reload()
        appToken = app.observe { [weak self] in self?.reload() }
    }

    private static var nowMs: Int64 { Int64(Date().timeIntervalSince1970 * 1000) }

    private static func clock(_ ms: Int64) -> String {
        let out = DateFormatter()
        out.setLocalizedDateFormatFromTemplate("jmm")
        return out.string(from: Date(timeIntervalSince1970: TimeInterval(ms) / 1000))
    }

    static func hostLine(_ host: SupervisorHost) -> String {
        if !host.host.online { return "Offline" }
        if let blocked = host.blocked { return "Couldn't start the Supervisor: \(blocked)" }
        switch host.state {
        case .off: return "Off"
        case .stopped:
            return host.waiting > 0 ? "Stopped by you · \(host.waiting) wake\(host.waiting == 1 ? "" : "s") waiting" : "Stopped by you"
        case .asleep, .awake:
            var parts = [host.watchEverything ? "Watching everything" : "Watching \(host.trees) tree\(host.trees == 1 ? "" : "s")"]
            parts.append("\(host.wakesThisHour) of \(host.wakesPerHour) wakes this hour")
            if let next = host.nextCheckMs { parts.append("next check \(clock(next))") }
            return parts.joined(separator: " · ")
        }
    }

    static func kindWord(_ entry: NeedsYouEntry) -> String {
        switch entry.kind {
        case let .item(_, why, _, _, _): why
        case .input: "Question"
        case .plan: "Plan ready"
        }
    }

    static func sendLine(_ entry: NeedsYouEntry) -> String? {
        switch entry.send {
        case let .sending(choice)?: "Sending to \(entry.dest) · → \(choice)"
        case let .sent(choice)?: "Sent to \(entry.dest) · → \(choice)"
        case let .failed(choice, reason)?: "Could not send: \(reason) · \(choice) kept"
        case nil: nil
        }
    }

    static func needsLine(_ entry: NeedsYouEntry) -> String {
        if let send = sendLine(entry) { return "\(entry.host.name) · \(send)" }
        var parts = [entry.host.name]
        switch entry.kind {
        case let .item(_, why, raisedByLead, _, _):
            parts.append(raisedByLead ? "\(why) · Raised by Lead" : why)
        case .input:
            parts.append("Question")
        case let .plan(_, steps):
            parts.append(steps.prefix(2).joined(separator: ", ") + (steps.count > 2 ? " +\(steps.count - 2) more" : ""))
        }
        parts.append(relativeTimeLabel(atMs: entry.atMs, nowMs: nowMs))
        return parts.joined(separator: " · ")
    }

    private func updateFilterMenu() {
        let all = app.supervisor.needsYou
        navigationItem.rightBarButtonItem = UIBarButtonItem(
            title: filter == .all ? "Filter" : filter.rawValue,
            image: UIImage(systemName: "line.3.horizontal.decrease.circle"),
            menu: UIMenu(children: Filter.allCases.map { option in
                UIAction(title: "\(option.rawValue) · \(all.filter(option.matches).count)", state: option == filter ? .on : .off) { [weak self] _ in
                    self?.filter = option
                    self?.reload()
                }
            })
        )
        navigationItem.rightBarButtonItem?.accessibilityIdentifier = "sv-filter"
    }

    private func reload() {
        let page = app.supervisor
        var snap = NSDiffableDataSourceSnapshot<String, Row>()
        if !page.hosts.isEmpty {
            snap.appendSections(["Status · \(page.hosts.count) Mac\(page.hosts.count == 1 ? "" : "s")"])
            snap.appendItems(page.hosts.map { host in
                Row(id: "host:\(host.host.deviceId)", title: host.host.name, subtitle: Self.hostLine(host), symbol: "desktopcomputer")
            })
        }
        let shown = page.needsYou.filter(filter.matches)
        if shown.isEmpty {
            snap.appendSections(["Needs you"])
            let empty = filter == .all ? "Nothing needs you" : "No \(filter.rawValue.lowercased())"
            snap.appendItems([Row(id: "needs-empty", title: empty, tint: Palette.secondary, tappable: false)])
        }
        var groups: [String] = []
        var byGroup: [String: [NeedsYouEntry]] = [:]
        for entry in shown {
            let key = "\(entry.host.deviceId)|\(entry.groupKey)"
            if byGroup[key] == nil { groups.append(key) }
            byGroup[key, default: []].append(entry)
        }
        for key in groups {
            let rows = byGroup[key] ?? []
            guard let first = rows.first else { continue }
            let section = "\(first.groupTitle) · \(rows.count) to answer"
            snap.appendSections([snap.sectionIdentifiers.contains(section) ? "\(section) · \(first.host.name)" : section])
            snap.appendItems(rows.map { entry in
                let glyph: StatusGlyph.Kind
                switch entry.kind {
                case .input: glyph = .dot(StatusTone.input)
                case .item, .plan: glyph = .dot(Palette.warning)
                }
                return Row(id: "needs:\(entry.key)", title: entry.title, subtitle: Self.needsLine(entry), glyph: glyph)
            })
        }
        if !page.doneToday.isEmpty {
            snap.appendSections(["Done today · \(page.doneToday.count)"])
            snap.appendItems(page.doneToday.map { task in
                var parts: [String] = []
                if let host = task.host { parts.append(host.name) }
                parts.append(task.progressTotal > 0 ? "Done · \(task.progressDone) of \(task.progressTotal)" : "Done")
                if let at = task.completedAtMs { parts.append(Self.clock(at)) }
                return Row(id: "done:\(task.id)", title: task.title, subtitle: parts.joined(separator: " · "), glyph: .check(StatusTone.done))
            })
        }
        if !page.wakes.isEmpty {
            snap.appendSections(["Wake log · \(page.wakes.count)"])
            snap.appendItems(page.wakes.map { wake in
                let events = wake.events.isEmpty ? "" : " · \(wake.events.count) event\(wake.events.count == 1 ? "" : "s") · " + wake.events.joined(separator: ", ")
                return Row(
                    id: "wake:\(wake.host.deviceId):\(wake.id)",
                    title: Self.clock(wake.atMs) + events,
                    subtitle: "\(wake.host.name) · \(wake.delivery)",
                    tint: wake.woke ? Palette.text : Palette.secondary,
                    tappable: false
                )
            })
        }
        dataSource.apply(snap, animatingDifferences: view.window != nil)
        updateFilterMenu()
    }

    func collectionView(_ collectionView: UICollectionView, shouldSelectItemAt indexPath: IndexPath) -> Bool {
        dataSource.itemIdentifier(for: indexPath)?.tappable ?? false
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: true)
        guard let row = dataSource.itemIdentifier(for: indexPath) else { return }
        let source = collectionView.cellForItem(at: indexPath)
        if row.id.hasPrefix("host:"), let host = app.supervisor.hosts.first(where: { "host:\($0.host.deviceId)" == row.id }) {
            hostActions(host, from: source)
        } else if row.id.hasPrefix("needs:"), let entry = app.supervisor.needsYou.first(where: { "needs:\($0.key)" == row.id }) {
            answer(entry, from: source)
        } else if row.id.hasPrefix("done:") {
            let taskId = String(row.id.dropFirst("done:".count))
            navigationController?.pushViewController(MissionControlViewController(app: app, taskId: taskId), animated: true)
        }
    }

    private func answer(_ entry: NeedsYouEntry, from source: UIView?) {
        switch entry.kind {
        case .input:
            TasksViewController.push(thread: entry.chatId, app: app, from: navigationController)
        case let .plan(taskId, _):
            navigationController?.pushViewController(MissionControlViewController(app: app, taskId: taskId), animated: true)
        case .item:
            let answers = SupervisorAnswersViewController(app: app, groupKey: entry.groupKey, hostId: entry.host.deviceId) { [weak self] chat in
                self?.dismiss(animated: true) {
                    guard let self else { return }
                    TasksViewController.push(thread: chat, app: self.app, from: self.navigationController)
                }
            }
            let nav = UINavigationController(rootViewController: answers)
            if let sheet = nav.sheetPresentationController {
                sheet.detents = [.medium(), .large()]
                sheet.prefersGrabberVisible = true
            }
            present(nav, animated: true)
        }
    }

    private func hostActions(_ host: SupervisorHost, from source: UIView?) {
        guard let client = app.client else { return }
        let device = host.host.deviceId
        let sheet = UIAlertController(title: "Supervisor on \(host.host.name)", message: Self.hostLine(host), preferredStyle: .actionSheet)
        if host.blocked != nil || host.state == .off {
            let fallback = app.lastDraft.config
            sheet.addAction(UIAlertAction(title: host.blocked == nil ? "Start" : "Try again", style: .default) { [weak self] _ in
                self?.run { _ = try await client.ensureSupervisor(deviceId: device, fallback: fallback) }
            })
        } else if host.state == .stopped {
            sheet.addAction(UIAlertAction(title: "Resume", style: .default) { [weak self] _ in
                self?.run { try await client.resumeSupervisor(deviceId: device) }
            })
        } else {
            sheet.addAction(UIAlertAction(title: "Stop", style: .destructive) { [weak self] _ in
                self?.confirmStop(host, from: source)
            })
        }
        if let chat = host.chatId {
            sheet.addAction(UIAlertAction(title: "Open conversation", style: .default) { [weak self] _ in
                guard let self else { return }
                TasksViewController.push(thread: chat, app: self.app, from: self.navigationController)
            })
        }
        sheet.addAction(UIAlertAction(title: host.watchEverything ? "Stop watching everything" : "Watch everything", style: .default) { [weak self] _ in
            self?.run { try await client.setWatchEverything(deviceId: device, on: !host.watchEverything) }
        })
        sheet.addAction(UIAlertAction(title: "Authority…", style: .default) { [weak self] _ in
            self?.presentAuthority(host.host)
        })
        sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        sheet.popoverPresentationController?.sourceView = source
        present(sheet, animated: true)
    }

    private func confirmStop(_ host: SupervisorHost, from source: UIView?) {
        guard let client = app.client else { return }
        let device = host.host.deviceId
        let alert = UIAlertController(title: "Stop the Supervisor on \(host.host.name)?", message: "Wakes wait until you resume it.", preferredStyle: .actionSheet)
        alert.addAction(UIAlertAction(title: "Stop", style: .destructive) { [weak self] _ in
            self?.run { try await client.stopSupervisor(deviceId: device) }
        })
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        alert.popoverPresentationController?.sourceView = source
        present(alert, animated: true)
    }

    private func run(_ action: @escaping () async throws -> Void) {
        Task { @MainActor in
            do { try await action() } catch {
                Toast.show(TasksErrors.message(error), in: self.view.window)
            }
        }
    }

    private func presentAuthority(_ host: HostRef) {
        let nav = UINavigationController(rootViewController: AuthorityViewController(app: app, host: host))
        if let sheet = nav.sheetPresentationController {
            sheet.detents = [.large()]
            sheet.prefersGrabberVisible = true
        }
        present(nav, animated: true)
    }
}

final class AuthorityViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private let host: HostRef
    private var settings: SupervisorSettings?
    private var failure: String?
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<String, Row>!

    struct Row: Hashable {
        let id: String
        let title: String
        var subtitle: String?
        var on: Bool?
    }

    init(app: AppModel, host: HostRef) {
        self.app = app
        self.host = host
        super.init(nibName: nil, bundle: nil)
        title = "Supervisor on \(host.name)"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        config.footerMode = .supplementary
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        view.addSubview(collectionView)
        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, Row> { [weak self] cell, _, row in
            var c = UIListContentConfiguration.subtitleCell()
            c.text = row.title
            c.textProperties.font = Fonts.ui(.sansMedium, 16)
            c.textProperties.color = row.on == nil ? Palette.secondary : Palette.text
            c.secondaryText = row.subtitle
            c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
            c.secondaryTextProperties.color = Palette.secondary
            c.secondaryTextProperties.numberOfLines = 0
            cell.contentConfiguration = c
            if let on = row.on {
                let toggle = UISwitch()
                toggle.isOn = on
                toggle.onTintColor = Palette.accent
                toggle.accessibilityIdentifier = "authority-\(row.id)"
                toggle.addAction(UIAction { [weak self, weak toggle] _ in
                    self?.toggled(row.id, toggle?.isOn ?? false)
                }, for: .valueChanged)
                cell.accessories = [.customView(configuration: .init(customView: toggle, placement: .trailing()))]
            } else {
                cell.accessories = []
            }
            var bg = UIBackgroundConfiguration.listCell()
            bg.backgroundColor = Palette.elevated
            cell.backgroundConfiguration = bg
        }
        let header = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionHeader) { [weak self] view, _, path in
            var c = UIListContentConfiguration.header()
            c.text = self?.dataSource.snapshot().sectionIdentifiers[path.section]
            view.contentConfiguration = c
        }
        let footer = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionFooter) { [weak self] view, _, path in
            var c = UIListContentConfiguration.footer()
            let section = self?.dataSource.snapshot().sectionIdentifiers[path.section]
            c.text = section == "Full authority" ? "Each Mac runs its own Supervisor; these switches apply to this one." : nil
            view.contentConfiguration = c
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { cv, path, row in
            cv.dequeueConfiguredReusableCell(using: cell, for: path, item: row)
        }
        dataSource.supplementaryViewProvider = { cv, kind, path in
            kind == UICollectionView.elementKindSectionHeader
                ? cv.dequeueConfiguredReusableSupplementary(using: header, for: path)
                : cv.dequeueConfiguredReusableSupplementary(using: footer, for: path)
        }
        reload()
        load()
    }

    private func load() {
        guard let client = app.client else { return }
        let device = host.deviceId
        Task { @MainActor in
            do {
                settings = try await client.supervisorSettings(deviceId: device)
                failure = nil
            } catch {
                failure = TasksErrors.message(error)
            }
            reload()
        }
    }

    private func reload() {
        var snap = NSDiffableDataSourceSnapshot<String, Row>()
        guard let s = settings else {
            snap.appendSections([" "])
            snap.appendItems([Row(id: "loading", title: failure ?? "Loading…")])
            dataSource.apply(snap, animatingDifferences: false)
            return
        }
        snap.appendSections(["Full authority"])
        snap.appendItems([
            Row(id: "decide", title: "Decide for me", subtitle: "Answers the Lead's Needs you decisions and approves plans. Each choice shows in the feed with why, and you can change it.", on: s.decideForMe),
            Row(id: "goal", title: "Change a mission's goal or spend", subtitle: "Lets it decide those too.", on: s.goalAndSpend),
        ])
        snap.appendSections(["Also allowed"])
        snap.appendItems([
            Row(id: "message", title: "Message a Lead or Peer directly", on: s.messageThreads),
            Row(id: "rebrief", title: "Re-brief a step, through its Lead", on: s.rebrief),
            Row(id: "pause", title: "Pause a step that blocks a shared resource", on: s.pauseSteps),
            Row(id: "start", title: "Start a new mission", on: s.startMissions),
        ])
        snap.appendSections(["Check in on its own"])
        snap.appendItems([Row(id: "heartbeat", title: s.heartbeatMinutes == 0 ? "Off" : "Every \(s.heartbeatMinutes) min")])
        snap.reconfigureItems(snap.itemIdentifiers)
        dataSource.apply(snap, animatingDifferences: false)
    }

    private func toggled(_ id: String, _ on: Bool) {
        guard let before = settings, let client = app.client else { return }
        var after = before
        switch id {
        case "decide": after.decideForMe = on
        case "goal": after.goalAndSpend = on
        case "message": after.messageThreads = on
        case "rebrief": after.rebrief = on
        case "pause": after.pauseSteps = on
        case "start": after.startMissions = on
        default: return
        }
        settings = after
        let device = host.deviceId
        Task { @MainActor in
            do {
                settings = try await client.setSupervisorSettings(deviceId: device, before: before, after: after)
            } catch {
                settings = before
                Toast.show(TasksErrors.message(error), in: view.window)
            }
            reload()
        }
    }
}
