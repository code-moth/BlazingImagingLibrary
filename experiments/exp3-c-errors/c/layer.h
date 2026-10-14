/* SPDX-License-Identifier: LGPL-2.1-or-later */
/*
 * Experiment 3: what the C layers for each library share. shim.c, for libjpeg-turbo and
 * OpenJPEG, and gdal.c, for GDAL, return the same statuses, take the same limits and report
 * the same description of a decode.
 */

#ifndef BIL_EXP3_LAYER_H
#define BIL_EXP3_LAYER_H

#include <stdint.h>
#include <string.h>

/* What a decode found. */
typedef struct {
    uint32_t width;
    uint32_t height;
    uint32_t components;
    uint32_t precision;
    uint32_t warnings;
} BilInfo;

/* The caller's limits: pixels in an Image, and bytes a decode may produce. 0 means no limit. */
typedef struct {
    uint64_t max_image_pixels;
    uint64_t max_decoded_bytes;
} BilLimits;

/* This layer's own statuses, internal to the experiment, apart from the interface's BilStatus. A
 * refusal by a limit is an error under section 3.3; it is told apart only for the checks. */
enum { SHIM_OK = 0, SHIM_ERROR = 1, SHIM_REFUSED = 2 };

/* a × b × c × d, or UINT64_MAX where it would overflow. */
static inline uint64_t product(uint64_t a, uint64_t b, uint64_t c, uint64_t d) {
    uint64_t v[4] = {a, b, c, d}, p = 1;
    for (int i = 0; i < 4; i++) {
        if (v[i] != 0 && p > UINT64_MAX / v[i]) return UINT64_MAX;
        p *= v[i];
    }
    return p;
}

/* Whether an Image of these dimensions and bytes for each sample breaks the caller's limits. */
static inline int too_large(const BilLimits *limits, uint64_t w, uint64_t h, uint64_t c,
                            uint64_t bytes) {
    return (limits->max_image_pixels && product(w, h, 1, 1) > limits->max_image_pixels) ||
           (limits->max_decoded_bytes && product(w, h, c, bytes) > limits->max_decoded_bytes);
}

static inline void copy_message(char *dst, size_t cap, const char *src) {
    if (!dst || cap == 0) return;
    size_t n = strlen(src);
    if (n >= cap) n = cap - 1;
    memcpy(dst, src, n);
    dst[n] = '\0';
}

#endif
