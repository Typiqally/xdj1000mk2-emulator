/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef SH7724_RWDT_H
#define SH7724_RWDT_H
#include <stdbool.h>
#include <stdint.h>
typedef struct Sh7724Rwdt {
    uint8_t count, csr;
    __uint128_t fraction;
    bool pending_reset;
} Sh7724Rwdt;
void sh7724_rwdt_init(Sh7724Rwdt *s);
bool sh7724_rwdt_read(Sh7724Rwdt *s, uint32_t off, unsigned size, uint32_t *value);
/* false means an unsupported access/effect, even if bounded readback is stored. */
bool sh7724_rwdt_write(Sh7724Rwdt *s, uint32_t off, uint32_t value, unsigned size);
/* Return true only on the first overflow, for the board's reset controller. */
bool sh7724_rwdt_advance(Sh7724Rwdt *s, uint64_t ns, uint64_t clock_num, uint64_t clock_den);
int64_t sh7724_rwdt_until_overflow(const Sh7724Rwdt *s, uint64_t clock_num, uint64_t clock_den);
void xdj_rwdt_init(void);
#endif
