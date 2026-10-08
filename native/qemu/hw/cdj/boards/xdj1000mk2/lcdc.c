/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "xdj.h"
#include "performance.h"
#include "hardware.h"
#include "ui/console.h"
#include "ui/pixel_ops.h"
#include "qemu/timer.h"

#define XDJ_VBLANK_NS 16666667

/* SH7724 LCDC main channel. Register definitions also published in Linux
 * include/video/sh_mobile_lcdc.h. The guest renders every displayed pixel.
 */
typedef struct XdjLcdc {
    MemoryRegion io, alias;
    QemuConsole *console;
    QEMUTimer *vblank;
    int64_t next_vblank_ns;
    qemu_irq irq;
    uint32_t reg[0x3000 / 4];
    unsigned bank;
    uint64_t frames;
    uint8_t reads[0x3000 / 4], writes[0x3000 / 4];
    /* The raw scanout rows as last converted, and the mode they belong to.
     * A refresh converts and reports only rows the guest changed, so the
     * display can be polled at the panel's rate without holding the BQL for
     * a whole frame each time. */
    uint8_t *shadow;
    unsigned shadow_width, shadow_height, shadow_format, shadow_swap;
} XdjLcdc;

/* SH7724 manual 39.3: A/B registers plus a mirror of the inactive
 * bank. Only these channel registers are banked; controls are shared. */
static bool lcdc_banked(hwaddr off)
{
    switch (off & 0xfff) {
    case 0x418: case 0x41c: case 0x420: case 0x424: case 0x428:
    case 0x430: case 0x434: case 0x438:
    case 0x448: case 0x44c: case 0x450: case 0x454:
    case 0x458: case 0x45c:
        return true;
    default:
        return false;
    }
}

static hwaddr lcdc_offset(XdjLcdc *s, hwaddr off)
{
    if (off >= 0x2000 && lcdc_banked(off)) {
        return (off & 0xfff) + (s->bank ? 0 : 0x1000);
    }
    return off;
}

static void lcdc_bank(XdjLcdc *s, unsigned bank)
{
    s->bank = bank & 1;
    s->reg[0x46c / 4] = (s->reg[0x46c / 4] & ~0x100u) | (s->bank << 8);
}

static void lcdc_irq(XdjLcdc *s)
{
    uint32_t r = s->reg[0x468 / 4];
    qemu_set_irq(s->irq, ((r >> 8) & r & 7) != 0);
}

static void lcdc_vblank(void *opaque)
{
    XdjLcdc *s = opaque;
    /* QEMU removes a fired timer and clears its expire_time before callback. */
    if (xdj_system_performance.enabled) {
        xdj_perf_vblank(qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) - s->next_vblank_ns);
    }
    if (s->reg[0x474 / 4] & 1) {
        uint32_t control = s->reg[0x478 / 4];
        lcdc_bank(s, control & 1 ? !s->bank : (control >> 1) & 1);
        s->reg[0x468 / 4] |= 7;
        s->frames++;
        lcdc_irq(s);
    }
    /* Rearm from the deadline, not from this callback's clock: adding the
     * period to a late callback makes every vblank's lateness permanent and
     * the panel slower than 60 Hz. Resynchronise only once a frame is lost. */
    int64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    s->next_vblank_ns += XDJ_VBLANK_NS;
    if (s->next_vblank_ns <= now) {
        s->next_vblank_ns = now + XDJ_VBLANK_NS;
    }
    timer_mod(s->vblank, s->next_vblank_ns);
}

static uint64_t lcdc_read(void *opaque, hwaddr off, unsigned size)
{
    XdjLcdc *s = opaque;
    if (s->reads[off / 4] < 3) {
        s->reads[off / 4]++;
        qemu_log_mask(LOG_UNIMP, "xdj.lcdc: read +%03x = %08x pc=%08x\n",
                      (unsigned)off, s->reg[off / 4],
                      current_cpu ? SUPERH_CPU(current_cpu)->env.pc : 0);
    }
    return s->reg[lcdc_offset(s, off) / 4];
}

static void lcdc_write(void *opaque, hwaddr off, uint64_t value, unsigned size)
{
    XdjLcdc *s = opaque;
    if (s->writes[off / 4] < 4) {
        s->writes[off / 4]++;
        qemu_log_mask(LOG_UNIMP, "xdj.lcdc: write +%03x = %08x pc=%08x\n",
                      (unsigned)off, (uint32_t)value,
                      current_cpu ? SUPERH_CPU(current_cpu)->env.pc : 0);
    }
    if (off == 0x468) {
        /* Status flags are cleared by writing zero; enable bits are R/W. */
        s->reg[off / 4] = (value & ~0xffu) | (s->reg[off / 4] & value & 0xff);
        lcdc_irq(s);
    } else if (off == 0x474) {
        if (value & 0x100) {
            memset(s->reg, 0, sizeof(s->reg));
            lcdc_bank(s, 0);
            lcdc_irq(s);
        } else {
            s->reg[off / 4] = value;
            s->reg[0x460 / 4] = (value & 1) ? 3 : 0;
        }
    } else {
        s->reg[lcdc_offset(s, off) / 4] = value;
        if (off == 0x484) {
            lcdc_bank(s, value);
        }
    }
}

static const MemoryRegionOps lcdc_ops = {
    .read = lcdc_read, .write = lcdc_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 4, .max_access_size = 4 },
};

static void lcdc_update(void *opaque)
{
    XDJ_PERF_SCOPE(display);
    XdjLcdc *s = opaque;
    unsigned bank = s->bank * 0x1000 / 4;
    unsigned width = ((s->reg[bank + 0x448 / 4] >> 16) & 0xfff) * 8;
    unsigned height = (s->reg[bank + 0x450 / 4] >> 16) & 0xfff;
    unsigned format = s->reg[bank + 0x424 / 4] & 0x1f;
    unsigned bpp = format == 3 ? 2 : format == 0xb ? 3 : 4;
    unsigned pitch = s->reg[bank + 0x438 / 4];
    uint32_t base = A7ADDR(s->reg[bank + 0x430 / 4]);
    DisplaySurface *surface;
    uint8_t *row;
    unsigned swap = 7 ^ (s->reg[0x47c / 4] & 7);
    unsigned y, x, row_bytes;
    int first = -1, last = -1;
    bool repaint;
    if (!(s->reg[0x474 / 4] & 1) || !width || !height) {
        return;
    }
    if (width > XDJ_LCD_WIDTH || height > XDJ_LCD_HEIGHT || pitch < width * bpp ||
        base < XDJ_MAIN_RAM_BASE || (uint64_t)base + (uint64_t)pitch * height >
        (uint64_t)XDJ_MAIN_RAM_BASE + XDJ_MAIN_RAM_SIZE) {
        xdj_hardware_violation("LCD mode/framebuffer outside physical panel/DDR bounds", base);
        return;
    }
    if (format != 0 && format != 3 && format != 0xb) {
        xdj_hardware_gap("sh7724.lcdc.format", 0xfe940424, format, 4, true);
        qemu_log_mask(LOG_UNIMP, "xdj.lcdc: unsupported pixel format 0x%x\n", format);
        return;
    }
    row_bytes = ROUND_UP(width * bpp, 8);
    repaint = !s->shadow || s->shadow_width != width || s->shadow_height != height ||
              s->shadow_format != format || s->shadow_swap != swap;
    if (repaint) {
        s->shadow = g_realloc(s->shadow, (size_t)row_bytes * height);
        s->shadow_width = width;
        s->shadow_height = height;
        s->shadow_format = format;
        s->shadow_swap = swap;
    }
    qemu_console_resize(s->console, width, height);
    surface = qemu_console_surface(s->console);
    row = g_malloc(row_bytes);
    for (y = 0; y < height; y++) {
        uint32_t *dst = (uint32_t *)(surface_data(surface) + y * surface_stride(surface));
        uint8_t *seen = s->shadow + (size_t)y * row_bytes;
        cpu_physical_memory_read(base + y * pitch, row, row_bytes);
        if (!repaint && !memcmp(row, seen, row_bytes)) {
            continue;
        }
        memcpy(seen, row, row_bytes);
        if (first < 0) { first = y; }
        last = y;
        for (x = 0; x < width; x++) {
            unsigned r, g, b;
            uint8_t pixel[4];
            unsigned k;
            /* The LCD bus handles big-endian 64-bit beats. LDDDSR swaps
             * byte, word and longword positions within each beat. */
            for (k = 0; k < bpp; k++) {
                unsigned byte = x * bpp + k;
                pixel[k] = row[(byte & ~7u) | ((byte & 7) ^ swap)];
            }
            if (format == 3) {
                uint16_t p = lduw_be_p(pixel);
                r = ((p >> 11) & 31) * 255 / 31;
                g = ((p >> 5) & 63) * 255 / 63;
                b = (p & 31) * 255 / 31;
            } else if (format == 0xb) {
                b = pixel[0]; g = pixel[1]; r = pixel[2];
            } else {
                r = pixel[1]; g = pixel[2]; b = pixel[3];
            }
            dst[x] = rgb_to_pixel32(r, g, b);
        }
    }
    g_free(row);
    if (first >= 0) {
        dpy_gfx_update(s->console, 0, first, width, last - first + 1);
    }
}

static const GraphicHwOps lcdc_graphic_ops = { .gfx_update = lcdc_update };

void xdj_lcdc_init(MemoryRegion *sysmem, qemu_irq irq)
{
    XdjLcdc *s = g_new0(XdjLcdc, 1);
    s->irq = irq;
    memory_region_init_io(&s->io, NULL, &lcdc_ops, s, "xdj.lcdc", 0x3000);
    memory_region_add_subregion(sysmem, 0xfe940000, &s->io);
    memory_region_init_alias(&s->alias, NULL, "xdj.lcdc-a7", &s->io, 0, 0x3000);
    memory_region_add_subregion(sysmem, 0x1e940000, &s->alias);
    s->console = graphic_console_init(NULL, 0, &lcdc_graphic_ops, s);
    qemu_console_resize(s->console, XDJ_LCD_WIDTH, XDJ_LCD_HEIGHT);
    s->vblank = timer_new_ns(QEMU_CLOCK_VIRTUAL, lcdc_vblank, s);
    s->next_vblank_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) + XDJ_VBLANK_NS;
    timer_mod(s->vblank, s->next_vblank_ns);
}
