/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "dsp_memory.h"
#include "hardware_profile.h"
uint32_t xdj_dsp_ram_remaining(uint32_t addr)
{
    static const struct { uint32_t base, size; } region[] = {
        { XDJ_DSP_L2_BASE, XDJ_DSP_L2_SIZE },
        { XDJ_DSP_L2_ALIAS, XDJ_DSP_L2_SIZE },
        { XDJ_DSP_L1P_BASE, XDJ_DSP_L1P_SIZE },
        { XDJ_DSP_L1P_ALIAS, XDJ_DSP_L1P_SIZE },
        { XDJ_DSP_L1D_BASE, XDJ_DSP_L1D_SIZE },
        { XDJ_DSP_L1D_ALIAS, XDJ_DSP_L1D_SIZE },
        { XDJ_DSP_SHARED_BASE, XDJ_DSP_SHARED_SIZE },
        { XDJ_DSP_SDRAM_BASE, XDJ_DSP_SDRAM_SIZE },
    };
    for (unsigned i = 0; i < sizeof region / sizeof region[0]; i++) {
        uint32_t offset = addr - region[i].base;
        if (offset < region[i].size) return region[i].size - offset;
    }
    return 0;
}
static bool valid(const uint8_t *memory, uint32_t remaining, unsigned size)
{
    return memory && (size == 1 || size == 2 || size == 4) && size <= remaining;
}
bool xdj_dsp_ram_read(const uint8_t *memory, uint32_t remaining, unsigned size, uint32_t *value)
{
    *value = 0;
    if (!valid(memory, remaining, size)) return false;
    /* RAM reads may cross a host word, but never the installed DSP region. */
    for (unsigned i = 0; i < size; i++) *value |= (uint32_t)memory[i] << (i * 8);
    return true;
}
bool xdj_dsp_ram_write(uint8_t *memory, uint32_t remaining, unsigned size, uint32_t value)
{
    if (!valid(memory, remaining, size)) return false;
    for (unsigned i = 0; i < size; i++) memory[i] = value >> (i * 8);
    return true;
}
