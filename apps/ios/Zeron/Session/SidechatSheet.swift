import UIKit

final class SidechatSheetViewController: UITableViewController {
    private let app: AppModel
    private let client: CoreClient
    private let sourceId: String
    private let sourceTitle: String
    private var sheet: SidechatSheet

    init(app: AppModel, client: CoreClient, sourceId: String, sourceTitle: String) {
        self.app = app
        self.client = client
        self.sourceId = sourceId
        self.sourceTitle = sourceTitle
        sheet = client.sidechatSheet(sourceChatId: sourceId, sourceTitle: sourceTitle)
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
        title = sheet.title
        navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "cell")
        tableView.tableHeaderView = contextHeader()
        tableView.tableFooterView = createFooter()
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        sheet = client.sidechatSheet(sourceChatId: sourceId, sourceTitle: sourceTitle)
        tableView.reloadData()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        for view in [tableView.tableHeaderView, tableView.tableFooterView].compactMap({ $0 }) {
            let size = view.systemLayoutSizeFitting(CGSize(width: tableView.bounds.width, height: 0), withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel)
            guard view.frame.size.height != size.height else { continue }
            view.frame.size.height = size.height
            if view === tableView.tableHeaderView { tableView.tableHeaderView = view } else { tableView.tableFooterView = view }
        }
    }

    func openSidechat(prompt: String?) {
        run { [client, sourceId] in try await client.openSidechat(sourceChatId: sourceId, prompt: prompt) }
    }

    private func create() {
        run { [client, sourceId] in try await client.createSidechat(sourceChatId: sourceId) }
    }

    private func run(_ work: @escaping () async throws -> String) {
        Task { @MainActor [weak self] in
            do {
                let id = try await work()
                self?.show(id)
            } catch {
                let alert = UIAlertController(title: nil, message: "\(error)", preferredStyle: .alert)
                alert.addAction(UIAlertAction(title: "OK", style: .default))
                self?.present(alert, animated: true)
            }
        }
    }

    private func show(_ chatId: String) {
        guard let session = SessionViewController(app: app, chatId: chatId) else { return }
        navigationController?.pushViewController(session, animated: true)
    }

    private func contextHeader() -> UIView {
        let label = UILabel()
        label.text = sheet.context
        label.font = Fonts.ui(.sans, 12)
        label.textColor = Palette.secondary
        label.textAlignment = .center
        label.numberOfLines = 0
        label.translatesAutoresizingMaskIntoConstraints = false
        let header = UIView()
        header.addSubview(label)
        NSLayoutConstraint.activate([
            label.topAnchor.constraint(equalTo: header.topAnchor, constant: 4),
            label.bottomAnchor.constraint(equalTo: header.bottomAnchor, constant: -8),
            label.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 24),
            label.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -24),
        ])
        return header
    }

    private func createFooter() -> UIView {
        var config = UIButton.Configuration.filled()
        config.title = "Create"
        config.cornerStyle = .large
        let button = UIButton(configuration: config, primaryAction: UIAction { [weak self] _ in self?.create() })
        button.accessibilityIdentifier = "sidechat-create"
        button.translatesAutoresizingMaskIntoConstraints = false
        let footer = UIView()
        footer.addSubview(button)
        NSLayoutConstraint.activate([
            button.topAnchor.constraint(equalTo: footer.topAnchor, constant: 12),
            button.bottomAnchor.constraint(equalTo: footer.bottomAnchor, constant: -12),
            button.leadingAnchor.constraint(equalTo: footer.leadingAnchor, constant: 20),
            button.trailingAnchor.constraint(equalTo: footer.trailingAnchor, constant: -20),
            button.heightAnchor.constraint(equalToConstant: 48),
        ])
        return footer
    }

    override func numberOfSections(in tableView: UITableView) -> Int { sheet.rows.isEmpty ? 0 : 1 }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { sheet.rows.count }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? { sheet.section }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let row = sheet.rows[indexPath.row]
        let cell = tableView.dequeueReusableCell(withIdentifier: "cell", for: indexPath)
        var content = UIListContentConfiguration.subtitleCell()
        content.text = row.title
        content.secondaryText = row.timeLeft
        content.secondaryTextProperties.color = Palette.secondary
        content.image = UIImage(systemName: "bubble.left")
        content.imageProperties.tintColor = Palette.secondary
        cell.contentConfiguration = content
        cell.accessoryType = .disclosureIndicator
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        show(sheet.rows[indexPath.row].chatId)
    }
}
