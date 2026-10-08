/* SPDX-License-Identifier: GPL-2.0-or-later */
/* SPRUH96B chapters 6/7: functional PLLC readback and SYSCLK dividers.
 * GO completes atomically; analog lock delay and phase alignment are not
 * synthesized. Divider programming stays separate from the active ratios. */
#include <string.h>
#include "da810_pll.h"

static const unsigned div_off[7] = {0x118, 0x11c, 0x120, 0x160, 0x164, 0x168, 0x16c};

void da810_pll_init(Da810Pll *p, uint64_t oscillator_hz)
{
    static const uint32_t reset_div[7] = {0x8000, 0x8001, 0x8002, 0x8003, 0x8002, 0x8000, 0x8005};
    memset(p, 0, sizeof *p);
    p->oscillator_hz = oscillator_hz;
    p->reg[0] = 0x44813c00;
    p->reg[0xe4/4] = 1; /* power-on reset */
    p->reg[0x100/4] = 0x72;
    p->reg[0x104/4] = 0x14;
    p->reg[0x110/4] = 0x13;
    p->reg[0x114/4] = p->reg[0x124/4] = 0x8000;
    p->reg[0x128/4] = 0x8001;
    p->reg[0x13c/4] = 4; /* oscillator stable when the device leaves reset */
    p->reg[0x140/4] = 0x7f;
    p->reg[0x148/4] = 3;
    for (unsigned i=0;i<7;i++) p->active_div[i] = p->reg[div_off[i]/4] = reset_div[i];
}

uint32_t da810_pll_read(Da810Pll *p, uint32_t off, unsigned size)
{
    unsigned a = off & ~3u;
    uint32_t v = 0;
    if (a == 0x14c) v = p->reg[0x148/4] & (p->reg[0x124/4] & 0x8000 ? 3 : 1);
    else if (a == 0x150) {
        for (unsigned i=0;i<7;i++) if (p->active_div[i] & 0x8000) v |= 1u << i;
    } else if (a < sizeof p->reg) v = p->reg[a/4];
    v >>= (off & 3) * 8;
    return size >= 4 ? v : v & ((1u << (size * 8)) - 1);
}

void da810_pll_write(Da810Pll *p, uint32_t off, uint32_t val, unsigned size)
{
    unsigned a = off & ~3u, shift = (off & 3) * 8;
    if (a >= sizeof p->reg) return;
    uint32_t lane = (size >= 4 ? UINT32_MAX : (1u << (size * 8)) - 1) << shift;
    uint32_t mask = 0;
    if (a == 0x100) mask = 0x12b;
    else if (a == 0x104 || a == 0x110) mask = 0x1f;
    else if (a == 0x114 || a == 0x124 || a == 0x128) mask = 0x801f;
    else if (a == 0x138) mask = 1;
    else if (a == 0x140) mask = 0x7f;
    else if (a == 0x148) mask = 3;
    for (unsigned i=0;i<7;i++) if (a == div_off[i]) mask = 0x801f;
    mask &= lane;
    uint32_t previous = p->reg[a/4];
    p->reg[a/4] = (previous & ~mask) | ((val << shift) & mask);
    for (unsigned i=0;i<7;i++) {
        if (a == div_off[i] && p->reg[a/4] != previous) p->reg[0x144/4] |= 1u << i;
    }
    if (a == 0x138 && (p->reg[a/4] & 1)) {
        for (unsigned i=0;i<7;i++) p->active_div[i] = p->reg[div_off[i]/4];
        p->reg[0x144/4] = 0;
    }
}

uint64_t da810_pll_auxclk(const Da810Pll *p)
{
    /* AUXCLK comes directly from the reference oscillator, independently
     * of the multiplier, bypass and SYSCLK dividers (SPRUH96B chapter 7). */
    return p->reg[0x148/4] & 1 ? p->oscillator_hz : 0;
}

uint64_t da810_pll_sysclk(const Da810Pll *p, unsigned clock)
{
    if (!clock || clock > 7) return 0;
    uint32_t d = p->active_div[clock-1], ctl = p->reg[0x100/4];
    if (!(d & 0x8000)) return 0;
    uint64_t num = p->oscillator_hz, den = (d & 31) + 1;
    if ((ctl & 1) && !(ctl & 0x22) && (ctl & 8)) {
        num *= (p->reg[0x110/4] & 31) + 1;
        uint32_t pre = p->reg[0x114/4], post = p->reg[0x128/4];
        if (pre & 0x8000) den *= (pre & 31) + 1;
        if (post & 0x8000) den *= (post & 31) + 1;
    }
    return num / den;
}
