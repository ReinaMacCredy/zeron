import UIKit

final class ProjectActionsViewController: UITableViewController {
    static let pollInterval: Duration = .seconds(1)

    var openTerminal: ((String) -> Void)?
    private let source: SessionSource
    private var actions: ProjectActions?
    private var loaded = false
    private var runs: [String: String] = [:]
    private var tabs: [String: TerminalTab] = [:]
    private var starting: Set<String> = []
    private var poll: Task<Void, Never>?

    init(source: SessionSource) {
        self.source = source
        super.init(style: .insetGrouped)
        title = "Project actions"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.backgroundColor = Palette.background
        tableView.accessibilityIdentifier = "project-actions"
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .done, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                actions = try await source.projectActions()
            } catch {
                Toast.show("Couldn't load project actions. Try again.", in: view.window)
            }
            loaded = true
            tableView.reloadData()
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        poll?.cancel()
        poll = nil
    }

    private var items: [ProjectActionItem] { actions?.actions ?? [] }

    override func numberOfSections(in tableView: UITableView) -> Int { 1 }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        items.isEmpty ? 1 : items.count
    }

    override func tableView(_ tableView: UITableView, titleForFooterInSection section: Int) -> String? {
        if let issue = actions?.issue { return issue }
        return items.isEmpty ? nil : "Each run opens a tab named after the action in Terminal."
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = UITableViewCell(style: .subtitle, reuseIdentifier: nil)
        var content = cell.defaultContentConfiguration()
        content.textProperties.font = Fonts.ui(.sans, 15)
        content.textProperties.color = Palette.text
        guard !items.isEmpty else {
            content.text = !loaded ? "Loading…" : actions == nil ? "This chat has no project." : "This project has no actions yet. Add them on the Mac."
            content.textProperties.color = Palette.secondary
            cell.contentConfiguration = content
            cell.selectionStyle = .none
            return cell
        }
        let item = items[indexPath.row]
        content.image = UIImage(systemName: item.icon)
        content.imageProperties.tintColor = Palette.secondary
        content.text = item.name
        content.secondaryText = item.command
        content.secondaryTextProperties.font = Fonts.ui(.mono, 12)
        content.secondaryTextProperties.color = Palette.secondary
        cell.contentConfiguration = content
        cell.accessoryView = accessory(for: item)
        cell.selectionStyle = runs[item.id] == nil ? .none : .default
        cell.accessibilityIdentifier = "project-action-\(item.id)"
        return cell
    }

    private func accessory(for item: ProjectActionItem) -> UIView {
        if starting.contains(item.id) || tabs[runs[item.id] ?? ""]?.state == .running {
            let label = Self.status("Running", color: Palette.accent, id: item.id)
            let spinner = UIActivityIndicatorView(style: .medium)
            spinner.startAnimating()
            let stack = UIStackView(arrangedSubviews: [spinner, label])
            stack.spacing = 6
            stack.sizeToFit()
            return stack
        }
        if let tab = tabs[runs[item.id] ?? ""] {
            return Self.status(tab.status, color: tab.state == .done ? Palette.success : Palette.danger, id: item.id)
        }
        var config = UIButton.Configuration.tinted()
        config.cornerStyle = .capsule
        config.title = "Run"
        config.image = UIImage(systemName: "play.fill", withConfiguration: UIImage.SymbolConfiguration(pointSize: 10))
        config.imagePadding = 4
        let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in self?.run(item) })
        button.accessibilityIdentifier = "project-action-run-\(item.id)"
        button.sizeToFit()
        return button
    }

    private static func status(_ text: String, color: UIColor, id: String) -> UILabel {
        let label = UILabel()
        label.text = text
        label.font = Fonts.ui(.sansMedium, 13)
        label.textColor = color
        label.accessibilityIdentifier = "project-action-status-\(id)"
        label.sizeToFit()
        return label
    }

    private func run(_ item: ProjectActionItem) {
        starting.insert(item.id)
        reload(item.id)
        Task { @MainActor [weak self] in
            guard let self else { return }
            do {
                runs[item.id] = try await source.runProjectAction(item.id)
                await refreshRuns()
                follow()
            } catch {
                Toast.show("Couldn't run \(item.name). Try again.", in: view.window)
            }
            starting.remove(item.id)
            reload(item.id)
        }
    }

    private func follow() {
        guard poll == nil else { return }
        poll = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: Self.pollInterval)
                guard let self else { return }
                await refreshRuns()
                if !tabs.values.contains(where: { $0.state == .running }) {
                    poll = nil
                    return
                }
            }
        }
    }

    private func refreshRuns() async {
        guard let all = try? await source.terminals() else { return }
        let running = Set(runs.values)
        let next = Dictionary(uniqueKeysWithValues: all.filter { running.contains($0.id) }.map { ($0.id, $0) })
        guard next != tabs else { return }
        tabs = next
        tableView.reloadData()
    }

    private func reload(_ id: String) {
        guard let row = items.firstIndex(where: { $0.id == id }) else { return }
        tableView.reloadRows(at: [IndexPath(row: row, section: 0)], with: .none)
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        guard indexPath.row < items.count, let terminal = runs[items[indexPath.row].id] else { return }
        openTerminal?(terminal)
    }
}
