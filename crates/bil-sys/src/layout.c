/* SPDX-License-Identifier: LGPL-2.1-or-later */
/*
 * The sizes and field offsets of OpenJPEG's structures as the C compiler lays them out from the
 * header found at build time, for the check that BIL's Rust declarations match them
 * (`openjpeg::layout_check`). Compiled only where the build finds openjpeg.h.
 */

#include <stddef.h>
#include <stdint.h>

#include "openjpeg.h"

void bil_sys_opj_layout(uint64_t *out) {
    out[0] = sizeof(opj_dparameters_t);
    out[1] = offsetof(opj_dparameters_t, decod_format);
    out[2] = offsetof(opj_dparameters_t, DA_x0);
    out[3] = offsetof(opj_dparameters_t, flags);
    out[4] = sizeof(opj_image_comp_t);
    out[5] = offsetof(opj_image_comp_t, prec);
    out[6] = offsetof(opj_image_comp_t, sgnd);
    out[7] = offsetof(opj_image_comp_t, data);
    out[8] = offsetof(opj_image_comp_t, alpha);
    out[9] = sizeof(opj_image_t);
    out[10] = offsetof(opj_image_t, comps);
    out[11] = offsetof(opj_image_t, icc_profile_len);
}
