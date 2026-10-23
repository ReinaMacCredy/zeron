import UIKit

final class PullRequestsViewController: UIViewController, UITableViewDataSource, UITableViewDelegate, UISearchResultsUpdating {
    enum Segment: Int { case pulls, git }

    private let app: AppModel
    private var segment: Segment
    private let table = UITableView(frame: .zero, style: .insetGrouped)
    private let segments = UISegmentedControl(items: ["Pull requests", "Git"])
    private let devicePill = UIButton(configuration: .gray())
    private let repoPill = UIButton(configuration: .gray())
    private let filter = UISegmentedControl(items: ["Authored", "Reviewing", "All"])
    private let viewPill = UIButton(configuration: .gray())
    private let sortPill = UIButton(configuration: .gray())
    private let header = UIStackView()
    private let gitHost = UIView()
    private var git: GitViewController?
    private let search = UISearchController(searchResultsController: nil)
    private var wantsHistory = false

    private var devices: [PrDevice] = []
    private var repositories: [PrRepository] = []
    private var deviceId: String?
    private var repository: String?
    private var board: PullRequestBoard?
    private var view_: PrBoardView?
    private var loading = false

    private static let starsKey = "pr.stars"
    private static let deviceKey = "pr.device"
    private static let repoKey = "pr.repository"

    init(app: AppModel, segment: Segment = .pulls) {
        self.app = app
        self.segment = segment
        super.init(nibName: nil, bundle: nil)
        title = "Pull requests"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .always
        search.searchResultsUpdater = self
        search.obscuresBackgroundDuringPresentation = false
        search.searchBar.placeholder = "Search pull requests"
        navigationItem.searchController = search
        navigationItem.rightBarButtonItem = UIBarButtonItem(image: UIImage(systemName: "arrow.clockwise"), primaryAction: UIAction { [weak self] _ in self?.refresh() })
        navigationItem.rightBarButtonItem?.accessibilityIdentifier = "pr-refresh"

        segments.selectedSegmentIndex = segment.rawValue
        segments.accessibilityIdentifier = "pr-segments"
        segments.addAction(UIAction { [weak self] _ in self?.switchSegment() }, for: .valueChanged)
        filter.selectedSegmentIndex = 0
        filter.accessibilityIdentifier = "pr-filter"
        filter.addAction(UIAction { [weak self] _ in self?.filterChanged() }, for: .valueChanged)
        for (pill, id) in [(devicePill, "pr-device"), (repoPill, "pr-repo"), (viewPill, "pr-view"), (sortPill, "pr-sort")] {
            pill.configuration?.cornerStyle = .capsule
            pill.configuration?.buttonSize = .small
            pill.showsMenuAsPrimaryAction = true
            pill.accessibilityIdentifier = id
        }
        let pills = UIStackView(arrangedSubviews: [devicePill, repoPill, UIView()])
        pills.spacing = 8
        let views = UIStackView(arrangedSubviews: [viewPill, sortPill, UIView()])
        views.spacing = 8
        header.axis = .vertical
        header.spacing = 10
        header.isLayoutMarginsRelativeArrangement = true
        header.directionalLayoutMargins = .init(top: 4, leading: 16, bottom: 8, trailing: 16)
        for v in [segments, pills, filter, views] { header.addArrangedSubview(v) }

        table.dataSource = self
        table.delegate = self
        table.backgroundColor = .clear
        table.accessibilityIdentifier = "pr-board"
        table.register(UITableViewCell.self, forCellReuseIdentifier: "row")
        gitHost.isHidden = true
        for v in [header, table, gitHost] {
            v.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(v)
        }
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
            header.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            header.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            table.topAnchor.constraint(equalTo: header.bottomAnchor),
            table.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            table.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            table.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            gitHost.topAnchor.constraint(equalTo: header.bottomAnchor),
            gitHost.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            gitHost.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            gitHost.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        loadDevices()
        switchSegment()
    }

    private func loadDevices() {
        devices = app.client?.pullRequestDevices() ?? []
        let saved = UserDefaults.standard.string(forKey: Self.deviceKey)
        let pick = devices.first { $0.id == saved && $0.online } ?? devices.first { $0.online } ?? devices.first
        selectDevice(pick?.id)
    }

    private func selectDevice(_ id: String?) {
        deviceId = id
        if let id { UserDefaults.standard.set(id, forKey: Self.deviceKey) }
        repositories = []
        repository = nil
        board = nil
        view_ = nil
        updatePills()
        table.reloadData()
        guard let id, let client = app.client else { return }
        Task { @MainActor in
            let repos = await client.pullRequestRepositories(deviceId: id)
            guard self.deviceId == id else { return }
            self.repositories = repos
            let saved = UserDefaults.standard.string(forKey: Self.repoKey)
            self.selectRepository(repos.first { $0.repository == saved }?.repository ?? repos.first?.repository)
        }
    }

    private func selectRepository(_ repo: String?) {
        repository = repo
        if let repo { UserDefaults.standard.set(repo, forKey: Self.repoKey) }
        updatePills()
        guard let repo, let deviceId, let client = app.client else {
            board = nil
            view_ = nil
            table.reloadData()
            attachGit()
            return
        }
        let board = client.pullRequestBoard(deviceId: deviceId, repository: repo, stars: UserDefaults.standard.stringArray(forKey: Self.starsKey) ?? [])
        self.board = board
        attachGit()
        load { await board.load(refresh: false) }
    }

    private var projectId: String? {
        repositories.first { $0.repository == repository }?.projectId
            ?? app.projectOptions.first { $0.device == deviceId && $0.git }?.id
    }

    private func updatePills() {
        let device = devices.first { $0.id == deviceId }
        devicePill.configuration?.title = device?.name ?? "No Mac"
        devicePill.configuration?.image = UIImage(systemName: "desktopcomputer", withConfiguration: UIImage.SymbolConfiguration(pointSize: 12))
        devicePill.configuration?.imagePadding = 6
        devicePill.menu = UIMenu(title: "Desktop devices", children: devices.map { d in
            UIAction(title: d.name, subtitle: d.online ? nil : "Offline", state: d.id == deviceId ? .on : .off) { [weak self] _ in self?.selectDevice(d.id) }
        })
        repoPill.configuration?.title = repository.map { "Repository: \(($0 as NSString).lastPathComponent)" } ?? "No repository"
        repoPill.menu = UIMenu(title: "Repository", children: repositories.map { r in
            UIAction(title: r.repository, state: r.repository == repository ? .on : .off) { [weak self] _ in self?.selectRepository(r.repository) }
        })
        let v = view_
        viewPill.configuration?.title = switch v?.view ?? .everything {
        case .everything: "Everything"
        case .needsYou: "Needs you"
        case .starred: "Starred"
        }
        viewPill.menu = UIMenu(children: [(PrPersonalView.everything, "Everything"), (.needsYou, "Needs you"), (.starred, "Starred")].map { item, label in
            UIAction(title: label, state: v?.view == item ? .on : .off) { [weak self] _ in self?.apply { $0.setView(view: item) } }
        })
        let field = v?.sortField ?? .updated
        let ascending = v?.ascending ?? false
        let fieldLabel: (PrSortField) -> String = {
            switch $0 {
            case .updated: "Recently updated"
            case .opened: "Recently opened"
            case .changes: "Most changes"
            }
        }
        sortPill.configuration?.title = fieldLabel(field)
        sortPill.configuration?.image = UIImage(systemName: ascending ? "arrow.up" : "arrow.down", withConfiguration: UIImage.SymbolConfiguration(pointSize: 11))
        sortPill.configuration?.imagePlacement = .trailing
        sortPill.configuration?.imagePadding = 4
        sortPill.menu = UIMenu(children: [
            UIMenu(options: .displayInline, children: [PrSortField.updated, .opened, .changes].map { f in
                UIAction(title: fieldLabel(f), state: f == field ? .on : .off) { [weak self] _ in self?.apply { $0.setSort(field: f, ascending: ascending) } }
            }),
            UIAction(title: ascending ? "Newest first" : "Oldest first", image: UIImage(systemName: "arrow.up.arrow.down")) { [weak self] _ in self?.apply { $0.setSort(field: field, ascending: !ascending) } },
        ])
        if let v {
            filter.selectedSegmentIndex = switch v.filter {
            case .authored: 0
            case .reviewing: 1
            case .all: 2
            }
        }
    }

    private func load(_ work: @escaping () async -> PrBoardView) {
        loading = true
        table.reloadData()
        Task { @MainActor in
            let next = await work()
            guard next.deviceId == self.deviceId, next.repository == self.repository else { return }
            self.loading = false
            self.view_ = next
            self.updatePills()
            self.table.reloadData()
        }
    }

    private func apply(_ change: (PullRequestBoard) -> PrBoardView) {
        guard let board else { return }
        view_ = change(board)
        updatePills()
        table.reloadData()
    }

    private func refresh() {
        if segment == .git { return git?.reload() ?? () }
        guard let board else { return loadDevices() }
        load { await board.load(refresh: true) }
    }

    private func filterChanged() {
        guard let board else { return }
        let f: PrFilter = [.authored, .reviewing, .all][filter.selectedSegmentIndex]
        load { await board.setFilter(filter: f) }
    }

    func updateSearchResults(for controller: UISearchController) {
        apply { $0.setQuery(query: controller.searchBar.text ?? "") }
    }

    private func toggleStar(_ row: PrRow) {
        var stars = UserDefaults.standard.stringArray(forKey: Self.starsKey) ?? []
        if let i = stars.firstIndex(of: row.starKey) { stars.remove(at: i) } else { stars.append(row.starKey) }
        UserDefaults.standard.set(stars, forKey: Self.starsKey)
        apply { $0.setStars(stars: stars) }
    }

    private enum Section { case notice(String, String), group(PrGroup), more }

    private var sections: [Section] {
        guard segment == .pulls else { return [] }
        if devices.isEmpty { return [.notice("No Mac serves pull requests", "Open Zeron on a Mac signed in to GitHub with gh, then refresh.")] }
        if repository == nil, !loading { return [.notice("No repository", "Add a project with a GitHub checkout on this Mac.")] }
        guard let v = view_ else { return [.notice("Loading pull requests…", "")] }
        if let error = v.error { return [.notice(error.title, error.body)] }
        if let empty = v.empty { return [.notice(empty, "")] }
        return v.groups.map { .group($0) } + (v.canLoadMore ? [.more] : [])
    }

    func numberOfSections(in tableView: UITableView) -> Int { sections.count }

    func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        if case let .group(g) = sections[section] { return g.rows.count }
        return 1
    }

    func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        if case let .group(g) = sections[section] { return "\(g.label) · \(g.rows.count)" }
        return nil
    }

    func tableView(_ tableView: UITableView, titleForFooterInSection section: Int) -> String? {
        guard section == sections.count - 1, let summary = view_?.summary, !summary.isEmpty else { return nil }
        return summary
    }

    func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        cell.backgroundColor = Palette.elevated
        cell.accessoryView = nil
        cell.accessoryType = .none
        switch sections[indexPath.section] {
        case let .notice(title, body):
            var c = UIListContentConfiguration.subtitleCell()
            c.text = title
            c.textProperties.font = Fonts.ui(.sansMedium, 16)
            c.secondaryText = body.isEmpty ? nil : body
            c.secondaryTextProperties.color = Palette.secondary
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = "pr-notice"
            cell.selectionStyle = .none
        case .more:
            var c = UIListContentConfiguration.cell()
            c.text = "Load more"
            c.textProperties.color = Palette.accent
            c.textProperties.alignment = .center
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = "pr-load-more"
            cell.selectionStyle = .default
        case let .group(g):
            let row = g.rows[indexPath.row]
            var c = UIListContentConfiguration.subtitleCell()
            c.text = (row.starred ? "\u{2605} " : "") + row.title
            c.textProperties.font = Fonts.ui(.sansMedium, 16)
            c.textProperties.color = Palette.text
            let sub = NSMutableAttributedString(string: row.status, attributes: [.foregroundColor: row.statusTone.color])
            sub.append(NSAttributedString(string: " · #\(row.number) · \(row.author)", attributes: [.foregroundColor: Palette.secondary]))
            c.secondaryAttributedText = sub
            c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
            c.image = UIImage(systemName: "arrow.triangle.pull")
            c.imageProperties.tintColor = row.glyphTone.color
            cell.contentConfiguration = c
            let updated = UILabel()
            updated.text = row.updated
            updated.font = Fonts.ui(.sans, 13)
            updated.textColor = Palette.tertiary
            updated.sizeToFit()
            cell.accessoryView = updated
            cell.accessibilityIdentifier = "pr-row-\(row.number)"
            cell.accessibilityLabel = row.accessibility
            cell.selectionStyle = .default
        }
        return cell
    }

    func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        switch sections[indexPath.section] {
        case .more:
            guard let board else { return }
            load { await board.loadMore() }
        case let .group(g):
            guard let deviceId else { return }
            let row = g.rows[indexPath.row]
            navigationController?.pushViewController(PullRequestViewController(app: app, deviceId: deviceId, url: row.url), animated: true)
        case .notice:
            break
        }
    }

    func tableView(_ tableView: UITableView, trailingSwipeActionsConfigurationForRowAt indexPath: IndexPath) -> UISwipeActionsConfiguration? {
        guard case let .group(g) = sections[indexPath.section] else { return nil }
        let row = g.rows[indexPath.row]
        let star = UIContextualAction(style: .normal, title: row.starred ? "Unstar" : "Star") { [weak self] _, _, done in
            self?.toggleStar(row)
            done(true)
        }
        star.backgroundColor = Palette.warning
        star.image = UIImage(systemName: row.starred ? "star.slash" : "star")
        return UISwipeActionsConfiguration(actions: [star])
    }

    private func switchSegment() {
        segment = Segment(rawValue: segments.selectedSegmentIndex) ?? .pulls
        let pulls = segment == .pulls
        filter.isHidden = !pulls
        viewPill.superview?.isHidden = !pulls
        repoPill.isHidden = !pulls
        navigationItem.searchController = pulls ? search : nil
        table.isHidden = !pulls
        gitHost.isHidden = pulls
        title = pulls ? "Pull requests" : "Git"
        if !pulls { attachGit() }
        table.reloadData()
    }

    private func attachGit() {
        guard segment == .git else { return }
        let project = projectId
        if let git, git.projectId == project, git.deviceId == deviceId { return }
        git?.willMove(toParent: nil)
        git?.view.removeFromSuperview()
        git?.removeFromParent()
        git = nil
        guard let deviceId else { return }
        let next = GitViewController(app: app, deviceId: deviceId, projectId: project)
        addChild(next)
        next.view.frame = gitHost.bounds
        next.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        gitHost.addSubview(next.view)
        next.didMove(toParent: self)
        git = next
        if wantsHistory { next.showHistory() }
    }

    func showGit(history: Bool) {
        segments.selectedSegmentIndex = Segment.git.rawValue
        switchSegment()
        wantsHistory = history
        if history { git?.showHistory() }
    }
}
