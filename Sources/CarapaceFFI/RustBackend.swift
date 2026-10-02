import CCarapace
import CarapaceKit
import Foundation

/// Drives a Rust core linked into this binary through the Carapace C ABI.
public struct RustBackend: CarapaceBackend {
    public init() {}

    public var abiVersion: UInt32 { carapace_abi_version() }
    public var schemaHash: UInt64 { carapace_schema_hash() }

    public func start(config: Data?) throws -> any CarapaceCore {
        var error: UnsafeMutablePointer<CChar>?
        let handle: OpaquePointer? = if let config {
            String(decoding: config, as: UTF8.self).withCString { carapace_start($0, &error) }
        } else {
            carapace_start(nil, &error)
        }
        guard let handle else {
            let message = takeString(error) ?? "the core failed to start without an error message"
            throw CarapaceError.core(message)
        }
        return RustCore(handle: handle)
    }

    public func query(_ query: Data) throws -> Data {
        var error: UnsafeMutablePointer<CChar>?
        let answer = query.withUnsafeBytes { raw in
            carapace_query(raw.bindMemory(to: UInt8.self).baseAddress, raw.count, &error)
        }
        guard let answer else {
            throw CarapaceError.core(takeString(error) ?? "the core failed a query without an error message")
        }
        return Data(takeString(answer)!.utf8)
    }
}

private func takeString(_ p: UnsafeMutablePointer<CChar>?) -> String? {
    guard let p else { return nil }
    defer { carapace_string_free(p) }
    return String(cString: p)
}

private final class Subscription: @unchecked Sendable {
    let handler: @Sendable (CarapaceNotice) -> Void
    private let lock = NSLock()
    private var cancelled = false
    init(_ handler: @escaping @Sendable (CarapaceNotice) -> Void) { self.handler = handler }
    func cancel() { lock.lock(); cancelled = true; lock.unlock() }
    var isCancelled: Bool { lock.lock(); defer { lock.unlock() }; return cancelled }
}

private final class RustCore: CarapaceCore, @unchecked Sendable {
    private var handle: OpaquePointer?
    private let lock = NSLock()
    private var subs: [Int: Unmanaged<Subscription>] = [:]

    init(handle: OpaquePointer) { self.handle = handle }

    func dispatch(_ action: Data) throws {
        guard let h = live() else { throw CarapaceError.core("the core has been stopped") }
        let message = action.withUnsafeBytes { raw in
            carapace_dispatch(h, raw.bindMemory(to: UInt8.self).baseAddress, raw.count)
        }
        if let text = takeString(message) { throw CarapaceError.core(text) }
    }

    func dispatchAndWait(_ action: Data) throws {
        guard let h = live() else { throw CarapaceError.core("the core has been stopped") }
        let message = action.withUnsafeBytes { raw in
            carapace_dispatch_wait(h, raw.bindMemory(to: UInt8.self).baseAddress, raw.count)
        }
        if let text = takeString(message) { throw CarapaceError.core(text) }
    }

    func snapshot() throws -> Data {
        guard let h = live() else { throw CarapaceError.core("the core has been stopped") }
        guard let text = takeString(carapace_state(h)) else { throw CarapaceError.core("the core returned no state") }
        return Data(text.utf8)
    }

    func subscribe(_ handler: @escaping @Sendable (CarapaceNotice) -> Void) -> Int {
        guard let h = live() else { return 0 }
        let sub = Unmanaged.passRetained(Subscription(handler))
        let id = Int(carapace_subscribe(h, { user, kind, data, len in
            guard let user else { return }
            let sub = Unmanaged<Subscription>.fromOpaque(user).takeUnretainedValue()
            if sub.isCancelled { return }
            let bytes = data.map { Data(bytes: $0, count: len) } ?? Data()
            switch kind {
            case 0: sub.handler(.state(bytes))
            case 1: sub.handler(.event(bytes))
            default: sub.handler(.fault(String(decoding: bytes, as: UTF8.self)))
            }
        }, sub.toOpaque()))
        lock.lock(); subs[id] = sub; lock.unlock()
        return id
    }

    func unsubscribe(_ id: Int) {
        lock.lock()
        let sub = subs[id]
        lock.unlock()
        // The memory stays until stop(): the core thread may be inside the callback right now.
        sub?.takeUnretainedValue().cancel()
        if let h = live() { carapace_unsubscribe(h, UInt64(id)) }
    }

    func stop() {
        lock.lock()
        let h = handle
        handle = nil
        let owned = subs
        subs = [:]
        lock.unlock()
        guard let h else { return }
        carapace_stop(h) // joins the core thread, so no callback runs after this returns
        owned.values.forEach { $0.release() }
    }

    deinit { stop() }

    private func live() -> OpaquePointer? {
        lock.lock(); defer { lock.unlock() }
        return handle
    }
}
