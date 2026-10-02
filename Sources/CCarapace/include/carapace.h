#ifndef CARAPACE_H
#define CARAPACE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The Carapace C ABI, version 1. Every core exports exactly these symbols.
 * Strings returned as char* are owned by the caller: free with carapace_string_free.
 * All payloads are UTF-8 JSON. */

typedef struct CarapaceHandle CarapaceHandle;

/* kind: 0 = state snapshot, 1 = event, 2 = fault. data is UTF-8, NOT NUL-terminated.
 * Runs on the core thread: copy what you need and return. */
typedef void (*CarapaceCallback)(void *user, uint32_t kind, const uint8_t *data, size_t len);

uint32_t carapace_abi_version(void);
uint64_t carapace_schema_hash(void);
char *carapace_schema(void);

/* config: JSON object, or NULL for defaults. On failure returns NULL and sets *error. */
CarapaceHandle *carapace_start(const char *config, char **error);
/* Returns NULL on success, otherwise an error message. */
char *carapace_dispatch(CarapaceHandle *handle, const uint8_t *json, size_t len);
/* Like carapace_dispatch, but returns only after the action was processed and subscribers were
 * notified. Never call from inside a subscription callback. */
char *carapace_dispatch_wait(CarapaceHandle *handle, const uint8_t *json, size_t len);
/* Pure, stateless query. Returns the answer JSON, or NULL with *error set. Callable from any thread. */
char *carapace_query(const uint8_t *json, size_t len, char **error);
char *carapace_state(CarapaceHandle *handle);
uint64_t carapace_subscribe(CarapaceHandle *handle, CarapaceCallback callback, void *user);
void carapace_unsubscribe(CarapaceHandle *handle, uint64_t id);
/* Joins the core thread. Never call from inside a callback. */
void carapace_stop(CarapaceHandle *handle);
void carapace_string_free(char *s);

#ifdef __cplusplus
}
#endif

#endif
