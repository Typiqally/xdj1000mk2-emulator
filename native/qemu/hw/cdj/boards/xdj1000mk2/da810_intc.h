/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef DA810_INTC_H
#define DA810_INTC_H
#include <stdint.h>
typedef struct Da810Intc {
    uint32_t flag[4], mask[4], expmask[4], mux[3];
    unsigned combiner;
    void (*pulse)(void *opaque, unsigned line);
    void *opaque;
} Da810Intc;
void da810_intc_init(Da810Intc *s, void (*pulse)(void *, unsigned), void *opaque);
void da810_intc_event(Da810Intc *s, unsigned event);
uint32_t da810_intc_read(Da810Intc *s, unsigned off);
void da810_intc_write(Da810Intc *s, unsigned off, uint32_t value);
#endif
