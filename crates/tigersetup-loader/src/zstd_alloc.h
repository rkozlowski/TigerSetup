/*
 * The allocator libzstd's decoder is compiled against in the loader.
 *
 * libzstd takes its allocator from `ZSTD_malloc`, `ZSTD_calloc` and
 * `ZSTD_free`, which `zstd_deps.h` defines as the C runtime's unless
 * `ZSTD_DEPS_MALLOC` is already defined. The build script defines it and
 * forces this header into every libzstd translation unit, so the decoder
 * allocates from the process heap and the loader links no C runtime
 * allocator at all.
 */

#ifndef TIGERSETUP_LOADER_ZSTD_ALLOC_H
#define TIGERSETUP_LOADER_ZSTD_ALLOC_H

#include <stddef.h>

void *tigersetup_loader_zstd_malloc(size_t size);
void *tigersetup_loader_zstd_calloc(size_t count, size_t size);
void tigersetup_loader_zstd_free(void *block);

#define ZSTD_malloc(s) tigersetup_loader_zstd_malloc(s)
#define ZSTD_calloc(n, s) tigersetup_loader_zstd_calloc((n), (s))
#define ZSTD_free(p) tigersetup_loader_zstd_free(p)

#endif
