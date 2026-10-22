import UIKit

final class ContextSheetViewController: UITableViewController {
    struct Content: Equatable {
        var card: ContextCard?
    }

    private let content: () -> Content
    private let toggleLong: () -> Void
    private let pick: (UInt32) -> Void
    private var shown = Content()

    init(content: @escaping () -> Content, toggleLong: @escaping () -> Void, pick: @escaping (UInt32) -> Void) {
        self.content = content
        self.toggleLong = toggleLong
        self.pick = pick
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
        title = "Context and stats"
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "cell")
        shown = content()
    }

    func reload() {
        let next = content()
        guard next != shown else { return }
        shown = next
        tableView.reloadData()
    }

    private enum Row { case long, track, readout }

    private var contextRows: [Row] { shown.card == nil ? [] : [.long, .track, .readout] }

    override func numberOfSections(in tableView: UITableView) -> Int { 1 }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        contextRows.count
    }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        shown.card == nil ? nil : "Context"
    }

    override func tableView(_ tableView: UITableView, titleForFooterInSection section: Int) -> String? {
        shown.card?.warning
    }

    override func tableView(_ tableView: UITableView, willDisplayFooterView view: UIView, forSection section: Int) {
        (view as? UITableViewHeaderFooterView)?.textLabel?.textColor = Palette.warning
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "cell", for: indexPath)
        cell.selectionStyle = .none
        cell.accessoryView = nil
        cell.contentConfiguration = nil
        cell.contentView.subviews.filter { $0.tag == 7 }.forEach { $0.removeFromSuperview() }
        guard let card = shown.card else { return cell }
        switch contextRows[indexPath.row] {
        case .long:
            var c = cell.defaultContentConfiguration()
            c.text = "Long context"
            cell.contentConfiguration = c
            let toggle = UISwitch()
            toggle.isOn = card.longContext
            toggle.accessibilityIdentifier = "context-long"
            toggle.addAction(UIAction { [weak self] _ in self?.toggleLong() }, for: .valueChanged)
            cell.accessoryView = toggle
        case .track:
            let track = UISegmentedControl(items: [card.shortLabel] + card.stops)
            track.tag = 7
            track.selectedSegmentIndex = Int(card.position)
            track.accessibilityIdentifier = "context-track"
            track.addAction(UIAction { [weak self, weak track] _ in
                guard let track else { return }
                self?.pick(UInt32(track.selectedSegmentIndex))
            }, for: .valueChanged)
            track.translatesAutoresizingMaskIntoConstraints = false
            cell.contentView.addSubview(track)
            NSLayoutConstraint.activate([
                track.leadingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.leadingAnchor),
                track.trailingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.trailingAnchor),
                track.topAnchor.constraint(equalTo: cell.contentView.topAnchor, constant: 10),
                track.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor, constant: -10),
            ])
        case .readout:
            var c = cell.defaultContentConfiguration()
            let text = NSMutableAttributedString(string: card.windowLabel, attributes: [.foregroundColor: card.longContext ? Palette.secondary : Palette.warning])
            text.append(NSAttributedString(string: " · \(card.thresholdLabel)", attributes: [.foregroundColor: Palette.secondary]))
            c.attributedText = text
            c.textProperties.font = .preferredFont(forTextStyle: .footnote)
            cell.contentConfiguration = c
            cell.accessibilityIdentifier = "context-readout"
        }
        return cell
    }
}
