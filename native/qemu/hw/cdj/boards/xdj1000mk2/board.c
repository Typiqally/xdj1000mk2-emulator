/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "../nxs2/cdj.h"
#include "usb_r8a66597.h"
#include "xdj.h"
#include "performance.h"
#include "cdj_pnl_link.h"
#include "hardware.h"
#include "sh7724_rwdt.h"
#include "sh7724_tpu.h"

/* MAIN IC6 SH7724, two 1-Gbit DDR2 chips and IC10 128-Mbit NOR.
 * Source: Pioneer XDJ-1000MK2 service schematic, MAIN 1/7 and 2/7.
 * Hardware shims belong here, never in the loaded firmware image.
 */
static const CdjBoardDesc xdj_board = {
    .name = "xdj1000mk2",
    .dram_phys = XDJ_MAIN_RAM_BASE,
    .dram_size = XDJ_MAIN_RAM_SIZE,
    .flash_phys = 0,
    .flash_size = XDJ_NOR_SIZE,
    .flash = { { XDJ_NOR_SECTORS, XDJ_NOR_SECTOR_SIZE } },
    .flash_id = { 0x00c2, 0x227e, 0x2221, 0x2201 },
    .fw_entry = 0xa8000800,
    .init_sp = 0xb8000000,
    .init_sr = 0x50000000,
    .periph_hz = XDJ_MAIN_PERIPH_HZ,
    .ccn_trace_lo = 0x800,
    .ccn_trace_hi = 0x860,
    .exit_report = cdj_intc_exit_report,
    .unsupported_access = xdj_hardware_gap,
};

static void xdj_regs(MemoryRegion *sysmem, const char *name, hwaddr addr,
                     hwaddr size, const CdjRegInit *init)
{
    cdj_regs(sysmem, name, addr, size, init);
    if (A7ADDR(addr) != addr) {
        MemoryRegionSection section = memory_region_find(sysmem, addr, size);
        MemoryRegion *alias = g_new0(MemoryRegion, 1);
        memory_region_init_alias(alias, NULL, name, section.mr,
                                 section.offset_within_region, size);
        memory_region_add_subregion(sysmem, A7ADDR(addr), alias);
        memory_region_unref(section.mr);
    }
}

static char *xdj_audio_state(Object *object, Error **errp)
{
    return xdj_da810_audio_state();
}

static void xdj_init(MachineState *machine)
{
    MemoryRegion *sysmem = get_system_memory();
    if (machine->ram_size != XDJ_MAIN_RAM_SIZE) {
        error_report("XDJ-1000MK2 has exactly 256 MiB main DDR2; RAM override rejected");
        exit(1);
    }
    if (strcmp(machine->cpu_type, TYPE_XDJ_SH7724_CPU)) {
        error_report("XDJ-1000MK2 requires the supported SH-4A ISA backend; CPU override rejected");
        exit(1);
    }
    xdj_hardware_init(sysmem, OBJECT(machine));
    SuperHCPU *cpu = cdj_board_init(machine, &xdj_board);
    CdjDmacDei dei[4];
    static const CdjRegInit cpg[] = { { 0x00, 0x00000000 }, { 0 } };
    int i;

    cdj_intc.mask_reads_inverted = true;
    /* Map fallback ranges first: the real four-byte INTC-B aliases must
     * win over the broad gap device, including IRQ5 priority/mask writes. */
    cdj_unimp("sh7724.intc-b", 0xa4140000, 0x1000);
    cdj_unimp("sh7724.intc-c", 0xa4090000, 0x1000);
    cdj_intc_init(sysmem, cpu);
    xdj_regs(sysmem, "sh7724.cpg", 0xa4150000, 0x1000, cpg);
    xdj_regs(sysmem, "sh7724.dbsc", 0xfd000000, 0x1000, NULL);
    xdj_regs(sysmem, "sh7724.bsc", 0xff800000, 0x1000, NULL);
    xdj_pfc_init(sysmem, xdj_da810_init(sysmem, 0x18000000));
    object_property_add_str(OBJECT(machine), "xdj-audio-state", xdj_audio_state, NULL);
    xdj_da810_performance_properties(OBJECT(machine));
    xdj_system_performance_properties(OBJECT(machine));
    xdj_rwdt_init();
    cdj_unimp("sh7724.cmt", 0xa44a0000, 0x1000);
    for (i = 0; i < 4; i++) {
        dei[i] = (CdjDmacDei){
            cdj_count_irq(cdj_intc.irqs[CDJ_DMAC0A_DEI0 + i], "DMAC0A")
        };
    }
    cdj_dmac_init(sysmem, "sh7724.dmac", 0xfe008000,
                  0x04d90000, CDJ_USB_SIZE, dei);
    xdj_da810_dma_init();
    cdj_tmu_init(sysmem, 0x1fd80000,
                 cdj_count_irq(cdj_intc.irqs[CDJ_TMU0_TUNI0], "TMU0 TUNI0"),
                 cdj_count_irq(cdj_intc.irqs[CDJ_TMU0_TUNI1], "TMU0 TUNI1"),
                 cdj_count_irq(cdj_intc.irqs[CDJ_TMU0_TUNI2], "TMU0 TUNI2"));
    cdj_tmu_init(sysmem, 0x1fd90000,
                 cdj_count_irq(cdj_intc.irqs[CDJ_TMU1_TUNI0], "TMU1 TUNI0"),
                 cdj_count_irq(cdj_intc.irqs[CDJ_TMU1_TUNI1], "TMU1 TUNI1"),
                 cdj_count_irq(cdj_intc.irqs[CDJ_TMU1_TUNI2], "TMU1 TUNI2"));
    for (i = 0; i < 2; i++) {
        g_autofree char *name = g_strdup_printf("xdj-scif%d", i);
        cdj_scif(sysmem, name, 0xffe00000 + i * 0x10000,
                 serial_hd(i) ?: qemu_chr_new(name, "null", NULL));
    }
    /* Stock MAIN 0949defe sends 0x20 bytes; the final two bytes are the
     * panel protocol checksum and sync. This models the MCU's serial peer.
     */
    xdj_panel_init(sysmem);
    xdj_panel_properties(OBJECT(machine));
    cdj_iic(sysmem, "xdj.iic0", 0xa4470000, 0, NULL);
    cdj_iic(sysmem, "xdj.iic1", 0xa4750000, 1, NULL);
    cdj_usb_init(sysmem, 0xa4d90000, cdj_intc.irqs[CDJ_USB1], false);
    cdj_usbf_init_at(sysmem, 0x04d80000, cdj_intc.irqs[CDJ_USB0]);
    cdj_ether_init_with_switch(sysmem, "sh7724.ether", 0x04600000,
                              cdj_intc.irqs[CDJ_ETHI], 0,
                              cdj_intc.irqs[CDJ_IRQ5]);
    /* TPU0 drives the LCD backlight PWM; no IRQ is enabled by MAIN. */
    xdj_tpu_init();
    xdj_lcdc_init(sysmem, cdj_count_irq(cdj_intc.irqs[CDJ_LCDCI], "LCDC"));
    xdj_m2dg_init(sysmem, cdj_intc.irqs[CDJ_M2DG_TRI],
                   cdj_intc.irqs[CDJ_M2DG_INI], cdj_intc.irqs[CDJ_M2DG_CEI]);
    cdj_unimp("sh7724.beu", 0xfe930000, 0x10000);
    cdj_unimp("sh7724.veu", 0xfe920000, 0x10000);
    cdj_irqcount_exit.notify = cdj_irqcount_dump;
    qemu_add_exit_notifier(&cdj_irqcount_exit);
    cdj_board_load(machine);
    cdj_board_start(cpu);
}

static void xdj_machine_init(MachineClass *mc)
{
    mc->desc = "Pioneer XDJ-1000MK2 (SH7724/DA810)";
    mc->init = xdj_init;
    mc->default_cpu_type = TYPE_XDJ_SH7724_CPU;
    mc->default_ram_size = XDJ_MAIN_RAM_SIZE;
}

DEFINE_MACHINE("xdj1000mk2", xdj_machine_init)
