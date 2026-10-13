/* SPDX-License-Identifier: LGPL-2.1-or-later */
/*
 * Experiment 3: the C layer between BIL and two C libraries, libjpeg-turbo and OpenJPEG.
 *
 * Every call into a library goes through a function here, and every one of them returns a
 * status, whatever the library does:
 *
 * - libjpeg reports a fatal error by calling error_exit, which by default prints to stderr and
 *   calls exit(). Here error_exit formats the message into the caller's buffer and longjmps
 *   back to the function that called the library, which frees everything and returns. The jump
 *   crosses only C frames: setjmp is here, in the same function that calls the library, and no
 *   Rust frame lies between them.
 * - libjpeg's warnings, such as corrupt data it recovered from, are counted, never printed.
 * - libjpeg can be made to loop over thousands of progressive scans; a progress monitor stops a
 *   decode after BIL_MAX_SCANS scans.
 * - OpenJPEG reports through callbacks, which here copy the message and never print.
 * - Before any pixel memory is taken, the size the header claims is checked against the
 *   caller's limits (section 10.3 of the specification), and libjpeg's own working memory is
 *   capped too, a choice for the experiment. OpenJPEG is told not to expand JP2 palettes, which
 *   would add components after the check; that also leaves a JP2 file's component mapping and
 *   channel definitions unapplied, which is proposed to the author as the driver's work.
 *
 * Nothing here writes to stdout or stderr, changes the locale, signal handlers or floating-point
 * environment, or keeps state between calls.
 */

#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <setjmp.h>

#include "jpeglib.h"
#include "openjpeg.h"

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

/* Progressive scans a decode may read, a limit the specification does not yet have. */
#define BIL_MAX_SCANS 500

/* a × b × c × d, or UINT64_MAX where it would overflow. */
static uint64_t product(uint64_t a, uint64_t b, uint64_t c, uint64_t d) {
    uint64_t v[4] = {a, b, c, d}, p = 1;
    for (int i = 0; i < 4; i++) {
        if (v[i] != 0 && p > UINT64_MAX / v[i]) return UINT64_MAX;
        p *= v[i];
    }
    return p;
}

/* Whether an Image of these dimensions and bytes for each sample breaks the caller's limits. */
static int too_large(const BilLimits *limits, uint64_t w, uint64_t h, uint64_t c, uint64_t bytes) {
    return (limits->max_image_pixels && product(w, h, 1, 1) > limits->max_image_pixels) ||
           (limits->max_decoded_bytes && product(w, h, c, bytes) > limits->max_decoded_bytes);
}

static void copy_message(char *dst, size_t cap, const char *src) {
    if (!dst || cap == 0) return;
    size_t n = strlen(src);
    if (n >= cap) n = cap - 1;
    memcpy(dst, src, n);
    dst[n] = '\0';
}

void bil_free(void *p) { free(p); }

/* ------------------------------------------------------------------------- libjpeg */

typedef struct {
    struct jpeg_error_mgr pub;
    jmp_buf jump;
    char *message;
    size_t message_cap;
    uint32_t warnings;
} BilJpegError;

static void bil_jpeg_error_exit(j_common_ptr cinfo) {
    BilJpegError *e = (BilJpegError *)cinfo->err;
    char text[JMSG_LENGTH_MAX];
    (*cinfo->err->format_message)(cinfo, text);
    copy_message(e->message, e->message_cap, text);
    longjmp(e->jump, 1);
}

static void bil_jpeg_output_message(j_common_ptr cinfo) {
    (void)cinfo; /* never printed */
}

static void bil_jpeg_emit_message(j_common_ptr cinfo, int level) {
    BilJpegError *e = (BilJpegError *)cinfo->err;
    if (level < 0) {
        /* A warning: corrupt data the library recovered from. Its text is kept, the first. */
        if (e->warnings == 0) {
            char text[JMSG_LENGTH_MAX];
            (*cinfo->err->format_message)(cinfo, text);
            copy_message(e->message, e->message_cap, text);
        }
        e->warnings++;
    }
}

typedef struct {
    struct jpeg_progress_mgr pub;
    BilJpegError *err;
} BilJpegProgress;

static void bil_jpeg_progress(j_common_ptr cinfo) {
    j_decompress_ptr d = (j_decompress_ptr)cinfo;
    if (cinfo->is_decompressor && d->input_scan_number > BIL_MAX_SCANS) {
        BilJpegError *e = (BilJpegError *)cinfo->err;
        copy_message(e->message, e->message_cap, "more progressive scans than BIL allows");
        longjmp(e->jump, 1);
    }
}

static void bil_jpeg_handlers(BilJpegError *e, char *message, size_t cap) {
    jpeg_std_error(&e->pub);
    e->pub.error_exit = bil_jpeg_error_exit;
    e->pub.output_message = bil_jpeg_output_message;
    e->pub.emit_message = bil_jpeg_emit_message;
    e->message = message;
    e->message_cap = cap;
    e->warnings = 0;
}

/*
 * Decodes a JPEG held in memory to 8-bit samples, pixel-interleaved, into a buffer this
 * function allocates and the caller frees with bil_free.
 */
int bil_jpeg_decode(const uint8_t *data, size_t len, const BilLimits *limits, uint8_t **out,
                    size_t *out_len, BilInfo *info, char *message, size_t cap) {
    /* Everything the jump must find again lives in this one struct, not in locals that a
     * longjmp may leave stale. */
    struct State {
        struct jpeg_decompress_struct cinfo;
        BilJpegError err;
        BilJpegProgress progress;
        uint8_t *pixels;
        int created;
        int refused;
    } *s = calloc(1, sizeof(struct State));
    *out = NULL;
    *out_len = 0;
    memset(info, 0, sizeof *info);
    copy_message(message, cap, "");
    if (!s) {
        copy_message(message, cap, "out of memory");
        return SHIM_ERROR;
    }
    bil_jpeg_handlers(&s->err, message, cap);
    s->cinfo.err = &s->err.pub;
    if (setjmp(s->err.jump)) {
        int refused = s->refused;
        info->warnings = s->err.warnings;
        if (s->created) jpeg_destroy_decompress(&s->cinfo);
        free(s->pixels);
        free(s);
        return refused ? SHIM_REFUSED : SHIM_ERROR;
    }
    jpeg_create_decompress(&s->cinfo);
    s->created = 1;
    /* libjpeg's working memory, such as a progressive file's whole-image coefficients, beside
     * the output: capped at twice the output's limit, a figure for the experiment. */
    if (limits->max_decoded_bytes) {
        uint64_t cap = product(limits->max_decoded_bytes, 2, 1, 1);
        s->cinfo.mem->max_memory_to_use = cap > (uint64_t)LONG_MAX ? LONG_MAX : (long)cap;
    }
    s->progress.pub.progress_monitor = bil_jpeg_progress;
    s->progress.err = &s->err;
    s->cinfo.progress = &s->progress.pub;
    jpeg_mem_src(&s->cinfo, data, (unsigned long)len);
    jpeg_read_header(&s->cinfo, TRUE);
    uint64_t w = s->cinfo.image_width, h = s->cinfo.image_height;
    uint64_t c = (uint64_t)s->cinfo.num_components;
    info->width = (uint32_t)w;
    info->height = (uint32_t)h;
    info->components = (uint32_t)c;
    info->precision = (uint32_t)s->cinfo.data_precision;
    if (too_large(limits, w, h, c, 1)) {
        s->refused = 1;
        copy_message(message, cap, "the Image is larger than the caller's limits");
        longjmp(s->err.jump, 1);
    }
    jpeg_start_decompress(&s->cinfo);
    size_t stride = (size_t)s->cinfo.output_width * (size_t)s->cinfo.output_components;
    size_t total = stride * s->cinfo.output_height;
    s->pixels = malloc(total ? total : 1);
    if (!s->pixels) {
        copy_message(message, cap, "out of memory");
        longjmp(s->err.jump, 1);
    }
    while (s->cinfo.output_scanline < s->cinfo.output_height) {
        JSAMPROW row = s->pixels + (size_t)s->cinfo.output_scanline * stride;
        jpeg_read_scanlines(&s->cinfo, &row, 1);
    }
    jpeg_finish_decompress(&s->cinfo);
    jpeg_destroy_decompress(&s->cinfo);
    info->warnings = s->err.warnings;
    *out = s->pixels;
    *out_len = total;
    free(s);
    return SHIM_OK;
}

/* Encodes 8-bit samples to a JPEG, for the experiment's seed files. */
int bil_jpeg_encode(const uint8_t *pixels, uint32_t width, uint32_t height, uint32_t components,
                    int quality, int progressive, uint8_t **out, size_t *out_len, char *message,
                    size_t cap) {
    struct State {
        struct jpeg_compress_struct cinfo;
        BilJpegError err;
        unsigned char *buffer;
        unsigned long size;
        int created;
    } *s = calloc(1, sizeof(struct State));
    *out = NULL;
    *out_len = 0;
    if (!s) return SHIM_ERROR;
    bil_jpeg_handlers(&s->err, message, cap);
    s->cinfo.err = &s->err.pub;
    if (setjmp(s->err.jump)) {
        /* The memory destination's buffer is not freed here: once it has grown, libjpeg has
         * freed the one s->buffer names and holds the newest itself, setting s->buffer only
         * when compression finishes. An encode that fails, which only a seed's could, leaks
         * that buffer. */
        if (s->created) jpeg_destroy_compress(&s->cinfo);
        free(s);
        return SHIM_ERROR;
    }
    jpeg_create_compress(&s->cinfo);
    s->created = 1;
    jpeg_mem_dest(&s->cinfo, &s->buffer, &s->size);
    s->cinfo.image_width = width;
    s->cinfo.image_height = height;
    s->cinfo.input_components = (int)components;
    s->cinfo.in_color_space = components == 3 ? JCS_RGB : JCS_GRAYSCALE;
    jpeg_set_defaults(&s->cinfo);
    jpeg_set_quality(&s->cinfo, quality, TRUE);
    if (progressive) jpeg_simple_progression(&s->cinfo);
    jpeg_start_compress(&s->cinfo, TRUE);
    size_t stride = (size_t)width * components;
    while (s->cinfo.next_scanline < s->cinfo.image_height) {
        JSAMPROW row = (JSAMPROW)(pixels + (size_t)s->cinfo.next_scanline * stride);
        jpeg_write_scanlines(&s->cinfo, &row, 1);
    }
    jpeg_finish_compress(&s->cinfo);
    jpeg_destroy_compress(&s->cinfo);
    *out = s->buffer;
    *out_len = s->size;
    free(s);
    return SHIM_OK;
}

/*
 * The control: decodes with libjpeg's own error handling, unchanged. On a corrupt file the
 * library prints to stderr and ends the process. Called only in a child process, to show what
 * the layer above prevents.
 */
int bil_jpeg_decode_unprotected(const uint8_t *data, size_t len) {
    struct jpeg_decompress_struct cinfo;
    struct jpeg_error_mgr err;
    cinfo.err = jpeg_std_error(&err);
    jpeg_create_decompress(&cinfo);
    jpeg_mem_src(&cinfo, data, (unsigned long)len);
    jpeg_read_header(&cinfo, TRUE);
    jpeg_destroy_decompress(&cinfo);
    return SHIM_OK;
}

/* ------------------------------------------------------------------------- OpenJPEG */

typedef struct {
    const uint8_t *data;
    size_t len;
    size_t pos;
} BilMemReader;

static OPJ_SIZE_T mem_read(void *buffer, OPJ_SIZE_T n, void *user) {
    BilMemReader *r = user;
    if (r->pos >= r->len) return (OPJ_SIZE_T)-1;
    size_t k = r->len - r->pos < n ? r->len - r->pos : n;
    memcpy(buffer, r->data + r->pos, k);
    r->pos += k;
    return k;
}

static OPJ_OFF_T mem_skip(OPJ_OFF_T n, void *user) {
    BilMemReader *r = user;
    /* OpenJPEG skips in a loop until the count is done or -1 comes back, so a skip that cannot
     * move reports -1 rather than 0. */
    if ((n > 0 && r->pos >= r->len) || (n < 0 && r->pos == 0)) return -1;
    if (n < 0) {
        if ((size_t)(-n) > r->pos) n = -(OPJ_OFF_T)r->pos;
    } else if ((size_t)n > r->len - r->pos) {
        n = (OPJ_OFF_T)(r->len - r->pos);
    }
    r->pos += (size_t)n;
    return n;
}

static OPJ_BOOL mem_seek(OPJ_OFF_T n, void *user) {
    BilMemReader *r = user;
    if (n < 0 || (size_t)n > r->len) return OPJ_FALSE;
    r->pos = (size_t)n;
    return OPJ_TRUE;
}

typedef struct {
    char *message;
    size_t cap;
    uint32_t warnings;
    int have_error;
} BilOpjMessages;

static void opj_error_cb(const char *text, void *user) {
    BilOpjMessages *m = user;
    if (!m->have_error) {
        copy_message(m->message, m->cap, text);
        /* OpenJPEG's messages end with a newline. */
        size_t n = strlen(m->message);
        if (n && m->message[n - 1] == '\n') m->message[n - 1] = '\0';
        m->have_error = 1;
    }
}

static void opj_warning_cb(const char *text, void *user) {
    (void)text;
    ((BilOpjMessages *)user)->warnings++;
}

static void opj_quiet_cb(const char *text, void *user) {
    (void)text;
    (void)user;
}

/*
 * Decodes a JPEG 2000 codestream (format 0) or JP2 file (format 2) held in memory. Each
 * component's samples, as 32-bit integers, follow one another in the buffer, each component at
 * its own size, which corrupt files may make differ.
 */
int bil_j2k_decode(const uint8_t *data, size_t len, int format, const BilLimits *limits,
                   int32_t **out, size_t *out_len, BilInfo *info, char *message, size_t cap) {
    *out = NULL;
    *out_len = 0;
    memset(info, 0, sizeof *info);
    copy_message(message, cap, "");
    BilMemReader reader = {data, len, 0};
    BilOpjMessages msgs = {message, cap, 0, 0};
    int status = SHIM_ERROR;
    opj_image_t *image = NULL;
    opj_codec_t *codec = opj_create_decompress(format == 2 ? OPJ_CODEC_JP2 : OPJ_CODEC_J2K);
    opj_stream_t *stream = opj_stream_create(OPJ_J2K_STREAM_CHUNK_SIZE, OPJ_TRUE);
    if (!codec || !stream) goto done;
    opj_set_error_handler(codec, opj_error_cb, &msgs);
    opj_set_warning_handler(codec, opj_warning_cb, &msgs);
    opj_set_info_handler(codec, opj_quiet_cb, &msgs);
    opj_stream_set_user_data(stream, &reader, NULL);
    opj_stream_set_user_data_length(stream, len);
    opj_stream_set_read_function(stream, mem_read);
    opj_stream_set_skip_function(stream, mem_skip);
    opj_stream_set_seek_function(stream, mem_seek);
    opj_dparameters_t params;
    opj_set_default_decoder_parameters(&params);
    params.flags |= OPJ_DPARAMETERS_IGNORE_PCLR_CMAP_CDEF_FLAG;
    if (!opj_setup_decoder(codec, &params)) goto done;
    if (!opj_read_header(stream, codec, &image) || !image) goto done;
    uint64_t w = image->x1 > image->x0 ? image->x1 - image->x0 : 0;
    uint64_t h = image->y1 > image->y0 ? image->y1 - image->y0 : 0;
    info->width = (uint32_t)w;
    info->height = (uint32_t)h;
    info->components = image->numcomps;
    info->precision = image->numcomps ? image->comps[0].prec : 0;
    if (too_large(limits, w, h, image->numcomps, 4)) {
        copy_message(message, cap, "the Image is larger than the caller's limits");
        status = SHIM_REFUSED;
        goto done;
    }
    if (!opj_decode(codec, stream, image) || !opj_end_decompress(codec, stream)) goto done;
    size_t total = 0;
    for (OPJ_UINT32 i = 0; i < image->numcomps; i++) {
        if (!image->comps[i].data) {
            copy_message(message, cap, "a component was not decoded");
            goto done;
        }
        total += (size_t)image->comps[i].w * image->comps[i].h;
    }
    /* Checked again on what was decoded, which corrupt files can make differ from what the
     * header said. */
    if (limits->max_decoded_bytes && product(total, 4, 1, 1) > limits->max_decoded_bytes) {
        copy_message(message, cap, "the decoded Image is larger than the caller's limits");
        status = SHIM_REFUSED;
        goto done;
    }
    int32_t *pixels = malloc((total ? total : 1) * sizeof(int32_t));
    if (!pixels) {
        copy_message(message, cap, "out of memory");
        goto done;
    }
    size_t at = 0;
    for (OPJ_UINT32 i = 0; i < image->numcomps; i++) {
        size_t n = (size_t)image->comps[i].w * image->comps[i].h;
        memcpy(pixels + at, image->comps[i].data, n * sizeof(int32_t));
        at += n;
    }
    *out = pixels;
    *out_len = total;
    status = SHIM_OK;
done:
    if (status == SHIM_ERROR && cap && message[0] == '\0') copy_message(message, cap, "decode failed");
    info->warnings = msgs.warnings;
    if (image) opj_image_destroy(image);
    if (stream) opj_stream_destroy(stream);
    if (codec) opj_destroy_codec(codec);
    return status;
}

typedef struct {
    uint8_t *data;
    size_t len;
    size_t cap;
    size_t pos;
} BilMemWriter;

static OPJ_SIZE_T mem_write(void *buffer, OPJ_SIZE_T n, void *user) {
    BilMemWriter *w = user;
    if (w->pos + n > w->cap) {
        size_t cap = (w->pos + n) * 2;
        uint8_t *grown = realloc(w->data, cap);
        if (!grown) return (OPJ_SIZE_T)-1;
        w->data = grown;
        w->cap = cap;
    }
    memcpy(w->data + w->pos, buffer, n);
    w->pos += n;
    if (w->pos > w->len) w->len = w->pos;
    return n;
}

static OPJ_OFF_T mem_wskip(OPJ_OFF_T n, void *user) {
    BilMemWriter *w = user;
    if (n < 0) return -1;
    if (w->pos + (size_t)n > w->cap) {
        size_t cap = (w->pos + (size_t)n) * 2;
        uint8_t *grown = realloc(w->data, cap);
        if (!grown) return -1;
        memset(grown + w->cap, 0, cap - w->cap);
        w->data = grown;
        w->cap = cap;
    }
    w->pos += (size_t)n;
    if (w->pos > w->len) w->len = w->pos;
    return n;
}

static OPJ_BOOL mem_wseek(OPJ_OFF_T n, void *user) {
    BilMemWriter *w = user;
    if (n < 0 || (size_t)n > w->len) return OPJ_FALSE;
    w->pos = (size_t)n;
    return OPJ_TRUE;
}

/* Encodes 8-bit samples, losslessly, to a codestream (format 0) or JP2 file (format 2), with
 * tiles of `tile` pixels when it is not 0, for the experiment's seed files. */
int bil_j2k_encode(const uint8_t *pixels, uint32_t width, uint32_t height, uint32_t components,
                   int format, uint32_t tile, uint8_t **out, size_t *out_len, char *message,
                   size_t cap) {
    *out = NULL;
    *out_len = 0;
    copy_message(message, cap, "");
    if (components != 1 && components != 3) {
        copy_message(message, cap, "seeds have 1 or 3 components");
        return SHIM_ERROR;
    }
    BilOpjMessages msgs = {message, cap, 0, 0};
    BilMemWriter writer = {NULL, 0, 0, 0};
    int status = SHIM_ERROR;
    opj_cparameters_t params;
    opj_set_default_encoder_parameters(&params);
    params.tcp_numlayers = 1;
    params.tcp_rates[0] = 0;
    params.cp_disto_alloc = 1;
    params.numresolution = 4;
    if (tile) {
        params.tile_size_on = OPJ_TRUE;
        params.cp_tdx = (int)tile;
        params.cp_tdy = (int)tile;
    }
    opj_image_cmptparm_t comp[3];
    memset(comp, 0, sizeof comp);
    for (uint32_t i = 0; i < components; i++) {
        comp[i].dx = comp[i].dy = 1;
        comp[i].w = width;
        comp[i].h = height;
        comp[i].prec = 8;
        comp[i].sgnd = 0;
    }
    opj_image_t *image =
        opj_image_create(components, comp, components == 3 ? OPJ_CLRSPC_SRGB : OPJ_CLRSPC_GRAY);
    opj_codec_t *codec = opj_create_compress(format == 2 ? OPJ_CODEC_JP2 : OPJ_CODEC_J2K);
    opj_stream_t *stream = opj_stream_create(OPJ_J2K_STREAM_CHUNK_SIZE, OPJ_FALSE);
    if (!image || !codec || !stream) goto done;
    image->x0 = image->y0 = 0;
    image->x1 = width;
    image->y1 = height;
    for (uint32_t i = 0; i < components; i++)
        for (size_t p = 0; p < (size_t)width * height; p++)
            image->comps[i].data[p] = pixels[p * components + i];
    opj_set_error_handler(codec, opj_error_cb, &msgs);
    opj_set_warning_handler(codec, opj_warning_cb, &msgs);
    opj_set_info_handler(codec, opj_quiet_cb, &msgs);
    opj_stream_set_user_data(stream, &writer, NULL);
    opj_stream_set_write_function(stream, mem_write);
    opj_stream_set_skip_function(stream, mem_wskip);
    opj_stream_set_seek_function(stream, mem_wseek);
    if (!opj_setup_encoder(codec, &params, image)) goto done;
    if (!opj_start_compress(codec, image, stream) || !opj_encode(codec, stream) ||
        !opj_end_compress(codec, stream))
        goto done;
    *out = writer.data;
    *out_len = writer.len;
    writer.data = NULL;
    status = SHIM_OK;
done:
    free(writer.data);
    if (stream) opj_stream_destroy(stream);
    if (codec) opj_destroy_codec(codec);
    if (image) opj_image_destroy(image);
    return status;
}

/*
 * The control for OpenJPEG: decodes with no handlers set, so that any message the library
 * prints on its own is seen.
 */
int bil_j2k_decode_unprotected(const uint8_t *data, size_t len, int format) {
    BilMemReader reader = {data, len, 0};
    opj_image_t *image = NULL;
    opj_codec_t *codec = opj_create_decompress(format == 2 ? OPJ_CODEC_JP2 : OPJ_CODEC_J2K);
    opj_stream_t *stream = opj_stream_create(OPJ_J2K_STREAM_CHUNK_SIZE, OPJ_TRUE);
    opj_stream_set_user_data(stream, &reader, NULL);
    opj_stream_set_user_data_length(stream, len);
    opj_stream_set_read_function(stream, mem_read);
    opj_stream_set_skip_function(stream, mem_skip);
    opj_stream_set_seek_function(stream, mem_seek);
    opj_dparameters_t params;
    opj_set_default_decoder_parameters(&params);
    opj_setup_decoder(codec, &params);
    if (opj_read_header(stream, codec, &image)) opj_decode(codec, stream, image);
    if (image) opj_image_destroy(image);
    opj_stream_destroy(stream);
    opj_destroy_codec(codec);
    return SHIM_OK;
}
