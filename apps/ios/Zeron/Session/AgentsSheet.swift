import UIKit

final class AgentsSheetViewController: UITableViewController {
    private let handle: SessionHandle
    private let open: (String) -> Void
    private var roster: BackgroundRosterView
    private var timer: Timer?

    private enum Section { case agents, shells }
    private var sections: [Section] {
        [roster.agents.isEmpty ? nil : .agents, roster.shells.isEmpty ? nil : .shells].compactMap { $0 }
    }

    init(handle: SessionHandle, open: @escaping (String) -> Void) {
        self.handle = handle
        self.open = open
        roster = handle.backgroundRoster(nowMs: nowMs())
        super.init(style: .insetGrouped)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    var wrapped: UINavigationController {
        let nav = UINavigationController(rootViewController: self)
        nav.modalPresentationStyle = .pageSheet
        if let s = nav.sheetPresentationController {
            s.detents = [.medium(), .large()]
            s.prefersGrabberVisible = true
        }
        return nav
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = roster.title
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "cell")
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        reload()
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.reload() }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        timer?.invalidate()
        timer = nil
    }

    private func reload() {
        roster = handle.backgroundRoster(nowMs: nowMs())
        tableView.reloadData()
    }

    private func stop(_ taskId: String, delegated: Bool) {
        Task { @MainActor [weak self, handle] in
            do {
                try await handle.stopBackgroundRun(taskId: taskId, delegated: delegated)
                self?.reload()
            } catch {
                let alert = UIAlertController(title: nil, message: "\(error)", preferredStyle: .alert)
                alert.addAction(UIAlertAction(title: "OK", style: .default))
                self?.present(alert, animated: true)
            }
        }
    }

    private func stopButton(id: String, action: @escaping () -> Void) -> UIButton {
        var config = UIButton.Configuration.gray()
        config.title = "Stop"
        config.buttonSize = .small
        config.cornerStyle = .capsule
        let button = UIButton(configuration: config, primaryAction: UIAction { _ in action() })
        button.accessibilityIdentifier = id
        button.sizeToFit()
        return button
    }

    override func numberOfSections(in tableView: UITableView) -> Int { sections.count }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        sections[section] == .agents ? roster.agents.count : roster.shells.count
    }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        sections[section] == .agents ? roster.agentsSection : roster.shellsSection
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "cell", for: indexPath)
        var content = UIListContentConfiguration.subtitleCell()
        content.secondaryTextProperties.color = Palette.secondary
        content.imageProperties.tintColor = Palette.secondary
        cell.accessoryView = nil
        cell.accessoryType = .none
        switch sections[indexPath.section] {
        case .agents:
            let run = roster.agents[indexPath.row]
            content.text = run.title
            content.secondaryText = run.line
            content.image = UIImage(systemName: run.delegated ? "arrow.triangle.branch" : "person.2")
            cell.accessibilityIdentifier = "agents-row-\(indexPath.row)"
            if run.canStop {
                cell.accessoryView = stopButton(id: "agents-stop-\(indexPath.row)") { [weak self] in
                    self?.stop(run.taskId, delegated: run.delegated)
                }
            } else if run.delegated {
                cell.accessoryType = .disclosureIndicator
            }
        case .shells:
            let shell = roster.shells[indexPath.row]
            content.text = shell.label
            content.textProperties.font = Fonts.ui(.mono, 14)
            content.secondaryText = shell.line
            content.image = UIImage(systemName: "terminal")
            cell.accessibilityIdentifier = "shells-row-\(indexPath.row)"
            cell.accessoryType = .disclosureIndicator
        }
        cell.contentConfiguration = content
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        switch sections[indexPath.section] {
        case .agents:
            let run = roster.agents[indexPath.row]
            guard run.delegated else { return }
            dismiss(animated: true) { [open] in open(run.taskId) }
        case .shells:
            let shell = roster.shells[indexPath.row]
            navigationController?.pushViewController(
                ShellOutputViewController(handle: handle, shell: shell) { [weak self] in
                    self?.stop(shell.taskId, delegated: false)
                },
                animated: true
            )
        }
    }
}

final class ShellOutputViewController: UIViewController {
    private let handle: SessionHandle
    private let shell: ShellRow
    private let onStop: () -> Void
    private let text = UITextView()
    private var timer: Timer?

    init(handle: SessionHandle, shell: ShellRow, onStop: @escaping () -> Void) {
        self.handle = handle
        self.shell = shell
        self.onStop = onStop
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        title = shell.label
        view.backgroundColor = .systemGroupedBackground
        if shell.canStop {
            let stop = UIBarButtonItem(title: "Stop", primaryAction: UIAction { [weak self] _ in
                self?.onStop()
                self?.navigationController?.popViewController(animated: true)
            })
            stop.accessibilityIdentifier = "shell-stop"
            navigationItem.rightBarButtonItem = stop
        }
        text.isEditable = false
        text.font = Fonts.ui(.mono, 12)
        text.textColor = Palette.text
        text.backgroundColor = .secondarySystemGroupedBackground
        text.layer.cornerRadius = 12
        text.textContainerInset = UIEdgeInsets(top: 12, left: 10, bottom: 12, right: 10)
        text.accessibilityIdentifier = "shell-output"
        text.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(text)
        NSLayoutConstraint.activate([
            text.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 12),
            text.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -12),
            text.leadingAnchor.constraint(equalTo: view.layoutMarginsGuide.leadingAnchor),
            text.trailingAnchor.constraint(equalTo: view.layoutMarginsGuide.trailingAnchor),
        ])
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        handle.watchShellOutput(taskId: shell.taskId)
        reload()
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.reload() }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        timer?.invalidate()
        timer = nil
        handle.watchShellOutput(taskId: nil)
    }

    private func reload() {
        let lines = handle.shellOutput(taskId: shell.taskId).joined(separator: "\n")
        guard text.text != lines else { return }
        text.text = lines
        text.scrollRangeToVisible(NSRange(location: (lines as NSString).length, length: 0))
    }
}

private func nowMs() -> Int64 { Int64(Date().timeIntervalSince1970 * 1000) }
