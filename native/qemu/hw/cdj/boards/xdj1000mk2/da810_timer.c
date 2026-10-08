/* SPDX-License-Identifier: GPL-2.0-or-later */
/* SPRUH96B chapter 26: internal-clock GP64 and dual GP32 modes.
 * Watchdog, pin capture and compare outputs remain unsupported;
 * record their use rather than synthesizing those events as GP64 timeouts. */
#include <string.h>
#include "da810_timer.h"
#define REG(t,o) ((t)->reg[(o)/4])

void da810_timer_init(Da810Timer *t, void (*event)(void *, unsigned), void *opaque)
{
    memset(t, 0, sizeof *t);
    REG(t,0) = 0x4472020c;
    t->event = event;
    t->opaque = opaque;
}

uint32_t da810_timer_read(Da810Timer *t, uint32_t off, unsigned size)
{
    unsigned a = off & ~3u;
    if (a >= sizeof t->reg) return 0;
    unsigned mode = (REG(t,0x24) >> 2) & 3;
    uint32_t v = REG(t,a);
    if (!mode && a == 0x10) t->shadow_high = REG(t,0x14);
    if (!mode && a == 0x14) v = t->shadow_high;
    if (mode == 1 && (REG(t,0x24) & 0x10) &&
                     ((a == 0x10 && (REG(t,0x20) & 0x400)) ||
                      (a == 0x14 && (REG(t,0x20) & 0x04000000)))) {
        REG(t,a) = 0;
    }
    v >>= (off & 3) * 8;
    return size >= 4 ? v : v & ((1u << (size * 8)) - 1);
}

void da810_timer_write(Da810Timer *t, uint32_t off, uint32_t val, unsigned size)
{
    unsigned a = off & ~3u, shift = (off & 3) * 8;
    if (a >= sizeof t->reg || !a) return;
    uint32_t mask = (size >= 4 ? UINT32_MAX : (1u << (size * 8)) - 1) << shift;
    val <<= shift;
    if (a == 0x44) {
        uint32_t en = mask & 0x00050005;
        REG(t,a) = (REG(t,a) & ~en) | (val & en);
        REG(t,a) &= ~(val & mask & 0x000a000a); /* status: write one to clear */
        return;
    }
    if (a == 4) mask &= 3;
    else if (a == 8) mask &= 0x00030033;
    else if (a == 12) mask &= 0x00030003;
    else if (a == 0x20) mask &= 0x04c03ffe;
    else if (a == 0x24) mask &= 0xff1f;
    else if (a == 0x28) { t->unsupported |= 1u << 2; return; }
    else if (a >= 0x2c && a < 0x34) return;
    else if (a >= 0x48 && a < 0x60) return;
    else if (a >= 0x60) t->unsupported |= 1u << 4;
    uint32_t old = REG(t,a);
    REG(t,a) = (old & ~mask) | (val & mask);
    if (a == 0x20) {
        for (unsigned h=0;h<2;h++) {
            unsigned s = h ? 22 : 6;
            if (((old >> s) & 3) != ((REG(t,a) >> s) & 3)) t->stopped[h] = false;
        }
        if (REG(t,a) & 0x3b00) t->unsupported |= 1u << 3;
    }
    if (a == 0x10 || a == 0x14) t->stopped[(a - 0x10)/4] = false;
    if (a == 0x24) {
        /* Software reset holds the current count (26.1.7.1). Only a
         * hardware reset clears it; firmware can write TIMn explicitly. */
        unsigned mode = (REG(t,a) >> 2) & 3;
        if (mode == 2) t->unsupported |= 1u << mode;
    }
}

static void match(Da810Timer *t, unsigned h)
{
    unsigned shift = h * 16;
    REG(t,0x44) |= 2u << shift;
    if (!(REG(t,0x24) & 0x10) || (REG(t,0x44) & (1u << shift))) {
        if (t->event) t->event(t->opaque, h);
    }
}

static void advance_counter(Da810Timer *t, unsigned h, bool wide, uint64_t ticks)
{
    unsigned o = 0x10 + h * 4;
    uint64_t count = REG(t,o), period = REG(t,o+8);
    unsigned ena = (REG(t,0x20) >> (h ? 22 : 6)) & 3;
    if (!ena || t->stopped[h]) return;
    if (wide) {
        count |= (uint64_t)REG(t,0x14) << 32;
        period |= (uint64_t)REG(t,0x1c) << 32;
    }
    while (ticks && period) {
        if (count == period) {
            if (ena == 1) { t->stopped[h] = true; break; }
            /* Reset and reload on the clock AFTER the period match. */
            count = 0; ticks--;
            if (ena == 3 && (REG(t,0x24) & 0x10)) {
                REG(t,o+8) = REG(t,0x34+h*4);
                period = REG(t,o+8);
                if (wide) {
                    REG(t,0x1c) = REG(t,0x38);
                    period |= (uint64_t)REG(t,0x1c) << 32;
                }
            }
            continue;
        }
        uint64_t distance = wide ? period - count : (uint32_t)(period - count);
        if (ticks < distance) { count += ticks; ticks = 0; }
        else {
            ticks -= distance; count = period; match(t,h);
            if (ena == 1) t->stopped[h] = true, ticks = 0;
        }
    }
    REG(t,o) = count;
    if (wide) REG(t,0x14) = count >> 32;
}

static void advance_chained(Da810Timer *t, uint64_t ticks)
{
    if (!((REG(t,0x20) >> 6) & 3) || t->stopped[0]) return;
    while (ticks && REG(t,0x1c)) {
        uint32_t count = REG(t,0x14), period = REG(t,0x1c);
        if (count == period) {
            /* Only the prescaler's reset, one clock after its match,
             * clocks TIM12. TIM34 does not generate TINT34 in this mode. */
            REG(t,0x14) = 0; ticks--;
            bool reload = REG(t,0x10) == REG(t,0x18) &&
                          (REG(t,0x24) & 0x10) &&
                          ((REG(t,0x20) >> 6) & 3) == 3;
            advance_counter(t,0,false,1);
            if (reload) REG(t,0x1c) = REG(t,0x38);
            if (t->stopped[0]) break;
        } else {
            uint32_t distance = period - count;
            uint64_t step = ticks < distance ? ticks : distance;
            REG(t,0x14) += step; ticks -= step;
        }
    }
}

void da810_timer_advance(Da810Timer *t, uint64_t ns, uint64_t auxclk_hz)
{
    uint32_t tgcr = REG(t,0x24), tcr = REG(t,0x20);
    unsigned mode = (tgcr >> 2) & 3;
    if (!auxclk_hz || mode == 2) return;
    unsigned __int128 scaled = (unsigned __int128)ns * auxclk_hz + t->fraction;
    uint64_t ticks = scaled / 1000000000;
    t->fraction = scaled % 1000000000;
    if (!mode) {
        if ((tgcr & 3) == 3 && !(tcr & 0x300) &&
            !(REG(t,8) & 0x30000)) advance_counter(t,0,true,ticks);
    } else if (mode == 3) {
        if ((tgcr & 3) == 3 && !(tcr & 0x300) &&
            !(REG(t,8) & 0x30000)) advance_chained(t,ticks);
    } else {
        if ((tgcr & 1) && !(tcr & 0x300) &&
            !(REG(t,8) & 0x30000)) advance_counter(t,0,false,ticks);
        if ((tgcr & 2) && ((tcr >> 22) & 3) && !t->stopped[1]) {
            uint64_t prescaled = ticks + ((tgcr >> 8) & 15);
            unsigned divider = ((tgcr >> 12) & 15) + 1;
            REG(t,0x24) = (tgcr & ~0xf00u) | (prescaled % divider) << 8;
            advance_counter(t,1,false,prescaled / divider);
        }
    }
}
