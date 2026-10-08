/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Renesas R01UH0174JJ0200 19.3/19.4. The WRFLG synchronization interval
 * is unspecified in these public descriptions; counter writes remain gaps.
 * Clock rate is supplied as a rational, never rounded to an assumed RTC rate. */
#include "sh7724_rwdt.h"
#include <string.h>
#include <limits.h>
static const unsigned divider[8] = {1, 4, 16, 32, 64, 128, 1024, 4096};
void sh7724_rwdt_init(Sh7724Rwdt *s)
{
    memset(s, 0, sizeof *s);
    s->csr = 0x87;
}
bool sh7724_rwdt_read(Sh7724Rwdt *s, uint32_t off, unsigned size, uint32_t *value)
{
    *value = 0;
    if (size != 1 || (off != 0 && off != 4)) return false;
    *value = off == 0 ? s->count : s->csr;
    return true;
}
bool sh7724_rwdt_write(Sh7724Rwdt *s, uint32_t off, uint32_t value, unsigned size)
{
    if (off != 0 && off != 4) return false;
    /* Byte/longword writes and incorrect protection keys have no effect. */
    if (size != 2 || value >> 8 != (off == 0 ? 0x5a : 0xa5)) return true;
    if (off == 0) {
        s->count = value;
        /* Counter value is modeled; WRFLG handshake timing is not. */
        return false;
    }
    bool supported = !(s->csr & 0x80) || (s->csr & 7) == (value & 7);
    /* A period's remainder cannot be reused for a different divider. This
     * phase restart is a model convention; the board records a fidelity gap. */
    if ((s->csr & 7) != (value & 7)) s->fraction = 0;
    /* TME/CKS/WOVF writable, SRSTF clear-only, reserved/WRFLG read-only. */
    s->csr = (value & 0x97) | (s->csr & value & 8);
    return supported;
}
bool sh7724_rwdt_advance(Sh7724Rwdt *s, uint64_t ns, uint64_t clock_num, uint64_t clock_den)
{
    if (!(s->csr & 0x80) || s->pending_reset || !clock_num || !clock_den) return false;
    __uint128_t period = (__uint128_t)clock_den * divider[s->csr & 7] * 1000000000;
    __uint128_t total = (__uint128_t)ns * clock_num + s->fraction;
    __uint128_t ticks = total / period;
    s->fraction = total % period;
    if (ticks >= 256u - s->count) {
        s->count = 0;
        s->csr |= 0x10;
        s->pending_reset = true;
        return true;
    }
    s->count += (unsigned)ticks;
    return false;
}
int64_t sh7724_rwdt_until_overflow(const Sh7724Rwdt *s, uint64_t clock_num, uint64_t clock_den)
{
    if (!(s->csr & 0x80) || s->pending_reset || !clock_num || !clock_den) return -1;
    __uint128_t period = (__uint128_t)clock_den * divider[s->csr & 7] * 1000000000;
    __uint128_t remaining = period * (256u - s->count) - s->fraction;
    __uint128_t ns = (remaining + clock_num - 1) / clock_num;
    return ns > INT64_MAX ? INT64_MAX : (int64_t)ns;
}
