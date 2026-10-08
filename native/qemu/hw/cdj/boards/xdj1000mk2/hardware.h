/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef XDJ_HARDWARE_H
#define XDJ_HARDWARE_H
#include "../nxs2/cdj.h"
#include "hardware_profile.h"
#define TYPE_XDJ_SH7724_CPU SUPERH_CPU_TYPE_NAME("sh7724")
void xdj_hardware_init(MemoryRegion *sysmem, Object *machine);
void xdj_hardware_gap(const char *name, hwaddr addr, uint64_t value,
                      unsigned size, bool write);
void xdj_hardware_dsp_gap(hwaddr addr, uint64_t value, unsigned size,
                          bool write, uint32_t pc);
void xdj_hardware_violation(const char *name, uint64_t value);
#endif
