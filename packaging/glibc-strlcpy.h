/* Force-included into libpg_query's C build (pg_parse) on glibc older than 2.38, which has no strlcpy: libpg_query
 * 18.0.0 assumes it does (fixed upstream in libpg_query #339, not yet in pg_parse). Each file gets a private copy,
 * so neither its example programs nor Lenz's executables need the symbol. packaging/package.sh adds it. */
#include <stddef.h>
#include <string.h>

static inline size_t lenz_strlcpy(char *dst, const char *src, size_t size) {
    size_t len = strlen(src);
    if (size != 0) {
        size_t n = len < size - 1 ? len : size - 1;
        memcpy(dst, src, n);
        dst[n] = '\0';
    }
    return len;
}
#define strlcpy lenz_strlcpy
