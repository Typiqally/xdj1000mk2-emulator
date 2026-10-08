/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef DA810_TIMER_H
#define DA810_TIMER_H
#include <stdbool.h>
#include <stdint.h>

typedef struct Da810Timer {
    uint32_t reg[0x80/4], shadow_high;
    uint64_t fraction;
    bool stopped[2];
    uint32_t unsupported;
    void (*event)(void *, unsigned half);
    void *opaque;
} Da810Timer;
void da810_timer_init(Da810Timer *t, void (*event)(void *, unsigned), void *opaque);
uint32_t da810_timer_read(Da810Timer *t, uint32_t off, unsigned size);
void da810_timer_write(Da810Timer *t, uint32_t off, uint32_t val, unsigned size);
void da810_timer_advance(Da810Timer *t, uint64_t ns, uint64_t auxclk_hz);
#endif
