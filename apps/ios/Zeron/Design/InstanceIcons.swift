import UIKit

@MainActor
final class InstanceIcons {
    static let shared = InstanceIcons()
    static let didChange = Notification.Name("InstanceIcons.didChange")

    private var icons: [String: UIImage] = [:]
    private var loaded: Set<String> = []

    func icon(deviceId: String, instanceId: String?, client: CoreClient?) -> UIImage? {
        guard let instanceId, !deviceId.isEmpty else { return nil }
        if let hit = icons["\(deviceId)/\(instanceId)"] { return hit }
        guard let client, loaded.insert(deviceId).inserted else { return nil }
        Task { @MainActor in
            var changed = false
            for row in await client.listProviderInstances(deviceId: deviceId) {
                guard let path = row.iconPath,
                      let svg = try? await client.readAttachment(deviceId: deviceId, path: path),
                      let png = svgIconPng(svg: svg, side: 48),
                      let image = UIImage(data: png, scale: 3)
                else { continue }
                icons["\(deviceId)/\(row.id)"] = image.withRenderingMode(.alwaysTemplate)
                changed = true
            }
            if changed { NotificationCenter.default.post(name: Self.didChange, object: nil) }
        }
        return nil
    }
}
