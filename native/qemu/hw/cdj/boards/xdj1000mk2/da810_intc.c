/* SPDX-License-Identifier: GPL-2.0-or-later */
/* C674x megamodule event controller, TI SPRU871 and DA810 SPRUH96B
 * table 6-7: UHPI_DSPINT is event 34. The guest chooses the CPU line.
 */
#include "da810_intc.h"
#include <string.h>

static void route(Da810Intc *s, unsigned event)
{
    unsigned n;
    for (n = 0; n < 12; n++) {
        if (((s->mux[n / 4] >> (8 * (n % 4))) & 127) == event && s->pulse) {
            s->pulse(s->opaque, n + 4);
        }
    }
}

static void combiners(Da810Intc *s)
{
    unsigned n;
    for (n = 0; n < 4; n++) {
        unsigned level = (s->flag[n] & ~s->mask[n]) != 0;
        unsigned previous = (s->combiner >> n) & 1;
        if (level != previous) {
            s->combiner ^= 1u << n;
            if (level) {
                route(s, n);
            }
        }
    }
}

void da810_intc_init(Da810Intc *s, void (*pulse)(void *, unsigned), void *opaque)
{
    memset(s, 0, sizeof(*s));
    memset(s->mask, 0xff, sizeof(s->mask));
    memset(s->expmask, 0xff, sizeof(s->expmask));
    /* TI SPRUFK5A figures 7-35..37: reset maps event n to CPUINTn. */
    s->mux[0] = 0x07060504;
    s->mux[1] = 0x0b0a0908;
    s->mux[2] = 0x0f0e0d0c;
    s->pulse = pulse;
    s->opaque = opaque;
}

void da810_intc_event(Da810Intc *s, unsigned event)
{
    if (event < 4 || event >= 128) {
        return;
    }
    s->flag[event / 32] |= 1u << (event % 32);
    route(s, event);
    combiners(s);
}

uint32_t da810_intc_read(Da810Intc *s, unsigned off)
{
    if (off < 0x10) { return s->flag[off / 4]; }
    if (off >= 0x80 && off < 0x90) { return s->mask[(off - 0x80) / 4]; }
    if (off >= 0xa0 && off < 0xb0) {
        unsigned n = (off - 0xa0) / 4;
        return s->flag[n] & ~s->mask[n];
    }
    if (off >= 0xc0 && off < 0xd0) { return s->expmask[(off - 0xc0) / 4]; }
    if (off >= 0xe0 && off < 0xf0) {
        unsigned n = (off - 0xe0) / 4;
        return s->flag[n] & ~s->expmask[n];
    }
    if (off >= 0x104 && off <= 0x10c) { return s->mux[(off - 0x104) / 4]; }
    return 0;
}

void da810_intc_write(Da810Intc *s, unsigned off, uint32_t value)
{
    if (off >= 0x20 && off < 0x30) {
        unsigned bit;
        for (bit = 0; bit < 32; bit++) {
            if (value & (1u << bit)) { da810_intc_event(s, (off - 0x20) * 8 + bit); }
        }
    } else if (off >= 0x40 && off < 0x50) {
        s->flag[(off - 0x40) / 4] &= ~value;
        combiners(s);
    } else if (off >= 0x80 && off < 0x90) {
        s->mask[(off - 0x80) / 4] = value;
        combiners(s);
    } else if (off >= 0xc0 && off < 0xd0) {
        s->expmask[(off - 0xc0) / 4] = value;
    } else if (off >= 0x104 && off <= 0x10c) {
        s->mux[(off - 0x104) / 4] = value;
    }
}
