import UIKit

protocol ReviewReloading: AnyObject {
    func reload()
}

extension ReviewTone {
    var color: UIColor? {
        switch self {
        case .plain: nil
        case .muted: Palette.secondary
        case .danger: Palette.danger
        case .success: Palette.success
        }
    }
}

extension ReviewFindingState {
    var ink: UIColor {
        switch self {
        case .fixed: Palette.success
        case .skipped: Palette.warning
        case .noChange, .open: Palette.secondary
        case .agentOnIt: Palette.accent
        }
    }

    var fill: UIColor? {
        switch self {
        case .fixed: Palette.success.withAlphaComponent(0.12)
        case .skipped: Palette.warning.withAlphaComponent(0.12)
        case .noChange: Palette.controlFill
        case .open, .agentOnIt: nil
        }
    }

    var border: UIColor? {
        switch self {
        case .open: Palette.tertiary
        case .agentOnIt: Palette.accent.withAlphaComponent(0.6)
        case .fixed, .skipped, .noChange: nil
        }
    }
}

enum ReviewUI {
    static func chip(_ review: ReviewChipView) -> ComposerChip {
        var title = review.label
        if let next = review.nextReviewer { title += " · \(next)" }
        if let add = review.additions, let del = review.deletions { title += "  +\(add) \u{2212}\(del)" }
        if let note = review.note { title += " · \(note)" }
        let symbol = review.refresh ? "arrow.clockwise" : review.clean ? "checkmark.circle" : "checklist"
        return ComposerChip(id: "review", title: title, symbol: symbol, tint: review.tone.color)
    }

    static func tag(_ text: String, ink: UIColor, fill: UIColor?, border: UIColor? = nil, font: UIFont = Fonts.ui(.sansSemibold, 11)) -> UIView {
        var config = UIButton.Configuration.plain()
        config.attributedTitle = AttributedString(text, attributes: AttributeContainer([.font: font, .foregroundColor: ink]))
        config.contentInsets = NSDirectionalEdgeInsets(top: 2, leading: 6, bottom: 2, trailing: 6)
        config.background.backgroundColor = fill ?? .clear
        config.background.cornerRadius = 5
        config.background.strokeColor = border
        config.background.strokeWidth = border == nil ? 0 : 1
        let tag = UIButton(configuration: config)
        tag.isUserInteractionEnabled = false
        tag.setContentHuggingPriority(.required, for: .horizontal)
        tag.setContentCompressionResistancePriority(.required, for: .horizontal)
        return tag
    }

    static func failure(_ error: Error, in host: UIViewController?) {
        let alert = UIAlertController(title: nil, message: GitText.failure(error, fallback: "\(error)"), preferredStyle: .alert)
        alert.addAction(UIAlertAction(title: "OK", style: .default))
        host?.present(alert, animated: true)
    }

    static func wrap(_ root: UIViewController, large: Bool = false) -> UINavigationController {
        let nav = UINavigationController(rootViewController: root)
        nav.modalPresentationStyle = .pageSheet
        if let sheet = nav.sheetPresentationController {
            sheet.detents = large ? [.large()] : [.medium(), .large()]
            sheet.prefersGrabberVisible = true
        }
        return nav
    }
}

final class ReviewFindingsViewController: UITableViewController, ReviewReloading {
    private let handle: SessionHandle
    private let taskId: String?
    private var findings: ReviewFindingsView?

    init(handle: SessionHandle, taskId: String?) {
        self.handle = handle
        self.taskId = taskId
        findings = handle.reviewFindings(taskId: taskId)
        super.init(style: .insetGrouped)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = findings?.title
        tableView.accessibilityIdentifier = "review-findings"
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "finding")
        if navigationController?.viewControllers.first === self {
            navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
                self?.dismiss(animated: true)
            })
        }
        layoutFooter()
    }

    func reload() {
        let next = handle.reviewFindings(taskId: taskId)
        guard next != findings else { return }
        findings = next
        tableView.reloadData()
        layoutFooter()
    }

    private func layoutFooter() {
        guard let findings else { return tableView.tableFooterView = nil }
        var buttons: [UIButton] = []
        if let fix = findings.fixLabel {
            var config = UIButton.Configuration.filled()
            config.title = fix
            config.cornerStyle = .large
            config.baseBackgroundColor = Palette.text
            config.baseForegroundColor = Palette.background
            let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in self?.fixAll() })
            button.accessibilityIdentifier = "review-fix-all"
            buttons.append(button)
        }
        var open = UIButton.Configuration.gray()
        open.title = findings.openChanges
        open.cornerStyle = .large
        let openButton = UIButton(configuration: open, primaryAction: UIAction { [weak self] _ in self?.openChanges(focus: nil) })
        openButton.accessibilityIdentifier = "review-open-changes"
        buttons.append(openButton)
        let stack = UIStackView(arrangedSubviews: buttons)
        stack.axis = .vertical
        stack.spacing = 8
        stack.isLayoutMarginsRelativeArrangement = true
        stack.directionalLayoutMargins = .init(top: 8, leading: 20, bottom: 16, trailing: 20)
        stack.frame = CGRect(x: 0, y: 0, width: tableView.bounds.width, height: CGFloat(buttons.count) * 50 + 24)
        tableView.tableFooterView = stack
    }

    private func fixAll() {
        _ = handle.fixReviewFindings(only: nil)
        presentingViewController?.dismiss(animated: true)
    }

    private func openChanges(focus: String?) {
        DiffSheetViewController.presentFindings(handle, from: self, title: findings?.changesTitle ?? "", focus: focus)
    }

    override func numberOfSections(in tableView: UITableView) -> Int { 1 }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { findings?.rows.count ?? 0 }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        [findings?.summary, findings?.note, findings?.retryError].compactMap { $0 }.joined(separator: "\n").nilIfEmpty
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "finding", for: indexPath)
        guard let row = findings?.rows[indexPath.row] else { return cell }
        cell.contentView.subviews.forEach { $0.removeFromSuperview() }
        cell.accessibilityIdentifier = "review-finding-\(row.id)"
        let title = UILabel()
        title.text = row.title
        title.font = Fonts.ui(.sansMedium, 15)
        title.textColor = Palette.text
        title.numberOfLines = 0
        let lines = UIStackView(arrangedSubviews: [title])
        lines.axis = .vertical
        lines.spacing = 3
        if let reason = row.reason {
            let label = UILabel()
            label.text = reason
            label.font = Fonts.ui(.sans, 13)
            label.textColor = Palette.secondary
            label.numberOfLines = 3
            lines.addArrangedSubview(label)
        }
        let meta = UILabel()
        meta.text = row.meta
        meta.font = Fonts.ui(.mono, 11)
        meta.textColor = Palette.tertiary
        meta.lineBreakMode = .byTruncatingMiddle
        let badge = ReviewUI.tag(row.stateLabel, ink: row.state.ink, fill: row.state.fill, border: row.state.border, font: Fonts.ui(.sansMedium, 11))
        let foot = UIStackView(arrangedSubviews: [badge, meta])
        foot.spacing = 6
        foot.alignment = .center
        lines.addArrangedSubview(foot)
        lines.setCustomSpacing(6, after: lines.arrangedSubviews[lines.arrangedSubviews.count - 2])
        let severity = ReviewUI.tag(row.severityWord, ink: row.severity.ink, fill: row.severity.wash)
        let stack = UIStackView(arrangedSubviews: [severity, lines])
        stack.spacing = 10
        stack.alignment = .top
        stack.alpha = row.dimmed ? 0.5 : 1
        stack.translatesAutoresizingMaskIntoConstraints = false
        cell.contentView.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.trailingAnchor),
            stack.topAnchor.constraint(equalTo: cell.contentView.topAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor, constant: -10),
        ])
        if row.dismissable {
            var config = UIButton.Configuration.plain()
            config.image = UIImage(systemName: "xmark", withConfiguration: UIImage.SymbolConfiguration(pointSize: 11, weight: .semibold))
            config.baseForegroundColor = Palette.secondary
            let dismiss = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in
                self?.handle.dismissReviewFinding(id: row.id)
            })
            dismiss.accessibilityLabel = "\(row.dismissLabel) \(row.title)"
            dismiss.accessibilityIdentifier = "review-dismiss-\(row.id)"
            dismiss.sizeToFit()
            cell.accessoryView = dismiss
        } else {
            cell.accessoryView = nil
        }
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        guard let row = findings?.rows[indexPath.row] else { return }
        openChanges(focus: row.id)
    }

    override func tableView(_ tableView: UITableView, trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
        guard let row = findings?.rows[indexPath.row], row.dismissable else { return nil }
        let dismiss = UIContextualAction(style: .destructive, title: row.dismissLabel) { [weak self] _, _, done in
            self?.handle.dismissReviewFinding(id: row.id)
            done(true)
        }
        return UISwipeActionsConfiguration(actions: [dismiss])
    }
}

final class ReviewOptionsViewController: UITableViewController, ReviewReloading {
    private let handle: SessionHandle
    private var options: ReviewOptionsView?
    private let start = UIButton(type: .system)

    private enum Section { case scope, reviewer }
    private let sections: [Section] = [.scope, .reviewer]

    init(handle: SessionHandle) {
        self.handle = handle
        try? handle.openReviewOptions()
        options = handle.reviewOptions()
        super.init(style: .insetGrouped)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = options?.title
        tableView.accessibilityIdentifier = "review-options"
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "row")
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        var config = UIButton.Configuration.filled()
        config.cornerStyle = .large
        config.baseBackgroundColor = Palette.text
        config.baseForegroundColor = Palette.background
        start.configuration = config
        start.accessibilityIdentifier = "review-start"
        start.addAction(UIAction { [weak self] _ in self?.startReview() }, for: .touchUpInside)
        let header = UIView(frame: CGRect(x: 0, y: 0, width: tableView.bounds.width, height: 66))
        start.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(start)
        NSLayoutConstraint.activate([
            start.leadingAnchor.constraint(equalTo: header.layoutMarginsGuide.leadingAnchor),
            start.trailingAnchor.constraint(equalTo: header.layoutMarginsGuide.trailingAnchor),
            start.topAnchor.constraint(equalTo: header.topAnchor, constant: 8),
            start.heightAnchor.constraint(equalToConstant: 50),
        ])
        tableView.tableHeaderView = header
        paintStart()
    }

    func reload() {
        for case let page as ReviewReloading in navigationController?.viewControllers ?? [] where page !== self { page.reload() }
        let next = handle.reviewOptions()
        guard next != options else { return }
        options = next
        tableView.reloadData()
        paintStart()
    }

    private func paintStart() {
        start.configuration?.title = options?.startLabel
        start.isEnabled = options.map { !$0.busy } ?? false
    }

    private func startReview() {
        let handle = handle
        let host = presentingViewController
        dismiss(animated: true)
        Task { @MainActor in
            do { try await handle.startReview() } catch { ReviewUI.failure(error, in: host) }
        }
    }

    override func numberOfSections(in tableView: UITableView) -> Int { sections.count }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        sections[section] == .scope ? options?.scopeHeading : options?.reviewerHeading
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        guard let options else { return 0 }
        return sections[section] == .scope ? options.scopes.count + 1 : options.reviewers.count
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        var content = UIListContentConfiguration.subtitleCell()
        content.secondaryTextProperties.color = Palette.secondary
        cell.accessoryView = nil
        cell.accessoryType = .none
        cell.selectionStyle = .default
        guard let options else { return cell }
        switch sections[indexPath.section] {
        case .scope where indexPath.row < options.scopes.count:
            let row = options.scopes[indexPath.row]
            content.text = row.label
            content.secondaryText = row.note
            content.textProperties.color = row.enabled ? Palette.text : Palette.tertiary
            content.image = UIImage(systemName: "checkmark")
            content.imageProperties.tintColor = row.selected ? Palette.accent : .clear
            if let add = row.additions, let del = row.deletions {
                let counts = UILabel()
                counts.attributedText = GitText.delta(add, del)
                counts.sizeToFit()
                cell.accessoryView = counts
            }
            cell.selectionStyle = row.enabled ? .default : .none
            cell.accessibilityIdentifier = "review-scope-\(row.key)"
        case .scope:
            content.text = options.commitLabel
            content.secondaryText = options.commitPicked
            content.image = UIImage(systemName: "checkmark")
            content.imageProperties.tintColor = options.commitSelected ? Palette.accent : .clear
            cell.accessoryType = .disclosureIndicator
            cell.accessibilityIdentifier = "review-scope-commit"
        case .reviewer:
            let row = options.reviewers[indexPath.row]
            content.text = row.label
            content.secondaryText = row.detail
            content.image = BrandMarks.image(for: row.brand, side: 18) ?? UIImage(systemName: row.pick == .auto ? "sparkles" : "person.crop.circle")
            content.imageProperties.tintColor = Palette.secondary
            if row.selected { cell.accessoryType = .checkmark }
            if let harness = row.modelsHarness {
                var config = UIButton.Configuration.plain()
                config.image = UIImage(systemName: "chevron.right", withConfiguration: UIImage.SymbolConfiguration(pointSize: 13, weight: .semibold))
                config.baseForegroundColor = Palette.tertiary
                let more = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in self?.openModels(harness) })
                more.accessibilityIdentifier = "review-reviewer-models-\(indexPath.row)"
                more.sizeToFit()
                if row.selected {
                    let check = UIImageView(image: UIImage(systemName: "checkmark"))
                    check.tintColor = Palette.accent
                    let pair = UIStackView(arrangedSubviews: [check, more])
                    pair.spacing = 4
                    pair.alignment = .center
                    pair.frame = CGRect(x: 0, y: 0, width: 56, height: 32)
                    cell.accessoryView = pair
                } else {
                    cell.accessoryView = more
                }
            }
            cell.accessibilityIdentifier = "review-reviewer-\(indexPath.row)"
        }
        cell.contentConfiguration = content
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        guard let options else { return }
        switch sections[indexPath.section] {
        case .scope where indexPath.row < options.scopes.count:
            let row = options.scopes[indexPath.row]
            guard row.enabled else { return }
            try? handle.setReviewScope(key: row.key)
        case .scope:
            navigationController?.pushViewController(ReviewCommitsViewController(handle: handle), animated: true)
        case .reviewer:
            try? handle.setReviewer(pick: options.reviewers[indexPath.row].pick)
        }
        reload()
    }

    private func openModels(_ harness: String) {
        navigationController?.pushViewController(ReviewerModelsViewController(handle: handle, harness: harness), animated: true)
    }
}

final class ReviewCommitsViewController: UITableViewController, ReviewReloading, UISearchResultsUpdating {
    private let handle: SessionHandle
    private var commits: ReviewCommitsView
    private let search = UISearchController(searchResultsController: nil)

    init(handle: SessionHandle) {
        self.handle = handle
        try? handle.loadReviewCommits()
        commits = handle.reviewCommits(query: "")
        super.init(style: .insetGrouped)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = handle.reviewOptions()?.commitLabel
        tableView.accessibilityIdentifier = "review-commits"
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "commit")
        search.searchResultsUpdater = self
        search.obscuresBackgroundDuringPresentation = false
        navigationItem.searchController = search
        navigationItem.hidesSearchBarWhenScrolling = false
    }

    func reload() {
        let next = handle.reviewCommits(query: search.searchBar.text ?? "")
        guard next != commits else { return }
        commits = next
        tableView.reloadData()
    }

    func updateSearchResults(for searchController: UISearchController) { reload() }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { commits.rows.count }

    override func tableView(_ tableView: UITableView, titleForFooterInSection section: Int) -> String? {
        !commits.loading && commits.rows.isEmpty ? commits.empty : nil
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "commit", for: indexPath)
        let row = commits.rows[indexPath.row]
        var content = UIListContentConfiguration.subtitleCell()
        content.text = row.subject
        content.secondaryText = row.detail
        content.secondaryTextProperties.font = Fonts.ui(.mono, 12)
        content.secondaryTextProperties.color = Palette.secondary
        cell.contentConfiguration = content
        cell.accessoryType = row.selected ? .checkmark : .none
        cell.accessibilityIdentifier = "review-commit-\(indexPath.row)"
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        let row = commits.rows[indexPath.row]
        try? handle.pickReviewCommit(sha: row.sha, subject: row.subject)
        navigationController?.popViewController(animated: true)
    }
}

final class ReviewerModelsViewController: UITableViewController, ReviewReloading {
    private let handle: SessionHandle
    private let harness: String
    private var models: ReviewerModelsView?

    init(handle: SessionHandle, harness: String) {
        self.handle = handle
        self.harness = harness
        try? handle.loadReviewerModels(harness: harness)
        models = try? handle.reviewerModels(harness: harness)
        super.init(style: .insetGrouped)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = models?.title
        tableView.accessibilityIdentifier = "review-models"
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "option")
    }

    func reload() {
        let next = try? handle.reviewerModels(harness: harness)
        guard next != models else { return }
        models = next
        tableView.reloadData()
    }

    private func rows(_ section: Int) -> [ReviewerOptionRow] {
        section == 0 ? models?.models ?? [] : models?.thinking ?? []
    }

    override func numberOfSections(in tableView: UITableView) -> Int { models?.thinking.isEmpty == false ? 2 : 1 }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        section == 1 ? models?.thinkingHeading : nil
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { rows(section).count }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "option", for: indexPath)
        let row = rows(indexPath.section)[indexPath.row]
        var content = UIListContentConfiguration.cell()
        content.text = row.label
        cell.contentConfiguration = content
        cell.accessoryType = row.selected ? .checkmark : .none
        cell.accessibilityIdentifier = "review-model-\(indexPath.section)-\(indexPath.row)"
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        try? handle.setReviewer(pick: rows(indexPath.section)[indexPath.row].pick)
        reload()
    }
}

private extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
