#ifndef EVA_GLUE_H
#define EVA_GLUE_H

struct macroblock_enc;

int eva_glue_enabled(void);
int eva_glue_use_stored_mvd(void);
/* 1 = inject camera syntax for this MB; 0 = run normal encode_one_macroblock. */
int eva_glue_should_inject(struct macroblock_enc *currMB);
/* Slice header QP the capture used for this frame, or -1 when unknown. */
int eva_glue_slice_qp(int frame_no, int first_mb);
void eva_glue_open(const char *dir, int width, int height);
void eva_glue_close(void);
void eva_glue_macroblock(struct macroblock_enc *currMB);

#endif
