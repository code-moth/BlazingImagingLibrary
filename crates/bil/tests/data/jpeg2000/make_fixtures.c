/* SPDX-License-Identifier: LGPL-2.1-or-later */
/* Writes small lossless JPEG 2000 codestreams for BIL's tests. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "openjpeg.h"

static int value(int x, int y, int c, int prec, int sgnd) {
    long m = 1L << prec;
    long v = ((long)x * 7 + (long)y * 13 + (long)c * 31 + (long)x * y) % m;
    return (int)(sgnd ? v - m / 2 : v);
}

static int write(const char *path, int w, int h, int nc, int prec, int sgnd, int tile, int jp2) {
    opj_image_cmptparm_t p[4];
    memset(p, 0, sizeof p);
    for (int c = 0; c < nc; c++) {
        p[c].dx = 1; p[c].dy = 1; p[c].w = w; p[c].h = h; p[c].prec = prec; p[c].sgnd = sgnd;
    }
    opj_image_t *im = opj_image_create(nc, p, nc == 3 ? OPJ_CLRSPC_SRGB : OPJ_CLRSPC_GRAY);
    im->x0 = 0; im->y0 = 0; im->x1 = w; im->y1 = h;
    for (int c = 0; c < nc; c++)
        for (int y = 0; y < h; y++)
            for (int x = 0; x < w; x++)
                im->comps[c].data[y * w + x] = value(x, y, c, prec, sgnd);
    opj_cparameters_t cp;
    opj_set_default_encoder_parameters(&cp);
    cp.tcp_numlayers = 1; cp.tcp_rates[0] = 0; cp.cp_disto_alloc = 1;
    cp.irreversible = 0;
    cp.numresolution = 3;
    if (tile) { cp.tile_size_on = OPJ_TRUE; cp.cp_tdx = tile; cp.cp_tdy = tile; }
    opj_codec_t *codec = opj_create_compress(jp2 ? OPJ_CODEC_JP2 : OPJ_CODEC_J2K);
    if (!opj_setup_encoder(codec, &cp, im)) return 1;
    opj_stream_t *s = opj_stream_create_default_file_stream(path, OPJ_FALSE);
    if (!s || !opj_start_compress(codec, im, s) || !opj_encode(codec, s) || !opj_end_compress(codec, s)) return 1;
    opj_stream_destroy(s); opj_destroy_codec(codec); opj_image_destroy(im);
    return 0;
}

int main(int argc, char **argv) {
    /* `make_fixtures timing` writes two large tiled codestreams, not kept in the repository,
       for examples/jpeg2000_tiles.rs (tests/RESULTS-jpeg2000-tiles.md). */
    if (argc > 1 && strcmp(argv[1], "timing") == 0)
        return write("big_gray.j2k", 4096, 4096, 1, 8, 0, 512, 0)
            | write("big_rgb.j2k", 2048, 2048, 3, 8, 0, 256, 0);
    return write("gray8.j2k", 37, 29, 1, 8, 0, 16, 0)
        | write("rgb8.j2k", 23, 19, 3, 8, 0, 0, 0)
        | write("gray16.j2k", 21, 17, 1, 16, 0, 0, 0)
        | write("signed12.j2k", 20, 18, 1, 12, 1, 0, 0)
        | write("gray8.jp2", 37, 29, 1, 8, 0, 16, 1);
}
