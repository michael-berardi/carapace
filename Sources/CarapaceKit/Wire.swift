import Foundation

/// A Carapace app, as described by its generated bindings.
public protocol CarapaceApp {
    associatedtype State: Decodable & Sendable
    associatedtype Action: Encodable & Sendable
    associatedtype Event: Decodable & Sendable
    associatedtype Config: Encodable & Sendable
    associatedtype Query: Encodable & Sendable
    associatedtype Answer: Decodable & Sendable

    static var name: String { get }
    /// Fingerprint of the schema the bindings were generated from.
    static var schemaHash: UInt64 { get }
}

/// Event type for cores that never ask the platform for anything.
public enum NoEvent: Decodable, Sendable, Hashable {
    public init(from decoder: Decoder) throws {
        throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "this core emits no events"))
    }
}

/// Query type for cores that export no pure queries.
public enum NoQuery: Encodable, Sendable, Hashable {
    public func encode(to encoder: Encoder) throws {}
}

/// Answer type for cores that export no pure queries.
public enum NoAnswer: Decodable, Sendable, Hashable {
    public init(from decoder: Decoder) throws {
        throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "this core exports no queries"))
    }
}

/// Anything a core can send as arbitrary JSON.
public enum JSONValue: Codable, Sendable, Hashable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let v = try? c.decode(Bool.self) { self = .bool(v) }
        else if let v = try? c.decode(Double.self) { self = .number(v) }
        else if let v = try? c.decode(String.self) { self = .string(v) }
        else if let v = try? c.decode([JSONValue].self) { self = .array(v) }
        else { self = .object(try c.decode([String: JSONValue].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let v): try c.encode(v)
        case .number(let v): try c.encode(v)
        case .string(let v): try c.encode(v)
        case .array(let v): try c.encode(v)
        case .object(let v): try c.encode(v)
        }
    }
}

/// What a running core tells its subscribers. Payloads are JSON.
public enum CarapaceNotice: Sendable {
    case state(Data)
    case event(Data)
    case fault(String)
}

/// One running core instance.
public protocol CarapaceCore: AnyObject, Sendable {
    /// Queue an action (JSON). Throws with the core's own message when it cannot be decoded.
    func dispatch(_ action: Data) throws
    /// Like `dispatch`, but returns once the action is processed and subscribers were told.
    /// Must not be called from a subscription handler.
    func dispatchAndWait(_ action: Data) throws
    func snapshot() throws -> Data
    /// The handler runs on the core's thread.
    func subscribe(_ handler: @escaping @Sendable (CarapaceNotice) -> Void) -> Int
    func unsubscribe(_ id: Int)
    /// Stops the core and joins its thread. Idempotent.
    func stop()
}

/// A way to start a core: the Rust C ABI (`RustBackend`) or a test double.
public protocol CarapaceBackend: Sendable {
    var abiVersion: UInt32 { get }
    var schemaHash: UInt64 { get }
    func start(config: Data?) throws -> any CarapaceCore
    /// A pure, stateless query (JSON in, JSON out). Safe to call from any thread.
    func query(_ query: Data) throws -> Data
}

public enum CarapaceError: Error, LocalizedError, Equatable {
    case abiMismatch(expected: UInt32, found: UInt32)
    case staleBindings(app: String, expected: UInt64, found: UInt64)
    case core(String)
    case decoding(String)

    public var errorDescription: String? {
        switch self {
        case let .abiMismatch(expected, found):
            return "Carapace ABI mismatch: CarapaceKit speaks version \(expected), the core library speaks \(found). Update both to the same release."
        case let .staleBindings(app, expected, found):
            return "The generated \(app) bindings are stale: they were generated from schema \(String(expected, radix: 16)) but the core library has schema \(String(found, radix: 16)). Run `cargo carapace gen swift` and rebuild."
        case let .core(message):
            return message
        case let .decoding(message):
            return message
        }
    }
}
