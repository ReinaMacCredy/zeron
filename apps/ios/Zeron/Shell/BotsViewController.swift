import UIKit

/// The Bots tab: your Bots (pinned sessions) and what needs you, the open
/// Inbox cards each Bot keeps on its engine (`BotInbox`).
final class BotsViewController: UITableViewController {
    private let app: AppModel
    private var bots: [SessionRowVM] = []
    private var cards: [(bot: SessionRowVM, card: [String: Any])] = []

    init(app: AppModel) {
        self.app = app
        super.init(style: .insetGrouped)
        title = "Bots"
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidLoad() {
        super.viewDidLoad()
        tableView.register(UITableViewCell.self, forCellReuseIdentifier: "row")
        refreshControl = UIRefreshControl()
        refreshControl?.addTarget(self, action: #selector(reload), for: .valueChanged)
        navigationItem.rightBarButtonItem = UIBarButtonItem(
            systemItem: .add,
            primaryAction: UIAction { [weak self] _ in self?.router?.presentNewSession(prompt: nil) })
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        reload()
    }

    @objc private func reload() {
        bots = app.sessions(inFolder: "pinned")
        tableView.reloadData()
        let bots = self.bots
        let client = app.client
        Task { [weak self] in
            var found: [(bot: SessionRowVM, card: [String: Any])] = []
            for bot in bots where !bot.deviceId.isEmpty {
                let params = "{\"botId\":\"\(bot.id)\",\"action\":\"list\"}"
                guard let reply = try? await client?.botCallJson(deviceId: bot.deviceId, method: "BotInbox", paramsJson: params),
                      let data = reply.data(using: .utf8),
                      let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let items = json["items"] as? [[String: Any]] else { continue }
                found += items.map { (bot, $0) }
            }
            await MainActor.run {
                self?.cards = found
                self?.tableView.reloadData()
                self?.refreshControl?.endRefreshing()
            }
        }
    }

    override func numberOfSections(in tableView: UITableView) -> Int { 2 }

    override func tableView(_ tableView: UITableView, titleForHeaderInSection section: Int) -> String? {
        section == 0 ? "Bots" : "Needs you"
    }

    override func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int {
        max(1, section == 0 ? bots.count : cards.count)
    }

    override func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let cell = tableView.dequeueReusableCell(withIdentifier: "row", for: indexPath)
        var c = cell.defaultContentConfiguration()
        if indexPath.section == 0 {
            if bots.isEmpty {
                c.text = "No Bots yet. Pin a session to make it a Bot."
                c.textProperties.color = .secondaryLabel
            } else {
                c.text = bots[indexPath.row].title
                c.image = UIImage(systemName: "person.crop.circle.fill")
                c.imageProperties.tintColor = UIColor(red: 0.75, green: 0.15, blue: 0.64, alpha: 1)
            }
        } else if cards.isEmpty {
            c.text = "Nothing needs you."
            c.textProperties.color = .secondaryLabel
        } else {
            let item = cards[indexPath.row]
            c.text = item.card["title"] as? String
            c.secondaryText = "\(item.bot.title) · \(item.card["summary"] as? String ?? "")"
            c.image = UIImage(systemName: "circle.fill")
            c.imageProperties.tintColor = .systemOrange
        }
        cell.contentConfiguration = c
        return cell
    }

    override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        if indexPath.section == 0, bots.indices.contains(indexPath.row) {
            router?.openSession(bots[indexPath.row].id)
        } else if cards.indices.contains(indexPath.row) {
            router?.openSession(cards[indexPath.row].bot.id)
        }
    }
}
