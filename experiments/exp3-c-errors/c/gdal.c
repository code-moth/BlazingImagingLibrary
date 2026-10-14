/* SPDX-License-Identifier: LGPL-2.1-or-later */
/*
 * Experiment 3: the C layer between BIL and GDAL, and a description of the state GDAL keeps for
 * the whole process.
 *
 * Built only when the experiment's `gdal` feature is on, in CI, where libgdal-dev is installed;
 * gdal_stub.c stands in for it everywhere else. Every call into GDAL goes through a function here,
 * and every one returns a status:
 *
 * - GDAL reports errors through an error handler, by default one that prints. Here a handler is
 *   pushed on the calling thread's own stack for the length of the call, and popped before it
 *   returns: it copies the first error's message and counts warnings, and prints nothing. The
 *   process's global handler, which the caller or another package may have set, is never touched
 *   (section 10.5).
 * - GDAL keeps the last error of each thread, which a caller that uses GDAL itself may read. It is
 *   saved before the call and put back after.
 * - GDAL is configured by options, global or for one thread. The few this layer needs are set for
 *   the calling thread alone, and each one's earlier value is put back after the call. No
 *   environment variable is read or set (section 10.5).
 * - Each Source is opened with a list of the drivers allowed to read it, so no other driver probes
 *   a corrupt file. The drivers are registered one by one, which loads nothing from a search path
 *   as registering them all would (section 10.5).
 * - The bytes are handed to GDAL as a file in its memory file system, which is removed after the
 *   call; GDAL is told not to list the directory and not to look for or write companion files.
 * - Before any pixel memory is taken, the size the dataset claims is checked against the caller's
 *   limits (section 10.3), and so is the size of one block, which GDAL's own working memory
 *   follows and which it offers no way to cap.
 *
 * One kind of error cannot be contained in the process: an error GDAL reports as CE_Fatal ends it,
 * after the handler returns, by calling abort(). The handler cannot jump out instead, since the
 * jump would cross C++ frames. The supervisor counts any such end as a crash.
 */

#define _GNU_SOURCE
#include <link.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cpl_conv.h"
#include "cpl_error.h"
#include "cpl_string.h"
#include "cpl_vsi.h"
#include "gdal.h"
#include "gdal_frmts.h"

#include "layer.h"

/* The formats this layer reads: NITF, and TIFF through GDAL's GeoTIFF driver. */
enum { BIL_GDAL_NITF = 0, BIL_GDAL_TIFF = 1 };

/* The one name the bytes of a decode take in GDAL's memory file system. A fixed name keeps
 * GDAL's messages, which can quote it, the same from one run to the next; the workers decode on
 * one thread each, so one name is enough here. */
#define BIL_INPUT "/vsimem/bil_exp3_input"
#define BIL_SEED "/vsimem/bil_exp3_seed"

static const char *driver_name(int format) { return format == BIL_GDAL_NITF ? "NITF" : "GTiff"; }

/*
 * Registers the drivers this layer uses, and only those: NITF, which reopens its JPEG blocks
 * through GDAL and so needs the JPEG driver too; GeoTIFF; and MEM, for the seed files. Each
 * registration does nothing where the driver is already registered. Returns GDAL's version.
 */
const char *bil_gdal_init(void) {
    GDALRegister_GTiff();
    GDALRegister_NITF();
    GDALRegister_JPEG();
    GDALRegister_MEM();
    return GDALVersionInfo("RELEASE_NAME");
}

/* ------------------------------------------------------------------ the call's surroundings */

typedef struct {
    char *message;
    size_t cap;
    uint32_t warnings;
    int have_error;
} BilGdalMessages;

static void CPL_STDCALL bil_gdal_handler(CPLErr cls, CPLErrorNum no, const char *text) {
    (void)no;
    BilGdalMessages *m = CPLGetErrorHandlerUserData();
    if (cls == CE_Debug || cls == CE_None) return;
    if (cls == CE_Warning) {
        /* Corrupt data GDAL went past. Its text is kept, the first, unless an error comes. */
        if (m->warnings == 0 && !m->have_error) copy_message(m->message, m->cap, text);
        m->warnings++;
        return;
    }
    if (!m->have_error) {
        copy_message(m->message, m->cap, text);
        m->have_error = 1;
    }
}

/* The options this layer sets, for the calling thread alone: no directory listing on open, so no
 * companion file is looked for, and no persistent auxiliary metadata, so none is read or
 * written. */
static const char *const bil_options[][2] = {
    {"GDAL_DISABLE_READDIR_ON_OPEN", "EMPTY_DIR"},
    {"GDAL_PAM_ENABLED", "NO"},
};
#define BIL_OPTION_COUNT (sizeof bil_options / sizeof bil_options[0])

/* What the call changes on the calling thread, kept to be put back. */
typedef struct {
    CPLErr last_type;
    CPLErrorNum last_no;
    char *last_message;
    char *old[BIL_OPTION_COUNT];
    int had[BIL_OPTION_COUNT];
} BilGdalSaved;

static void enter(BilGdalSaved *s, BilGdalMessages *m) {
    s->last_type = CPLGetLastErrorType();
    s->last_no = CPLGetLastErrorNo();
    s->last_message = CPLStrdup(CPLGetLastErrorMsg());
    for (size_t i = 0; i < BIL_OPTION_COUNT; i++) {
        const char *v = CPLGetThreadLocalConfigOption(bil_options[i][0], NULL);
        s->had[i] = v != NULL;
        s->old[i] = v ? CPLStrdup(v) : NULL;
        CPLSetThreadLocalConfigOption(bil_options[i][0], bil_options[i][1]);
    }
    CPLPushErrorHandlerEx(bil_gdal_handler, m);
}

static void leave(BilGdalSaved *s) {
    CPLPopErrorHandler();
    for (size_t i = 0; i < BIL_OPTION_COUNT; i++) {
        CPLSetThreadLocalConfigOption(bil_options[i][0], s->had[i] ? s->old[i] : NULL);
        CPLFree(s->old[i]);
    }
    CPLErrorSetState(s->last_type, s->last_no, s->last_message);
    CPLFree(s->last_message);
}

/* ------------------------------------------------------------------ decoding */

/*
 * Decodes a NITF (format 0) or TIFF (format 1) file held in memory, every band, to its first
 * band's sample type, pixel-interleaved, into a buffer this function allocates and the caller
 * frees with bil_free.
 */
int bil_gdal_decode(const uint8_t *data, size_t len, int format, const BilLimits *limits,
                    uint8_t **out, size_t *out_len, BilInfo *info, char *message, size_t cap) {
    *out = NULL;
    *out_len = 0;
    memset(info, 0, sizeof *info);
    copy_message(message, cap, "");
    BilGdalMessages msgs = {message, cap, 0, 0};
    BilGdalSaved saved;
    int status = SHIM_ERROR;
    uint8_t *pixels = NULL;
    GDALDatasetH ds = NULL;
    enter(&saved, &msgs);
    /* GDAL reads the caller's bytes in place, and never writes them: the file is read-only. */
    VSILFILE *f = VSIFileFromMemBuffer(BIL_INPUT, (GByte *)(uintptr_t)data, (vsi_l_offset)len, FALSE);
    if (!f) {
        copy_message(message, cap, "cannot hand the bytes to GDAL");
        goto done;
    }
    VSIFCloseL(f);
    const char *allowed[] = {driver_name(format), NULL};
    ds = GDALOpenEx(BIL_INPUT, GDAL_OF_RASTER | GDAL_OF_READONLY | GDAL_OF_VERBOSE_ERROR, allowed, NULL, NULL);
    if (!ds) goto done;
    uint64_t w = (uint64_t)GDALGetRasterXSize(ds), h = (uint64_t)GDALGetRasterYSize(ds);
    uint64_t c = (uint64_t)GDALGetRasterCount(ds);
    info->width = (uint32_t)w;
    info->height = (uint32_t)h;
    info->components = (uint32_t)c;
    if (c == 0) {
        copy_message(message, cap, "no bands");
        goto done;
    }
    GDALDataType type = GDALGetRasterDataType(GDALGetRasterBand(ds, 1));
    int bytes = GDALGetDataTypeSizeBytes(type);
    info->precision = (uint32_t)bytes * 8;
    if (bytes <= 0) {
        copy_message(message, cap, "unknown sample type");
        goto done;
    }
    if (too_large(limits, w, h, c, (uint64_t)bytes) || w > INT32_MAX || h > INT32_MAX) {
        copy_message(message, cap, "the Image is larger than the caller's limits");
        status = SHIM_REFUSED;
        goto done;
    }
    /* GDAL's working memory follows the block size the file declares, not the Image's: a driver
     * reads a whole block, of every band where they are interleaved, and a corrupt file can
     * declare a block far larger than its Image. GDAL offers no cap on that memory, so the block
     * is checked against section 10.3's cap, twice max_decoded_bytes, before anything is read. */
    int bx = 0, by = 0;
    GDALGetBlockSize(GDALGetRasterBand(ds, 1), &bx, &by);
    if (limits->max_decoded_bytes && bx > 0 && by > 0 &&
        product((uint64_t)bx, (uint64_t)by, c, (uint64_t)bytes) >
            product(limits->max_decoded_bytes, 2, 1, 1)) {
        copy_message(message, cap, "a block is larger than the caller's limits allow GDAL");
        status = SHIM_REFUSED;
        goto done;
    }
    size_t total = (size_t)(w * h * c * (uint64_t)bytes);
    pixels = malloc(total ? total : 1);
    if (!pixels) {
        copy_message(message, cap, "out of memory");
        goto done;
    }
    CPLErr e = GDALDatasetRasterIO(ds, GF_Read, 0, 0, (int)w, (int)h, pixels, (int)w, (int)h, type,
                                   (int)c, NULL, bytes * (int)c, (int)((uint64_t)bytes * c * w),
                                   bytes);
    if (e != CE_None || msgs.have_error) goto done;
    *out = pixels;
    *out_len = total;
    pixels = NULL;
    status = SHIM_OK;
done:
    if (ds) GDALClose(ds);
    VSIUnlink(BIL_INPUT);
    /* An error at close, such as one GDAL meets flushing a block, still makes the read fail. */
    if (status == SHIM_OK && msgs.have_error) {
        free(*out);
        *out = NULL;
        *out_len = 0;
        status = SHIM_ERROR;
    }
    free(pixels);
    if (status == SHIM_ERROR && cap && message[0] == '\0') copy_message(message, cap, "decode failed");
    info->warnings = msgs.warnings;
    leave(&saved);
    return status;
}

/*
 * Encodes 8-bit samples, pixel-interleaved, as one of four NITF (format 0) or TIFF (format 1)
 * files, for the experiment's seed files:
 *
 *   variant  NITF                               TIFF
 *   0        uncompressed, one block            uncompressed strips
 *   1        JPEG (C3), quality 90              deflate, 32 × 32 tiles
 *   2        uncompressed, 16 × 16 blocks       LZW with a predictor, band-interleaved
 *   3        JPEG (C3), quality 75              JPEG, 16 × 16 tiles
 */
int bil_gdal_encode(const uint8_t *pixels, uint32_t width, uint32_t height, uint32_t components,
                    int format, int variant, uint8_t **out, size_t *out_len, char *message,
                    size_t cap) {
    static const char *const nitf[4][4] = {
        {"IC=NC", NULL},
        {"IC=C3", "QUALITY=90", NULL},
        {"IC=NC", "BLOCKXSIZE=16", "BLOCKYSIZE=16", NULL},
        {"IC=C3", "QUALITY=75", NULL},
    };
    static const char *const tiff[4][5] = {
        {"COMPRESS=NONE", NULL},
        {"TILED=YES", "BLOCKXSIZE=32", "BLOCKYSIZE=32", "COMPRESS=DEFLATE", NULL},
        {"COMPRESS=LZW", "PREDICTOR=2", "INTERLEAVE=BAND", NULL},
        {"TILED=YES", "BLOCKXSIZE=16", "BLOCKYSIZE=16", "COMPRESS=JPEG", NULL},
    };
    *out = NULL;
    *out_len = 0;
    copy_message(message, cap, "");
    if (variant < 0 || variant > 3) {
        copy_message(message, cap, "no such variant");
        return SHIM_ERROR;
    }
    BilGdalMessages msgs = {message, cap, 0, 0};
    BilGdalSaved saved;
    int status = SHIM_ERROR;
    enter(&saved, &msgs);
    GDALDatasetH mem = GDALCreate(GDALGetDriverByName("MEM"), "", (int)width, (int)height,
                                  (int)components, GDT_Byte, NULL);
    GDALDatasetH copy = NULL;
    if (!mem) goto done;
    if (GDALDatasetRasterIO(mem, GF_Write, 0, 0, (int)width, (int)height, (void *)(uintptr_t)pixels,
                            (int)width, (int)height, GDT_Byte, (int)components, NULL,
                            (int)components, (int)(width * components), 1) != CE_None)
        goto done;
    char **options = (char **)(uintptr_t)(format == BIL_GDAL_NITF ? nitf[variant] : tiff[variant]);
    copy = GDALCreateCopy(GDALGetDriverByName(driver_name(format)), BIL_SEED, mem, FALSE, options,
                          NULL, NULL);
    if (!copy) goto done;
    GDALClose(copy);
    copy = NULL;
    vsi_l_offset n = 0;
    GByte *bytes = VSIGetMemFileBuffer(BIL_SEED, &n, TRUE);
    if (!bytes) goto done;
    /* Copied into memory the caller frees with bil_free, not with GDAL's own free. */
    *out = malloc(n ? (size_t)n : 1);
    if (*out) {
        memcpy(*out, bytes, (size_t)n);
        *out_len = (size_t)n;
        status = msgs.have_error ? SHIM_ERROR : SHIM_OK;
    }
    CPLFree(bytes);
done:
    if (copy) GDALClose(copy);
    if (mem) GDALClose(mem);
    VSIUnlink(BIL_SEED);
    if (status != SHIM_OK) {
        free(*out);
        *out = NULL;
        *out_len = 0;
        if (cap && message[0] == '\0') copy_message(message, cap, "encode failed");
    }
    leave(&saved);
    return status;
}

/*
 * The control: opens a file with GDAL's own error handling, unchanged. On a corrupt file GDAL's
 * default handler prints the error to stderr. Called only in a child process, to show what the
 * layer above prevents.
 */
int bil_gdal_decode_unprotected(const uint8_t *data, size_t len, int format) {
    VSILFILE *f = VSIFileFromMemBuffer(BIL_INPUT, (GByte *)(uintptr_t)data, (vsi_l_offset)len, FALSE);
    if (f) VSIFCloseL(f);
    const char *allowed[] = {driver_name(format), NULL};
    GDALDatasetH ds = GDALOpenEx(BIL_INPUT, GDAL_OF_RASTER | GDAL_OF_READONLY | GDAL_OF_VERBOSE_ERROR, allowed, NULL, NULL);
    if (ds) GDALClose(ds);
    VSIUnlink(BIL_INPUT);
    return SHIM_OK;
}

/* ------------------------------------------------------------------ GDAL's own state */

typedef struct {
    char *buf;
    size_t cap;
    size_t at;
} Out;

static void put(Out *o, const char *fmt, ...) CPL_PRINT_FUNC_FORMAT(2, 3);

static void put(Out *o, const char *fmt, ...) {
    if (o->at >= o->cap) return;
    va_list args;
    va_start(args, fmt);
    int n = vsnprintf(o->buf + o->at, o->cap - o->at, fmt, args);
    va_end(args);
    if (n > 0) o->at += (size_t)n;
    if (o->at > o->cap) o->at = o->cap;
}

static void put_list(Out *o, const char *name, char **list) {
    put(o, "%s=%d:", name, CSLCount(list));
    for (char **v = list; v && *v; v++) put(o, " %s", *v);
    put(o, "\n");
    CSLDestroy(list);
}

static int count_object(struct dl_phdr_info *info, size_t size, void *data) {
    (void)info;
    (void)size;
    (*(unsigned *)data)++;
    return 0;
}

/*
 * Writes a description of the state GDAL keeps for the whole process, or for the calling thread,
 * into buf, and returns its length: the error handler GDAL would call, with its user data, which
 * shows a handler left pushed as well as a global one replaced; the thread's last error; the
 * global and the thread's configuration options; the drivers registered, in order; the datasets
 * open; the files in GDAL's memory file system; the block cache's size and use; and the number of
 * shared objects loaded, which a plugin loaded from a search path would raise.
 */
size_t bil_gdal_state_describe(char *buf, size_t cap) {
    Out o = {buf, cap, 0};
    void *user = NULL;
    CPLErrorHandler handler = CPLGetErrorHandler(&user);
    put(&o, "gdal error handler=%p user data=%p\n", (void *)(uintptr_t)handler, user);
    put(&o, "gdal last error=%d %d %s\n", (int)CPLGetLastErrorType(), (int)CPLGetLastErrorNo(),
        CPLGetLastErrorMsg());
    put_list(&o, "gdal global options", CPLGetConfigOptions());
    put_list(&o, "gdal thread options", CPLGetThreadLocalConfigOptions());
    put(&o, "gdal drivers=%d:", GDALGetDriverCount());
    for (int i = 0; i < GDALGetDriverCount(); i++)
        put(&o, " %s", GDALGetDriverShortName(GDALGetDriver(i)));
    put(&o, "\n");
    GDALDatasetH *open = NULL;
    int open_count = 0;
    GDALGetOpenDatasets(&open, &open_count);
    put(&o, "gdal open datasets=%d\n", open_count);
    put_list(&o, "gdal memory files", VSIReadDir("/vsimem/"));
    put(&o, "gdal block cache=%lld bytes, %lld in use\n", (long long)GDALGetCacheMax64(),
        (long long)GDALGetCacheUsed64());
    unsigned objects = 0;
    dl_iterate_phdr(count_object, &objects);
    put(&o, "shared objects=%u\n", objects);
    return o.at;
}

/*
 * For the check of the check: makes one change of the kind the description must notice, or
 * undoes it. 1 sets a global configuration option, 2 one for the thread, 3 pushes an error
 * handler, 4 registers another driver, 5 leaves a dataset open, 6 leaves a file in the memory file
 * system, 7 sets the last error. Returns 0 where the change cannot be made.
 */
static void CPL_STDCALL bil_gdal_quiet(CPLErr cls, CPLErrorNum no, const char *text) {
    (void)cls;
    (void)no;
    (void)text;
}

int bil_gdal_state_poke(int what, int undo) {
    static GDALDatasetH left_open;
    static GByte left_file[16];
    static CPLErr last_type;
    static CPLErrorNum last_no;
    static char last_message[512];
    switch (what) {
    case 1:
        CPLSetConfigOption("BIL_EXP3_POKE", undo ? NULL : "YES");
        return 1;
    case 2:
        CPLSetThreadLocalConfigOption("BIL_EXP3_POKE", undo ? NULL : "YES");
        return 1;
    case 3:
        if (undo)
            CPLPopErrorHandler();
        else
            CPLPushErrorHandler(bil_gdal_quiet);
        return 1;
    case 4: {
        /* ENVI: a driver this layer never registers. */
        if (undo) {
            GDALDriverH d = GDALGetDriverByName("ENVI");
            if (!d) return 0;
            GDALDeregisterDriver(d);
            GDALDestroyDriver(d);
            return 1;
        }
        if (GDALGetDriverByName("ENVI")) return 0;
        GDALRegister_ENVI();
        return GDALGetDriverByName("ENVI") != NULL;
    }
    case 5:
        if (undo) {
            if (left_open) GDALClose(left_open);
            left_open = NULL;
            return 1;
        }
        left_open = GDALCreate(GDALGetDriverByName("MEM"), "", 1, 1, 1, GDT_Byte, NULL);
        return left_open != NULL;
    case 6:
        if (undo) return VSIUnlink("/vsimem/bil_exp3_poke") == 0;
        {
            VSILFILE *f = VSIFileFromMemBuffer("/vsimem/bil_exp3_poke", left_file, sizeof left_file, FALSE);
            if (!f) return 0;
            VSIFCloseL(f);
            return 1;
        }
    case 7:
        if (undo) {
            CPLErrorSetState(last_type, last_no, last_message);
            return 1;
        }
        last_type = CPLGetLastErrorType();
        last_no = CPLGetLastErrorNo();
        copy_message(last_message, sizeof last_message, CPLGetLastErrorMsg());
        CPLErrorSetState(CE_Failure, CPLE_AppDefined, "set by the check of the check");
        return 1;
    default:
        return 0;
    }
}
