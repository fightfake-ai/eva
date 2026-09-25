#ifndef EVA_DECDUMP_H
#define EVA_DECDUMP_H

/* Per-macroblock syntax dumps written from the JM *decoder*.
 *
 * The publishing path (third_party/jm-eva/eva_glue.c) replays a camera's
 * per-macroblock syntax. Until now that syntax could only come from our patched
 * JM encoder, which limited the whole construction to clips we encoded
 * ourselves. This writes the same records while decoding, so any conforming
 * H.264 stream can feed the publishing path.
 *
 * Record layout is byte-compatible with eva_dump.c; see that file for the
 * field-by-field description.
 */

struct macroblock_dec;

int  eva_decdump_enabled(void);
void eva_decdump_open(const char *dir);
void eva_decdump_close(void);

/* Called once per macroblock, before any syntax element is parsed. */
void eva_decdump_begin_mb(struct macroblock_dec *currMB);
/* Called once per macroblock, after it is parsed and reconstructed. */
void eva_decdump_macroblock(struct macroblock_dec *currMB);

/* Parse-time capture. The decoder dequantises its coefficient array in place,
 * so the quantised levels have to be taken as they are read off the bitstream.
 * cofac_idx is the encoder's cofAC first index: 0-3 luma, 4 Cb, 5 Cr. */
void eva_decdump_ac(int cofac_idx, int b4, int level, int run);
void eva_decdump_dc(int plane, int level, int run);
/* Coded (not absolute) intra prediction mode, as the bitstream carries it. */
void eva_decdump_intra_mode(int blk, int coded);

#endif
