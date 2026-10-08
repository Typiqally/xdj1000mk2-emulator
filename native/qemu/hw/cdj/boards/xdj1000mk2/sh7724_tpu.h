/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef SH7724_TPU_H
#define SH7724_TPU_H
#include <stdbool.h>
#include <stdint.h>
/* Renesas SH7724 chapter 21. Pins/interrupt routing and CPG power states
 * remain wrapper-level gaps. All registers require aligned 16-bit access. */
typedef struct Sh7724TpuChannel {
    uint16_t tcr, tmdr, tior, tier, tsr, count, tgr[4];
    uint16_t seen_flags;
    uint64_t fraction;
} Sh7724TpuChannel;
typedef struct Sh7724Tpu {
    uint16_t start;
    Sh7724TpuChannel channel[4];
} Sh7724Tpu;
void sh7724_tpu_init(Sh7724Tpu *s);
bool sh7724_tpu_read(Sh7724Tpu *s, unsigned off, unsigned size, uint32_t *value);
bool sh7724_tpu_write(Sh7724Tpu *s, unsigned off, uint32_t value, unsigned size);
bool sh7724_tpu_advance(Sh7724Tpu *s, uint64_t ns, uint32_t clock_num, uint32_t clock_den);
bool sh7724_tpu_configured(const Sh7724Tpu *s);
bool sh7724_tpu_buffer_pending(const Sh7724Tpu *s);
void xdj_tpu_init(void);
#endif
