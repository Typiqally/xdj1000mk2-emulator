/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "xdj.h"

/* Stock MAIN 09364c62 drives loader command1 on PTV1 and command2 on
 * PTE7. 09364bd0 polls active-low HINT at PTZ0. Command responses are
 * produced by the native DSP, rather than forcing the ready input.
 */
typedef struct XdjPfc {
    MemoryRegion io;
    uint8_t reg[0x200];
    const CdjDspWires *dsp;
    bool usb_stop;
} XdjPfc;
static XdjPfc *pfc;

void xdj_pfc_usb_stop(bool pressed)
{
    if (pfc) { pfc->usb_stop = pressed; }
}

static uint64_t pfc_read(void *opaque, hwaddr off, unsigned size)
{
    XdjPfc *s = opaque;
    uint64_t value = 0;
    unsigned i;
    for (i = 0; i < size; i++) {
        uint8_t byte = s->reg[off + i];
        /* PTH1 is the USB-A power switch's active-low fault output.
         * MAIN 1/7 and 6/7: CPU_USB_HSTPWRFL is pulled high at idle. */
        /* PTH4 is CPU_USB_HSTSTOP: the unpressed physical USB STOP
         * switch reads high. MAIN 0949c3b0 samples it into the native key
         * state; leaving it low ejects an otherwise mounted volume. */
        if (off + i == 0x2e) {
            byte |= 0x12;
            if (s->usb_stop) { byte &= ~0x10; }
        }
        if (off + i == 0x6c) {
            byte = (byte & ~1) | s->dsp->busy(s->dsp->opaque);
        }
        value |= (uint64_t)byte << (8 * i);
    }
    return value;
}

static void pfc_write(void *opaque, hwaddr off, uint64_t value, unsigned size)
{
    XdjPfc *s = opaque;
    unsigned i;
    for (i = 0; i < size; i++) {
        s->reg[off + i] = value >> (8 * i);
    }
    /* The native command writer updates PTE then PTV. The synchronous
     * host handshake must observe the completed pair; running millions of
     * DSP cycles between those adjacent byte stores consumes command3 as
     * command2 and loses the final loader transition.
     */
    if (off <= 0x64 && off + size > 0x64) {
        unsigned command = ((s->reg[0x64] >> 1) & 1) |
                           ((s->reg[0x28] >> 6) & 2);
        s->dsp->command(s->dsp->opaque, command);
    }
}

static const MemoryRegionOps pfc_ops = {
    .read = pfc_read, .write = pfc_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 1, .max_access_size = 4 },
};

void xdj_pfc_init(MemoryRegion *sysmem, const CdjDspWires *dsp)
{
    XdjPfc *s = g_new0(XdjPfc, 1);
    s->dsp = dsp;
    pfc = s;
    memory_region_init_io(&s->io, NULL, &pfc_ops, s, "xdj.pfc", sizeof(s->reg));
    memory_region_add_subregion(sysmem, 0x04050100, &s->io);
}
