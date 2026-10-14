/* SPDX-License-Identifier: LGPL-2.1-or-later */
/*
 * Experiment 3: what is linked in place of gdal.c where the `gdal` feature is off, as it is
 * wherever GDAL is not installed. bil_gdal_init returns no version, which the program reports as
 * GDAL not built; nothing else here is called.
 */

#include <stddef.h>
#include <stdint.h>

#include "layer.h"

const char *bil_gdal_init(void) { return NULL; }

int bil_gdal_decode(const uint8_t *data, size_t len, int format, const BilLimits *limits,
                    uint8_t **out, size_t *out_len, BilInfo *info, char *message, size_t cap) {
    (void)data, (void)len, (void)format, (void)limits, (void)info;
    *out = NULL;
    *out_len = 0;
    copy_message(message, cap, "GDAL is not built");
    return SHIM_ERROR;
}

int bil_gdal_encode(const uint8_t *pixels, uint32_t width, uint32_t height, uint32_t components,
                    int format, int variant, uint8_t **out, size_t *out_len, char *message,
                    size_t cap) {
    (void)pixels, (void)width, (void)height, (void)components, (void)format, (void)variant;
    *out = NULL;
    *out_len = 0;
    copy_message(message, cap, "GDAL is not built");
    return SHIM_ERROR;
}

int bil_gdal_decode_unprotected(const uint8_t *data, size_t len, int format) {
    (void)data, (void)len, (void)format;
    return SHIM_ERROR;
}

size_t bil_gdal_state_describe(char *buf, size_t cap) {
    (void)buf, (void)cap;
    return 0;
}

int bil_gdal_state_poke(int what, int undo) {
    (void)what, (void)undo;
    return 0;
}
