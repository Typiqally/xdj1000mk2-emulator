/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "hardware.h"
#include "sh7724_rwdt.h"
typedef struct XdjRwdt {
    MemoryRegion io, alias;
    QEMUTimer *timer;
    Sh7724Rwdt core;
    int64_t last_ns;
    bool reset_seen;
} XdjRwdt;
static void rwdt_update(XdjRwdt *s)
{
    int64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    uint64_t elapsed = now >= s->last_ns ? now - s->last_ns : 0;
    s->last_ns = now;
    if (sh7724_rwdt_advance(&s->core, elapsed, XDJ_RWDT_CLOCK_NUM, XDJ_RWDT_CLOCK_DEN)) {
        /* The counter requests a real QEMU reset, but other board devices lack
         * comprehensive reset callbacks/rail sequencing. Keep that gap visible. */
        xdj_hardware_gap("sh7724.rwdt.reset", 0xa4520000, s->core.csr, 1, true);
        qemu_system_reset_request(SHUTDOWN_CAUSE_GUEST_RESET);
    }
}
static void rwdt_schedule(XdjRwdt *s)
{
    int64_t delay = sh7724_rwdt_until_overflow(&s->core, XDJ_RWDT_CLOCK_NUM, XDJ_RWDT_CLOCK_DEN);
    if (delay < 0) timer_del(s->timer);
    else timer_mod(s->timer, delay > INT64_MAX - s->last_ns ? INT64_MAX : s->last_ns + delay);
}
static void rwdt_expire(void *opaque)
{
    XdjRwdt *s = opaque;
    rwdt_update(s);
    rwdt_schedule(s);
}
static uint64_t rwdt_read(void *opaque, hwaddr off, unsigned size)
{
    XdjRwdt *s = opaque;
    uint32_t value;
    rwdt_update(s);
    if (!sh7724_rwdt_read(&s->core, off, size, &value))
        xdj_hardware_gap("sh7724.rwdt.access", 0xa4520000 + off, 0, size, false);
    return value;
}
static void rwdt_write(void *opaque, hwaddr off, uint64_t value, unsigned size)
{
    XdjRwdt *s = opaque;
    rwdt_update(s);
    if (off == 4 && size == 2 && value >> 8 == 0xa5 &&
        (s->core.csr & 7) != (value & 7))
        xdj_hardware_gap("sh7724.rwdt.clock-phase", 0xa4520004, value, size, true);
    if (!sh7724_rwdt_write(&s->core, off, value, size))
        xdj_hardware_gap(off == 0 ? "sh7724.rwdt.write-sync" : "sh7724.rwdt.access",
                         0xa4520000 + off, value, size, true);
    rwdt_schedule(s);
}
static const MemoryRegionOps rwdt_ops = {
    .read = rwdt_read, .write = rwdt_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 4, .unaligned = true },
    .impl = { .min_access_size = 1, .max_access_size = 4, .unaligned = true },
};
static void rwdt_reset(void *opaque)
{
    XdjRwdt *s = opaque;
    if (s->reset_seen) {
        if (s->core.pending_reset) {
            /* Overflow initializes the counter/control; retained cause flags
             * across the physical reset sequence are not qualified. */
            sh7724_rwdt_init(&s->core);
            s->core.csr |= 0x18;
        } else {
            /* Manual reset retains RWDT registers (Renesas table 19.3). */
            s->core.csr |= 8;
        }
    }
    s->reset_seen = true;
    s->last_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    rwdt_schedule(s);
}
void xdj_rwdt_init(void)
{
    XdjRwdt *s = g_new0(XdjRwdt, 1);
    sh7724_rwdt_init(&s->core);
    s->last_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    s->timer = timer_new_ns(QEMU_CLOCK_VIRTUAL, rwdt_expire, s);
    memory_region_init_io(&s->io, NULL, &rwdt_ops, s, "sh7724.rwdt", 0x1000);
    memory_region_add_subregion(get_system_memory(), 0xa4520000, &s->io);
    memory_region_init_alias(&s->alias, NULL, "sh7724.rwdt.a7", &s->io, 0, 0x1000);
    memory_region_add_subregion(get_system_memory(), 0x04520000, &s->alias);
    qemu_register_reset(rwdt_reset, s);
    rwdt_schedule(s);
}
