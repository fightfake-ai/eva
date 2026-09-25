#ifndef EVA_DUMP_H
#define EVA_DUMP_H

struct macroblock_enc;
struct video_par;

void eva_dump_open(const char *dir);
void eva_dump_close(void);
void eva_dump_macroblock(struct macroblock_enc *currMB);

#endif
