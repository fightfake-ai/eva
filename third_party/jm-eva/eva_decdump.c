/*!
 * Eva syntax dumps written from JM ldecod.
 *
 * Mirrors eva_dump.c record for record, so the publishing glue cannot tell
 * whether a dump set came from our encoder or from a decoded bitstream.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "global.h"
#include "mbuffer.h"
#include "mb_access.h"
#include "eva_decdump.h"

#define EVA_PRED_BYTES   256
#define EVA_COEFF_BYTES  256
#define EVA_TYPE_BYTES   6
#define EVA_MV_BYTES     96
#define EVA_MODES_BYTES  16
#define EVA_B8X8_BYTES   8
#define EVA_MVD_BYTES    64
#define EVA_CHROMA_BYTES (2 + 2 * (2 * 18 * (int)sizeof(int)) + 2 * 4 * (2 * 65 * (int)sizeof(int)))
#define EVA_LUMA_COF_BYTES                                                                 \
    (4 + (int)sizeof(int64) + 2 * 18 * (int)sizeof(int) +                                  \
     4 * 4 * 2 * 65 * (int)sizeof(int))

static const byte EVA_SNGL_SCAN[16][2] = {
    {0, 0}, {1, 0}, {0, 1}, {0, 2}, {1, 1}, {2, 0}, {3, 0}, {2, 1},
    {1, 2}, {0, 3}, {1, 3}, {2, 2}, {3, 1}, {3, 2}, {2, 3}, {3, 3},
};

static FILE *fp_pred_y;
static FILE *fp_coeff_y;
static FILE *fp_type;
static FILE *fp_mv;
static FILE *fp_modes;
static FILE *fp_b8x8;
static FILE *fp_chroma;
static FILE *fp_mvd;
static FILE *fp_luma_cof;

static int mbs_per_frame;

/* Per-macroblock capture, filled by the parse-time hooks. Dimensions match the
 * encoder's cofAC[6][4][2][65] and cofDC[3][2][18]. */
static int ac_level[6][4][65];
static int ac_run[6][4][65];
static int ac_pos[6][4];
static int dc_level[3][18];
static int dc_run[3][18];
static int dc_pos[3];
static signed char ipred_coded[16];

int eva_decdump_enabled(void)
{
    return fp_pred_y != NULL;
}

static FILE *eva_open(const char *dir, const char *name)
{
    char path[1024];
    FILE *fp;

    snprintf(path, sizeof(path), "%s/%s", dir, name);
    fp = fopen(path, "wb");
    if (!fp) {
        fprintf(stderr, "eva_decdump_open: cannot write %s\n", path);
        exit(1);
    }
    return fp;
}

void eva_decdump_open(const char *dir)
{
    fp_pred_y   = eva_open(dir, "pred_y_enc");
    fp_coeff_y  = eva_open(dir, "coeff_y_enc");
    fp_type     = eva_open(dir, "type_enc");
    fp_mv       = eva_open(dir, "mv_enc");
    fp_modes    = eva_open(dir, "intra_modes_enc");
    fp_b8x8     = eva_open(dir, "b8x8_enc");
    fp_chroma   = eva_open(dir, "chroma_enc");
    fp_mvd      = eva_open(dir, "mvd_enc");
    fp_luma_cof = eva_open(dir, "luma_cof_enc");
}

void eva_decdump_close(void)
{
    FILE **all[] = {&fp_pred_y, &fp_coeff_y, &fp_type, &fp_mv, &fp_modes,
                    &fp_b8x8, &fp_chroma, &fp_mvd, &fp_luma_cof};
    size_t i;

    for (i = 0; i < sizeof(all) / sizeof(all[0]); ++i) {
        if (*all[i]) {
            fclose(*all[i]);
            *all[i] = NULL;
        }
    }
}

/* Opened on the first macroblock so no JM startup path needs patching. */
static void eva_decdump_maybe_open(void)
{
    static int tried;
    const char *dir;

    if (tried)
        return;
    tried = 1;
    dir = getenv("EVA_DECDUMP_DIR");
    if (dir && dir[0]) {
        eva_decdump_open(dir);
        atexit(eva_decdump_close);
    }
}

void eva_decdump_begin_mb(Macroblock *currMB)
{
    eva_decdump_maybe_open();
    if (!eva_decdump_enabled())
        return;
    (void)currMB;
    memset(ac_level, 0, sizeof(ac_level));
    memset(ac_run, 0, sizeof(ac_run));
    memset(ac_pos, 0, sizeof(ac_pos));
    memset(dc_level, 0, sizeof(dc_level));
    memset(dc_run, 0, sizeof(dc_run));
    memset(dc_pos, 0, sizeof(dc_pos));
    memset(ipred_coded, 0, sizeof(ipred_coded));
}

void eva_decdump_ac(int cofac_idx, int b4, int level, int run)
{
    int *pos;

    if (!eva_decdump_enabled())
        return;
    if (cofac_idx < 0 || cofac_idx >= 6 || b4 < 0 || b4 >= 4)
        return;
    pos = &ac_pos[cofac_idx][b4];
    if (*pos >= 64)
        return;
    ac_level[cofac_idx][b4][*pos] = level;
    ac_run[cofac_idx][b4][*pos] = run;
    ++(*pos);
}

void eva_decdump_dc(int plane, int level, int run)
{
    int *pos;

    if (!eva_decdump_enabled())
        return;
    if (plane < 0 || plane >= 3)
        return;
    pos = &dc_pos[plane];
    if (*pos >= 17)
        return;
    dc_level[plane][*pos] = level;
    dc_run[plane][*pos] = run;
    ++(*pos);
}

void eva_decdump_intra_mode(int blk, int coded)
{
    if (!eva_decdump_enabled())
        return;
    if (blk < 0 || blk >= 16)
        return;
    ipred_coded[blk] = (signed char)coded;
}

/* --------------------------------------------------------------- per record */

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

static int frame_idx = -1;
static int last_poc;
static int have_poc;

/* JM's own picture counter is bumped after a picture finishes, so it cannot
 * index records while one is being decoded. Follow the picture order count
 * instead: it changes exactly once per coded picture. */
static long eva_mb_offset(Macroblock *currMB)
{
    int poc = currMB->p_Slice->framepoc;

    if (!have_poc || poc != last_poc) {
        have_poc = 1;
        last_poc = poc;
        ++frame_idx;
    }
    return (long)frame_idx * mbs_per_frame + currMB->mbAddrX;
}

static void eva_put(FILE *fp, long off, int size, const void *buf)
{
    if (!fp)
        return;
    if (fseek(fp, off * size, SEEK_SET) != 0 ||
        fwrite(buf, 1, (size_t)size, fp) != (size_t)size) {
        fprintf(stderr, "eva_decdump: short write at record %ld\n", off);
        exit(1);
    }
}

/* Rebuild the 256-byte luma coefficient image the encoder dumps: the quantised
 * level of each position, biased by 128, in raster order inside the block. */
static void eva_coeff_image(Macroblock *currMB, unsigned char *out256)
{
    int jj, ii;

    /* Walks all sixteen (b8, b4) slots with the 4x4 scan and a 16-coefficient
     * limit, exactly as eva_dump.c does, including for 8x8-transform blocks
     * whose 64 pairs all sit under b4 = 0. */
    memset(out256, 128, EVA_COEFF_BYTES);
    (void)currMB;
    for (jj = 0; jj < 4; ++jj) {
        for (ii = 0; ii < 4; ++ii) {
            int b8 = 2 * (jj >> 1) + (ii >> 1);
            int b4 = 2 * (jj & 1) + (ii & 1);
            const int *lev = ac_level[b8][b4];
            const int *run = ac_run[b8][b4];
            int coeff = 0;
            int k = 0;

            while (k < 16 && lev[k] != 0) {
                int scan_idx, i, j, idx, stored;

                coeff += run[k] + 1;
                if (coeff > 16)
                    break;
                scan_idx = coeff - 1;
                i = EVA_SNGL_SCAN[scan_idx][0];
                j = EVA_SNGL_SCAN[scan_idx][1];
                idx = (jj * 4 + j) * 16 + (ii * 4 + i);
                stored = lev[k] + 128;
                if (stored < 0)
                    stored = 0;
                if (stored > 255)
                    stored = 255;
                out256[idx] = (unsigned char)stored;
                ++k;
            }
        }
    }
}

static short eva_ref_frame_no(Macroblock *currMB, PicMotionParams *pm, int list)
{
    int cur;

    if (pm->ref_idx[list] < 0)
        return -1;
    (void)currMB;
    cur = frame_idx;
    if (cur <= pm->ref_idx[list])
        return -1;
    return (short)(cur - 1 - pm->ref_idx[list]);
}

static void eva_mv_mb(Macroblock *currMB, unsigned char *out96)
{
    PicMotionParams **motion = currMB->p_Slice->dec_picture->mv_info;
    int bx, by, k = 0;

    memset(out96, 0, EVA_MV_BYTES);
    for (by = 0; by < 4; ++by) {
        for (bx = 0; bx < 4; ++bx) {
            PicMotionParams *pm = &motion[currMB->block_y + by][currMB->block_x + bx];
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

static void eva_mvd_mb(Macroblock *currMB, unsigned char *out64)
{
    int bx, by, k = 0;

    memset(out64, 0, EVA_MVD_BYTES);
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

static void eva_modes_mb(Macroblock *currMB, unsigned char *out16)
{
    int i;

    memset(out16, 0, EVA_MODES_BYTES);
    switch (currMB->mb_type) {
    case I16MB:
        out16[0] = (unsigned char)currMB->i16mode;
        break;
    case I4MB:
        for (i = 0; i < 16; ++i)
            out16[i] = (unsigned char)ipred_coded[i];
        break;
    case I8MB:
        for (i = 0; i < 4; ++i)
            out16[i] = (unsigned char)ipred_coded[4 * i];
        break;
    default:
        break;
    }
}

static void eva_b8x8_mb(Macroblock *currMB, unsigned char *out8)
{
    int i;

    memset(out8, 0, EVA_B8X8_BYTES);
    if (currMB->mb_type != P8x8)
        return;
    for (i = 0; i < 4; ++i) {
        out8[i * 2] = (unsigned char)currMB->b8mode[i];
        out8[i * 2 + 1] = (unsigned char)currMB->b8pdir[i];
    }
}

static void eva_luma_cof_mb(Macroblock *currMB, unsigned char *out)
{
    unsigned char *p = out;
    int b8, b4;
    int luma_cbp = currMB->cbp & 15;
    int64 cbp_blk = currMB->s_cbp[0].blk;

    memcpy(p, &luma_cbp, sizeof(int));
    p += sizeof(int);
    memcpy(p, &cbp_blk, sizeof(int64));
    p += sizeof(int64);

    /* cofDC[plane] is one allocation: 18 levels then 18 runs. */
    memcpy(p, dc_level[0], 18 * sizeof(int));
    p += 18 * sizeof(int);
    memcpy(p, dc_run[0], 18 * sizeof(int));
    p += 18 * sizeof(int);

    for (b8 = 0; b8 < 4; ++b8) {
        for (b4 = 0; b4 < 4; ++b4) {
            memcpy(p, ac_level[b8][b4], 65 * sizeof(int));
            p += 65 * sizeof(int);
            memcpy(p, ac_run[b8][b4], 65 * sizeof(int));
            p += 65 * sizeof(int);
        }
    }
}

static void eva_chroma_mb(Macroblock *currMB, unsigned char *out)
{
    unsigned char *p = out;
    int uv, b4;

    p[0] = (unsigned char)((currMB->cbp >> 4) & 0xff);
    p[1] = (unsigned char)currMB->c_ipred_mode;
    p += 2;

    for (uv = 1; uv <= 2; ++uv) {
        memcpy(p, dc_level[uv], 18 * sizeof(int));
        p += 18 * sizeof(int);
        memcpy(p, dc_run[uv], 18 * sizeof(int));
        p += 18 * sizeof(int);
    }
    for (uv = 4; uv <= 5; ++uv) {
        for (b4 = 0; b4 < 4; ++b4) {
            memcpy(p, ac_level[uv][b4], 65 * sizeof(int));
            p += 65 * sizeof(int);
            memcpy(p, ac_run[uv][b4], 65 * sizeof(int));
            p += 65 * sizeof(int);
        }
    }
}

void eva_decdump_macroblock(Macroblock *currMB)
{
    Slice *currSlice = currMB->p_Slice;
    VideoParameters *p_Vid = currMB->p_Vid;
    imgpel **mb_pred = currSlice->mb_pred[PLANE_Y];
    unsigned char pred256[EVA_PRED_BYTES];
    unsigned char coeff256[EVA_COEFF_BYTES];
    unsigned char type6[EVA_TYPE_BYTES];
    unsigned char mv96[EVA_MV_BYTES];
    unsigned char modes16[EVA_MODES_BYTES];
    unsigned char b8x8_8[EVA_B8X8_BYTES];
    unsigned char chroma[EVA_CHROMA_BYTES];
    unsigned char mvd64[EVA_MVD_BYTES];
    unsigned char luma_cof[EVA_LUMA_COF_BYTES];
    long off;
    int y, x;

    if (!eva_decdump_enabled())
        return;

    if (!mbs_per_frame) {
        mbs_per_frame = (int)p_Vid->PicSizeInMbs;
        if (mbs_per_frame <= 0) {
            fprintf(stderr, "eva_decdump: picture size unknown\n");
            exit(1);
        }
    }
    off = eva_mb_offset(currMB);

    for (y = 0; y < 16; ++y)
        for (x = 0; x < 16; ++x)
            pred256[y * 16 + x] = (unsigned char)mb_pred[y][x];

    eva_coeff_image(currMB, coeff256);
    eva_mv_mb(currMB, mv96);
    eva_modes_mb(currMB, modes16);
    eva_b8x8_mb(currMB, b8x8_8);
    eva_chroma_mb(currMB, chroma);
    eva_mvd_mb(currMB, mvd64);
    eva_luma_cof_mb(currMB, luma_cof);

    type6[0] = (unsigned char)eva_slice_byte(currSlice);
    type6[1] = (unsigned char)currMB->mb_type;
    type6[2] = (unsigned char)currMB->luma_transform_size_8x8_flag;
    type6[3] = (unsigned char)(currMB->i16mode ? currMB->i16mode : ipred_coded[0]);
    type6[4] = (unsigned char)currMB->qp;
    type6[5] = (unsigned char)currMB->qpc[0];

    eva_put(fp_pred_y, off, EVA_PRED_BYTES, pred256);
    eva_put(fp_coeff_y, off, EVA_COEFF_BYTES, coeff256);
    eva_put(fp_type, off, EVA_TYPE_BYTES, type6);
    eva_put(fp_mv, off, EVA_MV_BYTES, mv96);
    eva_put(fp_modes, off, EVA_MODES_BYTES, modes16);
    eva_put(fp_b8x8, off, EVA_B8X8_BYTES, b8x8_8);
    eva_put(fp_chroma, off, EVA_CHROMA_BYTES, chroma);
    eva_put(fp_mvd, off, EVA_MVD_BYTES, mvd64);
    eva_put(fp_luma_cof, off, EVA_LUMA_COF_BYTES, luma_cof);
}
