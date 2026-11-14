import UIKit

enum TaskImages {
    static func upload(_ images: [StagedImage], to deviceId: String, client: CoreClient) async throws -> [String] {
        var paths: [String] = []
        for image in images {
            paths.append(try await client.uploadAttachment(deviceId: deviceId, name: image.name, data: image.data, progress: nil))
        }
        return paths
    }
}

final class NewTaskViewController: UIViewController {
    private let app: AppModel
    private let opened: (String) -> Void
    private let titleField = UITextField()
    private let notes = UITextView()
    private let hostButton = UIButton(configuration: .gray())
    private let imagesButton = UIButton(configuration: .gray())
    private var hostId: String?
    private var images: [StagedImage] = []

    init(app: AppModel, opened: @escaping (String) -> Void) {
        self.app = app
        self.opened = opened
        super.init(nibName: nil, bundle: nil)
        title = "New task"
        hostId = app.hostOptions.first { $0.online }?.id ?? app.hostOptions.first?.id
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background
        navigationItem.leftBarButtonItem = UIBarButtonItem(systemItem: .cancel, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        let create = UIBarButtonItem(title: "Create", primaryAction: UIAction { [weak self] _ in self?.create() })
        create.style = .prominent
        create.accessibilityIdentifier = "new-task-create"
        navigationItem.rightBarButtonItem = create

        titleField.placeholder = "Title"
        titleField.font = Fonts.ui(.sansMedium, 17)
        titleField.accessibilityIdentifier = "new-task-title"
        notes.font = Fonts.ui(.sans, 15)
        notes.backgroundColor = Palette.elevated
        notes.layer.cornerRadius = 10
        notes.accessibilityIdentifier = "new-task-notes"
        notes.heightAnchor.constraint(equalToConstant: 120).isActive = true
        hostButton.showsMenuAsPrimaryAction = true
        hostButton.accessibilityIdentifier = "new-task-host"
        imagesButton.addAction(UIAction { [weak self] _ in
            guard let self else { return }
            AttachmentPicker(host: self, limit: 12 - images.count) { [weak self] picked in
                self?.images += picked
                self?.sync()
            }.presentPhotos()
        }, for: .primaryActionTriggered)
        let stack = UIStackView(arrangedSubviews: [titleField, notes, hostButton, imagesButton])
        stack.axis = .vertical
        stack.spacing = 14
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 16),
            stack.leadingAnchor.constraint(equalTo: view.layoutMarginsGuide.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: view.layoutMarginsGuide.trailingAnchor),
        ])
        sync()
        titleField.becomeFirstResponder()
    }

    private func sync() {
        let hosts = app.hostOptions
        hostButton.configuration?.title = "Runs on " + (hosts.first { $0.id == hostId }?.name ?? "no Mac")
        hostButton.menu = UIMenu(children: hosts.map { host in
            UIAction(title: host.online ? host.name : "\(host.name) · offline", state: host.id == hostId ? .on : .off) { [weak self] _ in
                self?.hostId = host.id
                self?.sync()
            }
        })
        imagesButton.configuration?.title = images.isEmpty ? "Add images" : "\(images.count) image\(images.count == 1 ? "" : "s") · add more"
    }

    private func create() {
        let text = [titleField.text ?? "", notes.text ?? ""].joined(separator: "\n")
        guard let client = app.client, let hostId, !(titleField.text ?? "").trimmingCharacters(in: .whitespaces).isEmpty else { return }
        navigationItem.rightBarButtonItem?.isEnabled = false
        Task { @MainActor in
            do {
                let paths = try await TaskImages.upload(images, to: hostId, client: client)
                let taskId = try await client.createTask(deviceId: hostId, draft: Self.draft(text: text, images: paths))
                opened(taskId)
            } catch {
                navigationItem.rightBarButtonItem?.isEnabled = true
                Toast.show(TasksErrors.message(error), in: view.window)
            }
        }
    }

    private static func draft(text: String, images: [String]) -> TaskDraft {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let lines = trimmed.split(separator: "\n", maxSplits: 1, omittingEmptySubsequences: false)
        let title = lines.first.map(String.init) ?? ""
        let notes = lines.count > 1 ? String(lines[1]).trimmingCharacters(in: .whitespacesAndNewlines) : ""
        return TaskDraft(title: title, notes: notes, spaceId: nil, images: images, context: [])
    }
}
