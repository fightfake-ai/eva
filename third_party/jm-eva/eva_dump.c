/*!
 * Eva per-MB syntax dump hook for JM lencod.
 * Writes pred_y_enc, coeff_y_enc, type_enc, mv_enc (y luma + L0 MV per 4×4).
 */

#include <stdio.h>
#include <string.h>
#include <stdlib.h>

#include "global.h"
#include "mbuffer.h"
#include "eva_dump.h"

static FILE *fp_pred_y = NULL;
static FILE *fp_coeff_y = NULL;
static FILE *fp_type = NULL;
static FILE *fp_mv = NULL;
static FILE *fp_modes = NULL;
static FILE *fp_b8x8 = NULL;
static FILE *fp_chroma = NULL;
static FILE *fp_mvd = NULL;
static FILE *fp_luma_cof = NULL;

/* luma_cbp (1) + pad(3) + cbp_blk (8) + cofDC[0] + cofAC[0..3] */
#define EVA_LUMA_COF_BYTES                                                                 \
    (4 + (int)sizeof(int64) + 2 * 18 * (int)sizeof(int) +                                  \
     4 * 4 * 2 * 65 * (int)sizeof(int))

static const byte EVA_SNGL_SCAN[16][2] = {
    {0, 0}, {1, 0}, {0, 1}, {0, 2}, {1, 1}, {2, 0}, {3, 0}, {2, 1},
    {1, 2}, {0, 3}, {1, 3}, {2, 2}, {3, 1}, {3, 2}, {2, 3}, {3, 3},
};

void eva_dump_open(const char *dir)
{
    char path[1024];
    snprintf(path, sizeof(path), "%s/pred_y_enc", dir);
    fp_pred_y = fopen(path, "wb");
    snprintf(path, sizeof(path), "%s/coeff_y_enc", dir);
    fp_coeff_y = fopen(path, "wb");
    snprintf(path, sizeof(path), "%s/type_enc", dir);
    fp_type = fopen(path, "wb");
    snprintf(path, sizeof(path), "%s/mv_enc", dir);
    fp_mv = fopen(path, "wb");
    snprintf(path, sizeof(path), "%s/intra_modes_enc", dir);
    fp_modes = fopen(path, "wb");
    snprintf(path, sizeof(path), "%s/b8x8_enc", dir);
    fp_b8x8 = fopen(path, "wb");
    snprintf(path, sizeof(path), "%s/chroma_enc", dir);
    fp_chroma = fopen(path, "wb");
    snprintf(path, sizeof(path), "%s/mvd_enc", dir);
    fp_mvd = fopen(path, "wb");
    snprintf(path, sizeof(path), "%s/luma_cof_enc", dir);
    fp_luma_cof = fopen(path, "wb");
    if (!fp_pred_y || !fp_coeff_y || !fp_type || !fp_mv || !fp_modes || !fp_b8x8 || !fp_chroma ||
        !fp_mvd || !fp_luma_cof) {
        fprintf(stderr, "eva_dump_open: cannot write to %s\n", dir);
        exit(1);
    }
}

void eva_dump_close(void)
{
    if (fp_pred_y) {
        fclose(fp_pred_y);
        fp_pred_y = NULL;
    }
    if (fp_coeff_y) {
        fclose(fp_coeff_y);
        fp_coeff_y = NULL;
    }
    if (fp_type) {
        fclose(fp_type);
        fp_type = NULL;
    }
    if (fp_mv) {
        fclose(fp_mv);
        fp_mv = NULL;
    }
    if (fp_modes) {
        fclose(fp_modes);
        fp_modes = NULL;
    }
    if (fp_b8x8) {
        fclose(fp_b8x8);
        fp_b8x8 = NULL;
    }
    if (fp_chroma) {
        fclose(fp_chroma);
        fp_chroma = NULL;
    }
    if (fp_mvd) {
        fclose(fp_mvd);
        fp_mvd = NULL;
    }
    if (fp_luma_cof) {
        fclose(fp_luma_cof);
        fp_luma_cof = NULL;
    }
}

static int eva_slice_byte(Slice *currSlice)
{
    switch (currSlice->slice_type) {
    case I_SLICE:
        return 2;
    case P_SLICE:
        return 1;
    default:
        return 0;
    }
}

static short eva_ref_frame_no(Macroblock *currMB, PicMotionParams *pm, int list)
{
    int cur;

    if (pm->ref_idx[list] < 0)
        return -1;

    cur = currMB->p_Vid->frame_no;
    /* B=0 IPPP: listX[0][ref_idx] → earlier frame; ref_idx 0 is previous picture. */
    if (cur <= pm->ref_idx[list])
        return -1;
    return (short)(cur - 1 - pm->ref_idx[list]);
}

static void eva_dump_luma_coeff_4x4(Slice *currSlice, Macroblock *currMB, int block_x, int block_y,
                                    unsigned char *out256)
{
    int pos_x = block_x >> 2;
    int pos_y = block_y >> 2;
    int b8 = 2 * (pos_y >> 1) + (pos_x >> 1);
    int b4 = 2 * (pos_y & 1) + (pos_x & 1);
    int *levels = currSlice->cofAC[b8][b4][0];
    int *runs = currSlice->cofAC[b8][b4][1];
    int coeff = 0;
    int k = 0;
    const byte *p_scan = &EVA_SNGL_SCAN[0][0];

    while (coeff < 16 && levels[k] != 0) {
        coeff += runs[k] + 1;
        if (coeff > 16)
            break;
        {
            int scan_idx = coeff - 1;
            int i = EVA_SNGL_SCAN[scan_idx][0];
            int j = EVA_SNGL_SCAN[scan_idx][1];
            int idx = (block_y + j) * 16 + (block_x + i);
            int stored = levels[k] + 128;
            if (stored < 0)
                stored = 0;
            if (stored > 255)
                stored = 255;
            out256[idx] = (unsigned char)stored;
        }
        ++k;
    }
    (void)p_scan;
    (void)currMB;
}

static void eva_dump_luma_coeff_mb(Slice *currSlice, Macroblock *currMB, unsigned char *out256)
{
    int bx, by;
    memset(out256, 128, 256);
    if (currMB->mb_type == I16MB) {
        for (by = 0; by < 16; by += 4) {
            for (bx = 0; bx < 16; bx += 4) {
                eva_dump_luma_coeff_4x4(currSlice, currMB, bx, by, out256);
            }
        }
        return;
    }
    for (by = 0; by < 16; by += 4) {
        for (bx = 0; bx < 16; bx += 4) {
            eva_dump_luma_coeff_4x4(currSlice, currMB, bx, by, out256);
        }
    }
}

static void eva_dump_intra_modes_mb(Macroblock *currMB, unsigned char *out16)
{
    int i;
    memset(out16, 0, 16);
    switch (currMB->mb_type) {
    case I16MB:
        out16[0] = (unsigned char)currMB->i16mode;
        break;
    case I4MB:
        for (i = 0; i < 16; ++i)
            out16[i] = (unsigned char)currMB->intra_pred_modes[i];
        break;
    case I8MB:
        for (i = 0; i < 4; ++i)
            out16[i] = (unsigned char)currMB->intra_pred_modes8x8[4 * i];
        break;
    default:
        break;
    }
}

static void eva_dump_b8x8_mb(Macroblock *currMB, unsigned char *out8)
{
    int i;
    memset(out8, 0, 8);
    if (currMB->mb_type != P8x8)
        return;
    for (i = 0; i < 4; ++i) {
        out8[i * 2] = (unsigned char)currMB->b8x8[i].mode;
        out8[i * 2 + 1] = (unsigned char)currMB->b8x8[i].pdir;
    }
}

static void eva_dump_mv_mb(Macroblock *currMB, unsigned char *out96)
{
    PicMotionParams **motion = currMB->p_Vid->enc_picture->mv_info;
    int bx, by;
    int k = 0;

    memset(out96, 0, 96);
    for (by = 0; by < 4; ++by) {
        for (bx = 0; bx < 4; ++bx) {
            int block_x = currMB->block_x + bx;
            int block_y = currMB->block_y + by;
            PicMotionParams *pm = &motion[block_y][block_x];
            short ref_fn = eva_ref_frame_no(currMB, pm, 0);
            short mv_x = pm->mv[0].mv_x;
            short mv_y = pm->mv[0].mv_y;
            out96[k++] = (unsigned char)(ref_fn & 0xff);
            out96[k++] = (unsigned char)((ref_fn >> 8) & 0xff);
            out96[k++] = (unsigned char)(mv_x & 0xff);
            out96[k++] = (unsigned char)((mv_x >> 8) & 0xff);
            out96[k++] = (unsigned char)(mv_y & 0xff);
            out96[k++] = (unsigned char)((mv_y >> 8) & 0xff);
        }
    }
}

static void eva_dump_mvd_mb(Macroblock *currMB, unsigned char *out64)
{
    int bx, by, k = 0;
    memset(out64, 0, 64);
    for (by = 0; by < 4; ++by) {
        for (bx = 0; bx < 4; ++bx) {
            short mx = currMB->mvd[0][by][bx][0];
            short my = currMB->mvd[0][by][bx][1];
            out64[k++] = (unsigned char)(mx & 0xff);
            out64[k++] = (unsigned char)((mx >> 8) & 0xff);
            out64[k++] = (unsigned char)(my & 0xff);
            out64[k++] = (unsigned char)((my >> 8) & 0xff);
        }
    }
}

static void eva_dump_chroma_mb(Macroblock *currMB, Slice *currSlice, unsigned char *out)
{
    /* Layout (little-endian ints follow 2-byte header):
     * [0] chroma_cbp (cbp>>4)
     * [1] c_ipred_mode
     * then cofDC[1], cofDC[2] each: 2×18 ints (level, run)
     * then cofAC[4], cofAC[5] each: 4×2×65 ints
     */
    unsigned char *p = out;
    int b4;

    p[0] = (unsigned char)((currMB->cbp >> 4) & 0xff);
    p[1] = (unsigned char)currMB->c_ipred_mode;
    p += 2;

    memcpy(p, currSlice->cofDC[1][0], 2 * 18 * sizeof(int));
    p += 2 * 18 * sizeof(int);
    memcpy(p, currSlice->cofDC[2][0], 2 * 18 * sizeof(int));
    p += 2 * 18 * sizeof(int);

    for (b4 = 0; b4 < 4; ++b4) {
        memcpy(p, currSlice->cofAC[4][b4][0], 2 * 65 * sizeof(int));
        p += 2 * 65 * sizeof(int);
    }
    for (b4 = 0; b4 < 4; ++b4) {
        memcpy(p, currSlice->cofAC[5][b4][0], 2 * 65 * sizeof(int));
        p += 2 * 65 * sizeof(int);
    }
}

#define EVA_CHROMA_BYTES (2 + 2 * (2 * 18 * (int)sizeof(int)) + 2 * 4 * (2 * 65 * (int)sizeof(int)))

static void eva_dump_luma_cof_mb(Macroblock *currMB, Slice *currSlice, unsigned char *out)
{
    /* Layout:
     * [0..3]   luma_cbp (cbp & 15) as int
     * [4..11]  cbp_blk as int64
     * then cofDC[0]: 2×18 ints
     * then cofAC[0..3]: each 4×2×65 ints
     */
    unsigned char *p = out;
    int b8, b4;
    int luma_cbp = currMB->cbp & 15;
    int64 cbp_blk = currMB->cbp_blk;

    memcpy(p, &luma_cbp, sizeof(int));
    p += sizeof(int);
    memcpy(p, &cbp_blk, sizeof(int64));
    p += sizeof(int64);

    memcpy(p, currSlice->cofDC[0][0], 2 * 18 * sizeof(int));
    p += 2 * 18 * sizeof(int);

    for (b8 = 0; b8 < 4; ++b8) {
        for (b4 = 0; b4 < 4; ++b4) {
            memcpy(p, currSlice->cofAC[b8][b4][0], 2 * 65 * sizeof(int));
            p += 2 * 65 * sizeof(int);
        }
    }
}

void eva_dump_macroblock(Macroblock *currMB)
{
    Slice *currSlice = currMB->p_Slice;
    imgpel **mb_pred = currSlice->mb_pred[PLANE_Y];
    unsigned char pred256[256];
    unsigned char coeff256[256];
    unsigned char type6[6];
    unsigned char mv96[96];
    unsigned char modes16[16];
    unsigned char b8x8_8[8];
    unsigned char chroma[EVA_CHROMA_BYTES];
    unsigned char mvd64[64];
    unsigned char luma_cof[EVA_LUMA_COF_BYTES];
    int y, x;

    if (!fp_pred_y)
        return;

    for (y = 0; y < 16; ++y) {
        for (x = 0; x < 16; ++x) {
            pred256[y * 16 + x] = (unsigned char)mb_pred[y][x];
        }
    }

    eva_dump_luma_coeff_mb(currSlice, currMB, coeff256);
    eva_dump_mv_mb(currMB, mv96);
    eva_dump_intra_modes_mb(currMB, modes16);
    eva_dump_b8x8_mb(currMB, b8x8_8);
    eva_dump_chroma_mb(currMB, currSlice, chroma);
    eva_dump_mvd_mb(currMB, mvd64);
    eva_dump_luma_cof_mb(currMB, currSlice, luma_cof);

    type6[0] = (unsigned char)eva_slice_byte(currSlice);
    type6[1] = (unsigned char)currMB->mb_type;
    type6[2] = currMB->luma_transform_size_8x8_flag;
    type6[3] = (unsigned char)(currMB->i16mode ? currMB->i16mode : currMB->intra_pred_modes[0]);
    type6[4] = (unsigned char)currMB->qp;
    type6[5] = (unsigned char)currMB->qpc[0];

    fwrite(pred256, 1, 256, fp_pred_y);
    fwrite(coeff256, 1, 256, fp_coeff_y);
    fwrite(type6, 1, 6, fp_type);
    fwrite(mv96, 1, 96, fp_mv);
    fwrite(modes16, 1, 16, fp_modes);
    fwrite(b8x8_8, 1, 8, fp_b8x8);
    fwrite(chroma, 1, EVA_CHROMA_BYTES, fp_chroma);
    fwrite(mvd64, 1, 64, fp_mvd);
    fwrite(luma_cof, 1, EVA_LUMA_COF_BYTES, fp_luma_cof);
}
