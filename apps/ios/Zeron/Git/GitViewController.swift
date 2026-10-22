import UIKit

final class GitViewController: UIViewController, UITableViewDataSource, UITableViewDelegate {
    let deviceId: String
    let projectId: String?
    private let app: AppModel
    private var checkoutPath: String?
    private var checkout: GitCheckout?
    private var tree: GitTreeView?
    private var history: GitHistoryView?
    private var showingHistory = false
    private var failure: String?
    private var busy = false

    private let table = UITableView(frame: .zero, style: .insetGrouped)
    private let branchLabel = UILabel()
    private let aheadLabel = UILabel()
    private let picker = UIButton(configuration: .plain())
    private let banner = UILabel()
    private let pullButton = UIButton(configuration: .gray())
    private let pushButton = UIButton(configuration: .gray())
    private let commitButton = UIButton(configuration: .filled())
    private let mode = UISegmentedControl(items: ["Working tree", "History"])

    init(app: AppModel, deviceId: String, projectId: String?) {
        self.app = app
        self.deviceId = deviceId
        self.projectId = projectId
        super.init(nibName: nil, bundle: nil)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .clear
        branchLabel.font = Fonts.ui(.monoSemibold, 20)
        branchLabel.textColor = Palette.text
        branchLabel.accessibilityIdentifier = "git-branch"
        aheadLabel.font = Fonts.ui(.sansMedium, 13)
        aheadLabel.textColor = Palette.secondary
        aheadLabel.accessibilityIdentifier = "git-ahead"
        picker.showsMenuAsPrimaryAction = true
        picker.configuration?.contentInsets = .zero
        picker.configuration?.imagePlacement = .trailing
        picker.configuration?.imagePadding = 4
        picker.configuration?.image = UIImage(systemName: "chevron.up.chevron.down", withConfiguration: UIImage.SymbolConfiguration(pointSize: 11))
        picker.contentHorizontalAlignment = .leading
        picker.accessibilityIdentifier = "git-checkout"
        banner.font = Fonts.ui(.sans, 13)
        banner.textColor = Palette.warning
        banner.numberOfLines = 0
        banner.accessibilityIdentifier = "git-agent-banner"
        for (button, title, symbol, id) in [(pullButton, "Pull", "arrow.down", "git-pull"), (pushButton, "Push", "arrow.up", "git-push"), (commitButton, "Commit", "checkmark", "git-commit")] {
            button.configuration?.title = title
            button.configuration?.image = UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 12, weight: .semibold))
            button.configuration?.imagePadding = 5
            button.configuration?.cornerStyle = .capsule
            button.accessibilityIdentifier = id
        }
        pullButton.addAction(UIAction { [weak self] _ in self?.pull() }, for: .touchUpInside)
        pushButton.addAction(UIAction { [weak self] _ in self?.confirmPush() }, for: .touchUpInside)
        commitButton.addAction(UIAction { [weak self] _ in self?.openCommit() }, for: .touchUpInside)
        let buttons = UIStackView(arrangedSubviews: [pullButton, pushButton, commitButton, UIView()])
        buttons.spacing = 8
        mode.selectedSegmentIndex = 0
        mode.accessibilityIdentifier = "git-mode"
        mode.addAction(UIAction { [weak self] _ in self?.switchMode() }, for: .valueChanged)
        let head = UIStackView(arrangedSubviews: [branchLabel, aheadLabel, picker, banner, buttons, mode])
        head.axis = .vertical
        head.spacing = 6
        head.setCustomSpacing(12, after: banner)
        head.setCustomSpacing(12, after: buttons)
        head.isLayoutMarginsRelativeArrangement = true
        head.directionalLayoutMargins = .init(top: 4, leading: 20, bottom: 4, trailing: 20)
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
        table.accessibilityIdentifier = "git-table"
        table.register(UITableViewCell.self, forCellReuseIdentifier: "row")
        table.register(GitHistoryCell.self, forCellReuseIdentifier: GitHistoryCell.id)
        table.refreshControl = UIRefreshControl(frame: .zero, primaryAction: UIAction { [weak self] _ in self?.reload() })
        table.frame = view.bounds
        table.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(table)
        openCheckout(nil)
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

    private func openCheckout(_ path: String?) {
        checkoutPath = path
        tree = nil
        history = nil
        failure = nil
        guard let projectId, let client = app.client else {
            failure = "No Git project on this Mac. Add a project with a Git checkout first."
            return paint()
        }
        checkout = try? client.gitCheckout(projectId: projectId, checkoutPath: path)
        paint()
        reload()
    }

    func reload() {
        guard let checkout else { return }
        Task { @MainActor in
            do {
                self.tree = try await checkout.refresh()
                self.failure = nil
                if self.showingHistory || self.history != nil { self.history = try await checkout.history(more: false) }
            } catch {
                self.failure = GitText.failure(error, fallback: "Couldn't read this checkout. Try again.")
            }
            self.table.refreshControl?.endRefreshing()
            self.paint()
        }
    }

    func showHistory() {
        mode.selectedSegmentIndex = 1
        switchMode()
    }

    private func switchMode() {
        showingHistory = mode.selectedSegmentIndex == 1
        if showingHistory, history == nil, let checkout {
            Task { @MainActor in
                do { self.history = try await checkout.history(more: false) } catch {
                    self.failure = GitText.failure(error, fallback: "Couldn't read the history. Try again.")
                }
                self.paint()
            }
        }
        paint()
    }

    private func paint() {
        let options = projectId.map { app.client?.gitCheckouts(projectId: $0) ?? [] } ?? []
        let current = options.first { $0.path == checkoutPath }
        let device = app.client?.devices().first { $0.id == deviceId }?.name ?? "Mac"
        let project = app.projectOptions.first { $0.id == projectId }?.name ?? ""
        let kind = checkoutPath == nil ? "main checkout" : "agent worktree"
        picker.configuration?.attributedTitle = AttributedString("\(device) · \(project) · \(kind)", attributes: AttributeContainer([.font: Fonts.ui(.sans, 13), .foregroundColor: Palette.secondary]))
        let main = options.filter { $0.path == nil }
        let worktrees = options.filter { $0.path != nil }
        picker.menu = UIMenu(children: [
            UIMenu(title: "Main checkout", options: .displayInline, children: main.map { option in
                UIAction(title: option.label, subtitle: option.detail, state: option.path == checkoutPath ? .on : .off) { [weak self] _ in self?.openCheckout(nil) }
            }),
            UIMenu(title: "Agent worktrees", options: .displayInline, children: worktrees.isEmpty ? [UIAction(title: "No agent worktrees", attributes: .disabled) { _ in }] : worktrees.map { option in
                UIAction(title: option.label, subtitle: option.detail, image: option.live ? UIImage(systemName: "circle.fill") : nil, state: option.path == checkoutPath ? .on : .off) { [weak self] _ in self?.openCheckout(option.path) }
            }),
        ])
        branchLabel.text = tree?.branch.isEmpty == false ? tree?.branch : current?.label ?? "Git"
        aheadLabel.text = tree.map { "\($0.ahead) ahead · \($0.behind) behind" + ($0.upstream.map { " · \($0)" } ?? "") }
        banner.text = tree?.agentBanner
        banner.isHidden = tree?.agentBanner == nil
        pullButton.configuration?.showsActivityIndicator = false
        pullButton.isEnabled = tree != nil && !busy && tree?.pullRefusal == nil
        let ahead = tree?.ahead ?? 0
        pushButton.configuration?.title = ahead > 0 ? "Push \(ahead)" : "Push"
        pushButton.isEnabled = tree != nil && !busy && tree?.pushRefusal == nil
        commitButton.isEnabled = tree != nil && !busy
        if let notice = tree?.notice, let checkout {
            Toast.show(notice.text, in: view.window)
            tree = checkout.dismissNotice()
        }
        view.setNeedsLayout()
        table.reloadData()
    }

    private func run(_ work: @escaping (GitCheckout) async throws -> GitTreeView) {
        guard let checkout, !busy else { return }
        busy = true
        paint()
        Task { @MainActor in
            do {
                self.tree = try await work(checkout)
                if self.history != nil { self.history = try? await checkout.history(more: false) }
            } catch {
                Toast.show(GitText.failure(error, fallback: "Git refused the change. Try again."), in: self.view.window)
            }
            self.busy = false
            self.paint()
        }
    }

    private func stage(_ row: GitFileRow) {
        run { row.staged ? try await $0.unstage(paths: [row.path]) : try await $0.stage(paths: [row.path]) }
    }

    private func pull() {
        if let refusal = tree?.pullRefusal { return Toast.show(refusal, in: view.window) }
        run { try await $0.pull() }
    }

    private func confirmPush() {
        guard let tree else { return }
        if let refusal = tree.pushRefusal { return Toast.show(refusal, in: view.window) }
        let alert = UIAlertController(title: tree.pushPrompt ?? "Push \(tree.branch)?", message: picker.configuration?.attributedTitle.map { String($0.characters) }, preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "Close", style: .cancel))
        alert.addAction(UIAlertAction(title: "Push", style: .default) { [weak self] _ in
            self?.pushButton.configuration?.title = "Pushing…"
            self?.run { try await $0.push() }
        })
        present(alert, animated: true)
    }

    private func openCommit() {
        guard let checkout, let tree else { return }
        let sheet = GitCommitSheet(checkout: checkout, tree: tree, where: picker.configuration?.attributedTitle.map { String($0.characters) } ?? "") { [weak self] next in
            self?.tree = next
            if self?.history != nil {
                Task { @MainActor in
                    self?.history = try? await checkout.history(more: false)
                    self?.paint()
                }
            }
            self?.paint()
        }
        let nav = UINavigationController(rootViewController: sheet)
        if let s = nav.sheetPresentationController {
            s.detents = [.medium(), .large()]
            s.prefersGrabberVisible = true
        }
        present(nav, animated: true)
    }

    private enum Row {
        case notice(String)
        case file(GitFileRow)
        case none(String)
        case stash(String)
        case uncommitted(Int)
        case commit(GitHistoryRow)
        case more
    }

    private struct Section {
        var header: String?
        var action: (String, String, () -> Void)?
        var rows: [Row]
    }

    private var sections: [Section] {
        if let failure, tree == nil { return [Section(rows: [.notice(failure)])] }
        guard let tree else { return [Section(rows: [.notice("Reading the checkout…")])] }
        if showingHistory {
            guard let history else { return [Section(rows: [.notice(failure ?? "Reading the history…")])] }
            let pending = tree.staged.count + tree.changes.count
            var rows: [Row] = pending > 0 ? [.uncommitted(pending)] : []
            rows += history.rows.map { .commit($0) }
            if history.canLoadMore { rows.append(.more) }
            return [Section(header: "\(history.rows.count) commits · \(tree.ahead) ahead · \(tree.behind) behind", rows: rows)]
        }
        var out: [Section] = []
        if let clean = tree.clean {
            out.append(Section(rows: [.notice(clean)]))
        } else {
            let unstageAll: () -> Void = { [weak self] in self?.run { try await $0.unstageAll() } }
            let stageAll: () -> Void = { [weak self] in self?.run { try await $0.stageAll() } }
            let staged: [Row] = tree.staged.isEmpty ? [.none("No staged files")] : tree.staged.map { .file($0) }
            let changes: [Row] = tree.changes.isEmpty ? [.none("No changes")] : tree.changes.map { .file($0) }
            out.append(Section(header: "Staged · \(tree.staged.count)", action: tree.staged.isEmpty ? nil : ("Unstage all", "git-unstage-all", unstageAll), rows: staged))
            out.append(Section(header: "Changes · \(tree.changes.count)", action: tree.changes.isEmpty ? nil : ("Stage all", "git-stage-all", stageAll), rows: changes))
        }
        if !tree.stashes.isEmpty {
            out.append(Section(header: "Stashes · \(tree.stashes.count)", rows: tree.stashes.map { .stash($0) }))
        }
        return out
    }

    func numberOfSections(in tableView: UITableView) -> Int { sections.count }
    func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { sections[section].rows.count }

    func tableView(_ tableView: UITableView, viewForHeaderInSection section: Int) -> UIView? {
        let s = sections[section]
        guard let header = s.header else { return nil }
        let label = UILabel()
        label.text = header
        label.font = Fonts.ui(.sansSemibold, 13)
        label.textColor = Palette.secondary
        let stack = UIStackView(arrangedSubviews: [label, UIView()])
        if let (title, id, action) = s.action {
            let button = UIButton(configuration: .borderless())
            button.configuration?.title = title
            button.configuration?.buttonSize = .small
            button.accessibilityIdentifier = id
            button.isEnabled = !busy
            button.addAction(UIAction { _ in action() }, for: .touchUpInside)
            stack.addArrangedSubview(button)
        }
        stack.isLayoutMarginsRelativeArrangement = true
        stack.directionalLayoutMargins = .init(top: 6, leading: 20, bottom: 2, trailing: 12)
        return stack
    }

    func tableView(_ tableView: UITableView, heightForHeaderInSection section: Int) -> CGFloat {
        sections[section].header == nil ? 8 : 36
    }

    func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let row = sections[indexPath.section].rows[indexPath.row]
        if case let .commit(commit) = row {
            let cell = tableView.dequeueReusableCell(withIdentifier: GitHistoryCell.id, for: indexPath) as! GitHistoryCell
            cell.show(commit, lanes: Int(history?.laneCount ?? 1))
            cell.accessibilityIdentifier = "git-commit-\(commit.shortSha)"
            return cell
        }
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        cell.backgroundColor = Palette.elevated
        cell.accessoryView = nil
        cell.accessoryType = .none
        cell.selectionStyle = .none
        cell.accessibilityIdentifier = nil
        var c = UIListContentConfiguration.subtitleCell()
        switch row {
        case let .notice(text):
            c.text = text
            c.textProperties.color = Palette.secondary
            cell.accessibilityIdentifier = "git-notice"
        case let .none(text):
            c.text = text
            c.textProperties.font = Fonts.ui(.sans, 14)
            c.textProperties.color = Palette.tertiary
        case let .stash(message):
            c.text = message
            c.textProperties.font = Fonts.ui(.sans, 14)
            c.image = UIImage(systemName: "tray.full")
            c.imageProperties.tintColor = Palette.secondary
        case let .uncommitted(count):
            c.text = "Uncommitted changes (\(count))"
            c.textProperties.font = Fonts.ui(.sansMedium, 14)
            c.textProperties.color = Palette.secondary
        case .more:
            c.text = "Load more"
            c.textProperties.color = Palette.accent
            cell.selectionStyle = .default
            cell.accessibilityIdentifier = "git-history-more"
        case let .file(file):
            let name = (file.path as NSString).lastPathComponent
            let dir = (file.path as NSString).deletingLastPathComponent
            let title = NSMutableAttributedString(string: file.letter + "  ", attributes: [.font: Fonts.ui(.monoSemibold, 13), .foregroundColor: letterColor(file.letter)])
            title.append(NSAttributedString(string: name, attributes: [.font: Fonts.ui(.sansMedium, 15), .foregroundColor: Palette.text]))
            c.attributedText = title
            let sub = NSMutableAttributedString(string: dir.isEmpty ? "" : dir + "  ", attributes: [.font: Fonts.ui(.sans, 12), .foregroundColor: Palette.secondary])
            sub.append(GitText.delta(file.additions, file.deletions))
            c.secondaryAttributedText = sub
            let button = UIButton(configuration: .gray())
            button.configuration?.title = file.staged ? "Unstage" : "Stage"
            button.configuration?.buttonSize = .small
            button.configuration?.cornerStyle = .capsule
            button.accessibilityIdentifier = "git-\(file.staged ? "unstage" : "stage")-\(name)"
            button.isEnabled = !busy
            button.addAction(UIAction { [weak self] _ in self?.stage(file) }, for: .touchUpInside)
            button.sizeToFit()
            cell.accessoryView = button
            cell.selectionStyle = .default
            cell.accessibilityIdentifier = "git-file-\(file.staged ? "staged" : "changed")-\(name)"
        case .commit:
            break
        }
        cell.contentConfiguration = c
        return cell
    }

    private func letterColor(_ letter: String) -> UIColor {
        switch letter {
        case "A", "?", "U": Palette.success
        case "D": Palette.danger
        case "R", "C": Palette.accent
        default: Palette.warning
        }
    }

    func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        guard let checkout else { return }
        switch sections[indexPath.section].rows[indexPath.row] {
        case let .file(file):
            DiffSheetViewController.present(from: self, title: (file.path as NSString).lastPathComponent, subtitle: "\((file.path as NSString).deletingLastPathComponent) · \(file.staged ? "Staged" : "Unstaged")") {
                try await checkout.fileDiff(path: file.path, staged: file.staged)
            }
        case let .commit(commit):
            parent?.navigationController?.pushViewController(GitCommitViewController(checkout: checkout, sha: commit.sha, title: commit.shortSha), animated: true)
        case .more:
            Task { @MainActor in
                self.history = try? await checkout.history(more: true)
                self.paint()
            }
        default:
            break
        }
    }
}

final class GitHistoryCell: UITableViewCell {
    static let id = "git-history"
    private let graph = GitGraphView()
    private let subject = UILabel()
    private let meta = UILabel()
    private var graphWidth: NSLayoutConstraint!

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        backgroundColor = Palette.elevated
        subject.font = Fonts.ui(.sansMedium, 14)
        subject.textColor = Palette.text
        subject.numberOfLines = 2
        meta.font = Fonts.ui(.sans, 12)
        meta.textColor = Palette.secondary
        meta.numberOfLines = 1
        let text = UIStackView(arrangedSubviews: [subject, meta])
        text.axis = .vertical
        text.spacing = 2
        for v in [graph, text] as [UIView] {
            v.translatesAutoresizingMaskIntoConstraints = false
            contentView.addSubview(v)
        }
        graphWidth = graph.widthAnchor.constraint(equalToConstant: 14)
        NSLayoutConstraint.activate([
            graph.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 10),
            graph.topAnchor.constraint(equalTo: contentView.topAnchor),
            graph.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
            graphWidth,
            text.leadingAnchor.constraint(equalTo: graph.trailingAnchor, constant: 8),
            text.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -12),
            text.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 8),
            text.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -8),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func show(_ row: GitHistoryRow, lanes: Int) {
        graph.row = row
        graphWidth.constant = CGFloat(max(1, min(lanes, 6))) * GitGraphView.lane
        subject.text = row.subject
        let m = NSMutableAttributedString()
        for chip in row.refs {
            let color: UIColor = chip.kind == "remote" ? Palette.secondary : chip.kind == "tag" ? Palette.warning : Palette.accent
            m.append(NSAttributedString(string: chip.label, attributes: [.font: Fonts.ui(.monoMedium, 11), .foregroundColor: color, .backgroundColor: color.withAlphaComponent(0.12)]))
            m.append(NSAttributedString(string: " "))
        }
        m.append(NSAttributedString(string: "\(row.author) · \(row.when) · \(row.shortSha)"))
        meta.attributedText = m
        accessibilityLabel = "\(row.subject), \(row.author), \(row.when), \(row.shortSha)"
    }
}

final class GitGraphView: UIView {
    static let lane: CGFloat = 12
    private static let colors: [UIColor] = Palette.projectDots

    var row: GitHistoryRow? { didSet { setNeedsDisplay() } }

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        contentMode = .redraw
    }

    required init?(coder: NSCoder) { fatalError() }

    override func draw(_ rect: CGRect) {
        guard let row, let ctx = UIGraphicsGetCurrentContext() else { return }
        let mid = bounds.midY
        let x = { (lane: UInt32) in CGFloat(lane) * Self.lane + Self.lane / 2 }
        let color = { (id: UInt32) in Self.colors[Int(id) % Self.colors.count] }
        ctx.setLineWidth(1.5)
        for segment in row.segments {
            ctx.setStrokeColor(color(segment.color).cgColor)
            ctx.setLineDash(phase: 0, lengths: segment.dashed ? [3, 3] : [])
            let path = UIBezierPath()
            switch segment.shape {
            case .through:
                path.move(to: CGPoint(x: x(segment.fromLane), y: 0))
                path.addLine(to: CGPoint(x: x(segment.toLane), y: bounds.height))
            case .incoming:
                path.move(to: CGPoint(x: x(segment.fromLane), y: 0))
                path.addLine(to: CGPoint(x: x(segment.toLane), y: mid))
            case .outgoing:
                path.move(to: CGPoint(x: x(segment.fromLane), y: mid))
                path.addLine(to: CGPoint(x: x(segment.toLane), y: bounds.height))
            }
            ctx.addPath(path.cgPath)
            ctx.strokePath()
        }
        ctx.setLineDash(phase: 0, lengths: [])
        let r: CGFloat = row.isHead ? 4.5 : 3.5
        let node = CGRect(x: x(row.lane) - r, y: mid - r, width: r * 2, height: r * 2)
        ctx.setFillColor((row.isHead ? Palette.background : color(row.color)).cgColor)
        ctx.fillEllipse(in: node)
        if row.isHead {
            ctx.setStrokeColor(color(row.color).cgColor)
            ctx.setLineWidth(2)
            ctx.strokeEllipse(in: node)
        }
    }
}

final class GitCommitViewController: UITableViewController {
    private let checkout: GitCheckout
    private let sha: String
    private var commit: GitCommitView?
    private var items: [DiffItem] = []
    private var failure: String?

    init(checkout: GitCheckout, sha: String, title: String) {
        self.checkout = checkout
        self.sha = sha
        super.init(style: .plain)
        self.title = "Commit \(title)"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        tableView.separatorStyle = .none
        tableView.accessibilityIdentifier = "git-commit-detail"
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "info")
        DiffPainter.register(tableView)
        navigationItem.rightBarButtonItem = UIBarButtonItem(title: "Copy SHA", primaryAction: UIAction { [weak self] _ in
            guard let self else { return }
            UIPasteboard.general.string = self.sha
            Toast.show("Copied \(String(self.sha.prefix(8)))", in: self.view.window)
        })
        navigationItem.rightBarButtonItem?.accessibilityIdentifier = "git-copy-sha"
        Task { @MainActor in
            do {
                let commit = try await checkout.commitDetail(sha: sha)
                self.commit = commit
                self.items = DiffItem.rows(commit.files)
            } catch {
                self.failure = GitText.failure(error, fallback: "Couldn't load this commit. Try again.")
            }
            self.tableView.reloadData()
        }
    }

    private var info: [(String, String)] {
        guard let commit else { return [("", failure ?? "Loading…")] }
        return [("", commit.subject), ("Author", commit.author), ("Date", commit.when), ("SHA", commit.sha), ("Files", "\(commit.files.count)")]
    }

    override func numberOfSections(in tableView: UITableView) -> Int { 2 }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        section == 0 ? info.count : items.count
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        guard indexPath.section == 0 else { return DiffPainter.cell(tableView, indexPath, items[indexPath.row]) }
        let (label, value) = info[indexPath.row]
        let cell = tableView.dequeueReusableCell(withIdentifier: "info", for: indexPath)
        cell.selectionStyle = .none
        cell.backgroundColor = .clear
        var c = label.isEmpty ? UIListContentConfiguration.cell() : UIListContentConfiguration.valueCell()
        c.text = label.isEmpty ? value : label
        c.textProperties.font = label.isEmpty ? Fonts.ui(.sansSemibold, 18) : Fonts.ui(.sans, 14)
        c.textProperties.color = label.isEmpty ? Palette.text : Palette.secondary
        if !label.isEmpty {
            c.secondaryText = value
            c.secondaryTextProperties.font = label == "SHA" ? Fonts.ui(.mono, 12) : Fonts.ui(.sans, 14)
            c.secondaryTextProperties.color = Palette.text
        }
        cell.contentConfiguration = c
        return cell
    }
}

final class GitCommitSheet: UIViewController, UITextViewDelegate {
    private let checkout: GitCheckout
    private var tree: GitTreeView
    private let place: String
    private let done: (GitTreeView) -> Void
    private let message = UITextView()
    private let counter = UILabel()
    private let note = UILabel()
    private let ai = UIButton(configuration: .tinted())
    private let go = UIButton(configuration: .filled())
    private let more = UIButton(configuration: .gray())
    private var kind = Kind.commit
    private var busy = false

    enum Kind { case commit, amend, all }

    init(checkout: GitCheckout, tree: GitTreeView, where place: String, done: @escaping (GitTreeView) -> Void) {
        self.checkout = checkout
        self.tree = tree
        self.place = place
        self.done = done
        super.init(nibName: nil, bundle: nil)
        title = "Commit"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.leftBarButtonItem = UIBarButtonItem(systemItem: .cancel, primaryAction: UIAction { [weak self] _ in self?.dismiss(animated: true) })
        let heading = UILabel()
        heading.text = "\(tree.branch) · \(place)"
        heading.font = Fonts.ui(.sans, 13)
        heading.textColor = Palette.secondary
        heading.numberOfLines = 0
        let label = UILabel()
        label.text = "Commit message"
        label.font = Fonts.ui(.sansSemibold, 14)
        counter.font = Fonts.ui(.mono, 12)
        counter.textColor = Palette.tertiary
        counter.accessibilityIdentifier = "git-commit-count"
        let top = UIStackView(arrangedSubviews: [label, UIView(), counter])
        message.font = Fonts.ui(.sans, 16)
        message.backgroundColor = Palette.elevated
        message.layer.cornerRadius = 12
        message.textContainerInset = .init(top: 10, left: 8, bottom: 10, right: 8)
        message.delegate = self
        message.accessibilityIdentifier = "git-commit-message"
        message.heightAnchor.constraint(equalToConstant: 120).isActive = true
        ai.configuration?.title = "Write with AI"
        ai.configuration?.image = UIImage(systemName: "sparkles")
        ai.configuration?.imagePadding = 6
        ai.configuration?.cornerStyle = .capsule
        ai.accessibilityIdentifier = "git-write-ai"
        ai.addAction(UIAction { [weak self] _ in self?.writeWithAI() }, for: .touchUpInside)
        note.font = Fonts.ui(.sans, 12)
        note.textColor = Palette.secondary
        note.numberOfLines = 0
        note.isHidden = true
        go.configuration?.cornerStyle = .capsule
        go.accessibilityIdentifier = "git-commit-go"
        go.addAction(UIAction { [weak self] _ in self?.commit() }, for: .touchUpInside)
        more.configuration?.image = UIImage(systemName: "chevron.down")
        more.configuration?.cornerStyle = .capsule
        more.showsMenuAsPrimaryAction = true
        more.accessibilityIdentifier = "git-commit-more"
        let actions = UIStackView(arrangedSubviews: [go, more])
        actions.spacing = 8
        let stack = UIStackView(arrangedSubviews: [heading, top, message, ai, note, actions])
        stack.axis = .vertical
        stack.spacing = 10
        stack.alignment = .fill
        ai.setContentHuggingPriority(.required, for: .horizontal)
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
        ])
        update()
    }

    func textViewDidChange(_ textView: UITextView) { update() }

    private func update() {
        let subject = message.text.split(separator: "\n", omittingEmptySubsequences: false).first.map(String.init) ?? ""
        counter.text = "\(subject.count)/\(tree.subjectLimit)"
        counter.textColor = subject.count > Int(tree.subjectLimit) ? Palette.warning : Palette.tertiary
        let files = tree.staged.count
        let empty = message.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let title = switch kind {
        case .commit: "Commit \(files) file\(files == 1 ? "" : "s")"
        case .amend: "Amend"
        case .all: "Commit all"
        }
        go.configuration?.title = busy ? "Committing…" : title
        let blocked = switch kind {
        case .commit: tree.commitRefusal != nil
        case .amend: !tree.canAmend
        case .all: tree.staged.isEmpty && tree.changes.isEmpty
        }
        go.isEnabled = !busy && !empty && !blocked
        ai.isEnabled = !busy && !tree.staged.isEmpty
        more.menu = UIMenu(title: "More commit options", children: [
            UIAction(title: "Commit", subtitle: "Commit the staged files", state: kind == .commit ? .on : .off) { [weak self] _ in self?.kind = .commit; self?.update() },
            UIAction(title: "Amend", subtitle: "Fold into the previous commit (not pushed)", attributes: tree.canAmend ? [] : .disabled, state: kind == .amend ? .on : .off) { [weak self] _ in self?.kind = .amend; self?.update() },
            UIAction(title: "Commit all", subtitle: "Stage everything, then commit", state: kind == .all ? .on : .off) { [weak self] _ in self?.kind = .all; self?.update() },
        ])
    }

    private func writeWithAI() {
        busy = true
        ai.configuration?.title = "Writing…"
        update()
        Task { @MainActor in
            do {
                self.message.text = try await self.checkout.generateMessage()
                self.ai.configuration?.title = "Rewrite"
                self.note.text = "Written from the staged diff by the Mac's commit model. Edit it before you commit."
                self.note.isHidden = false
            } catch {
                self.ai.configuration?.title = "Write with AI"
                Toast.show(GitText.failure(error, fallback: "Couldn't write a message. Try again."), in: self.view.window)
            }
            self.busy = false
            self.update()
        }
    }

    private func commit() {
        let text = message.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        busy = true
        update()
        let kind = self.kind
        Task { @MainActor in
            do {
                let next = try await self.checkout.commit(message: text, amend: kind == .amend, all: kind == .all)
                self.busy = false
                if next.notice?.error == true {
                    self.tree = self.checkout.dismissNotice()
                    self.update()
                    Toast.show(next.notice?.text ?? "", in: self.view.window)
                    return
                }
                self.done(next)
                self.dismiss(animated: true)
            } catch {
                self.busy = false
                self.update()
                Toast.show(GitText.failure(error, fallback: "Couldn't commit. Try again."), in: self.view.window)
            }
        }
    }
}
