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
    /// Guards `handle`, `subs` and `inflight`. `stop` waits for calls already inside the core, so
    /// the handle is never freed under another thread's call.
    private let cond = NSCondition()
    private var inflight = 0
    private var subs: [Int: Unmanaged<Subscription>] = [:]

    init(handle: OpaquePointer) { self.handle = handle }

    /// The live handle, counted as in use until `leave()`. Nil once stopped.
    private func enter() -> OpaquePointer? {
        cond.lock(); defer { cond.unlock() }
        guard let h = handle else { return nil }
        inflight += 1
        return h
    }

    private func leave() {
        cond.lock()
        inflight -= 1
        if inflight == 0 { cond.broadcast() }
        cond.unlock()
    }

    func dispatch(_ action: Data) throws {
        guard let h = enter() else { throw CarapaceError.core("the core has been stopped") }
        defer { leave() }
        let message = action.withUnsafeBytes { raw in
            carapace_dispatch(h, raw.bindMemory(to: UInt8.self).baseAddress, raw.count)
        }
        if let text = takeString(message) { throw CarapaceError.core(text) }
    }

    func dispatchAndWait(_ action: Data) throws {
        guard let h = enter() else { throw CarapaceError.core("the core has been stopped") }
        defer { leave() }
        let message = action.withUnsafeBytes { raw in
            carapace_dispatch_wait(h, raw.bindMemory(to: UInt8.self).baseAddress, raw.count)
        }
        if let text = takeString(message) { throw CarapaceError.core(text) }
    }

    func snapshot() throws -> Data {
        guard let h = enter() else { throw CarapaceError.core("the core has been stopped") }
        defer { leave() }
        guard let text = takeString(carapace_state(h)) else { throw CarapaceError.core("the core returned no state") }
        return Data(text.utf8)
    }

    func subscribe(_ handler: @escaping @Sendable (CarapaceNotice) -> Void) -> Int {
        guard let h = enter() else { return 0 }
        defer { leave() }
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
        cond.lock(); subs[id] = sub; cond.unlock()
        return id
    }

    func unsubscribe(_ id: Int) {
        guard let h = enter() else { return }
        defer { leave() }
        cond.lock()
        let sub = subs.removeValue(forKey: id)
        cond.unlock()
        sub?.takeUnretainedValue().cancel()
        // Returns once no callback for `id` is running or will start; only then is the memory freed.
        carapace_unsubscribe(h, UInt64(id))
        sub?.release()
    }

    func stop() {
        cond.lock()
        let h = handle
        handle = nil
        while inflight > 0 { cond.wait() }
        let owned = subs
        subs = [:]
        cond.unlock()
        guard let h else { return }
        carapace_stop(h) // joins the core thread, so no callback runs after this returns
        owned.values.forEach { $0.release() }
    }

    deinit { stop() }
}
