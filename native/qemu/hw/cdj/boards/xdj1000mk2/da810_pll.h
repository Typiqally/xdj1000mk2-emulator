/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef DA810_PLL_H
#define DA810_PLL_H
#include <stdint.h>

typedef struct Da810Pll {
    uint32_t reg[0x180 / 4];
    uint32_t active_div[7];
    uint64_t oscillator_hz;
} Da810Pll;

void da810_pll_init(Da810Pll *p, uint64_t oscillator_hz);
uint32_t da810_pll_read(Da810Pll *p, uint32_t off, unsigned size);
void da810_pll_write(Da810Pll *p, uint32_t off, uint32_t val, unsigned size);
uint64_t da810_pll_sysclk(const Da810Pll *p, unsigned clock);
uint64_t da810_pll_auxclk(const Da810Pll *p);
#endif
