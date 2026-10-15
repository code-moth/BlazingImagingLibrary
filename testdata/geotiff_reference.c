/* SPDX-License-Identifier: LGPL-2.1-or-later */
/*
 * Prints what GDAL finds in a TIFF file, one fact to a line, in the form BIL's
 * `crates/bil/examples/geotiff_dump.rs` prints what BIL's reader finds, so that
 * `compare_geotiff.py` can compare the two. A test reference only: it links GDAL, and is built
 * where libgdal-dev is installed.
 *
 *     cc geotiff_reference.c -o geotiff_reference $(gdal-config --cflags) $(gdal-config --libs)
 */

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cpl_conv.h"
#include "cpl_error.h"
#include "cpl_string.h"
#include "gdal.h"

static uint64_t fnv(uint64_t h, const unsigned char *p, size_t n) {
    for (size_t i = 0; i < n; i++) h = (h ^ p[i]) * 0x100000001b3ULL;
    return h;
}

/* GDAL's data types under the names of BIL's SampleType. */
static const char *type_name(GDALDataType t) {
    switch (t) {
    case GDT_Byte: return "U8";
    case GDT_Int8: return "I8";
    case GDT_UInt16: return "U16";
    case GDT_Int16: return "I16";
    case GDT_UInt32: return "U32";
    case GDT_Int32: return "I32";
    case GDT_UInt64: return "U64";
    case GDT_Int64: return "I64";
    case GDT_Float32: return "F32";
    case GDT_Float64: return "F64";
    case GDT_CInt16: return "ComplexI16";
    case GDT_CFloat32: return "ComplexF32";
    case GDT_CFloat64: return "ComplexF64";
    default: return "unsupported";
    }
}

static void image(GDALDatasetH ds, int i) {
    int w = GDALGetRasterXSize(ds), h = GDALGetRasterYSize(ds), c = GDALGetRasterCount(ds);
    GDALRasterBandH b1 = c ? GDALGetRasterBand(ds, 1) : NULL;
    GDALDataType t = b1 ? GDALGetRasterDataType(b1) : GDT_Unknown;
    printf("image %d size %d x %d x %d %s\n", i, h, w, c, type_name(t));
    for (int o = 0; b1 && o < GDALGetOverviewCount(b1); o++) {
        GDALRasterBandH ov = GDALGetOverview(b1, o);
        printf("image %d overview %d x %d\n", i, GDALGetRasterBandYSize(ov),
               GDALGetRasterBandXSize(ov));
    }
    int size = GDALGetDataTypeSizeBytes(t);
    unsigned char *buf = size ? malloc((size_t)w * h * size) : NULL;
    uint64_t hash = 0xcbf29ce484222325ULL;
    int ok = buf != NULL;
    for (int b = 1; ok && b <= c; b++) {
        ok = GDALRasterIO(GDALGetRasterBand(ds, b), GF_Read, 0, 0, w, h, buf, w, h, t, 0, 0) ==
             CE_None;
        if (ok) hash = fnv(hash, buf, (size_t)w * h * size);
    }
    free(buf);
    if (ok)
        printf("image %d pixels %016" PRIx64 "\n", i, hash);
    else
        printf("image %d pixels error %s\n", i, CPLGetLastErrorMsg());
    /* Each overview read whole, band after band, in its own size. */
    for (int o = 0; b1 && o < GDALGetOverviewCount(b1); o++) {
        GDALRasterBandH ov1 = GDALGetOverview(b1, o);
        int ow = GDALGetRasterBandXSize(ov1), oh = GDALGetRasterBandYSize(ov1);
        unsigned char *ob = size ? malloc((size_t)ow * oh * size) : NULL;
        uint64_t oh_hash = 0xcbf29ce484222325ULL;
        int ook = ob != NULL;
        for (int b = 1; ook && b <= c; b++) {
            GDALRasterBandH ov = GDALGetOverview(GDALGetRasterBand(ds, b), o);
            ook = ov && GDALRasterIO(ov, GF_Read, 0, 0, ow, oh, ob, ow, oh, t, 0, 0) == CE_None;
            if (ook) oh_hash = fnv(oh_hash, ob, (size_t)ow * oh * size);
        }
        free(ob);
        if (ook)
            printf("image %d overview %d pixels %016" PRIx64 "\n", i, o, oh_hash);
        else
            printf("image %d overview %d pixels error %s\n", i, o, CPLGetLastErrorMsg());
    }
}

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    GDALAllRegister();
    CPLSetConfigOption("GDAL_PAM_ENABLED", "NO");
    /* Overviews only from the file itself, as BIL reads them. */
    CPLSetConfigOption("GDAL_DISABLE_READDIR_ON_OPEN", "EMPTY_DIR");
    CPLPushErrorHandler(CPLQuietErrorHandler);
    const char *allowed[] = {"GTiff", NULL};
    GDALDatasetH ds = GDALOpenEx(argv[1], GDAL_OF_RASTER | GDAL_OF_READONLY, allowed, NULL, NULL);
    if (!ds) {
        printf("error %s\n", CPLGetLastErrorMsg());
        return 0;
    }
    char **sub = GDALGetMetadata(ds, "SUBDATASETS");
    int count = CSLCount(sub) / 2;
    printf("images %d\n", count ? count : 1);
    if (count == 0) {
        image(ds, 0);
    } else {
        for (int i = 0; i < count; i++) {
            char name[64];
            snprintf(name, sizeof name, "SUBDATASET_%d_NAME", i + 1);
            GDALDatasetH one = GDALOpenEx(CSLFetchNameValue(sub, name),
                                          GDAL_OF_RASTER | GDAL_OF_READONLY, allowed, NULL, NULL);
            if (!one) {
                printf("image %d error %s\n", i, CPLGetLastErrorMsg());
                continue;
            }
            image(one, i);
            GDALClose(one);
        }
    }
    GDALClose(ds);
    return 0;
}
