/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef XDJ_DSP_MEMORY_H
#define XDJ_DSP_MEMORY_H
#include <stdbool.h>
#include <stdint.h>
uint32_t xdj_dsp_ram_remaining(uint32_t addr);
bool xdj_dsp_ram_read(const uint8_t *memory, uint32_t remaining, unsigned size, uint32_t *value);
bool xdj_dsp_ram_write(uint8_t *memory, uint32_t remaining, unsigned size, uint32_t value);
#endif
