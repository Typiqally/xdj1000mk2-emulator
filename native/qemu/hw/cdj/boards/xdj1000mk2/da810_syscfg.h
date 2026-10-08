/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef DA810_SYSCFG_H
#define DA810_SYSCFG_H
#include <stdbool.h>
#include <stdint.h>
typedef struct Da810Syscfg {
    uint32_t reg[0x190/4];
    bool kick0, unlocked;
} Da810Syscfg;
void da810_syscfg_init(Da810Syscfg *s);
/* false means behavior is incomplete, even if register readback is retained. */
bool da810_syscfg_read(Da810Syscfg *s, uint32_t off, unsigned size, uint32_t *value);
bool da810_syscfg_write(Da810Syscfg *s, uint32_t off, uint32_t value, unsigned size);
#endif
