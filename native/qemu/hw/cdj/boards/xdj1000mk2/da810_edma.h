/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef DA810_EDMA_H
#define DA810_EDMA_H
#include <stdint.h>
#include <stdbool.h>
#define EDMA_PARAM_BASE 0x4000
#define EDMA_PARAM_SETS 128
#define EDMA_PARAM_END 0x5000
/* DA810: fixed 32-channel mapping, eight QDMA channels, four shadow regions.
 * The caller owns bus access and routes completion (0..3) / error (4) IRQs. */
typedef struct Da810Edma {
    uint8_t cfg[0x1000], param[32 * EDMA_PARAM_SETS], level[32];
    uint64_t er, eer, ier, ipr, ser, cer, emr, null_transfers;
    uint8_t qer, qeer, qser, qemr, irq_level[5];
    unsigned depth;
    uint32_t (*read_mem)(void *, uint32_t, unsigned);
    void (*write_mem)(void *, uint32_t, uint32_t, unsigned);
    /* Optional: move one whole incrementing array at once. Returns false to
     * have it moved word by word (not plain memory, or the ranges overlap,
     * where a forward word copy repeats what it has just written). */
    bool (*copy_mem)(void *, uint32_t src, uint32_t dst, unsigned len);
    void (*irq)(void *, unsigned, int);
    void *opaque;
} Da810Edma;
void da810_edma_init(Da810Edma *, uint32_t (*)(void *, uint32_t, unsigned),
                     void (*)(void *, uint32_t, uint32_t, unsigned),
                     void (*)(void *, unsigned, int), void *);
void da810_edma_event(Da810Edma *, unsigned, int);
uint32_t da810_edma_read(Da810Edma *, uint32_t, unsigned);
void da810_edma_write(Da810Edma *, uint32_t, uint32_t, unsigned);
#endif
