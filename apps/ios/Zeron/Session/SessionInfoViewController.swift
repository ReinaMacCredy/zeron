import UIKit

final class SessionInfoViewController: UITableViewController {
    var openSource: ((InfoSource) -> Void)?
    var loadUsage: (() async -> UsageMeter?)?
    var loadChanges: (() async -> ChatChangesView?)?
    var openChanges: ((String) -> Void)?
    private let sources: [InfoSource]
    private var changes: ChatChangesView?

    private enum Section { case changes, sources, usage }
    private var sections: [Section] {
        [changes == nil ? nil : .changes, sources.isEmpty ? nil : .sources, .usage].compactMap { $0 }
    }
    private var usage: UsageMeter?
    private var usageLoaded = false

    init(sources: [InfoSource]) {
        self.sources = sources
        super.init(style: .insetGrouped)
        title = "Session info"
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.backgroundColor = Palette.background
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .done, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        Task { @MainActor [weak self] in
            let usage = await self?.loadUsage?()
            guard let self else { return }
            self.usage = usage
            self.usageLoaded = true
            self.tableView.reloadData()
        }
        Task { @MainActor [weak self] in
            guard let changes = await self?.loadChanges?(), let self else { return }
            self.changes = changes
            self.tableView.reloadData()
        }
    }

    override func numberOfSections(in tableView: UITableView) -> Int { sections.count }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        switch sections[section] {
        case .changes: nil
        case .sources: "Sources"
        case .usage: "Usage"
        }
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        sections[section] == .sources ? sources.count : 1
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = UITableViewCell(style: .value1, reuseIdentifier: nil)
        var content = cell.defaultContentConfiguration()
        content.textProperties.font = Fonts.ui(.sans, 15)
        content.textProperties.color = Palette.text
        switch sections[indexPath.section] {
        case .changes:
            guard let changes else { break }
            content.image = UIImage(systemName: "plusminus")
            content.text = changes.title
            let counts = NSMutableAttributedString(string: "\(changes.files) ", attributes: [.font: Fonts.ui(.mono, 12), .foregroundColor: Palette.secondary])
            counts.append(GitText.delta(changes.additions, changes.deletions))
            let label = UILabel()
            label.attributedText = counts
            label.sizeToFit()
            cell.accessoryView = label
            cell.accessibilityIdentifier = "info-changes"
        case .usage:
            content.image = UIImage(systemName: "chart.bar")
            cell.selectionStyle = .none
            if let usage {
                content.text = usage.label
                cell.accessoryView = Self.meter(usage)
            } else {
                content.text = usageLoaded ? "Usage unavailable" : "Loading usage…"
                content.textProperties.color = Palette.secondary
            }
        case .sources:
            let source = sources[indexPath.row]
            content.image = UIImage(systemName: "doc")
            content.text = source.name
            content.textProperties.numberOfLines = 1
            content.textProperties.lineBreakMode = .byTruncatingMiddle
        }
        content.imageProperties.tintColor = Palette.secondary
        cell.contentConfiguration = content
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        switch sections[indexPath.section] {
        case .changes: if let changes { openChanges?(changes.title) }
        case .sources: openSource?(sources[indexPath.row])
        case .usage: break
        }
    }

    private static func meter(_ usage: UsageMeter) -> UIView {
        let bar = UIProgressView(progressViewStyle: .bar)
        bar.frame = CGRect(x: 0, y: 0, width: 96, height: 4)
        bar.progress = usage.fraction
        bar.trackTintColor = Palette.hairline
        bar.progressTintColor = switch usage.level {
        case .normal: Palette.accent
        case .warn: Palette.warning
        case .critical: Palette.danger
        }
        bar.layer.cornerRadius = 2
        bar.clipsToBounds = true
        return bar
    }
}
