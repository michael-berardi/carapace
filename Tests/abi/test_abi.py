"""Language-neutral proof of the Carapace C ABI.

Loads a core's cdylib with nothing but ctypes and drives it. Anything that can call C
(Dart FFI, Kotlin/JNA, C# P/Invoke, Qt, Go cgo, Ruby FFI) uses the same ten functions.

Usage: python3 tests/abi/test_abi.py <path-to-cdylib>
"""
import ctypes
import json
import sys
import threading
import time

lib = ctypes.CDLL(sys.argv[1])
c_char_p_owned = ctypes.c_void_p  # returned strings are owned: read, then free

CALLBACK = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_size_t)

lib.carapace_abi_version.restype = ctypes.c_uint32
lib.carapace_schema_hash.restype = ctypes.c_uint64
lib.carapace_schema.restype = c_char_p_owned
lib.carapace_start.restype = ctypes.c_void_p
lib.carapace_start.argtypes = [ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
lib.carapace_dispatch.restype = c_char_p_owned
lib.carapace_dispatch.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t]
lib.carapace_dispatch_wait.restype = c_char_p_owned
lib.carapace_dispatch_wait.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t]
lib.carapace_query.restype = c_char_p_owned
lib.carapace_query.argtypes = [ctypes.c_char_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_void_p)]
lib.carapace_state.restype = c_char_p_owned
lib.carapace_state.argtypes = [ctypes.c_void_p]
lib.carapace_subscribe.restype = ctypes.c_uint64
lib.carapace_subscribe.argtypes = [ctypes.c_void_p, CALLBACK, ctypes.c_void_p]
lib.carapace_unsubscribe.argtypes = [ctypes.c_void_p, ctypes.c_uint64]
lib.carapace_stop.argtypes = [ctypes.c_void_p]
lib.carapace_string_free.argtypes = [ctypes.c_void_p]


def take(ptr):
    if not ptr:
        return None
    text = ctypes.string_at(ptr).decode()
    lib.carapace_string_free(ptr)
    return text


def check(cond, msg):
    if not cond:
        print("FAIL:", msg)
        sys.exit(1)
    print("ok  ", msg)


check(lib.carapace_abi_version() == 1, "ABI version is 1")
schema = json.loads(take(lib.carapace_schema()))
check(schema["name"] == "Counter" and "Action" in schema["definitions"], "schema bundle describes the app")
check(lib.carapace_schema_hash() != 0, "schema hash is exported")

err = ctypes.c_void_p()
bad = lib.carapace_start(b"{not json", ctypes.byref(err))
check(not bad and "cannot decode config" in take(err), "bad config fails with a message, not a crash")

err = ctypes.c_void_p()
h = lib.carapace_start(b'{"start": 5}', ctypes.byref(err))
check(bool(h), "core starts from JSON config")
check(json.loads(take(lib.carapace_state(h)))["count"] == 5, "state reflects config")

notices, cond = [], threading.Condition()


@CALLBACK
def on_notice(_user, kind, data, length):
    with cond:
        notices.append((kind, ctypes.string_at(data, length).decode()))
        cond.notify_all()


sub = lib.carapace_subscribe(h, on_notice, None)
check(sub > 0, "subscribe returns an id")
with cond:
    check(any(k == 1 and "Core started" in t for k, t in notices), "start-up event is replayed to the first subscriber")

body = json.dumps({"type": "increment"}).encode()
check(lib.carapace_dispatch(h, body, len(body)) is None, "dispatch accepts a valid action")
with cond:
    cond.wait_for(lambda: any(k == 0 and '"count":6' in t for k, t in notices), timeout=3)
    check(any(k == 0 and '"count":6' in t for k, t in notices), "state notice arrives on the callback")

body = b'{"type":"nope"}'
msg = take(lib.carapace_dispatch(h, body, len(body)))
check(msg and "Counter: cannot decode action" in msg and "nope" in msg, "unknown action is rejected naming the app and input")

body = json.dumps({"type": "fetch"}).encode()
lib.carapace_dispatch(h, body, len(body))
with cond:
    cond.wait_for(lambda: any(k == 1 and "Fetched" in t for k, t in notices), timeout=3)
    check(any(k == 1 and "Fetched" in t for k, t in notices), "background work and events cross the ABI")

lib.carapace_unsubscribe(h, sub)
before = len(notices)
body = json.dumps({"type": "increment"}).encode()
lib.carapace_dispatch(h, body, len(body))
time.sleep(0.1)
check(len(notices) == before, "unsubscribe stops callbacks")

body = json.dumps({"type": "setStep", "step": 7}).encode()
check(lib.carapace_dispatch_wait(h, body, len(body)) is None, "dispatch_wait returns after processing")
check(json.loads(take(lib.carapace_state(h)))["step"] == 7, "state is already updated when dispatch_wait returns")

q = b'{"type":"describe","value":-3}'
err = ctypes.c_void_p()
check(json.loads(take(lib.carapace_query(q, len(q), ctypes.byref(err))))["text"] == "negative, odd", "pure query answers without a handle")
q = b'{"type":"nope"}'
check(lib.carapace_query(q, len(q), ctypes.byref(err)) is None and "cannot decode query" in take(err), "bad query fails with a message")

start = time.perf_counter()
lib.carapace_stop(h)
check(time.perf_counter() - start < 1.0, "stop joins the core thread promptly")
print("all ABI checks passed")
