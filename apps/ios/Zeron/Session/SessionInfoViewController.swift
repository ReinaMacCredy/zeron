import UIKit

final class SessionInfoViewController: UITableViewController {
    var openSource: ((InfoSource) -> Void)?
    var loadUsage: (() async -> UsageMeter?)?
    private let sources: [InfoSource]
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
            self.tableView.reloadSections(IndexSet(integer: self.usageSection), with: .none)
        }
    }

    private var usageSection: Int { sources.isEmpty ? 0 : 1 }

    override func numberOfSections(in tableView: UITableView) -> Int { sources.isEmpty ? 1 : 2 }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        section == usageSection ? "Usage" : "Sources"
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        section == usageSection ? 1 : sources.count
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = UITableViewCell(style: .value1, reuseIdentifier: nil)
        var content = cell.defaultContentConfiguration()
        content.textProperties.font = Fonts.ui(.sans, 15)
        content.textProperties.color = Palette.text
        if indexPath.section == usageSection {
            content.image = UIImage(systemName: "chart.bar")
            cell.selectionStyle = .none
            if let usage {
                content.text = usage.label
                cell.accessoryView = Self.meter(usage)
            } else {
                content.text = usageLoaded ? "Usage unavailable" : "Loading usage…"
                content.textProperties.color = Palette.secondary
            }
        } else {
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
        guard indexPath.section != usageSection else { return }
        openSource?(sources[indexPath.row])
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
