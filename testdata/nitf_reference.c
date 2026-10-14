/* SPDX-License-Identifier: LGPL-2.1-or-later */
/*
 * Prints what GDAL finds in a NITF file, one fact to a line, in the form BIL's
 * `crates/bil/examples/nitf_dump.rs` prints what BIL's reader finds, so that
 * `compare_nitf.py` can compare the two. A test reference only: it links GDAL, and is built
 * where libgdal-dev is installed.
 *
 *     cc nitf_reference.c -o nitf_reference $(gdal-config --cflags) $(gdal-config --libs)
 */

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cpl_conv.h"
#include "cpl_error.h"
#include "cpl_string.h"
#include "gdal.h"

/* The file given, for naming the pixel files. */
static const char *source_path = "";

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

static void field(GDALDatasetH ds, const char *prefix, const char *name) {
    char key[64];
    snprintf(key, sizeof key, "NITF_%s", name);
    const char *v = GDALGetMetadataItem(ds, key, NULL);
    if (v) printf("%sfield %s=%s\n", prefix, name, v);
}

/* The extensions GDAL reports for an image, which include the file header's: their tags, with
 * GDAL's numbering of repeats ("_2") taken off. */
static void extensions(GDALDatasetH ds, int image) {
    char **tre = GDALGetMetadata(ds, "TRE");
    for (char **t = tre; t && *t; t++) {
        char *key = NULL;
        CPLParseNameValue(*t, &key);
        if (!key) continue;
        char *u = strrchr(key, '_');
        if (u && u != key && strspn(u + 1, "0123456789") == strlen(u + 1) && u[1]) *u = '\0';
        printf("image %d all extension %s\n", image, key);
        CPLFree(key);
    }
}

static void image(GDALDatasetH ds, int i) {
    int w = GDALGetRasterXSize(ds), h = GDALGetRasterYSize(ds), c = GDALGetRasterCount(ds);
    GDALDataType t = c ? GDALGetRasterDataType(GDALGetRasterBand(ds, 1)) : GDT_Unknown;
    printf("image %d size %d x %d x %d %s\n", i, h, w, c, type_name(t));
    char prefix[32];
    snprintf(prefix, sizeof prefix, "image %d ", i);
    const char *names[] = {"IID1", "ISORCE", "IREP", "ICAT", "ABPP", "PJUST", "ICORDS", "IGEOLO"};
    for (size_t k = 0; k < sizeof names / sizeof *names; k++) field(ds, prefix, names[k]);
    extensions(ds, i);
    const char *ic = GDALGetMetadataItem(ds, "NITF_IC", NULL);
    /* JPEG 2000 is decoded where GDAL has its OpenJPEG driver, as BIL's build with the openjpeg
     * feature does; other compression is not compared. GDAL would prefer an ECW or Kakadu
     * driver where one is installed; Ubuntu's GDAL, which CI uses, has neither. */
    int jpeg2000 = ic && (!strcmp(ic, "C8") || !strcmp(ic, "M8")) &&
                   GDALGetDriverByName("JP2OpenJPEG") != NULL;
    if (ic && strcmp(ic, "NC") && strcmp(ic, "NM") && !jpeg2000) {
        printf("image %d pixels compressed\n", i);
        return;
    }
    int size = GDALGetDataTypeSizeBytes(t);
    unsigned char *buf = malloc((size_t)w * h * size);
    uint64_t hash = 0xcbf29ce484222325ULL;
    int ok = buf != NULL;
    /* For a closer look at a difference: the pixels, band after band, as nitf_dump writes
     * BIL's. */
    FILE *out = NULL;
    const char *dir = getenv("NITF_PIXELS_DIR");
    if (dir) {
        const char *base = strrchr(source_path, '/');
        char name[4096];
        snprintf(name, sizeof name, "%s/%s.%d.gdal", dir, base ? base + 1 : source_path, i);
        out = fopen(name, "wb");
    }
    for (int b = 1; ok && b <= c; b++) {
        ok = GDALRasterIO(GDALGetRasterBand(ds, b), GF_Read, 0, 0, w, h, buf, w, h, t, 0, 0) ==
             CE_None;
        if (ok) hash = fnv(hash, buf, (size_t)w * h * size);
        if (ok && out) fwrite(buf, 1, (size_t)w * h * size, out);
    }
    if (out) fclose(out);
    free(buf);
    if (ok)
        printf("image %d pixels %016" PRIx64 "\n", i, hash);
    else
        printf("image %d pixels error %s\n", i, CPLGetLastErrorMsg());
}

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    source_path = argv[1];
    GDALAllRegister();
    CPLSetConfigOption("GDAL_PAM_ENABLED", "NO");
    CPLPushErrorHandler(CPLQuietErrorHandler);
    GDALDatasetH ds = GDALOpen(argv[1], GA_ReadOnly);
    if (!ds) {
        printf("error %s\n", CPLGetLastErrorMsg());
        return 0;
    }
    const char *fhdr = GDALGetMetadataItem(ds, "NITF_FHDR", NULL);
    printf("version %s\n", fhdr ? fhdr : "?");
    char **sub = GDALGetMetadata(ds, "SUBDATASETS");
    int count = CSLCount(sub) / 2;
    /* GDAL presents a file with no image segments as one Image with no bands. */
    int none = count == 0 && GDALGetRasterCount(ds) == 0;
    printf("images %d\n", none ? 0 : count ? count : 1);
    const char *file_fields[] = {"OSTAID", "FTITLE", "FSCLAS", "ONAME"};
    for (size_t k = 0; k < 4; k++) field(ds, "file ", file_fields[k]);
    if (none) {
    } else if (count == 0) {
        image(ds, 0);
    } else {
        for (int i = 0; i < count; i++) {
            char name[64];
            snprintf(name, sizeof name, "SUBDATASET_%d_NAME", i + 1);
            GDALDatasetH one = GDALOpen(CSLFetchNameValue(sub, name), GA_ReadOnly);
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
