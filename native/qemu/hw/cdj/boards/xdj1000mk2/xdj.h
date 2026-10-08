/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef XDJ1000MK2_H
#define XDJ1000MK2_H
#include "../cdj2000/dsp_host.h"
const CdjDspWires *xdj_da810_init(MemoryRegion *sysmem, hwaddr base);
void xdj_da810_dma_init(void);
char *xdj_da810_audio_state(void);
void xdj_da810_performance_properties(Object *object);
void xdj_panel_init(MemoryRegion *sysmem);
void xdj_panel_properties(Object *machine);
void xdj_pfc_init(MemoryRegion *sysmem, const CdjDspWires *dsp);
/* The physical USB STOP switch on PTH4: pressed pulls the input low. */
void xdj_pfc_usb_stop(bool pressed);
void xdj_lcdc_init(MemoryRegion *sysmem, qemu_irq irq);
void xdj_m2dg_init(MemoryRegion *sysmem, qemu_irq trap, qemu_irq instruction, qemu_irq error);
#endif
