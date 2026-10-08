/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "sh7724_tpu.h"
#include <limits.h>
#include <string.h>

static int decode(unsigned off)
{
    if (off < 0x10 || off >= 0x100) return -1;
    unsigned reg = (off - 0x10) % 0x40;
    return reg < 0x28 && !(reg & 3) ? (int)reg : -1;
}
void sh7724_tpu_init(Sh7724Tpu *s)
{
    memset(s, 0, sizeof(*s));
    for (unsigned c = 0; c < 4; c++)
        for (unsigned r = 0; r < 4; r++) s->channel[c].tgr[r] = UINT16_MAX;
}
bool sh7724_tpu_read(Sh7724Tpu *s, unsigned off, unsigned size, uint32_t *value)
{
    *value = 0;
    if (size != 2) return false;
    if (!off) { *value = s->start; return true; }
    int r = decode(off);
    if (r < 0) return false;
    Sh7724TpuChannel *c = &s->channel[(off - 0x10) / 0x40];
    switch (r) {
    case 0: *value = c->tcr; break;
    case 4: *value = c->tmdr; break;
    case 8: *value = c->tior; break;
    case 12: *value = c->tier; break;
    case 16: *value = c->tsr; c->seen_flags = c->tsr; break;
    case 20: *value = c->count; break;
    default: *value = c->tgr[(r - 24) / 4]; break;
    }
    return true;
}
bool sh7724_tpu_write(Sh7724Tpu *s, unsigned off, uint32_t value, unsigned size)
{
    if (size != 2) return false;
    if (!off) { s->start = value & 15; return true; }
    int r = decode(off);
    if (r < 0) return false;
    Sh7724TpuChannel *c = &s->channel[(off - 0x10) / 0x40];
    switch (r) {
    case 0:
        /* Divider phase after a change is not physically qualified. */
        if (c->tcr != (value & 255)) c->fraction = 0;
        c->tcr = value & 255; break;
    case 4: c->tmdr = value & 0x77; break;
    case 8: c->tior = value & 7; break;
    case 12: c->tier = value & 31; break;
    case 16:
        c->tsr &= ~(c->seen_flags & ~value);
        c->seen_flags &= c->tsr; break;
    case 20: c->count = value; break;
    default: c->tgr[(r - 24) / 4] = value; break;
    }
    return true;
}
static bool buffered(const Sh7724TpuChannel *c, unsigned reg)
{
    return reg >= 2 && (c->tmdr & (0x10u << (reg - 2)));
}
static int clear_register(const Sh7724TpuChannel *c)
{
    unsigned clear = c->tcr >> 5;
    return clear == 1 ? 0 : clear == 2 ? 1 : clear == 5 ? 2 : clear == 6 ? 3 : -1;
}
static bool valid_channel(const Sh7724TpuChannel *c)
{
    unsigned clear = c->tcr >> 5, mode = c->tmdr & 7;
    int r = clear_register(c);
    /* External clocks, prohibited settings and buffer-as-clear are not
     * approximated. Bphi/1 both edges performs no operation (table21.5). */
    return (c->tcr & 7) < 4 && !((c->tcr & 0x10) && !(c->tcr & 7)) &&
        clear != 3 && clear != 7 && (r < 0 || !buffered(c, r)) &&
        (mode == 0 || mode == 2);
}
bool sh7724_tpu_configured(const Sh7724Tpu *s)
{
    for (unsigned n = 0; n < 4; n++)
        if ((s->start & (1u << n)) && !valid_channel(&s->channel[n])) return false;
    return true;
}
bool sh7724_tpu_buffer_pending(const Sh7724Tpu *s)
{
    for (unsigned n = 0; n < 4; n++) {
        const Sh7724TpuChannel *c = &s->channel[n];
        if (!(s->start & (1u << n))) continue;
        for (unsigned i = 0; i < 2; i++)
            if ((c->tmdr & (0x10u << i)) && c->tgr[i] != c->tgr[i + 2]) return true;
    }
    return false;
}
/* Counter compare acts on the count before the increment/clear edge, so a
 * periodic counter visits 0..TGR inclusive: period = TGR+1 (figure21.13).
 * Flag timing at sub-Bphi granularity and pin output remain unqualified. */
static void flags_between(Sh7724TpuChannel *c, unsigned first, unsigned end)
{
    for (unsigned i = 0; i < 4; i++)
        if (!buffered(c, i) && c->tgr[i] >= first && c->tgr[i] < end)
            c->tsr |= 1u << i;
    if (end == 65536) c->tsr |= 16;
}
static uint32_t until_clear(const Sh7724TpuChannel *c)
{
    int r = clear_register(c);
    return r < 0 ? 65536u - c->count : (uint16_t)(c->tgr[r] - c->count) + 1u;
}
static void stable_ticks(Sh7724TpuChannel *c, uint64_t ticks)
{
    /* At most two initial segments and one complete-cycle aggregation;
     * runtime is independent of elapsed ticks/PWM frequency. */
    int r = clear_register(c);
    uint32_t period = r < 0 ? 65536u : (uint32_t)c->tgr[r] + 1;
    if (c->count >= period) {
        uint32_t distance = 65536u - c->count;
        uint32_t step = ticks < distance ? (uint32_t)ticks : distance;
        flags_between(c, c->count, c->count + step);
        c->count += step; ticks -= step;
        if (!ticks) return;
    }
    uint32_t distance = period - c->count;
    uint32_t step = ticks < distance ? (uint32_t)ticks : distance;
    flags_between(c, c->count, c->count + step);
    c->count = step == distance ? 0 : c->count + step;
    ticks -= step;
    if (ticks >= period) { flags_between(c, 0, period); ticks %= period; }
    flags_between(c, 0, (unsigned)ticks);
    c->count += ticks;
}
static uint64_t next_match(const Sh7724TpuChannel *c, unsigned reg)
{
    uint32_t distance = (uint16_t)(c->tgr[reg] - c->count) + 1u;
    int clear = clear_register(c);
    uint32_t boundary = until_clear(c);
    if (clear < 0 || distance <= boundary) return distance;
    if (c->tgr[reg] > c->tgr[clear]) return UINT64_MAX;
    return (uint64_t)boundary + c->tgr[reg] + 1;
}
static void ticks_with_buffers(Sh7724TpuChannel *c, uint64_t ticks)
{
    while (ticks) {
        uint64_t next = UINT64_MAX;
        uint32_t boundary = until_clear(c);
        for (unsigned i = 0; i < 2; i++) {
            if (!(c->tmdr & (0x10u << i)) || c->tgr[i] == c->tgr[i + 2]) continue;
            uint64_t event = next_match(c, i);
            if (c->tmdr & 0x40) {
                if (clear_register(c) >= 0) next = boundary;
            } else if (event < next) next = event;
        }
        if (next > ticks) { stable_ticks(c, ticks); return; }
        bool clearing = clear_register(c) >= 0 && next == boundary;
        bool match[2] = {
            next == next_match(c, 0),
            next == next_match(c, 1),
        };
        stable_ticks(c, next); ticks -= next;
        for (unsigned i = 0; i < 2; i++)
            if ((c->tmdr & (0x10u << i)) && ((c->tmdr & 0x40) ? clearing : match[i]))
                c->tgr[i] = c->tgr[i + 2];
    }
}
bool sh7724_tpu_advance(Sh7724Tpu *s, uint64_t ns, uint32_t num, uint32_t den)
{
    if (!num || !den) return false;
    bool supported = true;
    for (unsigned n = 0; n < 4; n++) {
        Sh7724TpuChannel *c = &s->channel[n];
        if (!(s->start & (1u << n))) continue;
        if (!valid_channel(c)) { supported = false; continue; }
        unsigned divider = 1u << (2 * (c->tcr & 3));
        unsigned edges = (c->tcr & 0x10) ? 2 : 1;
        uint64_t denominator = UINT64_C(1000000000) * den * divider;
        __uint128_t elapsed = (__uint128_t)ns * num * edges + c->fraction;
        c->fraction = elapsed % denominator;
        ticks_with_buffers(c, (uint64_t)(elapsed / denominator));
    }
    return supported;
}
