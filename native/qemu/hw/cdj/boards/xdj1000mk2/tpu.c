/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "hardware.h"
#include "sh7724_tpu.h"

typedef struct XdjTpu {
    MemoryRegion io, alias;
    Sh7724Tpu core;
    int64_t last_ns;
} XdjTpu;

static void tpu_update(XdjTpu *s)
{
    int64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    uint64_t elapsed = now >= s->last_ns ? now - s->last_ns : 0;
    s->last_ns = now;
    /* Unsupported clocks/modes are reported at the configuring access. */
    sh7724_tpu_advance(&s->core, elapsed, XDJ_TPU_CLOCK_NUM, XDJ_TPU_CLOCK_DEN);
}
static uint64_t tpu_read(void *opaque, hwaddr off, unsigned size)
{
    XdjTpu *s = opaque;
    uint32_t value;
    unsigned reg = off >= 0x10 ? (off - 0x10) % 0x40 : 0;
    /* Steady TGR polling returns a stable value without a clock read or
     * advancing four counters. Buffers and TCNT/TSR still catch up exactly.
     * There is no per-PWM-edge callback; IRQ wakeups remain unsupported. */
    if (reg == 16 || reg == 20 || sh7724_tpu_buffer_pending(&s->core)) tpu_update(s);
    if (!sh7724_tpu_read(&s->core, off, size, &value))
        xdj_hardware_gap("sh7724.tpu.access", 0xa4c90000 + off, 0, size, false);
    return value;
}
static void tpu_write(void *opaque, hwaddr off, uint64_t value, unsigned size)
{
    XdjTpu *s = opaque;
    tpu_update(s);
    unsigned reg = off >= 0x10 ? (off - 0x10) % 0x40 : 0;
    unsigned channel = off >= 0x10 ? (off - 0x10) / 0x40 : 0;
    if (!sh7724_tpu_write(&s->core, off, value, size)) {
        xdj_hardware_gap("sh7724.tpu.access", 0xa4c90000 + off, value, size, true);
        return;
    }
    if (!sh7724_tpu_configured(&s->core))
        xdj_hardware_gap("sh7724.tpu.mode", 0xa4c90000 + off, value, size, true);
    if (off && reg == 12 && s->core.channel[channel].tier)
        xdj_hardware_gap("sh7724.tpu.irq", 0xa4c90000 + off, value, size, true);
    if (off && (reg == 0 || reg == 4 || reg == 8) && (s->core.start & (1u << channel)))
        xdj_hardware_gap("sh7724.tpu.active-config", 0xa4c90000 + off, value, size, true);
    if (!off && s->core.start) {
        /* Clock is nominal Bphi; complete CPG/module-standby coupling is
         * absent. Pin/PFC/electrical PWM is not qualified by these counters. */
        xdj_hardware_gap("sh7724.tpu.clock", 0xa4c90000, value, size, true);
        xdj_hardware_gap("sh7724.tpu.pins", 0xa4c90000, value, size, true);
    }
}
static const MemoryRegionOps tpu_ops = {
    .read = tpu_read, .write = tpu_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 4, .unaligned = true },
    .impl = { .min_access_size = 1, .max_access_size = 4, .unaligned = true },
};
static void tpu_reset(void *opaque)
{
    XdjTpu *s = opaque;
    sh7724_tpu_init(&s->core);
    s->last_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
}
void xdj_tpu_init(void)
{
    XdjTpu *s = g_new0(XdjTpu, 1);
    tpu_reset(s);
    memory_region_init_io(&s->io, NULL, &tpu_ops, s, "sh7724.tpu", 0x1000);
    memory_region_add_subregion(get_system_memory(), 0xa4c90000, &s->io);
    memory_region_init_alias(&s->alias, NULL, "sh7724.tpu.a7", &s->io, 0, 0x1000);
    memory_region_add_subregion(get_system_memory(), 0x04c90000, &s->alias);
    qemu_register_reset(tpu_reset, s);
}
