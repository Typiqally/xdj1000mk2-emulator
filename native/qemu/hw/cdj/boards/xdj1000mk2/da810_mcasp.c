/* SPDX-License-Identifier: GPL-2.0-or-later */
/* TI SPRUH96B chapter 22: McASP reset controls, serializers and DMA events.
 * XDJ external audio clock is 22.5792 MHz (Pioneer MAIN 4/7 schematic).
 * Models stereo I2S/DIT word transfers; pin waveforms are not synthesized. */
#include <string.h>
#include "da810_mcasp.h"

void da810_mcasp_init(Da810Mcasp *s, unsigned port,
                      void (*event)(void *, unsigned),
                      void (*sample)(void *, unsigned, unsigned, int32_t), void *opaque)
{
    memset(s, 0, sizeof *s);
    s->port = port; s->event = event; s->sample = sample; s->opaque = opaque;
    s->reg[0x04/4] = 1; /* PWRDEMU.SOFT reset */
}

static uint16_t transmitters(Da810Mcasp *s)
{
    uint16_t mask = 0;
    unsigned n, count = s->port == 0 ? 16 : s->port == 1 ? 12 : 4;
    for (n = 0; n < count; n++) {
        if ((s->reg[(0x180+4*n)/4] & 3) == 1) { mask |= 1u << n; }
    }
    return mask;
}

static void request(Da810Mcasp *s)
{
    s->reg[0xc0/4] |= 0x20; /* XDATA: serializer needs another word */
    if (!(s->reg[0xcc/4] & 1)) { s->event(s->opaque, 2*s->port + 1); }
}

uint32_t da810_mcasp_read(Da810Mcasp *s, uint32_t off, unsigned size)
{
    uint32_t v = 0, a = off & ~3u;
    if (a == 0x60) { v = s->reg[0x44/4] & 0x1f; }
    else if (a == 0xa0) { v = s->reg[0x44/4] & 0x1f00; }
    else if (a < sizeof s->reg) { v = s->reg[a/4]; }
    v >>= 8 * (off & 3);
    return size >= 4 ? v : v & ((1u << (8*size))-1);
}

void da810_mcasp_write(Da810Mcasp *s, uint32_t off, uint32_t val, unsigned size)
{
    uint32_t a = off & ~3u, previous = s->reg[0x44/4];
    uint32_t mask = (size >= 4 ? UINT32_MAX : (1u << (size*8))-1) << ((off&3)*8);
    uint32_t v = (val << ((off&3)*8)) & mask;
    if (a == 0x44 || a == 0x60 || a == 0xa0) {
        uint32_t bits = a == 0x60 ? 0x1f : a == 0xa0 ? 0x1f00 : 0x1f1f;
        bits &= mask;
        s->reg[0x44/4] = (previous & ~bits) | (v & bits);
        if (!(s->reg[0x44/4] & 0x400)) {
            s->valid = 0; s->phase = 0; s->slot = 0; s->reg[0xc0/4] = 0;
        } else if (!(previous & 0x400) && transmitters(s)) {
            request(s);
        }
    } else if (a == 0x80 || a == 0xc0) {
        s->reg[a/4] &= ~v; /* status is write one to clear */
    } else if ((a >= 0x200 && a < 0x240) || (a >= 0x2000 && a < 0x4000)) {
        unsigned n = a >= 0x2000 ? 0 : (a - 0x200)/4;
        /* DMA port walks the enabled serializers. The deck uses one. */
        if (a >= 0x2000) {
            uint16_t active = transmitters(s);
            if (!active) { return; }
            n = __builtin_ctz(active);
        }
        s->tx[n] = val; s->valid |= 1u << n;
        if ((s->valid & transmitters(s)) == transmitters(s)) { s->reg[0xc0/4] &= ~0x20u; }
    } else if (a < sizeof s->reg) {
        s->reg[a/4] = (s->reg[a/4] & ~mask) | v;
    }
}

void da810_mcasp_advance(Da810Mcasp *s, uint64_t ns)
{
    unsigned slots = (s->reg[0xac/4] >> 7) & 0x1ff;
    unsigned bits = (((s->reg[0xa8/4] >> 4) & 15)+1)*2;
    uint64_t bitclock = 22579200;
    uint16_t active;
    if ((s->reg[0x44/4] & 0x1f00) != 0x1f00 || !(active = transmitters(s))) { return; }
    if (s->reg[0x50/4] & 1) { /* DIT is always two subframes at 44.1kHz */
        slots = 2; bits = 32; bitclock = 2822400;
    } else {
        bitclock /= (s->reg[0xb4/4] & 0xfff) + 1;
        bitclock /= (s->reg[0xb0/4] & 31) + 1;
    }
    if (!slots || slots > 32 || !bits || ns > 1000000000) { return; }
    s->phase += ns * (bitclock / bits);
    while (s->phase >= 1000000000) {
        unsigned n;
        s->phase -= 1000000000;
        if ((s->reg[0xb8/4] >> s->slot) & 1) {
            for (n = 0; n < 16; n++) {
                uint32_t value;
                if (!(active & (1u << n))) { continue; }
                value = s->valid & (1u << n) ? s->tx[n] : 0;
                if (!(s->valid & (1u << n))) { s->underruns++; s->reg[0xc0/4] |= 1; }
                value &= s->reg[0xa4/4];
                if (s->port == 1 && n == 0) { s->sample(s->opaque, s->port, s->slot, value); }
            }
            s->words++; s->valid &= ~active;
            request(s);
        }
        s->slot = (s->slot + 1) % slots;
        s->reg[0xc4/4] = s->slot;
    }
}
