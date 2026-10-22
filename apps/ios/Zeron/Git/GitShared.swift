import UIKit

extension PrTone {
    var color: UIColor {
        switch self {
        case .muted: Palette.secondary
        case .positive: Palette.success
        case .warning: Palette.warning
        case .danger: Palette.danger
        case .merged: Palette.accent
        }
    }
}

extension PrGlyph {
    var symbol: String {
        switch self {
        case .pullRequest: "arrow.triangle.pull"
        case .closeCircle: "xmark.circle"
        case .document: "doc"
        case .check: "checkmark.circle"
        case .dangerTriangle: "exclamationmark.triangle"
        case .clockCircle: "clock"
        }
    }
}

enum GitText {
    static func failure(_ error: Error, fallback: String) -> String {
        switch error as? CoreError {
        case let .HostError(message), let .NotFound(message), let .InvalidArgument(message), let .Unsupported(message), let .HostUnavailable(message): message
        default: fallback
        }
    }

    static func chip(_ chip: PrChip) -> NSAttributedString {
        let out = NSMutableAttributedString()
        if let image = UIImage(systemName: chip.glyph.symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 12, weight: .semibold))?.withTintColor(chip.tone.color, renderingMode: .alwaysOriginal) {
            out.append(NSAttributedString(attachment: NSTextAttachment(image: image)))
            out.append(NSAttributedString(string: " "))
        }
        out.append(NSAttributedString(string: chip.label, attributes: [.foregroundColor: chip.tone.color, .font: Fonts.ui(.sansMedium, 14)]))
        return out
    }

    static func changes(_ text: String, font: UIFont = Fonts.ui(.monoMedium, 12)) -> NSAttributedString {
        let out = NSMutableAttributedString()
        for (i, part) in text.split(separator: " ").enumerated() {
            if i > 0 { out.append(NSAttributedString(string: " ")) }
            let color = part.hasPrefix("+") ? Palette.success : Palette.danger
            out.append(NSAttributedString(string: String(part), attributes: [.foregroundColor: color, .font: font]))
        }
        return out
    }

    static func delta(_ additions: UInt32, _ deletions: UInt32) -> NSAttributedString {
        var parts: [String] = []
        if additions > 0 || deletions == 0 { parts.append("+\(additions)") }
        if deletions > 0 { parts.append("\u{2212}\(deletions)") }
        return changes(parts.joined(separator: " "))
    }
}

enum DiffItem: Hashable {
    case file(path: String, status: String, additions: UInt32, deletions: UInt32)
    case notice(file: String, text: String)
    case hunk(file: String, index: Int, header: String)
    case line(file: String, hunk: Int, index: Int, line: DiffLineView)
    case thread(PrThread)

    static func rows(_ files: [DiffFileView]) -> [DiffItem] {
        var out: [DiffItem] = []
        for file in files {
            out.append(.file(path: file.path, status: file.status, additions: file.additions, deletions: file.deletions))
            for notice in file.notices { out.append(.notice(file: file.path, text: notice)) }
            for (h, hunk) in file.hunks.enumerated() {
                out.append(.hunk(file: file.path, index: h, header: hunk.header))
                for (i, line) in hunk.lines.enumerated() {
                    out.append(.line(file: file.path, hunk: h, index: i, line: line))
                    out += line.threads.map { .thread($0) }
                }
            }
        }
        return out
    }
}

final class DiffLineCell: UITableViewCell {
    static let id = "diff-line"
    private let gutter = UILabel()
    private let text = UILabel()
    private let bar = UIView()

    override init(style: UITableViewCell.CellStyle, reuseIdentifier: String?) {
        super.init(style: style, reuseIdentifier: reuseIdentifier)
        selectionStyle = .none
        gutter.font = Fonts.ui(.mono, 11)
        gutter.textColor = Palette.tertiary
        gutter.textAlignment = .right
        text.font = Fonts.ui(.mono, 12)
        text.numberOfLines = 0
        text.lineBreakMode = .byCharWrapping
        for v in [gutter, text, bar] {
            v.translatesAutoresizingMaskIntoConstraints = false
            contentView.addSubview(v)
        }
        NSLayoutConstraint.activate([
            bar.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            bar.topAnchor.constraint(equalTo: contentView.topAnchor),
            bar.bottomAnchor.constraint(equalTo: contentView.bottomAnchor),
            bar.widthAnchor.constraint(equalToConstant: 2),
            gutter.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 4),
            gutter.widthAnchor.constraint(equalToConstant: 56),
            gutter.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 1),
            text.leadingAnchor.constraint(equalTo: gutter.trailingAnchor, constant: 8),
            text.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -10),
            text.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 1),
            text.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -1),
        ])
    }

    required init?(coder: NSCoder) { fatalError() }

    func show(_ line: DiffLineView) {
        let old = line.oldNo.map(String.init) ?? ""
        let new = line.newNo.map(String.init) ?? ""
        gutter.text = line.kind == .meta ? "" : "\(old.padding(toLength: 4, withPad: " ", startingAt: 0)) \(new)"
        let sign = switch line.kind {
        case .add: "+"
        case .del: "\u{2212}"
        default: " "
        }
        text.text = line.kind == .meta ? line.text : sign + " " + line.text
        text.textColor = line.kind == .meta ? Palette.tertiary : Palette.color(.codeText)
        switch line.kind {
        case .add:
            contentView.backgroundColor = Palette.color(.diffAddWash)
            bar.backgroundColor = Palette.color(.diffAddBar)
        case .del:
            contentView.backgroundColor = Palette.color(.diffDelWash)
            bar.backgroundColor = Palette.color(.diffDelBar)
        default:
            contentView.backgroundColor = .clear
            bar.backgroundColor = .clear
        }
        accessibilityLabel = "\(sign == " " ? "" : sign == "+" ? "Added " : "Removed ")\(line.text)"
    }
}

struct DiffPainter {
    static func register(_ table: UITableView) {
        table.register(DiffLineCell.self, forCellReuseIdentifier: DiffLineCell.id)
        table.register(UITableViewCell.self, forCellReuseIdentifier: "diff-plain")
    }

    static func cell(_ table: UITableView, _ path: IndexPath, _ item: DiffItem, thread: (UITableViewCell, PrThread) -> Void = { _, _ in }) -> UITableViewCell {
        switch item {
        case let .line(_, _, _, line):
            let cell = table.dequeueReusableCell(withIdentifier: DiffLineCell.id, for: path) as! DiffLineCell
            cell.show(line)
            return cell
        case let .file(filePath, status, additions, deletions):
            let cell = plain(table, path)
            var c = UIListContentConfiguration.subtitleCell()
            let name = (filePath as NSString).lastPathComponent
            let dir = (filePath as NSString).deletingLastPathComponent
            c.text = name
            c.textProperties.font = Fonts.ui(.monoSemibold, 14)
            c.textProperties.color = Palette.text
            c.secondaryText = dir.isEmpty ? status : "\(dir)/ · \(status)"
            c.secondaryTextProperties.font = Fonts.ui(.mono, 11)
            c.secondaryTextProperties.color = Palette.secondary
            cell.contentConfiguration = c
            let counts = UILabel()
            counts.attributedText = GitText.delta(additions, deletions)
            counts.sizeToFit()
            cell.accessoryView = counts
            cell.backgroundColor = Palette.elevated
            cell.accessibilityIdentifier = "diff-file-\(name)"
            return cell
        case let .notice(_, text):
            let cell = plain(table, path)
            var c = UIListContentConfiguration.cell()
            c.text = text
            c.textProperties.font = Fonts.ui(.sans, 13)
            c.textProperties.color = Palette.secondary
            cell.contentConfiguration = c
            return cell
        case let .hunk(_, _, header):
            let cell = plain(table, path)
            var c = UIListContentConfiguration.cell()
            c.text = header
            c.textProperties.font = Fonts.ui(.mono, 11)
            c.textProperties.color = Palette.secondary
            c.textProperties.numberOfLines = 1
            c.directionalLayoutMargins = .init(top: 4, leading: 10, bottom: 4, trailing: 10)
            cell.contentConfiguration = c
            cell.backgroundColor = Palette.color(.diffHunk)
            return cell
        case let .thread(t):
            let cell = plain(table, path)
            thread(cell, t)
            return cell
        }
    }

    private static func plain(_ table: UITableView, _ path: IndexPath) -> UITableViewCell {
        let cell = table.dequeueReusableCell(withIdentifier: "diff-plain", for: path)
        cell.accessoryView = nil
        cell.accessoryType = .none
        cell.backgroundColor = .clear
        cell.selectionStyle = .none
        cell.accessibilityIdentifier = nil
        return cell
    }
}

final class DiffSheetViewController: UITableViewController {
    private var items: [DiffItem] = []
    private let subtitle: String?
    private let load: () async throws -> [DiffFileView]

    init(title: String, subtitle: String?, load: @escaping () async throws -> [DiffFileView]) {
        self.subtitle = subtitle
        self.load = load
        super.init(style: .plain)
        self.title = title
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        tableView.separatorStyle = .none
        tableView.accessibilityIdentifier = "diff-sheet"
        DiffPainter.register(tableView)
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in self?.dismiss(animated: true) })
        if let subtitle {
            let label = UILabel()
            label.text = subtitle
            label.font = Fonts.ui(.sansMedium, 13)
            label.textColor = Palette.secondary
            label.textAlignment = .center
            label.frame = CGRect(x: 0, y: 0, width: 0, height: 28)
            tableView.tableHeaderView = label
        }
        Task { @MainActor in
            do {
                let files = try await load()
                items = files.isEmpty ? [.notice(file: "", text: "No changes to show.")] : DiffItem.rows(files)
            } catch {
                items = [.notice(file: "", text: GitText.failure(error, fallback: "Couldn't load the diff. Try again."))]
            }
            tableView.reloadData()
        }
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { items.count }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        DiffPainter.cell(tableView, indexPath, items[indexPath.row])
    }

    static func present(from host: UIViewController, title: String, subtitle: String?, load: @escaping () async throws -> [DiffFileView]) {
        let nav = UINavigationController(rootViewController: DiffSheetViewController(title: title, subtitle: subtitle, load: load))
        if let sheet = nav.sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
        }
        host.present(nav, animated: true)
    }
}
