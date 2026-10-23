import UIKit

final class ThreadPickerController: SessionListController, UISearchResultsUpdating {
    private let include: (String) -> Bool
    private let onPick: (String) -> Void
    private var query = ""

    init(app: AppModel, title: String, include: @escaping (String) -> Bool, onPick: @escaping (String) -> Void) {
        self.include = include
        self.onPick = onPick
        super.init(app: app)
        self.title = title
        swipes = false
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        collectionView.accessibilityIdentifier = "thread-picker"
        navigationItem.leftBarButtonItem = UIBarButtonItem(systemItem: .cancel, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        let search = UISearchController(searchResultsController: nil)
        search.searchResultsUpdater = self
        search.obscuresBackgroundDuringPresentation = false
        search.searchBar.placeholder = "Threads"
        navigationItem.searchController = search
        navigationItem.hidesSearchBarWhenScrolling = false
    }

    func updateSearchResults(for searchController: UISearchController) {
        query = searchController.searchBar.text ?? ""
        reload(animated: true)
    }

    override func buildSections() -> [(id: String, header: String?, folders: [FolderRowVM], sessions: [SessionRowVM])] {
        [("threads", nil, [], app.search(query).filter { include($0.id) })]
    }

    override func openSession(_ id: String) {
        onPick(id)
    }

    static func present(from host: UIViewController, app: AppModel, title: String, include: @escaping (String) -> Bool = { _ in true }, onPick: @escaping (String) -> Void) {
        let picker = ThreadPickerController(app: app, title: title, include: include) { [weak host] id in
            host?.dismiss(animated: true) { onPick(id) }
        }
        let nav = UINavigationController(rootViewController: picker)
        if let sheet = nav.sheetPresentationController {
            sheet.detents = [.large()]
            sheet.prefersGrabberVisible = true
        }
        host.present(nav, animated: true)
    }
}
