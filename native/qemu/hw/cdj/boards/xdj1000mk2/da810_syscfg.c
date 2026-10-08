/* SPDX-License-Identifier: GPL-2.0-or-later */
/* TI SPRUH96B 10.2.1.2 and 10.5.5. Pinmux electrical effects and privilege
 * protection are unmodeled; writes to those registers stay coverage gaps. */
#include <string.h>
#include "da810_syscfg.h"
static bool pinmux(uint32_t off) { return off >= 0x120 && off <= 0x16c; }
void da810_syscfg_init(Da810Syscfg *s)
{
    memset(s, 0, sizeof *s);
    s->reg[0] = 0x4e840102;
}
bool da810_syscfg_read(Da810Syscfg *s, uint32_t off, unsigned size, uint32_t *value)
{
    *value = 0;
    if (size != 4 || (off & 3) || off >= sizeof s->reg) return false;
    if (off == 0 || off == 0x38 || off == 0x3c || pinmux(off)) {
        *value = s->reg[off/4];
        return true;
    }
    return false;
}
bool da810_syscfg_write(Da810Syscfg *s, uint32_t off, uint32_t value, unsigned size)
{
    if (size != 4 || (off & 3) || off >= sizeof s->reg) return false;
    if (off == 0) return true; /* REVID is read-only */
    if (off == 0x38) {
        s->reg[off/4] = value;
        s->kick0 = value == 0x83e70b13;
        if (!s->kick0) s->unlocked = false;
        return true;
    }
    if (off == 0x3c) {
        s->reg[off/4] = value;
        s->unlocked = s->kick0 && value == 0x95a4f1e0;
        if (value != 0x95a4f1e0) s->kick0 = false;
        return true;
    }
    if (pinmux(off) && s->unlocked) s->reg[off/4] = value;
    return false;
}
