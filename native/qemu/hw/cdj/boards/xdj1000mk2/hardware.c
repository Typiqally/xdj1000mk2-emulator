/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "hardware.h"
#include "qemu/thread.h"
#include "qemu/module.h"
/* Same SH-4A ISA engine; identify the board's CPU rather than an SH7785.
 * Canonical identifiers follow Linux cpu_probe masks and documented cache
 * capacities. Exact silicon stepping/undocumented ID bits remain unverified. */
static void sh7724_cpu_init(Object *obj)
{
    SuperHCPU *cpu = SUPERH_CPU(obj);
    cpu->env.id = 1u << 6; /* new SH7724 identity, distinct from SH7785 bit5 */
    cpu->env.features = SH_FEATURE_SH4A;
}
static void sh7724_cpu_class_init(ObjectClass *oc, void *data)
{
    SuperHCPUClass *scc = SUPERH_CPU_CLASS(oc);
    scc->pvr = XDJ_MAIN_PVR;
    scc->prr = XDJ_MAIN_PRR;
    scc->cvr = XDJ_MAIN_CVR;
}
static const TypeInfo sh7724_cpu_type = {
    .name = TYPE_XDJ_SH7724_CPU,
    .parent = TYPE_SH7785_CPU,
    .instance_init = sh7724_cpu_init,
    .class_init = sh7724_cpu_class_init,
};
static void register_sh7724_cpu(void) { type_register_static(&sh7724_cpu_type); }
type_init(register_sh7724_cpu)
/* Always-on bounded evidence; neither the DSP lock nor the PCM lock is taken.
 * The DSP worker and main CPU can both report accesses. No guest behavior is
 * inferred from zeros returned by placeholders. Counters span the whole run. */
#define GAP_REGIONS 128
typedef struct GapRegion {
    const char *name;
    hwaddr base, first_addr;
    uint32_t first_pc;
    uint64_t reads, writes;
} GapRegion;
static struct {
    MemoryRegion fallback;
    MemoryRegion il_ram, rs_ram, media_ram;
    QemuMutex lock;
    uint64_t reads, writes, violations;
    const char *first_name;
    hwaddr first_addr;
    uint64_t first_value;
    uint32_t first_pc;
    unsigned first_size;
    bool first_write;
    GapRegion regions[GAP_REGIONS];
    unsigned region_count;
    uint64_t region_overflow_accesses;
} evidence;

static void gap(const char *name, hwaddr addr, uint64_t value, unsigned size,
                bool write, uint32_t pc)
{
    qemu_mutex_lock(&evidence.lock);
    if (!evidence.first_name) {
        evidence.first_name = name;
        evidence.first_addr = addr; evidence.first_value = value;
        evidence.first_size = size; evidence.first_write = write; evidence.first_pc = pc;
    }
    if (write) evidence.writes++; else evidence.reads++;
    hwaddr base = addr & ~(hwaddr)0xfff;
    unsigned index;
    for (index = 0; index < evidence.region_count; index++) {
        GapRegion *r = &evidence.regions[index];
        if (r->base == base && !strcmp(r->name, name)) break;
    }
    if (index == evidence.region_count && index < GAP_REGIONS) {
        GapRegion *r = &evidence.regions[evidence.region_count++];
        r->name = name; r->base = base;
        r->first_addr = addr; r->first_pc = pc;
    }
    if (index < GAP_REGIONS) {
        if (write) evidence.regions[index].writes++;
        else evidence.regions[index].reads++;
    } else {
        evidence.region_overflow_accesses++;
    }
    qemu_mutex_unlock(&evidence.lock);
}
void xdj_hardware_gap(const char *name, hwaddr addr, uint64_t value,
                      unsigned size, bool write)
{
    gap(name, addr, value, size, write,
        current_cpu ? SUPERH_CPU(current_cpu)->env.pc : 0);
}
void xdj_hardware_dsp_gap(hwaddr addr, uint64_t value, unsigned size,
                          bool write, uint32_t pc)
{
    gap("da810.unsupported", addr, value, size, write, pc);
}
void xdj_hardware_violation(const char *name, uint64_t value)
{
    bool first;
    qemu_mutex_lock(&evidence.lock);
    first = ++evidence.violations == 1;
    qemu_mutex_unlock(&evidence.lock);
    /* First violation suffices for failed qualification; logging is bounded. */
    if (first)
        info_report("xdj hardware profile violation: %s = %" PRIu64, name, value);
}
static MemTxResult fallback_read(void *opaque, hwaddr addr, uint64_t *value,
                                 unsigned size, MemTxAttrs attrs)
{
    xdj_hardware_gap("sh7724.unmapped", addr, 0, size, false);
    *value = 0;
    return MEMTX_DECODE_ERROR;
}
static MemTxResult fallback_write(void *opaque, hwaddr addr, uint64_t value,
                                  unsigned size, MemTxAttrs attrs)
{
    xdj_hardware_gap("sh7724.unmapped", addr, value, size, true);
    return MEMTX_DECODE_ERROR;
}
static const MemoryRegionOps fallback_ops = {
    .read_with_attrs = fallback_read, .write_with_attrs = fallback_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 8 },
};
static char *hardware_state(Object *obj, Error **errp)
{
    char *first, *result;
    qemu_mutex_lock(&evidence.lock);
    GString *regions = g_string_new("[");
    for (unsigned i = 0; i < evidence.region_count; i++) {
        GapRegion *r = &evidence.regions[i];
        g_string_append_printf(regions, "%s{\"device\":\"%s\",\"base\":%" PRIu64
            ",\"first_address\":%" PRIu64 ",\"first_pc\":%u,\"reads\":%" PRIu64
            ",\"writes\":%" PRIu64 "}", i ? "," : "", r->name,
            (uint64_t)r->base, (uint64_t)r->first_addr, r->first_pc, r->reads, r->writes);
    }
    g_string_append_c(regions, ']');
    first = evidence.first_name ? g_strdup_printf(
        "{\"device\":\"%s\",\"address\":%" PRIu64 ",\"pc\":%u,"
        "\"value\":%" PRIu64 ",\"size\":%u,\"write\":%s}",
        evidence.first_name, (uint64_t)evidence.first_addr, evidence.first_pc,
        evidence.first_value, evidence.first_size, evidence.first_write ? "true" : "false")
        : g_strdup("null");
    result = g_strdup_printf("{\"schema\":1,\"profile_sha256\":\"%s\","
        "\"target_cpu\":\"SH7724\",\"cpu_backend\":\"SH7724 descriptor/shared SH-4A\","
        "\"silicon_stepping\":\"unverified\","
        "\"main_core_hz\":%u,\"main_ram_bytes\":%u,\"nor_bytes\":%u,"
        "\"dsp_sdram_bytes\":%u,\"panel_cpu_executed\":false,"
        "\"main_il_ram_bytes\":%u,\"main_rs_ram_bytes\":%u,\"main_media_ram_bytes\":%u,"
        "\"main_internal_memory_protection_qualified\":false,"
        "\"hardware_reference\":\"unverified\",\"unsupported_reads\":%" PRIu64 ","
        "\"unsupported_writes\":%" PRIu64 ",\"profile_violations\":%" PRIu64 ","
        "\"coverage\":\"partial: main placeholders/unmapped bus, outer DSP MMIO/HPI\","
        "\"first_gap\":%s,\"gap_region_capacity\":%u,"
        "\"gap_region_overflow_accesses\":%" PRIu64 ",\"gap_regions\":%s}",
        XDJ_PROFILE_SHA256, XDJ_MAIN_CORE_HZ,
        XDJ_MAIN_RAM_SIZE, XDJ_NOR_SIZE, XDJ_DSP_SDRAM_SIZE,
        XDJ_MAIN_IL_RAM_SIZE, XDJ_MAIN_RS_RAM_SIZE, XDJ_MAIN_MERAM_RAM_SIZE,
        evidence.reads, evidence.writes, evidence.violations, first,
        GAP_REGIONS, evidence.region_overflow_accesses, regions->str);
    g_free(first);
    g_string_free(regions, true);
    qemu_mutex_unlock(&evidence.lock);
    return result;
}
void xdj_hardware_init(MemoryRegion *sysmem, Object *machine)
{
    qemu_mutex_init(&evidence.lock);
    /* A low-priority observer retains MEMTX_DECODE_ERROR for unmapped accesses.
     * All real devices take priority; holes never become successful fake RAM. */
    memory_region_init_io(&evidence.fallback, NULL, &fallback_ops, NULL,
                         "xdj.unsupported-bus", UINT64_C(0x100000000));
    memory_region_add_subregion_overlap(sysmem, 0, &evidence.fallback, -100);
    /* Renesas chapters 10/11/41: dedicated IL/RS RAM and MERAM direct data,
     * independent of L1/L2 and external DDR. No invented A7 aliases.
     * RAMCR protection/coherency and R-standby behavior remain unqualified. */
    memory_region_init_ram(&evidence.il_ram, NULL, "xdj.il-ram", XDJ_MAIN_IL_RAM_SIZE, &error_fatal);
    memory_region_add_subregion(sysmem, XDJ_MAIN_IL_RAM_BASE, &evidence.il_ram);
    memory_region_init_ram(&evidence.rs_ram, NULL, "xdj.rs-ram", XDJ_MAIN_RS_RAM_SIZE, &error_fatal);
    memory_region_add_subregion(sysmem, XDJ_MAIN_RS_RAM_BASE, &evidence.rs_ram);
    /* This is table41.1's direct data window, not the E8000000 ICB controls.
     * ICB conversion/cache/prefetch, PASCR and mirror modes remain gaps. */
    memory_region_init_ram(&evidence.media_ram, NULL, "xdj.media-ram", XDJ_MAIN_MERAM_RAM_SIZE, &error_fatal);
    memory_region_add_subregion(sysmem, XDJ_MAIN_MERAM_RAM_BASE, &evidence.media_ram);
    object_property_add_str(machine, "xdj-hardware-state", hardware_state, NULL);
}
