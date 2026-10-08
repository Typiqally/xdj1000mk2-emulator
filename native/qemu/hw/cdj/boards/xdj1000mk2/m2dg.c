/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "xdj.h"
#include "performance.h"

/* SH7724 2DG display-list processor. Public opcode/register definitions:
 * DirectFB gfxdrivers/sh772x/sh7723_blt.[ch] and kernel-module/sh7723.c.
 * BITBLT center/extents: Renesas SH7734 manual 11.4.5, same opcode format.
 * The display list and all pixels originate in guest RAM.
 */
typedef struct XdjM2dg {
    MemoryRegion io, alias;
    qemu_irq irq[3];
    uint32_t reg[0x1000 / 4];
    uint8_t reads[0x1000 / 4], writes[0x1000 / 4];
    int ox, oy;
    uint64_t lists, commands;
} XdjM2dg;

static void m2dg_irq(XdjM2dg *s)
{
    unsigned n;
    for (n = 0; n < 3; n++) {
        qemu_set_irq(s->irq[n], (s->reg[1] & s->reg[3] & (1u << n)) != 0);
    }
}

static bool m2dg_ram(uint32_t addr, unsigned length)
{
    return addr >= 0x08000000 && (uint64_t)addr + length <= 0x18000000;
}

static bool m2dg_word(uint32_t *cursor, uint32_t *value)
{
    uint8_t bytes[4];
    if (!m2dg_ram(*cursor, 4) || (*cursor & 3)) {
        return false;
    }
    cpu_physical_memory_read(*cursor, bytes, 4);
    *value = ldl_le_p(bytes);
    *cursor += 4;
    return true;
}

static bool m2dg_fill(XdjM2dg *s, uint32_t opcode, uint32_t args[5])
{
    unsigned left = args[2] >> 16, top = args[3] >> 16;
    unsigned width = left + (args[2] & 0xffff) + 1;
    unsigned height = top + (args[3] & 0xffff) + 1;
    unsigned pitch = s->reg[0x5c / 4];
    uint32_t base = A7ADDR(s->reg[0x50 / 4]);
    int x = (int16_t)(args[4] >> 16) + s->ox - left;
    int y = (int16_t)args[4] + s->oy - top;
    int xmin = 0, ymin = 0, xmax = pitch - 1, ymax = 2047;
    unsigned dx, dy;
    uint8_t row[4096];
    int first, last;
    /* Bound untrusted display lists and destination arithmetic. */
    if (width > 2048 || height > 2048 || !pitch || pitch > 4096 || args[0] != 0xcc) {
        return false;
    }
    if (opcode & 0x2000) {
        uint32_t low = s->reg[0xd4 / 4], high = s->reg[0xd8 / 4];
        xmin = (int16_t)(low >> 16); ymin = (int16_t)low;
        xmax = MIN(xmax, (int16_t)(high >> 16)); ymax = (int16_t)high;
    }
    first = MAX(x, MAX(0, xmin));
    last = MIN(x + (int)width - 1, xmax);
    if (first > last) { return true; }
    for (dx = 0; dx < last - first + 1; dx++) {
        stw_le_p(row + dx * 2, args[1]);
    }
    for (dy = 0; dy < height; dy++) {
        int py = y + dy;
        if (py < ymin || py > ymax || py < 0) {
            continue;
        }
        uint64_t addr = (uint64_t)base + ((uint64_t)py * pitch + first) * 2;
        unsigned length = (last - first + 1) * 2;
        if (addr > UINT32_MAX || !m2dg_ram(addr, length)) {
            return false;
        }
        cpu_physical_memory_write(addr, row, length);
    }
    return true;
}

/* POLYGON4A axis-aligned textured quad, including reversed axes. Hardware
 * vertices are inclusive: an unscaled W-by-H texture spans W-1, H-1. */
static bool m2dg_texture(XdjM2dg *s, uint32_t opcode, uint32_t args[7], uint32_t cmd)
{
    int x[4], y[4], dx, dy, px, py;
    unsigned i, tw = args[1] >> 16, th = args[1] & 0xffff;
    unsigned spitch = s->reg[0x58 / 4], dpitch = s->reg[0x5c / 4];
    uint32_t src = A7ADDR(s->reg[0x4c / 4]);
    uint32_t dst = A7ADDR(s->reg[0x50 / 4]);
    unsigned sx = args[0] >> 16, sy = args[0] & 0xffff;
    unsigned txoff = args[2] >> 16, tyoff = args[2] & 0xffff;
    uint8_t pixel[2];
    if (opcode & (0x200 | 0x3d)) { /* work, color offset and source alpha */
        return false;
    }
    if (!(opcode & 0x100)) {
        src = (opcode & 0x80) ? A7ADDR(cmd + args[0]) : A7ADDR(args[0]);
        sx = sy = 0; spitch = tw;
    }
    if (!tw || !th || tw > 4096 || th > 4096 || !spitch || spitch > 4096 ||
        !dpitch || dpitch > 4096 || sx + tw > spitch || sy + th > 4096 ||
        txoff >= tw || tyoff >= th) {
        return false;
    }
    for (i = 0; i < 4; i++) {
        x[i] = (int16_t)(args[3+i] >> 16) + s->ox;
        y[i] = (int16_t)args[3+i] + s->oy;
    }
    if (x[0] != x[3] || x[1] != x[2] || y[0] != y[1] || y[2] != y[3]) {
        return false; /* skewed quads need the hardware scan conversion */
    }
    dx = abs(x[1] - x[0]); dy = abs(y[3] - y[0]);
    if (dx >= 4096 || dy >= 4096) {
        return false;
    }
    /* Bulk row transfers retain QEMU's RAM dirty tracking and code
     * invalidation. Do not prefetch overlapping surfaces: the original
     * forward pixel traversal intentionally observes its preceding writes. */
    int xmin = MAX(0, MIN(x[0], x[1]));
    int xmax = MIN((int)dpitch - 1, MAX(x[0], x[1]));
    int ymin = MAX(0, MIN(y[0], y[3]));
    int ymax = MIN(4095, MAX(y[0], y[3]));
    if (opcode & 0x2000) {
        xmin = MAX(xmin, (int16_t)(s->reg[0xd4/4] >> 16));
        ymin = MAX(ymin, (int16_t)s->reg[0xd4/4]);
        xmax = MIN(xmax, (int16_t)(s->reg[0xd8/4] >> 16));
        ymax = MIN(ymax, (int16_t)s->reg[0xd8/4]);
    }
    if (xmin > xmax || ymin > ymax) { return true; }
    uint64_t sbegin = (uint64_t)src + ((uint64_t)sy * spitch + sx) * 2;
    uint64_t send = (uint64_t)src + ((uint64_t)(sy + th - 1) * spitch + sx + tw) * 2;
    uint64_t dbegin = (uint64_t)dst + ((uint64_t)ymin * dpitch + xmin) * 2;
    uint64_t dend = (uint64_t)dst + ((uint64_t)ymax * dpitch + xmax + 1) * 2;
    if (sbegin >= 0x08000000 && send <= 0x18000000 &&
        dbegin >= 0x08000000 && dend <= 0x18000000 &&
        (send <= dbegin || dend <= sbegin)) {
        uint8_t texture[8192], row[8192];
        unsigned length = (xmax - xmin + 1) * 2;
        for (py = 0; py <= dy; py++) {
            int yy = y[0] + (y[3] >= y[0] ? py : -py);
            unsigned ty = (opcode & 0x40) ? py : (dy ? (uint64_t)py * (th-1) / dy : 0);
            bool changed = false;
            if (yy < ymin || yy > ymax) { continue; }
            ty = (ty + tyoff) % th + sy;
            uint64_t da = (uint64_t)dst + ((uint64_t)yy * dpitch + xmin) * 2;
            cpu_physical_memory_read((uint64_t)src + ((uint64_t)ty * spitch + sx) * 2,
                                     texture, tw * 2);
            if (opcode & (0x800 | 2)) { cpu_physical_memory_read(da, row, length); }
            for (int xx = xmin; xx <= xmax; xx++) {
                unsigned px = abs(xx - x[0]);
                unsigned tx = (opcode & 0x40) ? px : (dx ? (uint64_t)px * (tw-1) / dx : 0);
                tx = (tx + txoff) % tw;
                uint16_t value = lduw_le_p(texture + tx * 2);
                uint8_t *pixel = row + (xx - xmin) * 2;
                if ((opcode & 0x800) && value == (uint16_t)s->reg[0x80/4]) { continue; }
                if (opcode & 2) {
                    uint16_t back = lduw_le_p(pixel);
                    unsigned a = s->reg[0x88/4] & 255;
                    unsigned r = (((value >> 11) * a + (back >> 11) * (255-a)) / 255) << 11;
                    unsigned g = ((((value >> 5) & 63) * a + ((back >> 5) & 63) * (255-a)) / 255) << 5;
                    unsigned b = ((value & 31) * a + (back & 31) * (255-a)) / 255;
                    value = r | g | b;
                }
                stw_le_p(pixel, value); changed = true;
            }
            if (changed) { cpu_physical_memory_write(da, row, length); }
        }
        return true;
    }
    for (py = 0; py <= dy; py++) {
        int yy = y[0] + (y[3] >= y[0] ? py : -py);
        unsigned ty = ((opcode & 0x40) ? py : (dy ? (uint64_t)py * (th-1) / dy : 0));
        ty = (ty + tyoff) % th + sy;
        for (px = 0; px <= dx; px++) {
            int xx = x[0] + (x[1] >= x[0] ? px : -px);
            unsigned tx = ((opcode & 0x40) ? px : (dx ? (uint64_t)px * (tw-1) / dx : 0));
            uint64_t sa, da;
            uint16_t value;
            tx = (tx + txoff) % tw + sx;
            if (xx < 0 || xx >= dpitch || yy < 0 || yy > 4095) {
                continue;
            }
            if ((opcode & 0x2000) &&
                (xx < (int16_t)(s->reg[0xd4/4] >> 16) ||
                 yy < (int16_t)s->reg[0xd4/4] ||
                 xx > (int16_t)(s->reg[0xd8/4] >> 16) ||
                 yy > (int16_t)s->reg[0xd8/4])) {
                continue;
            }
            sa = (uint64_t)src + ((uint64_t)ty * spitch + tx) * 2;
            da = (uint64_t)dst + ((uint64_t)yy * dpitch + xx) * 2;
            if (sa > UINT32_MAX || da > UINT32_MAX ||
                !m2dg_ram(sa, 2) || !m2dg_ram(da, 2)) {
                return false;
            }
            cpu_physical_memory_read(sa, pixel, 2);
            value = lduw_le_p(pixel);
            if ((opcode & 0x800) && value == (uint16_t)s->reg[0x80/4]) {
                continue;
            }
            if (opcode & 2) {
                uint8_t old[2];
                uint16_t back;
                unsigned a = s->reg[0x88/4] & 255;
                unsigned r, g, b;
                cpu_physical_memory_read(da, old, 2);
                back = lduw_le_p(old);
                r = (((value >> 11) * a + (back >> 11) * (255-a)) / 255) << 11;
                g = ((((value >> 5) & 63) * a + ((back >> 5) & 63) * (255-a)) / 255) << 5;
                b = ((value & 31) * a + (back & 31) * (255-a)) / 255;
                stw_le_p(pixel, r | g | b);
            }
            cpu_physical_memory_write(da, pixel, 2);
        }
    }
    return true;
}

static void m2dg_run(XdjM2dg *s)
{
    XDJ_PERF_SCOPE(graphics);
    uint32_t pc = A7ADDR(s->reg[0x48 / 4]), op = 0, a[7];
    unsigned budget, n;
    s->lists++;
    s->reg[0] |= 1;
    for (budget = 0; budget < 65536; budget++) {
        if (!m2dg_word(&pc, &op)) {
            break;
        }
        s->commands++;
        if (op == 0) {
            s->reg[1] |= 1; /* TRAP completion */
            s->reg[0] &= ~1u;
            m2dg_irq(s);
            return;
        }
        if (op == 0x18000000) {
            if (!m2dg_word(&pc, &a[0]) || !m2dg_word(&pc, &a[1]) ||
                a[0] >= sizeof(s->reg) || (a[0] & 3)) {
                break;
            }
            s->reg[a[0] / 4] = a[1];
        } else if ((op & 0xff000000) == 0xa0000000) {
            for (n = 0; n < 5 && m2dg_word(&pc, &a[n]); n++) { }
            if (n != 5 || !m2dg_fill(s, op, a)) {
                break;
            }
        } else if ((op & 0xff000000) == 0x82000000) {
            uint32_t cmd = pc - 4;
            for (n = 0; n < 7 && m2dg_word(&pc, &a[n]); n++) { }
            if (n != 7 || !m2dg_texture(s, op, a, cmd)) {
                break;
            }
        } else if (op == 0x40000000 || op == 0x48000000) {
            if (!m2dg_word(&pc, &a[0])) {
                break;
            }
            if (op == 0x40000000) {
                s->ox = (int16_t)(a[0] >> 16); s->oy = (int16_t)a[0];
            }
        } else if (op == 0x08000000 || (op & 0xff000000) == 0x12000000) {
            /* NOP and SYNC: transfers are already completed synchronously. */
        } else if (op == 0x08008000) {
            s->reg[1] |= 2;
            m2dg_irq(s);
        } else {
            break;
        }
    }
    qemu_log_mask(LOG_UNIMP, "xdj.2dg: unsupported/invalid list word %08x at %08x\n", op, pc - 4);
    s->reg[1] |= 4;
    s->reg[0] &= ~1u;
    m2dg_irq(s);
}

static uint64_t m2dg_read(void *opaque, hwaddr off, unsigned size)
{
    XdjM2dg *s = opaque;
    if (s->reads[off / 4] < 3) {
        s->reads[off / 4]++;
        qemu_log_mask(LOG_UNIMP, "xdj.2dg: read +%03x=%08x pc=%08x\n",
                      (unsigned)off, s->reg[off / 4],
                      current_cpu ? SUPERH_CPU(current_cpu)->env.pc : 0);
    }
    return s->reg[off / 4];
}

static void m2dg_write(void *opaque, hwaddr off, uint64_t val, unsigned size)
{
    XdjM2dg *s = opaque;
    if (s->writes[off / 4] < 6) {
        s->writes[off / 4]++;
        qemu_log_mask(LOG_UNIMP, "xdj.2dg: write +%03x=%08x pc=%08x\n",
                      (unsigned)off, (uint32_t)val,
                      current_cpu ? SUPERH_CPU(current_cpu)->env.pc : 0);
    }
    if (off == 8) {
        s->reg[1] &= ~val;
        m2dg_irq(s);
    } else if (off == 0 && (val & 0x80000000)) {
        memset(s->reg, 0, sizeof(s->reg));
        s->ox = s->oy = 0;
        m2dg_irq(s);
    } else {
        s->reg[off / 4] = val;
        if (off == 0 && (val & 1)) {
            m2dg_run(s);
        }
        if (off == 0xc) {
            m2dg_irq(s);
        }
    }
}

static const MemoryRegionOps m2dg_ops = {
    .read = m2dg_read, .write = m2dg_write, .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 4, .max_access_size = 4 },
};

void xdj_m2dg_init(MemoryRegion *sysmem, qemu_irq trap, qemu_irq instruction, qemu_irq error)
{
    XdjM2dg *s = g_new0(XdjM2dg, 1);
    s->irq[0] = trap; s->irq[1] = instruction; s->irq[2] = error;
    memory_region_init_io(&s->io, NULL, &m2dg_ops, s, "xdj.2dg", 0x1000);
    memory_region_add_subregion(sysmem, 0x04680000, &s->io);
    memory_region_init_alias(&s->alias, NULL, "xdj.2dg-p2", &s->io, 0, 0x1000);
    memory_region_add_subregion(sysmem, 0xa4680000, &s->alias);
}
