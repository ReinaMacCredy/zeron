import SwiftUI
import UIKit

final class HostPageViewController: UIViewController, UICollectionViewDelegate {
    private let app: AppModel
    private let deviceId: String
    private var pageId: String
    private var page: HostPage?
    private var collectionView: UICollectionView!
    private var dataSource: UICollectionViewDiffableDataSource<Int, String>!
    private let spinner = UIActivityIndicatorView(style: .medium)

    init(app: AppModel, deviceId: String, page: String, title: String) {
        self.app = app
        self.deviceId = deviceId
        self.pageId = page
        super.init(nibName: nil, bundle: nil)
        self.title = title
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.headerMode = .supplementary
        config.footerMode = .supplementary
        config.trailingSwipeActionsConfigurationProvider = { [weak self] path in self?.swipeActions(at: path) }
        collectionView = UICollectionView(frame: view.bounds, collectionViewLayout: UICollectionViewCompositionalLayout.list(using: config))
        collectionView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        collectionView.backgroundColor = .clear
        collectionView.delegate = self
        view.addSubview(collectionView)

        let cell = UICollectionView.CellRegistration<UICollectionViewListCell, String> { [weak self] cell, path, _ in
            guard let self, let row = self.row(at: path) else { return }
            self.configure(cell, row)
        }
        let header = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionHeader) { [weak self] view, _, path in
            var c = UIListContentConfiguration.groupedHeader()
            c.text = self?.section(path.section)?.title
            view.contentConfiguration = c
        }
        let footer = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionFooter) { [weak self] view, _, path in
            var c = UIListContentConfiguration.groupedFooter()
            c.text = self?.section(path.section)?.footer
            view.contentConfiguration = c
        }
        dataSource = UICollectionViewDiffableDataSource(collectionView: collectionView) { cv, path, id in
            cv.dequeueConfiguredReusableCell(using: cell, for: path, item: id)
        }
        dataSource.supplementaryViewProvider = { cv, kind, path in
            cv.dequeueConfiguredReusableSupplementary(using: kind == UICollectionView.elementKindSectionFooter ? footer : header, for: path)
        }
        spinner.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(spinner)
        NSLayoutConstraint.activate([
            spinner.centerXAnchor.constraint(equalTo: view.centerXAnchor),
            spinner.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 32),
        ])
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        load()
    }

    private func load() {
        guard let client = app.client else { return }
        if page == nil { spinner.startAnimating() }
        Task { @MainActor in
            do {
                show(try await client.hostPage(deviceId: deviceId, page: pageId))
            } catch {
                spinner.stopAnimating()
                if page == nil { show(HostPage(title: title ?? "", sections: [HostSection(title: nil, footer: nil, rows: [HostRow(id: "error", title: "Couldn't reach this Mac", subtitle: "\(error)", value: nil, kind: .info, actions: [])])])) }
            }
        }
    }

    private func show(_ page: HostPage) {
        spinner.stopAnimating()
        self.page = page
        title = page.title
        var s = NSDiffableDataSourceSnapshot<Int, String>()
        for (ix, section) in page.sections.enumerated() {
            s.appendSections([ix])
            s.appendItems(section.rows.map { "\(ix)/\($0.id)" }, toSection: ix)
        }
        s.reconfigureItems(s.itemIdentifiers)
        dataSource.apply(s, animatingDifferences: false)
    }

    private func section(_ ix: Int) -> HostSection? {
        guard let page, ix < page.sections.count else { return nil }
        return page.sections[ix]
    }

    private func row(at path: IndexPath) -> HostRow? {
        guard let section = section(path.section), path.item < section.rows.count else { return nil }
        return section.rows[path.item]
    }

    private func configure(_ cell: UICollectionViewListCell, _ row: HostRow) {
        var bg = UIBackgroundConfiguration.listGroupedCell()
        bg.backgroundColor = Palette.elevated
        cell.backgroundConfiguration = bg
        cell.accessibilityIdentifier = "host-\(row.id)"
        if case let .meter(meter) = row.kind {
            cell.accessories = []
            cell.contentConfiguration = UIHostingConfiguration { MeterRow(row: row, meter: meter) }
            return
        }
        var c = UIListContentConfiguration.valueCell()
        if row.subtitle != nil { c = .subtitleCell() }
        if case let .text(_, placeholder) = row.kind {
            c = .subtitleCell()
            let text = row.value ?? ""
            c.secondaryText = text.isEmpty ? placeholder : text
            c.secondaryTextProperties.numberOfLines = 3
        }
        c.text = row.title
        c.textProperties.font = Fonts.ui(.sansMedium, 16)
        c.textProperties.color = Palette.text
        if c.secondaryText == nil { c.secondaryText = row.subtitle }
        c.secondaryTextProperties.font = Fonts.ui(.sans, 13)
        c.secondaryTextProperties.color = Palette.secondary
        if case let .action(destructive) = row.kind {
            c.textProperties.color = destructive ? Palette.danger : Palette.accent
        }
        if row.id.hasPrefix("accent:#"), let hex = UInt32(row.id.dropFirst(8), radix: 16) {
            c.image = UIImage(systemName: "circle.fill")
            c.imageProperties.tintColor = UIColor(hex: hex)
        }
        cell.contentConfiguration = c
        var accessories: [UICellAccessory] = []
        if let value = row.value, !row.isText {
            accessories.append(.label(text: value, options: .init(tintColor: Palette.secondary, font: Fonts.ui(.sans, 15))))
        }
        switch row.kind {
        case .link, .text: accessories.append(.disclosureIndicator())
        case let .check(on) where on: accessories.append(.checkmark(options: .init(tintColor: Palette.accent)))
        case let .toggle(on):
            let toggle = UISwitch()
            toggle.isOn = on
            toggle.onTintColor = Palette.accent
            toggle.accessibilityIdentifier = "toggle-\(row.id)"
            toggle.addAction(UIAction { [weak self, weak toggle] _ in
                self?.act(row.id, value: (toggle?.isOn ?? false) ? "true" : "false")
            }, for: .valueChanged)
            accessories.append(.customView(configuration: .init(customView: toggle, placement: .trailing())))
        default: break
        }
        cell.accessories = accessories
    }

    private func swipeActions(at path: IndexPath) -> UISwipeActionsConfiguration? {
        guard let row = row(at: path) else { return nil }
        let actions = row.actions.filter { $0.id != row.id }
        guard !actions.isEmpty else { return nil }
        return UISwipeActionsConfiguration(actions: actions.map { action in
            UIContextualAction(style: action.destructive ? .destructive : .normal, title: action.title) { [weak self] _, _, done in
                self?.confirm(action, at: path)
                done(false)
            }
        })
    }

    private func confirm(_ action: HostRowAction, at path: IndexPath) {
        guard let message = action.confirm else { return act(action.id, value: nil) }
        let sheet = UIAlertController(title: message, message: nil, preferredStyle: .actionSheet)
        sheet.addAction(UIAlertAction(title: action.title, style: action.destructive ? .destructive : .default) { [weak self] _ in
            self?.act(action.id, value: nil)
        })
        sheet.addAction(UIAlertAction(title: "Cancel", style: .cancel))
        sheet.popoverPresentationController?.sourceView = collectionView.cellForItem(at: path)
        present(sheet, animated: true)
    }

    private func act(_ id: String, value: String?) {
        guard let client = app.client else { return }
        spinner.startAnimating()
        collectionView.isUserInteractionEnabled = false
        Task { @MainActor in
            defer {
                spinner.stopAnimating()
                collectionView.isUserInteractionEnabled = true
            }
            do {
                let result = try await client.hostAction(deviceId: deviceId, page: pageId, action: id, value: value)
                if let id = result.pageId { pageId = id }
                if let page = result.page { show(page) } else { navigationController?.popViewController(animated: true) }
                if let url = result.openUrl.flatMap(URL.init(string:)) { await UIApplication.shared.open(url) }
            } catch {
                Toast.show("\(error)", in: view.window)
                load()
            }
        }
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt path: IndexPath) {
        collectionView.deselectItem(at: path, animated: true)
        guard let row = row(at: path) else { return }
        switch row.kind {
        case let .link(page):
            navigationController?.pushViewController(HostPageViewController(app: app, deviceId: deviceId, page: page, title: row.title), animated: true)
        case let .check(on) where !on:
            act(row.id, value: nil)
        case let .action(destructive):
            confirm(row.actions.first { $0.id == row.id } ?? HostRowAction(id: row.id, title: row.title, destructive: destructive, confirm: nil), at: path)
        case let .text(multiline, placeholder):
            let editor = TextFieldEditor(title: row.title, text: row.value ?? "", placeholder: placeholder, multiline: multiline) { [weak self] text in
                self?.act(row.id, value: text)
            }
            navigationController?.pushViewController(editor, animated: true)
        default:
            break
        }
    }
}

private extension HostRow {
    var isText: Bool {
        if case .text = kind { return true }
        return false
    }
}

private final class TextFieldEditor: UIViewController {
    private let text: String
    private let placeholder: String?
    private let multiline: Bool
    private let onSave: (String) -> Void
    private let field = UITextField()
    private let area = UITextView()

    init(title: String, text: String, placeholder: String?, multiline: Bool, onSave: @escaping (String) -> Void) {
        self.text = text
        self.placeholder = placeholder
        self.multiline = multiline
        self.onSave = onSave
        super.init(nibName: nil, bundle: nil)
        self.title = title
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.largeTitleDisplayMode = .never
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .save, primaryAction: UIAction { [weak self] _ in
            guard let self else { return }
            self.onSave(self.multiline ? self.area.text : (self.field.text ?? ""))
            self.navigationController?.popViewController(animated: true)
        })
        let input: UIView = multiline ? area : field
        input.translatesAutoresizingMaskIntoConstraints = false
        input.backgroundColor = Palette.elevated
        input.layer.cornerRadius = 12
        input.accessibilityIdentifier = "host-text-editor"
        view.addSubview(input)
        if multiline {
            area.text = text
            area.font = Fonts.ui(.sans, 16)
            area.textColor = Palette.text
            area.textContainerInset = UIEdgeInsets(top: 12, left: 10, bottom: 12, right: 10)
        } else {
            field.text = text
            field.placeholder = placeholder
            field.font = Fonts.ui(.sans, 16)
            field.textColor = Palette.text
            field.leftView = UIView(frame: CGRect(x: 0, y: 0, width: 14, height: 1))
            field.leftViewMode = .always
            field.clearButtonMode = .whileEditing
        }
        let guide = view.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            input.topAnchor.constraint(equalTo: guide.topAnchor, constant: 16),
            input.leadingAnchor.constraint(equalTo: guide.leadingAnchor, constant: 16),
            input.trailingAnchor.constraint(equalTo: guide.trailingAnchor, constant: -16),
            multiline ? input.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor, constant: -16) : input.heightAnchor.constraint(equalToConstant: 48),
        ])
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        (multiline ? area as UIView : field).becomeFirstResponder()
    }
}

private struct MeterRow: View {
    let row: HostRow
    let meter: HostMeter

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(row.title).font(Font(Fonts.ui(.sansMedium, 15)))
                if let value = row.value { Text(value).font(Font(Fonts.ui(.sans, 15))).foregroundStyle(Color(Palette.secondary)) }
                Spacer()
                if let resets = row.subtitle { Text(resets).font(Font(Fonts.ui(.sans, 13))).foregroundStyle(Color(Palette.secondary)) }
            }
            GeometryReader { geo in
                ZStack(alignment: .leading) {
                    Capsule().fill(Color(Palette.tertiary).opacity(0.25))
                    Capsule().fill(color(meter.tone)).frame(width: geo.size.width * CGFloat(min(max(meter.fill, 0), 1)))
                    if let marker = meter.marker {
                        Rectangle().fill(color(meter.markerTone)).frame(width: 2, height: 12)
                            .offset(x: geo.size.width * CGFloat(min(max(marker, 0), 1)) - 1)
                    }
                }
            }
            .frame(height: 6)
            if meter.footLeft != nil || meter.footRight != nil {
                HStack {
                    if let left = meter.footLeft { Text(left).foregroundStyle(Color(meter.footLeftAlert ? Palette.danger : Palette.secondary)) }
                    Spacer()
                    if let right = meter.footRight { Text(right).foregroundStyle(Color(meter.footRightAlert ? Palette.danger : Palette.secondary)) }
                }
                .font(Font(Fonts.ui(.sans, 12)))
            }
        }
        .foregroundStyle(Color(Palette.text))
        .padding(.vertical, 4)
    }

    private func color(_ tone: HostTone) -> Color {
        switch tone {
        case .normal: Color(Palette.accent)
        case .warn: Color(Palette.warning)
        case .critical: Color(Palette.danger)
        }
    }
}
