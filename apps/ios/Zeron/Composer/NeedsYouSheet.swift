import UIKit

struct NeedsYouAsk {
    let chatId: String
    let requestId: String
    let questions: [UserInputQuestion]
    let approval: Bool
    let destructive: Bool
}

enum NeedsYouFilter: CaseIterable {
    case all, questions, decisions, plans

    var label: String {
        switch self {
        case .all: "All"
        case .questions: "Questions"
        case .decisions: "Decisions"
        case .plans: "Plans"
        }
    }
}

extension NeedsYouEntry {
    var input: NeedsYouAsk? {
        guard case let .input(chatId, requestId, questions, approval, destructive) = kind else { return nil }
        return NeedsYouAsk(chatId: chatId, requestId: requestId, questions: questions, approval: approval, destructive: destructive)
    }

    var itemId: String? {
        guard case let .item(itemId, _, _, _, _) = kind else { return nil }
        return itemId
    }

    var isThread: Bool { input != nil && taskId == nil && !groupKey.isEmpty }

    var filter: NeedsYouFilter {
        switch kind {
        case .input: .questions
        case .item: .decisions
        case .plan: .plans
        }
    }

    var symbol: String {
        switch kind {
        case let .input(_, _, _, approval, _): approval ? "exclamationmark.circle" : "questionmark.circle"
        case .item: "questionmark.circle"
        case .plan: "list.bullet.clipboard"
        }
    }

    var tint: UIColor { isThread ? Palette.ask : Palette.warning }

    var kindLabel: String {
        switch kind {
        case let .input(_, _, questions, approval, _): questions.isEmpty ? "Waiting for you" : approval ? "Approval" : "Question"
        case let .item(_, why, _, _, _): why
        case .plan: "Plan ready"
        }
    }

    var actionLabel: String {
        switch kind {
        case let .input(_, _, questions, approval, _): questions.isEmpty ? "Open" : approval ? "Approve" : "Answer"
        case .item: "Answer"
        case .plan: "Approve"
        }
    }

    var rowTitle: String {
        guard input != nil else { return title }
        return asker
    }

    var failure: String? {
        guard case let .failed(_, reason) = send else { return nil }
        return reason
    }

    var locked: Bool {
        switch send {
        case .sending?, .sent?: true
        default: false
        }
    }

    var sendLine: String? {
        switch send {
        case let .sending(choice)?: "Sending to \(dest) · → \(choice)"
        case let .sent(choice)?: "Sent to \(dest) · → \(choice)"
        case let .failed(choice, reason)?: "Could not send: \(reason) · \(choice) kept"
        case nil: nil
        }
    }

    var questions: [SessionChrome.Question] {
        (input?.questions ?? []).map {
            SessionChrome.Question(id: $0.id, header: $0.header, text: $0.question, options: $0.options, multiSelect: $0.multiSelect, prefill: $0.prefill, multiline: $0.multiline)
        }
    }

    func meta(_ app: AppModel) -> String {
        var parts = [kindLabel]
        switch kind {
        case .input where isThread:
            if let project = app.row(chatId)?.project?.name { parts.append(project) }
            parts.append(relativeTimeLabel(atMs: atMs, nowMs: Int64(Date().timeIntervalSince1970 * 1000)))
        case .input:
            if input?.questions.isEmpty == false { parts.append(title) }
        case let .item(_, _, raisedByLead, _, _):
            parts.append(raisedByLead ? "Raised by Lead" : "Raised by Supervisor")
        case .plan:
            parts = [asker]
        }
        return parts.joined(separator: " · ")
    }
}

enum NeedsYou {
    static func present(app: AppModel, from host: UIViewController, excluding chatId: String?, opening entry: NeedsYouEntry? = nil, onClose: ((NeedsYouEntry?) -> Void)? = nil) {
        if let entry, controller(for: entry, app: app) == nil {
            (host.view.window?.rootViewController as? AppRouter)?.openSession(entry.chatId)
            return
        }
        let list = NeedsYouListController(app: app, excluding: chatId)
        let nav = NeedsYouNavigationController(rootViewController: list)
        nav.onClose = onClose
        if let sheet = nav.sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
            sheet.prefersScrollingExpandsWhenScrolledToEdge = false
            if entry != nil { sheet.selectedDetentIdentifier = .large }
        }
        if let entry, let next = controller(for: entry, app: app) {
            nav.pushViewController(next, animated: false)
        }
        host.present(nav, animated: true)
    }

    static func all(_ app: AppModel, excluding chatId: String?) -> [NeedsYouEntry] {
        app.needsYou.filter { $0.input == nil || $0.chatId != chatId }
    }

    static func waiting(_ app: AppModel, excluding chatId: String?) -> [NeedsYouEntry] {
        app.needsYou.filter { $0.input != nil && $0.chatId != chatId }
    }

    static func threads(_ app: AppModel) -> [SessionRowVM] {
        var seen = Set<String>()
        return app.needsYou.compactMap { entry in
            guard entry.input != nil, seen.insert(entry.chatId).inserted, let row = app.row(entry.chatId) else { return nil }
            return AppModel.vm(row)
        }
    }

    static func openThread(_ chatId: String, from vc: UIViewController) {
        let window = vc.view.window
        vc.presentingViewController?.dismiss(animated: true) {
            (window?.rootViewController as? AppRouter)?.openSession(chatId)
        }
    }

    static func controller(for entry: NeedsYouEntry, app: AppModel) -> UIViewController? {
        switch entry.kind {
        case let .input(_, _, questions, _, _):
            return questions.isEmpty ? nil : NeedsYouQuestionController(app: app, entry: entry)
        case .item:
            return NeedsYouGroupController(app: app, groupKey: entry.groupKey, title: entry.groupTitle)
        case .plan:
            return NeedsYouPlanController(app: app, entry: entry)
        }
    }
}

final class NeedsYouNavigationController: UINavigationController {
    var onClose: ((NeedsYouEntry?) -> Void)?
    var sent: NeedsYouEntry?

    override func viewDidLoad() {
        super.viewDidLoad()
        navigationBar.titleTextAttributes = [.font: Fonts.ui(.sansSemibold, 16), .foregroundColor: Palette.text]
        navigationBar.tintColor = Palette.text
    }

    override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        guard isBeingDismissed || presentingViewController == nil else { return }
        let close = onClose
        onClose = nil
        close?(sent)
    }

    func expand() {
        guard let sheet = sheetPresentationController else { return }
        sheet.animateChanges { sheet.selectedDetentIdentifier = .large }
    }
}

final class NeedsYouListController: UITableViewController {
    private struct Group {
        let key: String
        let title: String
        var entries: [NeedsYouEntry]
    }

    private let app: AppModel
    private let excluding: String?
    private var filter = NeedsYouFilter.all
    private var groups: [Group] = []
    private var token: AnyObject?
    private let filters = UISegmentedControl()

    init(app: AppModel, excluding: String?) {
        self.app = app
        self.excluding = excluding
        super.init(style: .insetGrouped)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.backgroundColor = Palette.background
        tableView.accessibilityIdentifier = "needs-you-list"
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "row")
        navigationItem.backButtonTitle = "Needs you"
        filters.addAction(UIAction { [weak self] _ in
            guard let self else { return }
            self.filter = NeedsYouFilter.allCases[max(0, self.filters.selectedSegmentIndex)]
            self.reload()
        }, for: .valueChanged)
        filters.accessibilityIdentifier = "needs-you-filters"
        let header = UIView(frame: CGRect(x: 0, y: 0, width: 0, height: 44))
        filters.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(filters)
        NSLayoutConstraint.activate([
            filters.leadingAnchor.constraint(equalTo: header.layoutMarginsGuide.leadingAnchor),
            filters.trailingAnchor.constraint(equalTo: header.layoutMarginsGuide.trailingAnchor),
            filters.centerYAnchor.constraint(equalTo: header.centerYAnchor),
        ])
        tableView.tableHeaderView = header
        token = app.observeNeedsYou { [weak self] in self?.reload() }
        reload()
    }

    private func reload() {
        let all = NeedsYou.all(app, excluding: excluding)
        title = "Needs you · \(all.count)"
        filters.removeAllSegments()
        for (index, f) in NeedsYouFilter.allCases.enumerated() {
            let count = f == .all ? all.count : all.filter { $0.filter == f }.count
            filters.insertSegment(withTitle: "\(f.label) · \(count)", at: index, animated: false)
        }
        filters.selectedSegmentIndex = NeedsYouFilter.allCases.firstIndex(of: filter) ?? 0
        var next: [Group] = []
        for entry in all where filter == .all || entry.filter == filter {
            let key = entry.isThread ? "threads" : entry.groupKey
            if let index = next.firstIndex(where: { $0.key == key }) {
                next[index].entries.append(entry)
            } else {
                next.append(Group(key: key, title: entry.isThread ? "Threads" : entry.groupTitle, entries: [entry]))
            }
        }
        if let threads = next.firstIndex(where: { $0.key == "threads" }), threads > 0 {
            next.insert(next.remove(at: threads), at: 0)
        }
        groups = next
        tableView.reloadData()
        if all.isEmpty, navigationController?.topViewController === self {
            presentingViewController?.dismiss(animated: true)
        }
    }

    override func numberOfSections(in tableView: UITableView) -> Int { groups.count }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { groups[section].entries.count }

    override func tableView(_ tableView: UITableView, viewForHeaderInSection section: Int) -> UIView? {
        let group = groups[section]
        var c = UIListContentConfiguration.header()
        c.text = group.title
        let items = group.entries.filter { $0.itemId != nil }.count
        let cell = UITableViewHeaderFooterView()
        cell.contentConfiguration = c
        guard items > 0 else { return cell }
        let open = UIButton(configuration: .plain(), primaryAction: UIAction { [weak self] _ in
            guard let self else { return }
            self.navigationController?.pushViewController(NeedsYouGroupController(app: self.app, groupKey: group.key, title: group.title), animated: true)
            (self.navigationController as? NeedsYouNavigationController)?.expand()
        })
        open.configuration?.title = "\(items) to answer"
        open.configuration?.image = UIImage(systemName: "chevron.right", withConfiguration: UIImage.SymbolConfiguration(pointSize: 11, weight: .semibold))
        open.configuration?.imagePlacement = .trailing
        open.configuration?.imagePadding = 4
        open.configuration?.baseForegroundColor = Palette.warning
        open.configuration?.contentInsets = .zero
        open.accessibilityIdentifier = "needs-you-group-\(group.key)"
        open.translatesAutoresizingMaskIntoConstraints = false
        cell.contentView.addSubview(open)
        NSLayoutConstraint.activate([
            open.trailingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.trailingAnchor),
            open.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor, constant: -6),
        ])
        return cell
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let entry = groups[indexPath.section].entries[indexPath.row]
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        var c = UIListContentConfiguration.subtitleCell()
        c.image = UIImage(systemName: entry.symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 17, weight: .semibold))
        c.imageProperties.tintColor = entry.tint
        c.text = entry.rowTitle
        c.textProperties.font = Fonts.ui(.sansSemibold, 16)
        c.textProperties.numberOfLines = 2
        let meta = NSMutableAttributedString(string: "\(entry.host.name) · \(entry.meta(app))", attributes: [.font: Fonts.ui(.sans, 13), .foregroundColor: Palette.secondary])
        if let line = entry.sendLine {
            meta.append(NSAttributedString(string: "\n" + line, attributes: [.font: Fonts.ui(.sansMedium, 13), .foregroundColor: entry.failure == nil ? Palette.secondary : Palette.danger]))
        }
        c.secondaryAttributedText = meta
        c.secondaryTextProperties.numberOfLines = 0
        c.textToSecondaryTextVerticalPadding = 3
        cell.contentConfiguration = c
        let action = UILabel()
        action.text = entry.actionLabel
        action.font = Fonts.ui(.sansSemibold, 14)
        action.textColor = Palette.accent
        action.sizeToFit()
        cell.accessoryView = action
        cell.accessibilityIdentifier = "needs-you-\(entry.key)"
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        let entry = groups[indexPath.section].entries[indexPath.row]
        guard let next = NeedsYou.controller(for: entry, app: app) else {
            return NeedsYou.openThread(entry.chatId, from: self)
        }
        navigationController?.pushViewController(next, animated: true)
        (navigationController as? NeedsYouNavigationController)?.expand()
    }
}

final class NeedsYouQuestionController: UIViewController {
    private let app: AppModel
    private let entry: NeedsYouEntry
    private let panel = QuestionPanel()
    private var token: AnyObject?

    init(app: AppModel, entry: NeedsYouEntry) {
        self.app = app
        self.entry = entry
        super.init(nibName: nil, bundle: nil)
        title = app.row(entry.chatId)?.title ?? entry.asker
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        let scroll = UIScrollView()
        scroll.alwaysBounceVertical = true
        scroll.keyboardDismissMode = .interactive
        scroll.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(scroll)
        let kind = UILabel()
        kind.text = "\(entry.kindLabel) · \(entry.asker) · \(entry.host.name)"
        kind.font = Fonts.ui(.sansMedium, 13)
        kind.textColor = entry.tint
        kind.numberOfLines = 0
        let note = UILabel()
        note.text = "This changes files or the remote. Review it in the thread."
        note.font = Fonts.ui(.sans, 14)
        note.textColor = Palette.secondary
        note.numberOfLines = 0
        note.isHidden = entry.input?.destructive != true
        let stack = UIStackView(arrangedSubviews: [kind, panel, note])
        stack.axis = .vertical
        stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(stack)
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
            stack.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 8),
            stack.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -16),
            stack.leadingAnchor.constraint(equalTo: scroll.frameLayoutGuide.leadingAnchor, constant: 16),
            stack.trailingAnchor.constraint(equalTo: scroll.frameLayoutGuide.trailingAnchor, constant: -16),
        ])
        let chatId = entry.chatId
        panel.onOpenThread = { [weak self] in
            guard let self else { return }
            NeedsYou.openThread(chatId, from: self)
        }
        panel.quickAnswer = entry.input?.destructive != true
        panel.configure(entry.questions)
        panel.onSubmit = { [weak self] answers in self?.send(answers) }
        token = app.observeNeedsYou { [weak self] in
            guard let self, !self.app.needsYou.contains(where: { $0.key == self.entry.key }) else { return }
            if (self.navigationController as? NeedsYouNavigationController)?.sent == nil {
                self.navigationController?.popViewController(animated: true)
            }
        }
    }

    private func send(_ answers: [(questionId: String, labels: [String])]) {
        guard let nav = navigationController as? NeedsYouNavigationController else { return }
        nav.sent = entry
        guard app.answer(entry, answers) else {
            nav.sent = nil
            Toast.show("Couldn't send. Try again.", in: view.window, atTop: true)
            return
        }
        let window = view.window
        let dest = entry.dest
        if nav.viewControllers.count > 1, NeedsYou.all(app, excluding: nil).count > 0 {
            nav.popToRootViewController(animated: true)
            Toast.show("Sent to \(dest)", in: window, atTop: true)
        } else {
            nav.presentingViewController?.dismiss(animated: true) {
                Toast.show("Sent to \(dest)", in: window, atTop: true)
            }
        }
    }
}

final class NeedsYouGroupController: UITableViewController {
    private let app: AppModel
    private let groupKey: String
    private var entries: [NeedsYouEntry] = []
    private var picks: [String: String] = [:]
    private var sending = false
    private var token: AnyObject?
    private let footer = UILabel()
    private let sendButton = UIButton(configuration: .filled())

    init(app: AppModel, groupKey: String, title: String) {
        self.app = app
        self.groupKey = groupKey
        super.init(style: .insetGrouped)
        self.title = title
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.backgroundColor = Palette.background
        tableView.accessibilityIdentifier = "needs-you-group"
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "row")
        footer.font = Fonts.ui(.sans, 13)
        footer.textColor = Palette.secondary
        sendButton.configuration?.baseBackgroundColor = Palette.accent
        sendButton.configuration?.cornerStyle = .capsule
        sendButton.accessibilityIdentifier = "needs-you-send"
        sendButton.addAction(UIAction { [weak self] _ in self?.send() }, for: .touchUpInside)
        let bar = UIStackView(arrangedSubviews: [footer, UIView(), sendButton])
        bar.alignment = .center
        bar.isLayoutMarginsRelativeArrangement = true
        bar.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 12, leading: 20, bottom: 12, trailing: 20)
        bar.frame = CGRect(x: 0, y: 0, width: 0, height: 64)
        tableView.tableFooterView = bar
        token = app.observeNeedsYou { [weak self] in self?.reload() }
        reload()
    }

    private func reload() {
        entries = app.needsYou.filter { $0.groupKey == groupKey && !$0.isThread }
        picks = picks.filter { key, _ in items.contains { $0.key == key } }
        for entry in entries where picks[entry.key] == nil {
            if case let .failed(choice, _)? = entry.send { picks[entry.key] = choice }
        }
        if entries.isEmpty, navigationController?.topViewController === self {
            navigationController?.popViewController(animated: true)
            return
        }
        tableView.reloadData()
        updateFooter()
    }

    private var items: [NeedsYouEntry] { entries.filter { $0.itemId != nil && !$0.locked } }

    private func updateFooter() {
        footer.text = "\(picks.count) of \(items.count) picked"
        let dest = items.first?.dest ?? "the Lead"
        sendButton.configuration?.title = picks.count > 1 ? "Send \(picks.count) to \(dest)" : "Send to \(dest)"
        sendButton.isEnabled = !picks.isEmpty && !sending
    }

    override func numberOfSections(in tableView: UITableView) -> Int { entries.count }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        guard case let .item(_, _, _, _, options) = entries[section].kind, !entries[section].locked else { return 1 }
        return options.count
    }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        let entry = entries[section]
        return "\(entry.meta(app)) · \(entry.host.name)"
    }

    override func tableView(_ tableView: UITableView, titleForFooterInSection section: Int) -> String? {
        let entry = entries[section]
        if let line = entry.sendLine { return line }
        if case let .item(_, _, _, detail, _) = entry.kind, !detail.isEmpty { return detail }
        return nil
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let entry = entries[indexPath.section]
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        var c = UIListContentConfiguration.cell()
        c.textProperties.numberOfLines = 0
        switch entry.kind {
        case .item where entry.locked:
            c.text = entry.title
            c.secondaryText = entry.sendLine
            c.secondaryTextProperties.color = Palette.secondary
            cell.accessoryType = .none
            cell.accessibilityIdentifier = "needs-you-\(entry.key)"
        case let .item(_, _, _, _, options):
            let option = options[indexPath.row]
            c.text = indexPath.row == 0 ? "\(entry.title)\n\(option)" : option
            if indexPath.row == 0 {
                let text = NSMutableAttributedString(string: entry.title + "\n", attributes: [.font: Fonts.ui(.sansSemibold, 16), .foregroundColor: Palette.text])
                text.append(NSAttributedString(string: option, attributes: [.font: Fonts.ui(.sans, 16), .foregroundColor: Palette.text]))
                c.attributedText = text
            }
            cell.accessoryType = picks[entry.key] == option ? .checkmark : .none
            cell.accessibilityIdentifier = "needs-you-option-\(entry.key)-\(indexPath.row)"
        default:
            c.text = entry.rowTitle
            c.secondaryText = entry.questions.allSatisfy({ !$0.options.isEmpty }) ? "Answer" : "Typed answer · Reply in conversation"
            c.secondaryTextProperties.color = Palette.accent
            cell.accessoryType = .disclosureIndicator
            cell.accessibilityIdentifier = "needs-you-\(entry.key)"
        }
        cell.contentConfiguration = c
        cell.tintColor = Palette.accent
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        let entry = entries[indexPath.section]
        if entry.locked { return }
        guard case let .item(_, _, _, _, options) = entry.kind else {
            guard let next = NeedsYou.controller(for: entry, app: app) else { return NeedsYou.openThread(entry.chatId, from: self) }
            navigationController?.pushViewController(next, animated: true)
            return
        }
        let option = options[indexPath.row]
        picks[entry.key] = picks[entry.key] == option ? nil : option
        tableView.reloadSections([indexPath.section], with: .none)
        updateFooter()
    }

    private func send() {
        let picked = items.compactMap { entry -> (String, ItemAnswer)? in
            guard let choice = picks[entry.key], let itemId = entry.itemId else { return nil }
            return (entry.host.deviceId, ItemAnswer(itemId: itemId, choice: choice))
        }
        guard !picked.isEmpty else { return }
        sending = true
        updateFooter()
        let dest = items.first?.dest ?? "the Lead"
        let window = view.window
        Task { @MainActor in
            var failed = false
            for (deviceId, answers) in Dictionary(grouping: picked, by: { $0.0 }) {
                do {
                    try await app.resolve(deviceId: deviceId, answers.map(\.1))
                } catch {
                    failed = true
                }
            }
            sending = false
            picks = [:]
            reload()
            if !failed {
                (navigationController as? NeedsYouNavigationController)?.sent = items.first
                Toast.show("Sent to \(dest)", in: window, atTop: true)
            }
        }
    }
}

final class NeedsYouPlanController: UIViewController {
    private let app: AppModel
    private let entry: NeedsYouEntry

    init(app: AppModel, entry: NeedsYouEntry) {
        self.app = app
        self.entry = entry
        super.init(nibName: nil, bundle: nil)
        title = entry.taskTitle ?? entry.groupTitle
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        guard case let .plan(taskId, steps) = entry.kind else { return }
        let host = UILabel()
        host.text = entry.host.name
        host.font = Fonts.ui(.sansMedium, 13)
        host.textColor = Palette.secondary
        let headline = UILabel()
        headline.text = entry.title
        headline.font = Fonts.ui(.sansSemibold, 17)
        headline.textColor = Palette.warning
        headline.numberOfLines = 0
        var lines = steps.prefix(4).enumerated().map { "Step \($0.offset + 1) · \($0.element)" }
        if steps.count > 4 { lines.append("+\(steps.count - 4) more") }
        let list = UILabel()
        list.text = lines.joined(separator: "\n")
        list.font = Fonts.ui(.sans, 15)
        list.textColor = Palette.text
        list.numberOfLines = 0
        let open = UIButton(configuration: .gray(), primaryAction: UIAction { [weak self] _ in
            guard let self else { return }
            self.navigationController?.pushViewController(MissionControlViewController(app: self.app, taskId: taskId), animated: true)
        })
        open.configuration?.title = "Open mission"
        open.accessibilityIdentifier = "needs-you-open-mission"
        let approve = UIButton(configuration: .filled(), primaryAction: UIAction { [weak self] _ in self?.approve(taskId) })
        approve.configuration?.title = "Approve and run"
        approve.configuration?.baseBackgroundColor = Palette.accent
        approve.accessibilityIdentifier = "needs-you-approve-plan"
        let buttons = UIStackView(arrangedSubviews: [open, approve])
        buttons.spacing = 10
        buttons.distribution = .fillEqually
        let stack = UIStackView(arrangedSubviews: [host, headline, list, buttons])
        stack.axis = .vertical
        stack.spacing = 12
        stack.setCustomSpacing(20, after: list)
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            stack.leadingAnchor.constraint(equalTo: view.layoutMarginsGuide.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: view.layoutMarginsGuide.trailingAnchor),
        ])
    }

    private func approve(_ taskId: String) {
        guard let client = app.client else { return }
        let window = view.window
        Task { @MainActor in
            do {
                try await client.approvePlan(taskId: taskId)
                (navigationController as? NeedsYouNavigationController)?.sent = entry
                navigationController?.popToRootViewController(animated: true)
                Toast.show("Approved. The lead starts the steps.", in: window, atTop: true)
            } catch {
                Toast.show(TasksErrors.message(error), in: window, atTop: true)
            }
        }
    }
}

final class NeedsYouPill: UIControl {
    private let glyph = UIImageView()
    private let name = UILabel()
    private let kind = UILabel()
    private let more = UILabel()
    private let moreWell = UIView()

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = Palette.elevated.withAlphaComponent(0.96)
        layer.cornerRadius = 16
        layer.cornerCurve = .continuous
        layer.borderWidth = 1
        layer.borderColor = Palette.ask.cgColor
        layer.shadowColor = UIColor.black.cgColor
        layer.shadowOpacity = 0.12
        layer.shadowRadius = 8
        layer.shadowOffset = CGSize(width: 0, height: 3)
        glyph.tintColor = Palette.ask
        glyph.contentMode = .center
        name.font = Fonts.ui(.sansSemibold, 13)
        name.textColor = Palette.text
        name.lineBreakMode = .byTruncatingTail
        kind.font = Fonts.ui(.sansMedium, 13)
        kind.textColor = Palette.secondary
        more.font = Fonts.ui(.sansSemibold, 11)
        more.textColor = Palette.ask
        moreWell.backgroundColor = Palette.ask.withAlphaComponent(0.14)
        moreWell.layer.cornerRadius = 9
        moreWell.addSubview(more)
        let stack = UIStackView(arrangedSubviews: [glyph, name, kind, moreWell])
        stack.spacing = 6
        stack.alignment = .center
        stack.isUserInteractionEnabled = false
        stack.translatesAutoresizingMaskIntoConstraints = false
        more.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        for v in [kind, moreWell] as [UIView] { v.setContentCompressionResistancePriority(.required, for: .horizontal) }
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 32),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            glyph.widthAnchor.constraint(equalToConstant: 17),
            more.leadingAnchor.constraint(equalTo: moreWell.leadingAnchor, constant: 6),
            more.trailingAnchor.constraint(equalTo: moreWell.trailingAnchor, constant: -6),
            more.centerYAnchor.constraint(equalTo: moreWell.centerYAnchor),
            moreWell.heightAnchor.constraint(equalToConstant: 18),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .button
        accessibilityIdentifier = "needs-you-pill"
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (self: NeedsYouPill, _) in
            self.layer.borderColor = Palette.ask.resolvedColor(with: self.traitCollection).cgColor
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override var isHighlighted: Bool {
        didSet { UIView.animate(withDuration: 0.15) { self.alpha = self.isHighlighted ? 0.7 : 1 } }
    }

    func configure(_ waiting: [NeedsYouEntry], app: AppModel) {
        guard let first = waiting.first else { return }
        let title = app.row(first.chatId)?.title ?? first.asker
        glyph.image = UIImage(systemName: first.symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 14, weight: .semibold))
        name.text = title
        kind.text = "· \(first.kindLabel)"
        more.text = "+\(waiting.count - 1)"
        moreWell.isHidden = waiting.count < 2
        accessibilityLabel = "\(title), \(first.kindLabel)" + (waiting.count > 1 ? ", \(waiting.count - 1) more waiting" : "")
    }
}

final class NeedsYouChip: UIControl {
    var count = 0 {
        didSet {
            guard count != oldValue else { return }
            label.text = "\(count) need\(count == 1 ? "s" : "") you"
            accessibilityLabel = label.text
            invalidateIntrinsicContentSize()
        }
    }
    private let label = UILabel()
    private let dot = UIView()

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = Palette.ask.withAlphaComponent(0.14)
        layer.cornerRadius = 13
        layer.cornerCurve = .continuous
        label.font = Fonts.ui(.sansSemibold, 12.5)
        label.textColor = Palette.ask
        dot.backgroundColor = Palette.ask
        dot.layer.cornerRadius = 4
        for v in [dot, label] {
            v.translatesAutoresizingMaskIntoConstraints = false
            v.isUserInteractionEnabled = false
            addSubview(v)
        }
        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 26),
            dot.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 9),
            dot.centerYAnchor.constraint(equalTo: centerYAnchor),
            dot.widthAnchor.constraint(equalToConstant: 8),
            dot.heightAnchor.constraint(equalToConstant: 8),
            label.leadingAnchor.constraint(equalTo: dot.trailingAnchor, constant: 6),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        isAccessibilityElement = true
        accessibilityTraits = .button
        accessibilityIdentifier = "needs-you-chip"
    }

    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: CGSize {
        CGSize(width: 9 + 8 + 6 + ceil(label.intrinsicContentSize.width) + 10, height: 26)
    }

    override var isHighlighted: Bool {
        didSet { UIView.animate(withDuration: 0.15) { self.alpha = self.isHighlighted ? 0.6 : 1 } }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        dot.layer.removeAnimation(forKey: "pulse")
        guard window != nil, !UIAccessibility.isReduceMotionEnabled else { return }
        let pulse = CABasicAnimation(keyPath: "opacity")
        pulse.fromValue = 1
        pulse.toValue = 0.35
        pulse.duration = 0.6
        pulse.autoreverses = true
        pulse.repeatCount = .infinity
        pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        dot.layer.add(pulse, forKey: "pulse")
    }
}
