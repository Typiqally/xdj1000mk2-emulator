/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef DA810_MCASP_H
#define DA810_MCASP_H
#include <stdint.h>
typedef struct Da810Mcasp {
    uint32_t reg[0x400 / 4], tx[16];
    uint16_t valid;
    unsigned port, slot;
    uint64_t phase, words, underruns;
    void (*event)(void *, unsigned);
    void (*sample)(void *, unsigned, unsigned, int32_t);
    void *opaque;
} Da810Mcasp;
void da810_mcasp_init(Da810Mcasp *, unsigned, void (*)(void *, unsigned),
                      void (*)(void *, unsigned, unsigned, int32_t), void *);
uint32_t da810_mcasp_read(Da810Mcasp *, uint32_t, unsigned);
void da810_mcasp_write(Da810Mcasp *, uint32_t, uint32_t, unsigned);
void da810_mcasp_advance(Da810Mcasp *, uint64_t);
#endif
