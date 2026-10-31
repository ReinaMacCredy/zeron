import Foundation

/// A fetch is a host round trip: keystrokes share the one in flight, and a failure is not kept.
final class InvocationCatalogs {
    private var found: [String: InvocationCatalog] = [:]
    private var pending: [String: Task<InvocationCatalog, Never>] = [:]

    @MainActor
    func get(_ key: String, fetch: @escaping () async -> InvocationCatalog) async -> InvocationCatalog {
        if let hit = found[key] { return hit }
        if let task = pending[key] { return await task.value }
        let task = Task { [weak self] in
            let catalog = await fetch()
            self?.pending[key] = nil
            if catalog.warning == nil { self?.found[key] = catalog }
            return catalog
        }
        pending[key] = task
        return await task.value
    }

    var skillNames: [String] {
        found.filter { $0.key.hasSuffix("|true") }.values.flatMap { $0.entries.map(\.name) }
    }
}
