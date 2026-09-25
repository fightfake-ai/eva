/*!
 * Eva CABAC glue for JM lencod: inject merged pred/coeff/type/mv per MB, then write_macroblock.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "global.h"
#include "mbuffer.h"
#include "fmo.h"
#include "mode_decision.h"
#include "mv_search.h"
#include "md_common.h"
#include "transform.h"
#include "mc_prediction.h"
#include "macroblock.h"
#include "blk_prediction.h"
#include "mb_access.h"
#include "intra4x4.h"
#include "intra16x16.h"
#include "intra8x8.h"
#include "eva_glue.h"

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

static const byte EVA_SNGL_SCAN8x8[64][2] = {
    {0, 0}, {1, 0}, {0, 1}, {0, 2}, {1, 1}, {2, 0}, {3, 0}, {2, 1},
    {1, 2}, {0, 3}, {0, 4}, {1, 3}, {2, 2}, {3, 1}, {4, 0}, {5, 0},
    {4, 1}, {3, 2}, {2, 3}, {1, 4}, {0, 5}, {0, 6}, {1, 5}, {2, 4},
    {3, 3}, {4, 2}, {5, 1}, {6, 0}, {7, 0}, {6, 1}, {5, 2}, {4, 3},
    {3, 4}, {2, 5}, {1, 6}, {0, 7}, {1, 7}, {2, 6}, {3, 5}, {4, 4},
    {5, 3}, {6, 2}, {7, 1}, {7, 2}, {6, 3}, {5, 4}, {4, 5}, {3, 6},
    {2, 7}, {3, 7}, {4, 6}, {5, 5}, {6, 4}, {7, 3}, {7, 4}, {6, 5},
    {5, 6}, {4, 7}, {5, 7}, {6, 6}, {7, 5}, {7, 6}, {6, 7}, {7, 7},
};

static FILE *fp_pred = NULL;
static FILE *fp_coeff = NULL;
static FILE *fp_type = NULL;
static FILE *fp_mv = NULL;
static FILE *fp_modes = NULL;
static FILE *fp_b8x8 = NULL;
static FILE *fp_chroma = NULL;
static FILE *fp_mvd = NULL;
static FILE *fp_luma_cof = NULL;
static int glue_mbs_per_frame = 0;
static int glue_have_modes = 0;
static int glue_have_b8x8 = 0;
static int glue_have_chroma = 0;
static int glue_have_mvd = 0;
static int glue_have_luma_cof = 0;
static int glue_exact = 0;
static int glue_ipcm_count = 0;
#define GLUE_IPCM_MAX_FRAMES 32
static int glue_ipcm_frame[GLUE_IPCM_MAX_FRAMES];
static int glue_ipcm_frames_seen;
static int glue_encode_count = 0;
static unsigned char *glue_encode_slots = NULL; /* 1 = encode (do not inject) */
static int glue_encode_slots_n = 0;
static FILE *glue_ipcm_log = NULL;

extern void update_qp(Macroblock *currMB);
extern void update_qp_cbp(Macroblock *currMB);
extern void FindSkipModeMotionVector(Macroblock *currMB);
extern void SetMotionVectorsMBPSlice(Macroblock *currMB);

int eva_glue_enabled(void)
{
    return fp_pred != NULL;
}

int eva_glue_use_stored_mvd(void)
{
    return glue_have_mvd;
}

int eva_glue_should_inject(Macroblock *currMB)
{
    int off;
    if (!glue_encode_slots)
        return 1;
    off = currMB->p_Vid->frame_no * glue_mbs_per_frame + currMB->mbAddrX;
    if (off < 0 || off >= glue_encode_slots_n)
        return 1;
    if (glue_encode_slots[off]) {
        ++glue_encode_count;
        return 0;
    }
    return 1;
}

void eva_glue_open(const char *dir, int width, int height)
{
    char path[1024];
    int mb_w = width >> 4;
    int mb_h = height >> 4;

    glue_mbs_per_frame = mb_w * mb_h;
    if (glue_mbs_per_frame <= 0) {
        fprintf(stderr, "eva_glue_open: invalid size %dx%d\n", width, height);
        exit(1);
    }

    snprintf(path, sizeof(path), "%s/pred_y_enc", dir);
    fp_pred = fopen(path, "rb");
    snprintf(path, sizeof(path), "%s/coeff_y_enc", dir);
    fp_coeff = fopen(path, "rb");
    snprintf(path, sizeof(path), "%s/type_enc", dir);
    fp_type = fopen(path, "rb");
    snprintf(path, sizeof(path), "%s/mv_enc", dir);
    fp_mv = fopen(path, "rb");
    if (!fp_pred || !fp_coeff || !fp_type || !fp_mv) {
        fprintf(stderr, "eva_glue_open: missing pred/coeff/type/mv in %s\n", dir);
        exit(1);
    }

    snprintf(path, sizeof(path), "%s/intra_modes_enc", dir);
    fp_modes = fopen(path, "rb");
    glue_have_modes = fp_modes != NULL;

    snprintf(path, sizeof(path), "%s/b8x8_enc", dir);
    fp_b8x8 = fopen(path, "rb");
    glue_have_b8x8 = fp_b8x8 != NULL;

    snprintf(path, sizeof(path), "%s/chroma_enc", dir);
    fp_chroma = fopen(path, "rb");
    glue_have_chroma = fp_chroma != NULL;

    snprintf(path, sizeof(path), "%s/mvd_enc", dir);
    fp_mvd = fopen(path, "rb");
    glue_have_mvd = fp_mvd != NULL;

    snprintf(path, sizeof(path), "%s/luma_cof_enc", dir);
    fp_luma_cof = fopen(path, "rb");
    glue_have_luma_cof = fp_luma_cof != NULL;

    {
        const char *exact = getenv("EVA_GLUE_EXACT");
        glue_exact = exact != NULL && exact[0] != '\0' && exact[0] != '0';
    }
    glue_ipcm_count = 0;
    glue_ipcm_frames_seen = 0;
    glue_encode_count = 0;
    {
        int i;
        for (i = 0; i < GLUE_IPCM_MAX_FRAMES; ++i)
            glue_ipcm_frame[i] = 0;
    }

    glue_encode_slots = NULL;
    glue_encode_slots_n = 0;
    {
        const char *slots = getenv("EVA_GLUE_ENCODE_SLOTS");
        if (slots && slots[0]) {
            FILE *fp = fopen(slots, "r");
            int f, mx, my, n = 0;
            glue_encode_slots_n = GLUE_IPCM_MAX_FRAMES * glue_mbs_per_frame;
            glue_encode_slots = (unsigned char *)calloc((size_t)glue_encode_slots_n, 1);
            if (fp && glue_encode_slots) {
                while (fscanf(fp, "%d %d %d", &f, &mx, &my) == 3) {
                    int off;
                    if (f < 0 || f >= GLUE_IPCM_MAX_FRAMES || mx < 0 || mx >= mb_w ||
                        my < 0 || my >= mb_h)
                        continue;
                    off = f * glue_mbs_per_frame + my * mb_w + mx;
                    glue_encode_slots[off] = 1;
                    ++n;
                }
                fprintf(stderr, "eva_glue: encode slots %d from %s\n", n, slots);
                fclose(fp);
            } else {
                fprintf(stderr, "eva_glue: cannot load encode slots %s\n", slots);
                free(glue_encode_slots);
                glue_encode_slots = NULL;
                if (fp)
                    fclose(fp);
            }
        }
    }

    glue_ipcm_log = NULL;
    {
        const char *logp = getenv("EVA_GLUE_IPCMLOG");
        if (logp && logp[0]) {
            glue_ipcm_log = fopen(logp, "w");
            if (!glue_ipcm_log)
                fprintf(stderr, "eva_glue: cannot open IPCM log %s\n", logp);
        }
    }
}

void eva_glue_close(void)
{
    if (glue_exact || glue_encode_slots) {
        int i;
        fprintf(stderr, "eva_glue: exact IPCM macroblocks %d\n", glue_ipcm_count);
        for (i = 0; i < glue_ipcm_frames_seen && i < GLUE_IPCM_MAX_FRAMES; ++i)
            fprintf(stderr, "eva_glue: frame %d IPCM %d\n", i, glue_ipcm_frame[i]);
        if (glue_encode_slots)
            fprintf(stderr, "eva_glue: encode_one_macroblock calls %d\n", glue_encode_count);
    }
    if (glue_ipcm_log) {
        fclose(glue_ipcm_log);
        glue_ipcm_log = NULL;
    }
    free(glue_encode_slots);
    glue_encode_slots = NULL;
    if (fp_pred) {
        fclose(fp_pred);
        fp_pred = NULL;
    }
    if (fp_coeff) {
        fclose(fp_coeff);
        fp_coeff = NULL;
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
    glue_have_modes = 0;
    glue_have_b8x8 = 0;
    glue_have_chroma = 0;
    glue_have_mvd = 0;
    glue_have_luma_cof = 0;
}

static long eva_glue_mb_offset(Macroblock *currMB)
{
    VideoParameters *p_Vid = currMB->p_Vid;
    return (long)(p_Vid->frame_no * glue_mbs_per_frame + currMB->mbAddrX);
}

static int eva_glue_read_type6(Macroblock *currMB, unsigned char *type6)
{
    long off = eva_glue_mb_offset(currMB);
    if (fseek(fp_type, off * EVA_TYPE_BYTES, SEEK_SET) ||
        fread(type6, 1, EVA_TYPE_BYTES, fp_type) != EVA_TYPE_BYTES) {
        return 0;
    }
    return 1;
}

/* Slice header QP of the capture. Rate-controlled captures move the frame QP
 * around, and the replay has no rate controller to rediscover it. */
int eva_glue_slice_qp(int frame_no, int first_mb)
{
    unsigned char type6[EVA_TYPE_BYTES];
    long off;

    if (!fp_type || glue_mbs_per_frame <= 0)
        return -1;
    off = (long)frame_no * glue_mbs_per_frame + first_mb;
    if (fseek(fp_type, off * EVA_TYPE_BYTES, SEEK_SET) ||
        fread(type6, 1, EVA_TYPE_BYTES, fp_type) != EVA_TYPE_BYTES) {
        return -1;
    }
    return (int)type6[4];
}

static int eva_glue_prev_qp(Macroblock *currMB)
{
    VideoParameters *p_Vid = currMB->p_Vid;
    int prev_addr = FmoGetPreviousMBNr(p_Vid, currMB->mbAddrX);
    unsigned char type6[EVA_TYPE_BYTES];
    Macroblock tmp;

    if (prev_addr < 0)
        return -1;

    memset(&tmp, 0, sizeof(tmp));
    tmp.p_Vid = p_Vid;
    tmp.mbAddrX = prev_addr;
    if (!eva_glue_read_type6(&tmp, type6))
        return -1;
    return (int)type6[4];
}

static int eva_glue_read_mb(Macroblock *currMB, unsigned char *pred256,
                            unsigned char *coeff256, unsigned char *type6,
                            unsigned char *mv96, unsigned char *modes16,
                            unsigned char *b8x8_8, unsigned char *chroma,
                            unsigned char *mvd64, unsigned char *luma_cof)
{
    long off = eva_glue_mb_offset(currMB);

    if (fseek(fp_pred, off * EVA_PRED_BYTES, SEEK_SET) ||
        fread(pred256, 1, EVA_PRED_BYTES, fp_pred) != EVA_PRED_BYTES) {
        return 0;
    }
    if (fseek(fp_coeff, off * EVA_COEFF_BYTES, SEEK_SET) ||
        fread(coeff256, 1, EVA_COEFF_BYTES, fp_coeff) != EVA_COEFF_BYTES) {
        return 0;
    }
    if (fseek(fp_type, off * EVA_TYPE_BYTES, SEEK_SET) ||
        fread(type6, 1, EVA_TYPE_BYTES, fp_type) != EVA_TYPE_BYTES) {
        return 0;
    }
    if (fseek(fp_mv, off * EVA_MV_BYTES, SEEK_SET) ||
        fread(mv96, 1, EVA_MV_BYTES, fp_mv) != EVA_MV_BYTES) {
        return 0;
    }

    memset(modes16, 0, EVA_MODES_BYTES);
    if (glue_have_modes) {
        if (fseek(fp_modes, off * EVA_MODES_BYTES, SEEK_SET) ||
            fread(modes16, 1, EVA_MODES_BYTES, fp_modes) != EVA_MODES_BYTES) {
            return 0;
        }
    }

    memset(b8x8_8, 0, EVA_B8X8_BYTES);
    if (glue_have_b8x8) {
        if (fseek(fp_b8x8, off * EVA_B8X8_BYTES, SEEK_SET) ||
            fread(b8x8_8, 1, EVA_B8X8_BYTES, fp_b8x8) != EVA_B8X8_BYTES) {
            return 0;
        }
    }

    memset(chroma, 0, EVA_CHROMA_BYTES);
    if (glue_have_chroma) {
        if (fseek(fp_chroma, off * EVA_CHROMA_BYTES, SEEK_SET) ||
            fread(chroma, 1, EVA_CHROMA_BYTES, fp_chroma) != EVA_CHROMA_BYTES) {
            return 0;
        }
    }

    memset(mvd64, 0, EVA_MVD_BYTES);
    if (glue_have_mvd) {
        if (fseek(fp_mvd, off * EVA_MVD_BYTES, SEEK_SET) ||
            fread(mvd64, 1, EVA_MVD_BYTES, fp_mvd) != EVA_MVD_BYTES) {
            return 0;
        }
    }

    memset(luma_cof, 0, EVA_LUMA_COF_BYTES);
    if (glue_have_luma_cof) {
        if (fseek(fp_luma_cof, off * EVA_LUMA_COF_BYTES, SEEK_SET) ||
            fread(luma_cof, 1, EVA_LUMA_COF_BYTES, fp_luma_cof) != EVA_LUMA_COF_BYTES) {
            return 0;
        }
    }

    return 1;
}

static void eva_glue_apply_luma_cof(Macroblock *currMB, Slice *currSlice,
                                    const unsigned char *luma_cof)
{
    const unsigned char *p = luma_cof;
    int b8, b4;
    int luma_cbp;
    int64 cbp_blk;

    if (!glue_have_luma_cof)
        return;

    memcpy(&luma_cbp, p, sizeof(int));
    p += sizeof(int);
    memcpy(&cbp_blk, p, sizeof(int64));
    p += sizeof(int64);

    memcpy(currSlice->cofDC[0][0], p, 2 * 18 * sizeof(int));
    p += 2 * 18 * sizeof(int);

    for (b8 = 0; b8 < 4; ++b8) {
        for (b4 = 0; b4 < 4; ++b4) {
            memcpy(currSlice->cofAC[b8][b4][0], p, 2 * 65 * sizeof(int));
            p += 2 * 65 * sizeof(int);
        }
    }

    currMB->cbp = (currMB->cbp & ~15) | (luma_cbp & 15);
    currMB->cbp_blk = cbp_blk;
}

static void eva_glue_load_mvd(Macroblock *currMB, const unsigned char *mvd64)
{
    int bx, by, k = 0;
    if (!glue_have_mvd)
        return;
    for (by = 0; by < 4; ++by) {
        for (bx = 0; bx < 4; ++bx) {
            short mx = (short)(mvd64[k] | (mvd64[k + 1] << 8));
            short my = (short)(mvd64[k + 2] | (mvd64[k + 3] << 8));
            k += 4;
            currMB->mvd[0][by][bx][0] = mx;
            currMB->mvd[0][by][bx][1] = my;
            currMB->mvd[1][by][bx][0] = 0;
            currMB->mvd[1][by][bx][1] = 0;
        }
    }
}

static void eva_glue_apply_chroma(Macroblock *currMB, Slice *currSlice,
                                  const unsigned char *chroma)
{
    const unsigned char *p = chroma;
    int b4;

    if (!glue_have_chroma)
        return;

    currMB->cbp = (currMB->cbp & 15) | ((p[0] & 0xff) << 4);
    currMB->c_ipred_mode = p[1];
    p += 2;

    memcpy(currSlice->cofDC[1][0], p, 2 * 18 * sizeof(int));
    p += 2 * 18 * sizeof(int);
    memcpy(currSlice->cofDC[2][0], p, 2 * 18 * sizeof(int));
    p += 2 * 18 * sizeof(int);

    for (b4 = 0; b4 < 4; ++b4) {
        memcpy(currSlice->cofAC[4][b4][0], p, 2 * 65 * sizeof(int));
        p += 2 * 65 * sizeof(int);
    }
    for (b4 = 0; b4 < 4; ++b4) {
        memcpy(currSlice->cofAC[5][b4][0], p, 2 * 65 * sizeof(int));
        p += 2 * 65 * sizeof(int);
    }
}

static void eva_glue_clear_cof_mb(Slice *currSlice)
{
    int b8, b4;
    int num_b8 = 4 + currSlice->p_Vid->num_blk8x8_uv;
    for (b8 = 0; b8 < num_b8; ++b8) {
        for (b4 = 0; b4 < 4; ++b4) {
            memset(currSlice->cofAC[b8][b4][0], 0, 65 * sizeof(int));
            memset(currSlice->cofAC[b8][b4][1], 0, 65 * sizeof(int));
        }
    }
    memset(currSlice->cofDC[0][0], 0, 2 * 18 * sizeof(int));
    memset(currSlice->cofDC[1][0], 0, 2 * 18 * sizeof(int));
    memset(currSlice->cofDC[2][0], 0, 2 * 18 * sizeof(int));
}

static int eva_glue_load_coeff_4x4(Slice *currSlice, int block_x, int block_y,
                                   const unsigned char *coeff256)
{
    int pos_x = block_x >> 2;
    int pos_y = block_y >> 2;
    int b8 = 2 * (pos_y >> 1) + (pos_x >> 1);
    int b4 = 2 * (pos_y & 1) + (pos_x & 1);
    int *levels = currSlice->cofAC[b8][b4][0];
    int *runs = currSlice->cofAC[b8][b4][1];
    int k = 0;
    int last_scan = -1;
    int scan_idx;

    levels[0] = 0;
    for (scan_idx = 0; scan_idx < 16; ++scan_idx) {
        int i = EVA_SNGL_SCAN[scan_idx][0];
        int j = EVA_SNGL_SCAN[scan_idx][1];
        int idx = (block_y + j) * 16 + (block_x + i);
        int stored = coeff256[idx];
        int level;

        if (stored == 128)
            continue;
        level = stored - 128;
        if (level == 0)
            continue;
        runs[k] = scan_idx - last_scan - 1;
        if (runs[k] < 0)
            runs[k] = 0;
        levels[k] = level;
        last_scan = scan_idx;
        ++k;
        if (k >= 64)
            break;
    }
    levels[k] = 0;
    return k > 0;
}

static void eva_glue_update_cbp(Macroblock *currMB, Slice *currSlice,
                                const unsigned char *coeff256, int mb_type)
{
    int block_x, block_y;
    int cbp = 0;
    int64 cbp_blk = 0;

    if (mb_type == 0) {
        currMB->cbp = 0;
        currMB->cbp_blk = 0;
        return;
    }

    for (block_y = 0; block_y < 16; block_y += 4) {
        for (block_x = 0; block_x < 16; block_x += 4) {
            int pos_x = block_x >> 2;
            int pos_y = block_y >> 2;
            int b8 = 2 * (pos_y >> 1) + (pos_x >> 1);
            int mask = 1 << b8;
            int blk_mask = (block_x >> 2) + block_y;
            int nonzero = 0;
            int scan_idx;

            for (scan_idx = 0; scan_idx < 16; ++scan_idx) {
                int i = EVA_SNGL_SCAN[scan_idx][0];
                int j = EVA_SNGL_SCAN[scan_idx][1];
                int idx = (block_y + j) * 16 + (block_x + i);
                if (coeff256[idx] != 128) {
                    nonzero = 1;
                    break;
                }
            }
            if (nonzero) {
                cbp |= mask;
                cbp_blk |= ((int64)1) << blk_mask;
            }
            (void)eva_glue_load_coeff_4x4(currSlice, block_x, block_y, coeff256);
        }
    }

    currMB->cbp = cbp;
    currMB->cbp_blk = cbp_blk;
}

static void eva_glue_load_pred(Macroblock *currMB, Slice *currSlice,
                               const unsigned char *pred256)
{
    imgpel **mb_pred = currSlice->mb_pred[PLANE_Y];
    imgpel **imgY = currMB->p_Vid->enc_picture->imgY;
    int x, y;

    for (y = 0; y < 16; ++y) {
        for (x = 0; x < 16; ++x) {
            imgpel p = (imgpel)pred256[y * 16 + x];
            mb_pred[y][x] = p;
            /* Prediction first. eva_glue_reconstruct adds the dequantised residual. */
            imgY[currMB->pix_y + y][currMB->pix_x + x] = p;
        }
    }
}

/* Mirror set_modes_and_refs_for_blocks for I4/I8/I16. */
static void eva_glue_set_intra_state(Macroblock *currMB)
{
    int i;
    currMB->is_intra_block = TRUE;
    currMB->IntraChromaPredModeFlag = 1;

    switch (currMB->mb_type) {
    case I4MB:
        currMB->luma_transform_size_8x8_flag = 0;
        for (i = 0; i < 4; ++i) {
            currMB->b8x8[i].mode = IBLOCK;
            currMB->b8x8[i].pdir = -1;
            currMB->b8x8[i].bipred = 0;
            currMB->b8x8[i].ref[0] = -1;
            currMB->b8x8[i].ref[1] = -1;
        }
        break;
    case I8MB:
        currMB->luma_transform_size_8x8_flag = TRUE;
        for (i = 0; i < 4; ++i) {
            currMB->b8x8[i].mode = I8MB;
            currMB->b8x8[i].pdir = -1;
            currMB->b8x8[i].bipred = 0;
            currMB->b8x8[i].ref[0] = -1;
            currMB->b8x8[i].ref[1] = -1;
        }
        break;
    case I16MB:
        for (i = 0; i < 4; ++i) {
            currMB->b8x8[i].mode = 0;
            currMB->b8x8[i].pdir = -1;
            currMB->b8x8[i].bipred = 0;
            currMB->b8x8[i].ref[0] = -1;
            currMB->b8x8[i].ref[1] = -1;
        }
        break;
    default:
        for (i = 0; i < 4; ++i) {
            currMB->b8x8[i].pdir = -1;
            currMB->b8x8[i].ref[0] = -1;
            currMB->b8x8[i].ref[1] = -1;
        }
        break;
    }
}

/* Mirror set_modes_and_refs_for_blocks_p_slice for modes we dump. */
static void eva_glue_set_p_state(Macroblock *currMB, const unsigned char *b8x8_8)
{
    int i;
    short mode = currMB->mb_type;

    currMB->is_intra_block = FALSE;
    currMB->IntraChromaPredModeFlag = 0;
    currMB->ar_mode = mode;

    switch (mode) {
    case 0: /* PSKIP */
        for (i = 0; i < 4; ++i) {
            currMB->b8x8[i].mode = 0;
            currMB->b8x8[i].pdir = 0;
            currMB->b8x8[i].bipred = 0;
            currMB->b8x8[i].ref[0] = 0;
            currMB->b8x8[i].ref[1] = -1;
        }
        break;
    case 1: /* P16x16 */
    case 2: /* P16x8 */
    case 3: /* P8x16 */
        for (i = 0; i < 4; ++i) {
            currMB->b8x8[i].mode = (char)mode;
            currMB->b8x8[i].pdir = 0;
            currMB->b8x8[i].bipred = 0;
            currMB->b8x8[i].ref[0] = 0;
            currMB->b8x8[i].ref[1] = -1;
        }
        break;
    case P8x8:
        for (i = 0; i < 4; ++i) {
            if (glue_have_b8x8) {
                currMB->b8x8[i].mode = b8x8_8[i * 2];
                currMB->b8x8[i].pdir = b8x8_8[i * 2 + 1];
            } else {
                currMB->b8x8[i].mode = 4; /* SMB8x8 fallback */
                currMB->b8x8[i].pdir = 0;
            }
            currMB->b8x8[i].bipred = 0;
            currMB->b8x8[i].ref[0] = 0;
            currMB->b8x8[i].ref[1] = -1;
        }
        break;
    default:
        break;
    }
}

static void eva_glue_apply_intra_modes(Macroblock *currMB, const unsigned char *type6,
                                       const unsigned char *modes16)
{
    int i;

    switch (currMB->mb_type) {
    case I16MB:
        currMB->i16mode = (char)(glue_have_modes ? modes16[0] : type6[3]);
        currMB->i16offset = I16Offset(currMB->cbp, currMB->i16mode);
        break;
    case I4MB:
        if (glue_have_modes) {
            for (i = 0; i < 16; ++i)
                currMB->intra_pred_modes[i] = (char)modes16[i];
        } else {
            for (i = 0; i < 16; ++i)
                currMB->intra_pred_modes[i] = (char)type6[3];
        }
        break;
    case I8MB:
        if (glue_have_modes) {
            for (i = 0; i < 4; ++i)
                currMB->intra_pred_modes8x8[4 * i] = (char)modes16[i];
        } else {
            for (i = 0; i < 4; ++i)
                currMB->intra_pred_modes8x8[4 * i] = (char)type6[3];
        }
        break;
    default:
        break;
    }
}

static void eva_glue_apply_b8x8(Macroblock *currMB, const unsigned char *b8x8_8)
{
    (void)currMB;
    (void)b8x8_8;
    /* P8x8 handled in eva_glue_set_p_state. */
}

static void eva_glue_load_mv(Macroblock *currMB, const unsigned char *mv96)
{
    VideoParameters *p_Vid = currMB->p_Vid;
    Slice *currSlice = currMB->p_Slice;
    PicMotionParams **motion = p_Vid->enc_picture->mv_info;
    int cur = p_Vid->frame_no;
    int bx, by;
    int k = 0;

    for (by = 0; by < 4; ++by) {
        for (bx = 0; bx < 4; ++bx) {
            int block_x = currMB->block_x + bx;
            int block_y = currMB->block_y + by;
            PicMotionParams *pm = &motion[block_y][block_x];
            short ref_fn = (short)(mv96[k] | (mv96[k + 1] << 8));
            short mv_x = (short)(mv96[k + 2] | (mv96[k + 3] << 8));
            short mv_y = (short)(mv96[k + 4] | (mv96[k + 5] << 8));
            k += 6;

            pm->mv[0].mv_x = mv_x;
            pm->mv[0].mv_y = mv_y;
            if (ref_fn < 0) {
                pm->ref_idx[0] = -1;
                pm->ref_pic[0] = NULL;
            } else {
                int ref_idx = cur - 1 - (int)ref_fn;
                pm->ref_idx[0] = (char)ref_idx;
                if (ref_idx >= 0 && ref_idx < currSlice->listXsize[LIST_0]) {
                    pm->ref_pic[0] =
                        currSlice->listX[currMB->list_offset + LIST_0][ref_idx];
                } else {
                    pm->ref_pic[0] = NULL;
                }
            }
            pm->ref_idx[1] = -1;
            pm->ref_pic[1] = NULL;
        }
    }
}

/* Inverse scale used by quant_4x4_normal / quant_ac4x4_normal. */
static int eva_glue_inv4(int level, int inv_scale, int qp_per)
{
    return rshift_rnd_sf(((level * inv_scale) << qp_per), 4);
}

/* Scatter a run/level list into a 4x4. start=0 includes DC; start=1 is AC only. */
static void eva_glue_place_4x4(const int *lev, const int *run, int **dst,
                               int y0, int x0, LevelQuantParams **q, int qp_per,
                               int start, int clear)
{
    int pos = start;
    int k = 0;
    int y, x;

    if (clear) {
        for (y = 0; y < 4; ++y)
            for (x = 0; x < 4; ++x)
                dst[y0 + y][x0 + x] = 0;
    }
    while (lev[k] != 0 && k < 16) {
        int sx, sy;
        pos += run[k];
        if (pos >= 0 && pos < 16) {
            sx = EVA_SNGL_SCAN[pos][0];
            sy = EVA_SNGL_SCAN[pos][1];
            dst[y0 + sy][x0 + sx] =
                eva_glue_inv4(lev[k], q[sy][sx].InvScaleComp, qp_per);
        }
        ++pos;
        ++k;
    }
}

static int eva_glue_nz4(int **b, int y0, int x0)
{
    int y, x;
    for (y = 0; y < 4; ++y)
        for (x = 0; x < 4; ++x)
            if (b[y0 + y][x0 + x] != 0)
                return 1;
    return 0;
}

static void eva_glue_zero_rres(int **mb_rres, int y0, int x0, int n)
{
    int y, x;
    for (y = 0; y < n; ++y)
        for (x = 0; x < n; ++x)
            mb_rres[y0 + y][x0 + x] = 0;
}

static void eva_glue_predict_i4(Macroblock *currMB, int block_x, int block_y);
static void eva_glue_predict_i8(Macroblock *currMB, int b8);

static void eva_glue_recon_luma_4x4(Macroblock *currMB, int qp, int intra)
{
    Slice *currSlice = currMB->p_Slice;
    VideoParameters *p_Vid = currMB->p_Vid;
    QuantParameters *p_Quant = p_Vid->p_Quant;
    int qp_per = p_Quant->qp_per_matrix[qp];
    LevelQuantParams **q = p_Quant->q_params_4x4[PLANE_Y][intra][qp];
    int **tblk = currSlice->tblk16x16;
    int **mb_rres = currSlice->mb_rres[PLANE_Y];
    imgpel **mb_pred = currSlice->mb_pred[PLANE_Y];
    imgpel **imgY = p_Vid->enc_picture->imgY;
    int max_pel = p_Vid->max_imgpel_value;
    int block_y, block_x;

    for (block_y = 0; block_y < 16; block_y += 4) {
        for (block_x = 0; block_x < 16; block_x += 4) {
            int pos_x = block_x >> 2;
            int pos_y = block_y >> 2;
            int b8 = 2 * (pos_y >> 1) + (pos_x >> 1);
            int b4 = 2 * (pos_y & 1) + (pos_x & 1);

            if (currMB->mb_type == I4MB)
                eva_glue_predict_i4(currMB, block_x, block_y);

            if (!intra && (currMB->cbp & (1 << b8)) == 0) {
                eva_glue_zero_rres(mb_rres, block_y, block_x, 4);
                sample_reconstruct(&imgY[currMB->pix_y + block_y], &mb_pred[block_y],
                                   &mb_rres[block_y], block_x, currMB->pix_x + block_x,
                                   BLOCK_SIZE, BLOCK_SIZE, max_pel, DQ_BITS);
                continue;
            }

            eva_glue_place_4x4(currSlice->cofAC[b8][b4][0],
                               currSlice->cofAC[b8][b4][1],
                               tblk, block_y, block_x, q, qp_per, 0, 1);
            if (eva_glue_nz4(tblk, block_y, block_x))
                inverse4x4(tblk, mb_rres, block_y, block_x);
            else
                eva_glue_zero_rres(mb_rres, block_y, block_x, 4);
            sample_reconstruct(&imgY[currMB->pix_y + block_y], &mb_pred[block_y],
                               &mb_rres[block_y], block_x, currMB->pix_x + block_x,
                               BLOCK_SIZE, BLOCK_SIZE, max_pel, DQ_BITS);
        }
    }
}

static void eva_glue_recon_luma_8x8(Macroblock *currMB, int qp, int intra)
{
    Slice *currSlice = currMB->p_Slice;
    VideoParameters *p_Vid = currMB->p_Vid;
    QuantParameters *p_Quant = p_Vid->p_Quant;
    int qp_per = p_Quant->qp_per_matrix[qp];
    LevelQuantParams **q = p_Quant->q_params_8x8[PLANE_Y][intra][qp];
    int **mb_rres = currSlice->mb_rres[PLANE_Y];
    imgpel **mb_pred = currSlice->mb_pred[PLANE_Y];
    imgpel **imgY = p_Vid->enc_picture->imgY;
    int max_pel = p_Vid->max_imgpel_value;
    int b8;

    for (b8 = 0; b8 < 4; ++b8) {
        int block_x = 8 * (b8 & 1);
        int block_y = 8 * (b8 >> 1);
        const int *lev = currSlice->cofAC[b8][0][0];

        if (currMB->mb_type == I8MB)
            eva_glue_predict_i8(currMB, b8);
        if (!intra && (currMB->cbp & (1 << b8)) == 0) {
            eva_glue_zero_rres(mb_rres, block_y, block_x, 8);
            sample_reconstruct(&imgY[currMB->pix_y + block_y], &mb_pred[block_y],
                               &mb_rres[block_y], block_x, currMB->pix_x + block_x,
                               BLOCK_SIZE_8x8, BLOCK_SIZE_8x8, max_pel, DQ_BITS_8);
            continue;
        }
        const int *run = currSlice->cofAC[b8][0][1];
        int pos = 0;
        int k = 0;
        int any = 0;

        eva_glue_zero_rres(mb_rres, block_y, block_x, 8);
        while (lev[k] != 0 && k < 64) {
            pos += run[k];
            if (pos >= 0 && pos < 64) {
                int sx = EVA_SNGL_SCAN8x8[pos][0];
                int sy = EVA_SNGL_SCAN8x8[pos][1];
                mb_rres[block_y + sy][block_x + sx] =
                    rshift_rnd_sf(((lev[k] * q[sy][sx].InvScaleComp) << qp_per), 6);
                any = 1;
            }
            ++pos;
            ++k;
        }
        if (any)
            inverse8x8(&mb_rres[block_y], &mb_rres[block_y], block_x);
        sample_reconstruct(&imgY[currMB->pix_y + block_y], &mb_pred[block_y],
                           &mb_rres[block_y], block_x, currMB->pix_x + block_x,
                           BLOCK_SIZE_8x8, BLOCK_SIZE_8x8, max_pel, DQ_BITS_8);
    }
}

/* Syntax element -> real intra mode. -1 means "most probable". */
static int eva_glue_ipmode_from_syntax(int syntax, int mpm)
{
    if (syntax < 0)
        return mpm;
    if (syntax < mpm)
        return syntax;
    return syntax + 1;
}

static int eva_glue_mpm(Macroblock *currMB, int block_x, int block_y, int up_8x8, int left_8x8)
{
    VideoParameters *p_Vid = currMB->p_Vid;
    PixelPos left_block, top_block;
    int *mb_size = p_Vid->mb_size[IS_LUMA];
    char up_mode, left_mode;

    get4x4Neighbour(currMB, block_x - 1, block_y, mb_size, &left_block);
    get4x4Neighbour(currMB, block_x, block_y - 1, mb_size, &top_block);
    if (top_block.available)
        up_mode = (up_8x8 ? p_Vid->ipredmode8x8 : p_Vid->ipredmode)[top_block.pos_y][top_block.pos_x];
    else
        up_mode = -1;
    if (left_block.available)
        left_mode = (left_8x8 ? p_Vid->ipredmode8x8 : p_Vid->ipredmode)[left_block.pos_y][left_block.pos_x];
    else
        left_mode = -1;
    if (up_mode < 0 || left_mode < 0)
        return DC_PRED;
    return up_mode < left_mode ? up_mode : left_mode;
}

static void eva_glue_mark_ipred_dc(Macroblock *currMB)
{
    VideoParameters *p_Vid = currMB->p_Vid;
    int j;

    for (j = currMB->block_y; j < currMB->block_y + BLOCK_MULTIPLE; ++j) {
        memset(&p_Vid->ipredmode[j][currMB->block_x], DC_PRED, BLOCK_MULTIPLE);
        memset(&p_Vid->ipredmode8x8[j][currMB->block_x], DC_PRED, BLOCK_MULTIPLE);
    }
}

/* I4 prediction is rebuilt from reconstructed neighbors. The dumped predictor can
 * disagree with the prediction a decoder derives from those same neighbors. */
static void eva_glue_predict_i4(Macroblock *currMB, int block_x, int block_y)
{
    Slice *currSlice = currMB->p_Slice;
    int b8 = 2 * (block_y >> 3) + (block_x >> 3);
    int b4 = 2 * ((block_y >> 2) & 1) + ((block_x >> 2) & 1);
    int mpm = eva_glue_mpm(currMB, block_x, block_y, 0, 0);
    int ipmode = eva_glue_ipmode_from_syntax(currMB->intra_pred_modes[4 * b8 + b4], mpm);
    int left_available, up_available, all_available;
    int pic_pix_x = currMB->pix_x + block_x;
    int pic_pix_y = currMB->pix_y + block_y;
    imgpel **mpr;
    imgpel **mb_pred = currSlice->mb_pred[PLANE_Y];
    int j;

    currSlice->set_intrapred_4x4(currMB, PLANE_Y, pic_pix_x, pic_pix_y,
                                  &left_available, &up_available, &all_available);
    get_intrapred_4x4(currMB, PLANE_Y, ipmode, block_x, block_y, left_available, up_available);
    mpr = currSlice->mpr_4x4[PLANE_Y][ipmode];
    for (j = 0; j < 4; ++j)
        memcpy(&mb_pred[block_y + j][block_x], mpr[j], 4 * sizeof(imgpel));
    currMB->p_Vid->ipredmode[currMB->block_y + (block_y >> 2)]
                            [currMB->block_x + (block_x >> 2)] = (char)ipmode;
}

/* I8 prediction is rebuilt from reconstructed neighbors, one 8x8 at a time, so the
 * next 8x8 sees this block's real mode. The dumped predictor was fitted to the
 * original neighbors and is wrong once a neighbor has changed. */
static void eva_glue_predict_i8(Macroblock *currMB, int b8)
{
    Slice *currSlice = currMB->p_Slice;
    int block_x = 8 * (b8 & 1);
    int block_y = 8 * (b8 >> 1);
    int mpm = eva_glue_mpm(currMB, block_x, block_y, b8 >> 1, b8 & 1);
    int ipmode = eva_glue_ipmode_from_syntax(currMB->intra_pred_modes8x8[4 * b8], mpm);
    int left_available, up_available, all_available;
    int pic_pix_x = currMB->pix_x + block_x;
    int pic_pix_y = currMB->pix_y + block_y;
    imgpel **mpr;
    imgpel **mb_pred = currSlice->mb_pred[PLANE_Y];
    int j;
    int by = currMB->block_y + (block_y >> 2);
    int bx = currMB->block_x + (block_x >> 2);
    VideoParameters *p_Vid = currMB->p_Vid;

    currSlice->set_intrapred_8x8(currMB, PLANE_Y, pic_pix_x, pic_pix_y,
                                  &left_available, &up_available, &all_available);
    get_intrapred_8x8(currMB, PLANE_Y, ipmode, left_available, up_available);
    mpr = currSlice->mpr_8x8[PLANE_Y][ipmode];
    for (j = 0; j < 8; ++j)
        memcpy(&mb_pred[block_y + j][block_x], mpr[j], 8 * sizeof(imgpel));

    p_Vid->ipredmode[by][bx] = (char)ipmode;
    p_Vid->ipredmode[by][bx + 1] = (char)ipmode;
    p_Vid->ipredmode[by + 1][bx] = (char)ipmode;
    p_Vid->ipredmode[by + 1][bx + 1] = (char)ipmode;
    p_Vid->ipredmode8x8[by][bx] = (char)ipmode;
    p_Vid->ipredmode8x8[by][bx + 1] = (char)ipmode;
    p_Vid->ipredmode8x8[by + 1][bx] = (char)ipmode;
    p_Vid->ipredmode8x8[by + 1][bx + 1] = (char)ipmode;
}

static void eva_glue_predict_i16(Macroblock *currMB)
{
    Slice *currSlice = currMB->p_Slice;
    int left_available, up_available, all_available;
    int mode = currMB->i16mode;
    imgpel **mpr;
    imgpel **mb_pred = currSlice->mb_pred[PLANE_Y];
    int j;

    currSlice->set_intrapred_16x16(currMB, PLANE_Y, &left_available, &up_available, &all_available);
    get_intrapred_16x16(currMB, PLANE_Y, mode, left_available, up_available);
    mpr = currSlice->mpr_16x16[PLANE_Y][mode];
    for (j = 0; j < 16; ++j)
        memcpy(mb_pred[j], mpr[j], 16 * sizeof(imgpel));
    eva_glue_mark_ipred_dc(currMB);
}

static void eva_glue_recon_luma_16x16(Macroblock *currMB, int qp)
{
    Slice *currSlice = currMB->p_Slice;
    VideoParameters *p_Vid = currMB->p_Vid;
    QuantParameters *p_Quant = p_Vid->p_Quant;
    int qp_per = p_Quant->qp_per_matrix[qp];
    LevelQuantParams **q = p_Quant->q_params_4x4[PLANE_Y][1][qp];
    int **tblk = currSlice->tblk16x16;
    int **mb_rres = currSlice->mb_rres[PLANE_Y];
    imgpel **mb_pred = currSlice->mb_pred[PLANE_Y];
    imgpel **imgY = p_Vid->enc_picture->imgY;
    int *DCLevel = currSlice->cofDC[PLANE_Y][0];
    int *DCRun = currSlice->cofDC[PLANE_Y][1];
    int dc[4][4];
    int *dc_rows[4];
    int pos, k, j, i, block_y, block_x;
    int inv_dc = q[0][0].InvScaleComp;

    eva_glue_predict_i16(currMB);

    memset(dc, 0, sizeof(dc));
    pos = 0;
    k = 0;
    while (DCLevel[k] != 0 && k < 16) {
        pos += DCRun[k];
        if (pos >= 0 && pos < 16) {
            int sx = EVA_SNGL_SCAN[pos][0];
            int sy = EVA_SNGL_SCAN[pos][1];
            dc[sy][sx] = DCLevel[k];
        }
        ++pos;
        ++k;
    }
    for (j = 0; j < 4; ++j)
        dc_rows[j] = dc[j];
    ihadamard4x4(dc_rows, dc_rows);

    for (j = 0; j < 16; ++j) {
        memset(tblk[j], 0, 16 * sizeof(int));
        memset(mb_rres[j], 0, 16 * sizeof(int));
    }

    for (j = 0; j < 4; ++j)
        for (i = 0; i < 4; ++i)
            tblk[j << 2][i << 2] =
                rshift_rnd_sf(((dc[j][i] * inv_dc) << qp_per), 6);

    for (block_y = 0; block_y < 16; block_y += 4) {
        for (block_x = 0; block_x < 16; block_x += 4) {
            int pos_x = block_x >> 2;
            int pos_y = block_y >> 2;
            int b8 = 2 * (pos_y >> 1) + (pos_x >> 1);
            int b4 = 2 * (pos_y & 1) + (pos_x & 1);

            eva_glue_place_4x4(currSlice->cofAC[b8][b4][0],
                               currSlice->cofAC[b8][b4][1],
                               tblk, block_y, block_x, q, qp_per, 1, 0);
            if (eva_glue_nz4(tblk, block_y, block_x))
                inverse4x4(tblk, mb_rres, block_y, block_x);
        }
    }

    sample_reconstruct(&imgY[currMB->pix_y], mb_pred, mb_rres, 0, currMB->pix_x,
                       16, 16, p_Vid->max_imgpel_value, DQ_BITS);
}

static void eva_glue_seed_all_mv(Macroblock *currMB)
{
    Slice *currSlice = currMB->p_Slice;
    VideoParameters *p_Vid = currMB->p_Vid;
    PicMotionParams **motion = p_Vid->enc_picture->mv_info;
    int by, bx;

    for (by = 0; by < 4; ++by) {
        for (bx = 0; bx < 4; ++bx) {
            PicMotionParams *pm = &motion[currMB->block_y + by][currMB->block_x + bx];
            int b8 = 2 * (by >> 1) + (bx >> 1);
            int ref = pm->ref_idx[LIST_0];
            int mode = currMB->b8x8[b8].mode;

            if (ref < 0)
                ref = 0;
            if (mode < 0 || mode > P8x8)
                mode = 0;
            if (ref >= p_Vid->max_num_references)
                continue;
            currSlice->all_mv[LIST_0][ref][mode][by][bx] = pm->mv[LIST_0];
        }
    }
}

static void eva_glue_predict_chroma(Macroblock *currMB)
{
    Slice *currSlice = currMB->p_Slice;
    VideoParameters *p_Vid = currMB->p_Vid;
    int yuv = p_Vid->yuv_format - 1;
    int uv, block_y, block_x;

    if (p_Vid->yuv_format == YUV400 || yuv < 0 || yuv > 2)
        return;

    if (currMB->mb_type > P8x8) {
        currSlice->intra_chroma_prediction(currMB, NULL, NULL, NULL);
        for (uv = 0; uv < 2; ++uv) {
            for (block_y = 0; block_y < p_Vid->mb_cr_size_y; block_y += BLOCK_SIZE) {
                for (block_x = 0; block_x < p_Vid->mb_cr_size_x; block_x += BLOCK_SIZE)
                    IntraChromaPrediction4x4(currMB, uv + 1, block_x, block_y);
            }
        }
        return;
    }

    eva_glue_seed_all_mv(currMB);
    for (uv = 0; uv < 2; ++uv) {
        for (block_y = 0; block_y < p_Vid->mb_cr_size_y; block_y += BLOCK_SIZE) {
            for (block_x = 0; block_x < p_Vid->mb_cr_size_x; block_x += BLOCK_SIZE) {
                int block8 = block8x8_idx[yuv][block_y >> 2][block_x >> 2];
                int list_mode[2];
                char list_ref_idx[2];
                short p_dir = 0;
                char bipred_me = currMB->b8x8[block8].bipred;

                currSlice->set_modes_and_reframe(currMB, block8, &p_dir, list_mode,
                                                 list_ref_idx);
                chroma_prediction_4x4(currMB, uv, block_x, block_y, p_dir,
                                      list_mode[0], list_mode[1], list_ref_idx[0],
                                      list_ref_idx[1], bipred_me);
            }
        }
    }
}

static void eva_glue_copy_chroma_pred(Macroblock *currMB, int uv)
{
    Slice *currSlice = currMB->p_Slice;
    VideoParameters *p_Vid = currMB->p_Vid;
    imgpel **dst = &p_Vid->enc_picture->imgUV[uv][currMB->pix_c_y];
    imgpel **src = currSlice->mb_pred[uv + 1];
    int y, x;

    for (y = 0; y < p_Vid->mb_cr_size_y; ++y)
        for (x = 0; x < p_Vid->mb_cr_size_x; ++x)
            dst[y][currMB->pix_c_x + x] = src[y][x];
}

/* Chroma 4:2:0: 2x2 DC Hadamard plus 4x4 AC, then add to the chroma prediction. */
static void eva_glue_recon_chroma_420(Macroblock *currMB)
{
    Slice *currSlice = currMB->p_Slice;
    VideoParameters *p_Vid = currMB->p_Vid;
    QuantParameters *p_Quant = p_Vid->p_Quant;
    int intra = is_intra(currMB) ? 1 : 0;
    int uv;
    int chroma_cbp = currMB->cbp >> 4;

    if (!intra && chroma_cbp == 0) {
        eva_glue_copy_chroma_pred(currMB, 0);
        eva_glue_copy_chroma_pred(currMB, 1);
        return;
    }

    for (uv = 0; uv < 2; ++uv) {
        int cur_qp = currMB->qp_scaled[uv + 1];
        int qp_per = p_Quant->qp_per_matrix[cur_qp];
        LevelQuantParams **q = p_Quant->q_params_4x4[uv + 1][intra][cur_qp];
        int **mb_rres = currSlice->mb_rres[uv + 1];
        imgpel **mb_pred = currSlice->mb_pred[uv + 1];
        int *DCLevel = currSlice->cofDC[uv + 1][0];
        int *DCRun = currSlice->cofDC[uv + 1][1];
        int m1[4];
        int pos, k, b4, y;
        int inv_dc = q[0][0].InvScaleComp;

        for (y = 0; y < 8; ++y)
            memset(mb_rres[y], 0, 8 * sizeof(int));

        memset(m1, 0, sizeof(m1));
        pos = 0;
        k = 0;
        while (DCLevel[k] != 0 && k < 4) {
            pos += DCRun[k];
            if (pos >= 0 && pos < 4)
                m1[pos] = (DCLevel[k] * inv_dc) << qp_per;
            ++pos;
            ++k;
        }
        ihadamard2x2(m1, m1);
        mb_rres[0][0] = m1[0] >> 5;
        mb_rres[0][4] = m1[1] >> 5;
        mb_rres[4][0] = m1[2] >> 5;
        mb_rres[4][4] = m1[3] >> 5;

        for (b4 = 0; b4 < 4; ++b4) {
            int n1 = (b4 & 1) ? 4 : 0;
            int n2 = (b4 & 2) ? 4 : 0;
            if (intra || chroma_cbp == 2)
                eva_glue_place_4x4(currSlice->cofAC[4 + uv][b4][0],
                               currSlice->cofAC[4 + uv][b4][1],
                               mb_rres, n2, n1, q, qp_per, 1, 0);
            if (mb_rres[n2][n1] != 0 || eva_glue_nz4(mb_rres, n2, n1))
                inverse4x4(mb_rres, mb_rres, n2, n1);
        }

        sample_reconstruct(&p_Vid->enc_picture->imgUV[uv][currMB->pix_c_y],
                           mb_pred, mb_rres, 0, currMB->pix_c_x,
                           p_Vid->mb_cr_size_x, p_Vid->mb_cr_size_y,
                           p_Vid->max_pel_value_comp[uv + 1], DQ_BITS);
    }
}

/* Input frame is the target picture. A copied macroblock whose reconstructed
 * samples already equal that target keeps its syntax. Anything else is written
 * as I_PCM of the target, which a decoder reproduces exactly when deblocking is off.
 */
static int eva_glue_recon_matches_org(Macroblock *currMB)
{
    VideoParameters *p_Vid = currMB->p_Vid;
    imgpel **orgY = p_Vid->pImgOrg[PLANE_Y];
    imgpel **imgY = p_Vid->enc_picture->imgY;
    int y, x, uv;

    for (y = 0; y < 16; ++y)
        for (x = 0; x < 16; ++x)
            if (imgY[currMB->pix_y + y][currMB->pix_x + x] !=
                orgY[currMB->pix_y + y][currMB->pix_x + x])
                return 0;

    if (p_Vid->yuv_format == YUV400)
        return 1;

    for (uv = 0; uv < 2; ++uv) {
        imgpel **orgC = p_Vid->pImgOrg[uv + 1];
        imgpel **imgC = p_Vid->enc_picture->imgUV[uv];
        for (y = 0; y < p_Vid->mb_cr_size_y; ++y)
            for (x = 0; x < p_Vid->mb_cr_size_x; ++x)
                if (imgC[currMB->pix_c_y + y][currMB->pix_c_x + x] !=
                    orgC[currMB->pix_c_y + y][currMB->pix_c_x + x])
                    return 0;
    }
    return 1;
}

static void eva_glue_force_ipcm(Macroblock *currMB)
{
    VideoParameters *p_Vid = currMB->p_Vid;
    imgpel **orgY = p_Vid->pImgOrg[PLANE_Y];
    imgpel **imgY = p_Vid->enc_picture->imgY;
    int y, x, uv;

    for (y = 0; y < 16; ++y)
        for (x = 0; x < 16; ++x)
            imgY[currMB->pix_y + y][currMB->pix_x + x] =
                orgY[currMB->pix_y + y][currMB->pix_x + x];

    if (getenv("EVA_GLUE_RESLOG")) {
        /* Residual needed for lossless exactness: target − prediction already in mb_pred. */
        static FILE *reslog;
        imgpel **mb_pred = currMB->p_Slice->mb_pred[0];
        int nz = 0, sad = 0, maxa = 0;
        if (!reslog) {
            reslog = fopen(getenv("EVA_GLUE_RESLOG"), "w");
            if (reslog)
                fprintf(reslog, "frame mx my nz_y sad_y max_y nz_uv sad_uv bytes_raw\n");
        }
        if (reslog) {
            int nz_uv = 0, sad_uv = 0;
            for (y = 0; y < 16; ++y)
                for (x = 0; x < 16; ++x) {
                    int r = (int)orgY[currMB->pix_y + y][currMB->pix_x + x] - (int)mb_pred[y][x];
                    if (r) {
                        ++nz;
                        sad += r < 0 ? -r : r;
                        if (r < 0)
                            r = -r;
                        if (r > maxa)
                            maxa = r;
                    }
                }
            if (p_Vid->yuv_format != YUV400) {
                for (uv = 0; uv < 2; ++uv) {
                    imgpel **orgC = p_Vid->pImgOrg[uv + 1];
                    imgpel **predC = currMB->p_Slice->mb_pred[uv + 1];
                    for (y = 0; y < p_Vid->mb_cr_size_y; ++y)
                        for (x = 0; x < p_Vid->mb_cr_size_x; ++x) {
                            int r = (int)orgC[currMB->pix_c_y + y][currMB->pix_c_x + x] -
                                    (int)predC[y][x];
                            if (r) {
                                ++nz_uv;
                                sad_uv += r < 0 ? -r : r;
                            }
                        }
                }
            }
            fprintf(reslog, "%d %d %d %d %d %d %d %d %d\n",
                    p_Vid->frame_no, currMB->mb_x, currMB->mb_y, nz, sad, maxa, nz_uv,
                    sad_uv, 256 + 2 * p_Vid->mb_cr_size_y * p_Vid->mb_cr_size_x);
        }
    }

    if (p_Vid->yuv_format != YUV400) {
        for (uv = 0; uv < 2; ++uv) {
            imgpel **orgC = p_Vid->pImgOrg[uv + 1];
            imgpel **imgC = p_Vid->enc_picture->imgUV[uv];
            for (y = 0; y < p_Vid->mb_cr_size_y; ++y)
                for (x = 0; x < p_Vid->mb_cr_size_x; ++x)
                    imgC[currMB->pix_c_y + y][currMB->pix_c_x + x] =
                        orgC[currMB->pix_c_y + y][currMB->pix_c_x + x];
        }
    }

    currMB->mb_type = IPCM;
    currMB->best_mode = IPCM;
    currMB->cbp = 0;
    currMB->cbp_blk = 0;
    /* A decoder never reads this flag for I_PCM and treats the neighbour as 0.
     * Leaving the injected I8 value makes the next block's transform-size context disagree. */
    currMB->luma_transform_size_8x8_flag = FALSE;
    {
        int i;
        for (i = 0; i < 4; ++i) {
            currMB->b8x8[i].mode = IPCM;
            currMB->b8x8[i].pdir = -1;
        }
    }
    eva_glue_mark_ipred_dc(currMB);
    {
        int j, i;
        PicMotionParams **motion = p_Vid->enc_picture->mv_info;
        for (j = 0; j < 4; ++j)
            for (i = 0; i < 4; ++i) {
                PicMotionParams *pm = &motion[currMB->block_y + j][currMB->block_x + i];
                pm->mv[LIST_0].mv_x = 0;
                pm->mv[LIST_0].mv_y = 0;
                pm->mv[LIST_1].mv_x = 0;
                pm->mv[LIST_1].mv_y = 0;
                pm->ref_idx[LIST_0] = -1;
                pm->ref_idx[LIST_1] = -1;
                pm->ref_pic[LIST_0] = NULL;
                pm->ref_pic[LIST_1] = NULL;
            }
        for (j = 0; j < 4; ++j)
            for (i = 0; i < (4 + p_Vid->num_blk8x8_uv); ++i)
                p_Vid->nz_coeff[currMB->mbAddrX][j][i] = 16;
    }
    /* A raw block has no motion-vector difference. The next block's arithmetic
     * coder uses the neighbour difference as context; the decoder stores 0. */
    memset(currMB->mvd, 0, sizeof(currMB->mvd));
    currMB->prev_dqp = 0;
    currMB->skip_flag = 0;
    currMB->is_intra_block = TRUE;
    ++glue_ipcm_count;
    if (glue_ipcm_log)
        fprintf(glue_ipcm_log, "%d %d %d\n", p_Vid->frame_no, currMB->mb_x, currMB->mb_y);
    if (p_Vid->frame_no >= 0 && p_Vid->frame_no < GLUE_IPCM_MAX_FRAMES) {
        ++glue_ipcm_frame[p_Vid->frame_no];
        if (p_Vid->frame_no + 1 > glue_ipcm_frames_seen)
            glue_ipcm_frames_seen = p_Vid->frame_no + 1;
    }
}

/* Motion vector the decoder will use: predicted vector plus the stored difference.
 * Neighbours are whatever earlier blocks left in mv_info, including I_PCM as intra. */
static void eva_glue_set_part_mv(Macroblock *currMB, int px, int py, int sx, int sy)
{
    VideoParameters *p_Vid = currMB->p_Vid;
    Slice *currSlice = currMB->p_Slice;
    PicMotionParams **motion = p_Vid->enc_picture->mv_info;
    PixelPos neigh[4];
    MotionVector pred;
    int bx0 = px >> 2;
    int by0 = py >> 2;
    int nx = sx >> 2;
    int ny = sy >> 2;
    int x, y;
    int ref = motion[currMB->block_y + by0][currMB->block_x + bx0].ref_idx[LIST_0];
    short mvx, mvy;
    StorablePicture *refpic = NULL;

    if (ref < 0)
        ref = 0;
    get_neighbors(currMB, neigh, px, py, sx);
    currMB->GetMVPredictor(currMB, neigh, &pred, (short)ref, motion, LIST_0, px, py, sx, sy);
    mvx = (short)(pred.mv_x + currMB->mvd[0][by0][bx0][0]);
    mvy = (short)(pred.mv_y + currMB->mvd[0][by0][bx0][1]);
    if (ref < currSlice->listXsize[LIST_0])
        refpic = currSlice->listX[currMB->list_offset + LIST_0][ref];
    for (y = 0; y < ny; ++y) {
        for (x = 0; x < nx; ++x) {
            PicMotionParams *pm = &motion[currMB->block_y + by0 + y][currMB->block_x + bx0 + x];
            pm->mv[LIST_0].mv_x = mvx;
            pm->mv[LIST_0].mv_y = mvy;
            pm->ref_idx[LIST_0] = (char)ref;
            pm->ref_pic[LIST_0] = refpic;
        }
    }
}

static void eva_glue_mc_block(Macroblock *currMB, int b8, int px, int py, int sx, int sy)
{
    Slice *currSlice = currMB->p_Slice;
    VideoParameters *p_Vid = currMB->p_Vid;
    DecodedPictureBuffer *p_Dpb = p_Vid->p_Dpb_layer[p_Vid->dpb_layer_id];
    int list_mode[2];
    char list_ref_idx[2];
    short p_dir = 0;

    currSlice->set_modes_and_reframe(currMB, b8, &p_dir, list_mode, list_ref_idx);
    p_Dpb->pf_luma_prediction(currMB, px, py, sx, sy, p_dir, list_mode, list_ref_idx,
                              currMB->b8x8[b8].bipred);
}

/* Inter luma prediction from the reference the decoder has, using the motion
 * vector the decoder derives (predictor plus the stored difference). P-skip
 * keeps the vector FindSkipModeMotionVector already stored. */
static void eva_glue_predict_inter_luma(Macroblock *currMB)
{
    int b8;

    if (currMB->mb_type == P8x8) {
        for (b8 = 0; b8 < 4; ++b8) {
            int mode = currMB->b8x8[b8].mode;
            int px = (b8 & 1) << 3;
            int py = (b8 >> 1) << 3;
            int sh, sv, x, y;
            if (mode < 4 || mode > 7)
                mode = 4;
            sh = part_size[mode][0] << 2;
            sv = part_size[mode][1] << 2;
            for (y = py; y < py + 8; y += sv)
                for (x = px; x < px + 8; x += sh)
                    eva_glue_set_part_mv(currMB, x, y, sh, sv);
        }
    } else if (currMB->mb_type != PSKIP) {
        int sh = block_size[currMB->mb_type][0];
        int sv = block_size[currMB->mb_type][1];
        int x, y;
        for (y = 0; y < 16; y += sv)
            for (x = 0; x < 16; x += sh)
                eva_glue_set_part_mv(currMB, x, y, sh, sv);
    }

    eva_glue_seed_all_mv(currMB);

    if (currMB->mb_type == PSKIP || currMB->mb_type == P16x16) {
        eva_glue_mc_block(currMB, 0, 0, 0, 16, 16);
        return;
    }
    if (currMB->mb_type == P16x8) {
        eva_glue_mc_block(currMB, 0, 0, 0, 16, 8);
        eva_glue_mc_block(currMB, 2, 0, 8, 16, 8);
        return;
    }
    if (currMB->mb_type == P8x16) {
        eva_glue_mc_block(currMB, 0, 0, 0, 8, 16);
        eva_glue_mc_block(currMB, 1, 8, 0, 8, 16);
        return;
    }
    for (b8 = 0; b8 < 4; ++b8) {
        int mode = currMB->b8x8[b8].mode;
        int px = (b8 & 1) << 3;
        int py = (b8 >> 1) << 3;
        int sh, sv, x, y;
        if (mode < 4 || mode > 7)
            mode = 4;
        sh = part_size[mode][0] << 2;
        sv = part_size[mode][1] << 2;
        for (y = py; y < py + 8; y += sv)
            for (x = px; x < px + 8; x += sh)
                eva_glue_mc_block(currMB, b8, x, y, sh, sv);
    }
}

static void eva_glue_reconstruct(Macroblock *currMB)
{
    VideoParameters *p_Vid = currMB->p_Vid;
    int intra = is_intra(currMB) ? 1 : 0;
    int qp = currMB->qp_scaled[PLANE_Y];

    if (currMB->mb_type == IPCM)
        return;

    if (currMB->mb_type <= P8x8)
        eva_glue_predict_inter_luma(currMB);

    if (currMB->mb_type == I16MB)
        eva_glue_recon_luma_16x16(currMB, qp);
    else if (currMB->luma_transform_size_8x8_flag) {
        eva_glue_recon_luma_8x8(currMB, qp, intra);
        if (currMB->mb_type != I8MB)
            eva_glue_mark_ipred_dc(currMB);
    } else {
        eva_glue_recon_luma_4x4(currMB, qp, intra);
        if (currMB->mb_type != I4MB)
            eva_glue_mark_ipred_dc(currMB);
    }

    eva_glue_predict_chroma(currMB);
    if (p_Vid->yuv_format == YUV420 && glue_have_chroma)
        eva_glue_recon_chroma_420(currMB);
    else if (p_Vid->yuv_format != YUV400) {
        eva_glue_copy_chroma_pred(currMB, 0);
        eva_glue_copy_chroma_pred(currMB, 1);
    }

    if (glue_exact && !eva_glue_recon_matches_org(currMB))
        eva_glue_force_ipcm(currMB);
}

void eva_glue_macroblock(Macroblock *currMB)
{
    Slice *currSlice = currMB->p_Slice;
    unsigned char pred256[EVA_PRED_BYTES];
    unsigned char coeff256[EVA_COEFF_BYTES];
    unsigned char type6[EVA_TYPE_BYTES];
    unsigned char mv96[EVA_MV_BYTES];
    unsigned char modes16[EVA_MODES_BYTES];
    unsigned char b8x8_8[EVA_B8X8_BYTES];
    unsigned char chroma[EVA_CHROMA_BYTES];
    unsigned char mvd64[EVA_MVD_BYTES];
    unsigned char luma_cof[EVA_LUMA_COF_BYTES];

    if (!fp_pred)
        return;

    if (!eva_glue_read_mb(currMB, pred256, coeff256, type6, mv96, modes16, b8x8_8,
                          chroma, mvd64, luma_cof)) {
        fprintf(stderr, "eva_glue: read failed frame %d mb %d\n",
                currMB->p_Vid->frame_no, currMB->mbAddrX);
        exit(1);
    }

    eva_glue_clear_cof_mb(currSlice);

    currMB->mb_type = type6[1];
    currMB->luma_transform_size_8x8_flag = type6[2];
    currMB->qp = type6[4];
    currMB->qpc[0] = type6[5];
    {
        int prev_qp = eva_glue_prev_qp(currMB);
        if (prev_qp < 0) {
            currMB->prev_qp = currMB->qp;
            currMB->prev_dqp = 0;
        } else {
            currMB->prev_qp = (short)prev_qp;
            currMB->prev_dqp = (short)(currMB->qp - prev_qp);
        }
    }
    currMB->best_mode = currMB->mb_type;
    currMB->c_ipred_mode = DC_PRED_8;
    currMB->prev_recode_mb = FALSE;

    update_qp(currMB);
    eva_glue_load_pred(currMB, currSlice, pred256);
    if (glue_have_luma_cof)
        eva_glue_apply_luma_cof(currMB, currSlice, luma_cof);
    else
        eva_glue_update_cbp(currMB, currSlice, coeff256, currMB->mb_type);
    eva_glue_apply_chroma(currMB, currSlice, chroma);
    if (is_intra(currMB))
        eva_glue_set_intra_state(currMB);
    else if (currSlice->slice_type == P_SLICE || currSlice->slice_type == SP_SLICE) {
        eva_glue_set_p_state(currMB, b8x8_8);
    }
    /* Keep transform flag from dump (I8 already forced TRUE in set_intra). */
    if (currMB->mb_type == I8MB)
        currMB->luma_transform_size_8x8_flag = TRUE;
    else if (currMB->mb_type != I4MB)
        currMB->luma_transform_size_8x8_flag = type6[2];
    eva_glue_apply_intra_modes(currMB, type6, modes16);

    if (currSlice->slice_type != I_SLICE && currSlice->slice_type != SI_SLICE) {
        if (currMB->mb_type == 0) {
            PicMotionParams **motion = currMB->p_Vid->enc_picture->mv_info;
            int bx, by;
            FindSkipModeMotionVector(currMB);
            for (by = 0; by < 4; ++by) {
                for (bx = 0; bx < 4; ++bx) {
                    PicMotionParams *pm =
                        &motion[currMB->block_y + by][currMB->block_x + bx];
                    pm->ref_idx[0] = 0;
                    pm->ref_idx[1] = -1;
                    if (currSlice->listXsize[LIST_0] > 0)
                        pm->ref_pic[0] =
                            currSlice->listX[currMB->list_offset + LIST_0][0];
                    else
                        pm->ref_pic[0] = NULL;
                    pm->ref_pic[1] = NULL;
                }
            }
            SetMotionVectorsMBPSlice(currMB);
        } else {
            eva_glue_load_mv(currMB, mv96);
        }
        eva_glue_load_mvd(currMB, mvd64);
    }

    update_qp_cbp(currMB);
    eva_glue_reconstruct(currMB);
    if (currMB->mbAddrX + 1 == currMB->p_Vid->PicSizeInMbs) {
        const char *pre = getenv("EVA_GLUE_PREDEC");
        if (pre && pre[0]) {
            VideoParameters *p_Vid = currMB->p_Vid;
            FILE *fp = fopen(pre, "ab");
            int y, x;
            if (fp) {
                for (y = 0; y < p_Vid->height; ++y)
                    for (x = 0; x < p_Vid->width; ++x) {
                        unsigned char s = (unsigned char)p_Vid->enc_picture->imgY[y][x];
                        fwrite(&s, 1, 1, fp);
                    }
                if (p_Vid->yuv_format != YUV400) {
                    int uv, ch = p_Vid->height >> 1, cw = p_Vid->width >> 1;
                    for (uv = 0; uv < 2; ++uv)
                        for (y = 0; y < ch; ++y)
                            for (x = 0; x < cw; ++x) {
                                unsigned char s = (unsigned char)p_Vid->enc_picture->imgUV[uv][y][x];
                                fwrite(&s, 1, 1, fp);
                            }
                }
                fclose(fp);
            }
        }
    }
}
