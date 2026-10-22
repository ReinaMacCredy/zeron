import UIKit

final class PullRequestViewController: UIViewController, UITableViewDataSource, UITableViewDelegate, UITextViewDelegate {
    enum Tab: Int { case summary, code, activity }

    private let app: AppModel
    private let deviceId: String
    private let page: PullRequestPage?
    private var selected: Tab = .summary
    private var detail: PrDetailView?
    private var controls: PrControlsView?
    private var code: [DiffItem] = []
    private var codeLoaded = false
    private var failure: String?
    private var busy = false

    private let table = UITableView(frame: .zero, style: .insetGrouped)
    private let tabs = UISegmentedControl(items: ["Summary", "Code", "Activity"])
    private let titleLabel = UILabel()
    private let metaLabel = UILabel()
    private let askButton = UIButton(configuration: .tinted())
    private let composer = UIView()
    private let input = UITextView()
    private let placeholder = UILabel()
    private let reviewButton = UIButton(configuration: .gray())
    private let sendButton = UIButton(configuration: .filled())
    private var composerBottom: NSLayoutConstraint!

    init(app: AppModel, deviceId: String, url: String) {
        self.app = app
        self.deviceId = deviceId
        self.page = app.client?.pullRequestPage(deviceId: deviceId, url: url)
        super.init(nibName: nil, bundle: nil)
        title = pullRequestLink(url: url).map { "#\($0.number)" } ?? "Pull request"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        navigationItem.rightBarButtonItem = UIBarButtonItem(image: UIImage(systemName: "arrow.clockwise"), primaryAction: UIAction { [weak self] _ in self?.reload(refresh: true) })

        titleLabel.font = Fonts.ui(.sansSemibold, 22)
        titleLabel.textColor = Palette.text
        titleLabel.numberOfLines = 0
        titleLabel.accessibilityIdentifier = "pr-title"
        metaLabel.font = Fonts.ui(.sans, 13)
        metaLabel.textColor = Palette.secondary
        metaLabel.numberOfLines = 0
        askButton.configuration?.title = "Ask about this PR"
        askButton.configuration?.image = UIImage(systemName: "bubble.left.and.text.bubble.right")
        askButton.configuration?.imagePadding = 6
        askButton.configuration?.cornerStyle = .capsule
        askButton.accessibilityIdentifier = "pr-ask"
        askButton.addAction(UIAction { [weak self] _ in self?.ask() }, for: .touchUpInside)
        tabs.selectedSegmentIndex = 0
        tabs.accessibilityIdentifier = "pr-tabs"
        tabs.addAction(UIAction { [weak self] _ in self?.switchTab() }, for: .valueChanged)
        let head = UIStackView(arrangedSubviews: [titleLabel, metaLabel, askButton, tabs])
        head.axis = .vertical
        head.alignment = .leading
        head.spacing = 8
        head.setCustomSpacing(12, after: askButton)
        tabs.widthAnchor.constraint(equalTo: head.widthAnchor).isActive = true
        head.isLayoutMarginsRelativeArrangement = true
        head.directionalLayoutMargins = .init(top: 12, leading: 20, bottom: 8, trailing: 20)
        let headHost = UIView()
        head.translatesAutoresizingMaskIntoConstraints = false
        headHost.addSubview(head)
        NSLayoutConstraint.activate([
            head.topAnchor.constraint(equalTo: headHost.topAnchor),
            head.leadingAnchor.constraint(equalTo: headHost.leadingAnchor),
            head.trailingAnchor.constraint(equalTo: headHost.trailingAnchor),
            head.bottomAnchor.constraint(equalTo: headHost.bottomAnchor),
        ])
        table.tableHeaderView = headHost

        table.dataSource = self
        table.delegate = self
        table.backgroundColor = .clear
        table.keyboardDismissMode = .interactive
        table.accessibilityIdentifier = "pr-detail"
        table.register(UITableViewCell.self, forCellReuseIdentifier: "row")
        DiffPainter.register(table)
        buildComposer()
        for v in [table, composer] {
            v.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(v)
        }
        composerBottom = composer.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor)
        NSLayoutConstraint.activate([
            table.topAnchor.constraint(equalTo: view.topAnchor),
            table.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            table.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            table.bottomAnchor.constraint(equalTo: composer.topAnchor),
            composer.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            composer.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            composerBottom,
        ])
        switchTab()
        reload(refresh: false)
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        guard let head = table.tableHeaderView else { return }
        let size = head.systemLayoutSizeFitting(CGSize(width: table.bounds.width, height: 0), withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel)
        if head.frame.height != size.height {
            head.frame.size = CGSize(width: table.bounds.width, height: size.height)
            table.tableHeaderView = head
        }
    }

    private func buildComposer() {
        composer.backgroundColor = Palette.elevated
        input.font = Fonts.ui(.sans, 15)
        input.backgroundColor = Palette.controlFill
        input.layer.cornerRadius = 10
        input.textContainerInset = .init(top: 8, left: 6, bottom: 8, right: 6)
        input.isScrollEnabled = false
        input.delegate = self
        input.accessibilityIdentifier = "pr-comment"
        placeholder.text = "Write a comment…"
        placeholder.font = Fonts.ui(.sans, 15)
        placeholder.textColor = Palette.tertiary
        reviewButton.configuration?.title = "Review"
        reviewButton.configuration?.cornerStyle = .capsule
        reviewButton.showsMenuAsPrimaryAction = true
        reviewButton.accessibilityIdentifier = "pr-review"
        sendButton.configuration?.image = UIImage(systemName: "arrow.up")
        sendButton.configuration?.cornerStyle = .capsule
        sendButton.accessibilityIdentifier = "pr-send"
        sendButton.addAction(UIAction { [weak self] _ in self?.sendComment() }, for: .touchUpInside)
        let row = UIStackView(arrangedSubviews: [input, reviewButton, sendButton])
        row.spacing = 8
        row.alignment = .bottom
        for v in [row, placeholder] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            composer.addSubview(v)
        }
        NSLayoutConstraint.activate([
            row.topAnchor.constraint(equalTo: composer.topAnchor, constant: 8),
            row.leadingAnchor.constraint(equalTo: composer.leadingAnchor, constant: 12),
            row.trailingAnchor.constraint(equalTo: composer.trailingAnchor, constant: -12),
            row.bottomAnchor.constraint(equalTo: composer.safeAreaLayoutGuide.bottomAnchor, constant: -8),
            input.heightAnchor.constraint(greaterThanOrEqualToConstant: 38),
            input.heightAnchor.constraint(lessThanOrEqualToConstant: 120),
            placeholder.leadingAnchor.constraint(equalTo: input.leadingAnchor, constant: 11),
            placeholder.centerYAnchor.constraint(equalTo: input.centerYAnchor),
        ])
        updateComposer()
    }

    func textViewDidChange(_ textView: UITextView) {
        updateComposer()
    }

    private func updateComposer() {
        let text = input.text.trimmingCharacters(in: .whitespacesAndNewlines)
        placeholder.isHidden = !input.text.isEmpty
        sendButton.isEnabled = !text.isEmpty && !busy
        let judge = detail?.canJudge ?? false
        reviewButton.menu = UIMenu(title: "The text in the box becomes the review summary.", children: [
            UIAction(title: "Approve", image: UIImage(systemName: "checkmark.circle"), attributes: judge ? [] : .disabled) { [weak self] _ in self?.submitReview(.approve) },
            UIAction(title: "Request changes", image: UIImage(systemName: "exclamationmark.bubble"), attributes: judge && !text.isEmpty ? [] : .disabled) { [weak self] _ in self?.submitReview(.requestChanges) },
            UIAction(title: "Comment as review", image: UIImage(systemName: "text.bubble"), attributes: text.isEmpty ? .disabled : []) { [weak self] _ in self?.submitReview(.comment) },
        ])
    }

    private func reload(refresh: Bool) {
        guard let page else { return }
        Task { @MainActor in
            do {
                let detail = try await page.load(refresh: refresh)
                self.show(detail)
                self.failure = nil
                if self.controls == nil || refresh { self.controls = try? await page.controls() }
                if refresh || self.selected == .code { await self.loadCode(refresh: refresh) }
            } catch {
                self.failure = GitText.failure(error, fallback: "Couldn't load this pull request. Try again.")
            }
            self.table.reloadData()
        }
    }

    private func show(_ detail: PrDetailView) {
        self.detail = detail
        title = "#\(detail.number)"
        titleLabel.text = detail.title
        metaLabel.text = "\(detail.repository) · \(detail.author) · #\(detail.number) · Loaded just now"
        tabs.setTitle("Code \(detail.fileCount)", forSegmentAt: Tab.code.rawValue)
        tabs.setTitle("Activity \(detail.activity.count)", forSegmentAt: Tab.activity.rawValue)
        view.setNeedsLayout()
        updateComposer()
    }

    private func loadCode(refresh: Bool) async {
        guard let page else { return }
        do {
            code = DiffItem.rows(try await page.code(refresh: refresh).files)
        } catch {
            code = [.notice(file: "", text: GitText.failure(error, fallback: "Couldn't load the diff. Try again."))]
        }
        codeLoaded = true
        if selected == .code { table.reloadData() }
    }

    private func switchTab() {
        selected = Tab(rawValue: tabs.selectedSegmentIndex) ?? .summary
        composer.isHidden = selected != .activity
        composerBottom.isActive = selected == .activity
        table.separatorStyle = selected == .code ? .none : .singleLine
        if selected == .code, !codeLoaded, detail != nil {
            Task { @MainActor in await self.loadCode(refresh: false) }
        }
        table.reloadData()
    }

    private enum Row {
        case info(String, NSAttributedString, id: String, tap: (() -> Void)? = nil)
        case text(String, color: UIColor = Palette.text, id: String? = nil)
        case action(String, symbol: String, id: String, tap: () -> Void, destructive: Bool = false)
        case check(PrCheckRow)
        case activity(PrActivity)
        case diff(DiffItem)
    }

    private struct Section {
        var header: String?
        var footer: String?
        var rows: [Row]
    }

    private var sections: [Section] {
        if let failure, detail == nil { return [Section(rows: [.text(failure, color: Palette.danger, id: "pr-failure")])] }
        guard let d = detail else { return [Section(rows: [.text("Loading…", color: Palette.secondary)])] }
        switch selected {
        case .summary: return summary(d)
        case .code:
            guard codeLoaded else { return [Section(rows: [.text("Loading the diff…", color: Palette.secondary)])] }
            return [Section(rows: code.map { .diff($0) })]
        case .activity:
            if d.activity.isEmpty { return [Section(rows: [.text("No comments or reviews yet.", color: Palette.secondary)])] }
            return [Section(header: d.openThreads > 0 ? "\(d.openThreads) open conversation\(d.openThreads == 1 ? "" : "s")" : nil, rows: d.activity.map { .activity($0) })]
        }
    }

    private func plain(_ text: String, color: UIColor = Palette.text) -> NSAttributedString {
        NSAttributedString(string: text, attributes: [.foregroundColor: color, .font: Fonts.ui(.sans, 14)])
    }

    private func summary(_ d: PrDetailView) -> [Section] {
        var out: [Section] = []
        var overview: [Row] = [.info("Status", GitText.chip(d.state), id: "pr-status")]
        if let review = d.review { overview.append(.info("Review", GitText.chip(review), id: "pr-review-state")) }
        overview.append(.info("Branch", plain(d.branch, color: Palette.secondary), id: "pr-branch"))
        overview.append(.info("Changes", {
            let s = NSMutableAttributedString(attributedString: plain("\(d.fileCount) file\(d.fileCount == 1 ? "" : "s") · ", color: Palette.secondary))
            s.append(GitText.changes(d.changes))
            return s
        }(), id: "pr-changes"))
        let labels = controls?.labels.filter(\.applied).map(\.name) ?? []
        let reviewers = controls?.reviewers.filter(\.requested).map(\.login) ?? []
        let triage = controls?.canTriage ?? false
        overview.append(.info("Labels", plain(labels.isEmpty ? "None" : labels.joined(separator: ", "), color: Palette.secondary), id: "pr-labels", tap: triage ? { [weak self] in self?.pickLabels() } : nil))
        overview.append(.info("Reviewers", plain(reviewers.isEmpty ? "None" : reviewers.joined(separator: ", "), color: Palette.secondary), id: "pr-reviewers", tap: triage ? { [weak self] in self?.pickReviewers() } : nil))
        out.append(Section(header: "Overview", rows: overview))

        var assessment: [Row] = [.info("CI", GitText.chip(d.ci), id: "pr-ci")]
        assessment += d.blockers.map { .text($0, color: Palette.danger) }
        assessment += d.attention.map { .text($0, color: Palette.warning) }
        assessment += d.missing.map { .text($0, color: Palette.secondary) }
        if let next = d.handoffs.first { assessment.append(.text("Next: \(next.label)", color: Palette.secondary, id: "pr-next")) }
        out.append(Section(header: "Current assessment", rows: assessment))

        if d.isOpen, controls?.canWrite == true, let methods = controls?.mergeMethods, !methods.isEmpty {
            out.append(Section(header: "Merge pull request", footer: "Into the base branch once you confirm.", rows: methods.map { m in
                .action(m.label, symbol: "arrow.triangle.merge", id: "pr-merge-\(m.method)", tap: { [weak self] in self?.confirmMerge(m.method) })
            }))
        }

        out.append(Section(header: "Checks · \(d.checks.count)", footer: d.checksEmpty, rows: d.checks.map { .check($0) }))

        if !d.handoffs.isEmpty {
            out.append(Section(header: "Hand off to an agent", footer: "Opens a new session with the prompt ready to send.", rows: d.handoffs.map { h in
                .action(h.label, symbol: "arrow.up.forward.app", id: "pr-handoff-\(h.kind)", tap: { [weak self] in self?.handoff(h.kind) })
            }))
        }
        if !d.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            out.append(Section(header: "Description", rows: [.text(d.body, color: Palette.text, id: "pr-body")]))
        }
        return out
    }

    func numberOfSections(in tableView: UITableView) -> Int { sections.count }
    func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { sections[section].rows.count }
    func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? { sections[section].header }
    func tableView(_ tableView: UITableView, titleForFooterInSection section: Int) -> String? { sections[section].footer }

    func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let row = sections[indexPath.section].rows[indexPath.row]
        if case let .diff(item) = row {
            return DiffPainter.cell(tableView, indexPath, item) { [weak self] cell, thread in self?.paintThread(cell, thread) }
        }
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        cell.backgroundColor = Palette.elevated
        cell.accessoryView = nil
        cell.accessoryType = .none
        cell.selectionStyle = .none
        cell.accessibilityIdentifier = nil
        switch row {
        case let .info(label, value, id, tap):
            var c = UIListContentConfiguration.valueCell()
            c.text = label
            c.textProperties.font = Fonts.ui(.sans, 15)
            c.textProperties.color = Palette.secondary
            c.secondaryAttributedText = value
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = id
            if tap != nil {
                cell.accessoryType = .disclosureIndicator
                cell.selectionStyle = .default
            }
        case let .text(text, color, id):
            var c = UIListContentConfiguration.cell()
            c.text = text
            c.textProperties.font = Fonts.ui(.sans, 15)
            c.textProperties.color = color
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = id
        case let .action(title, symbol, id, _, destructive):
            var c = UIListContentConfiguration.cell()
            c.text = title
            c.textProperties.font = Fonts.ui(.sansMedium, 16)
            c.textProperties.color = destructive ? Palette.danger : Palette.accent
            c.image = UIImage(systemName: symbol)
            c.imageProperties.tintColor = destructive ? Palette.danger : Palette.accent
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = id
            cell.selectionStyle = .default
        case let .check(check):
            var c = UIListContentConfiguration.valueCell()
            c.text = check.name
            c.textProperties.font = Fonts.ui(.sans, 15)
            c.secondaryAttributedText = GitText.chip(check.chip)
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = "pr-check-\(check.name)"
        case let .activity(item):
            paintActivity(cell, item)
        case .diff:
            break
        }
        return cell
    }

    func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        switch sections[indexPath.section].rows[indexPath.row] {
        case let .info(_, _, _, tap): tap?()
        case let .action(_, _, _, tap, _): tap()
        case let .check(check):
            if let url = check.url.flatMap(URL.init(string:)) { UIApplication.shared.open(url) }
        default: break
        }
    }

    private func paintActivity(_ cell: UITableViewCell, _ item: PrActivity) {
        var c = UIListContentConfiguration.subtitleCell()
        let head = NSMutableAttributedString(string: item.comment.author, attributes: [.font: Fonts.ui(.sansSemibold, 14), .foregroundColor: Palette.text])
        if let state = item.state {
            head.append(NSAttributedString(string: "  "))
            head.append(GitText.chip(state))
        }
        head.append(NSAttributedString(string: "  \(item.comment.when)", attributes: [.font: Fonts.ui(.sans, 12), .foregroundColor: Palette.tertiary]))
        c.attributedText = head
        var body = item.comment.body
        if let thread = item.thread {
            let place = thread.line.map { "\(thread.path):\($0)" } ?? thread.path
            let state = thread.resolved ? "Resolved" : thread.outdated ? "Outdated" : "Unresolved"
            body = "\(place) · \(state)\n\(body)"
            for reply in thread.comments.dropFirst() {
                body += "\n\n\(reply.author) · \(reply.when)\n\(reply.body)"
            }
        }
        c.secondaryText = body
        c.secondaryTextProperties.font = Fonts.ui(.sans, 14)
        c.secondaryTextProperties.color = Palette.text
        c.textToSecondaryTextVerticalPadding = 6
        cell.contentConfiguration = c
        cell.accessibilityIdentifier = "pr-activity-\(item.id)"
        if let thread = item.thread {
            cell.accessoryView = threadButtons(thread)
        }
    }

    private func paintThread(_ cell: UITableViewCell, _ thread: PrThread) {
        var c = UIListContentConfiguration.subtitleCell()
        c.text = "\(thread.comments.first?.author ?? "") · \(thread.resolved ? "Resolved" : "Unresolved")"
        c.textProperties.font = Fonts.ui(.sansSemibold, 13)
        c.secondaryText = thread.comments.map { "\($0.author): \($0.body)" }.joined(separator: "\n")
        c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
        c.directionalLayoutMargins = .init(top: 8, leading: 72, bottom: 8, trailing: 12)
        cell.contentConfiguration = c
        cell.backgroundColor = Palette.color(.cardBackground)
        cell.accessoryView = threadButtons(thread)
        cell.accessibilityIdentifier = "pr-line-thread-\(thread.id)"
    }

    private func threadButtons(_ thread: PrThread) -> UIView {
        let reply = UIButton(configuration: .borderless())
        reply.configuration?.title = "Reply"
        reply.accessibilityIdentifier = "pr-reply-\(thread.id)"
        reply.addAction(UIAction { [weak self] _ in self?.reply(thread) }, for: .touchUpInside)
        let resolve = UIButton(configuration: .borderless())
        resolve.configuration?.title = thread.resolved ? "Unresolve" : "Resolve"
        resolve.accessibilityIdentifier = "pr-resolve-\(thread.id)"
        resolve.addAction(UIAction { [weak self] _ in self?.resolve(thread) }, for: .touchUpInside)
        let stack = UIStackView(arrangedSubviews: [reply, resolve])
        stack.axis = .vertical
        stack.spacing = 0
        stack.frame = CGRect(x: 0, y: 0, width: 96, height: 72)
        return stack
    }

    private func write(_ toast: String?, _ work: @escaping (PullRequestPage) async throws -> Void) {
        guard let page, !busy else { return }
        busy = true
        updateComposer()
        Task { @MainActor in
            defer {
                self.busy = false
                self.updateComposer()
                self.table.reloadData()
            }
            do {
                try await work(page)
                if let toast { Toast.show(toast, in: self.view.window) }
            } catch {
                Toast.show(GitText.failure(error, fallback: "GitHub refused the change. Try again."), in: self.view.window)
            }
        }
    }

    private func sendComment() {
        let body = input.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !body.isEmpty else { return }
        write("Comment posted") { [weak self] page in
            let detail = try await page.comment(body: body)
            self?.input.text = ""
            self?.input.resignFirstResponder()
            self?.show(detail)
        }
    }

    private func submitReview(_ event: PrReviewEvent) {
        let body = input.text.trimmingCharacters(in: .whitespacesAndNewlines)
        let toast = switch event {
        case .approve: "Approved"
        case .requestChanges: "Changes requested"
        case .comment: "Review submitted"
        }
        write(toast) { [weak self] page in
            let detail = try await page.review(event: event, body: body)
            self?.input.text = ""
            self?.input.resignFirstResponder()
            self?.show(detail)
        }
    }

    private func reply(_ thread: PrThread) {
        let alert = UIAlertController(title: "Reply", message: thread.line.map { "\(thread.path):\($0)" } ?? thread.path, preferredStyle: .alert)
        alert.addTextField { $0.placeholder = "Reply…"; $0.accessibilityIdentifier = "pr-reply-text" }
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        alert.addAction(UIAlertAction(title: "Reply", style: .default) { [weak self, weak alert] _ in
            let body = alert?.textFields?.first?.text?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            guard !body.isEmpty else { return }
            self?.write("Reply posted") { page in
                let detail = try await page.reply(threadId: thread.id, body: body)
                self?.show(detail)
            }
        })
        present(alert, animated: true)
    }

    private func resolve(_ thread: PrThread) {
        write(thread.resolved ? "Conversation unresolved" : "Conversation resolved") { [weak self] page in
            let detail = try await page.resolve(threadId: thread.id, resolved: !thread.resolved)
            self?.show(detail)
        }
    }

    private func confirmMerge(_ method: PrMergeMethod) {
        guard let page else { return }
        let confirm = page.mergeConfirm(method: method)
        let alert = UIAlertController(title: confirm.title, message: confirm.message, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        alert.addAction(UIAlertAction(title: confirm.action, style: .default) { [weak self] _ in
            self?.write("Pull request merged") { page in
                let detail = try await page.merge(method: method)
                self?.show(detail)
            }
        })
        present(alert, animated: true)
    }

    private func pickLabels() {
        guard let controls else { return }
        let picker = PickerSheet(title: "Change labels", search: "Search labels", items: controls.labels.map { .init(id: $0.name, title: $0.name, detail: nil, on: $0.applied) }) { [weak self] item in
            self?.write(item.on ? "Removed \(item.title)" : "Added \(item.title)") { page in
                self?.controls = try await page.setLabel(name: item.id, add: !item.on)
            }
            return !item.on
        }
        presentSheet(picker)
    }

    private func pickReviewers() {
        guard let controls else { return }
        let picker = PickerSheet(title: "Request a review", search: "Search people with access", items: controls.reviewers.map { .init(id: $0.login, title: $0.login, detail: $0.team ? "Team" : nil, on: $0.requested) }) { [weak self] item in
            let team = controls.reviewers.first { $0.login == item.id }?.team ?? false
            self?.write(item.on ? "Review request removed from \(item.title)" : "Review requested from \(item.title)") { page in
                self?.controls = try await page.setReviewer(login: item.id, team: team, add: !item.on)
            }
            return !item.on
        }
        presentSheet(picker)
    }

    private func presentSheet(_ vc: UIViewController) {
        let nav = UINavigationController(rootViewController: vc)
        if let sheet = nav.sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
        }
        present(nav, animated: true)
    }

    private func handoff(_ kind: PrHandoffKind) {
        guard let page, let prompt = try? page.handoffPrompt(kind: kind) else { return }
        Task { @MainActor in
            await self.startSession(prompt: prompt, page: page)
        }
    }

    private func ask() {
        guard let page, let context = try? page.askContext() else { return }
        Task { @MainActor in
            await self.startSession(prompt: context.block + "\n\n", page: page)
        }
    }

    private func startSession(prompt: String, page: PullRequestPage) async {
        let project = await page.handoffProject()
        var draft = app.lastDraft
        draft.projectId = project
        draft.hostId = project == nil ? deviceId : nil
        app.lastDraft = draft
        (view.window?.rootViewController as? AppRouter)?.presentNewSession(prompt: prompt)
    }
}

final class PickerSheet: UITableViewController, UISearchResultsUpdating {
    struct Item {
        let id: String
        let title: String
        let detail: String?
        var on: Bool
    }

    private var items: [Item]
    private var query = ""
    private let search: String
    private let toggle: (Item) -> Bool

    init(title: String, search: String, items: [Item], toggle: @escaping (Item) -> Bool) {
        self.items = items
        self.search = search
        self.toggle = toggle
        super.init(style: .insetGrouped)
        self.title = title
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "row")
        tableView.accessibilityIdentifier = "picker-sheet"
        let controller = UISearchController(searchResultsController: nil)
        controller.searchResultsUpdater = self
        controller.obscuresBackgroundDuringPresentation = false
        controller.searchBar.placeholder = search
        navigationItem.searchController = controller
        navigationItem.hidesSearchBarWhenScrolling = false
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .done, primaryAction: UIAction { [weak self] _ in self?.dismiss(animated: true) })
    }

    private var shown: [Int] {
        items.indices.filter { query.isEmpty || items[$0].title.localizedCaseInsensitiveContains(query) }
    }

    func updateSearchResults(for controller: UISearchController) {
        query = controller.searchBar.text ?? ""
        tableView.reloadData()
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { shown.count }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let item = items[shown[indexPath.row]]
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        var c = UIListContentConfiguration.subtitleCell()
        c.text = item.title
        c.secondaryText = item.detail
        c.secondaryTextProperties.color = Palette.secondary
        cell.contentConfiguration = c
        cell.accessoryType = item.on ? .checkmark : .none
        cell.accessibilityIdentifier = "picker-\(item.id)"
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        let i = shown[indexPath.row]
        items[i].on = toggle(items[i])
        tableView.reloadRows(at: [indexPath], with: .none)
    }
}
