/* SPDX-License-Identifier: GPL-2.0-or-later */
/* Adapted from the reference EDMA3 controller for DA810 (SPRUH96B).
 * Transfer controllers execute guest-programmed PaRAM synchronously. */
#include <string.h>

#include "da810_edma.h"
static uint32_t reg_get(uint32_t reg, uint32_t off, unsigned size) {
    uint32_t v = reg >> (8 * (off & 3));
    return size >= 4 ? v : v & ((1u << (8 * size)) - 1);
}
static uint32_t reg_put(uint32_t reg, uint32_t off, uint32_t val, unsigned size) {
    unsigned shift = 8 * (off & 3);
    uint32_t mask = (size >= 4 ? UINT32_MAX : (1u << (8 * size)) - 1) << shift;
    return (reg & ~mask) | ((val << shift) & mask);
}
void da810_edma_init(Da810Edma *s, uint32_t (*read_mem)(void *, uint32_t, unsigned),
                     void (*write_mem)(void *, uint32_t, uint32_t, unsigned),
                     void (*irq)(void *, unsigned, int), void *opaque) {
    memset(s, 0, sizeof *s);
    s->read_mem = read_mem; s->write_mem = write_mem;
    s->irq = irq; s->opaque = opaque;
}

enum { OPT_SAM = 1, OPT_DAM = 2, OPT_SYNCDIM = 4, OPT_STATIC = 8,
       OPT_TCINTEN = 1u << 20, OPT_ITCINTEN = 1u << 21,
       OPT_TCCHEN = 1u << 22, OPT_ITCCHEN = 1u << 23 };

static uint32_t cfg32(const Da810Edma *e, uint32_t off)
{
    const uint8_t *m = e->cfg + off;
    return m[0] | m[1] << 8 | m[2] << 16 | (uint32_t)m[3] << 24;
}

static void cfg32_put(Da810Edma *e, uint32_t off, uint32_t v)
{
    uint8_t *m = e->cfg + off;
    m[0] = v; m[1] = v >> 8; m[2] = v >> 16; m[3] = v >> 24;
}

static uint64_t drae(const Da810Edma *e, unsigned region)
{
    return cfg32(e, 0x340 + 8 * region);
}

static uint32_t param_word(const Da810Edma *e, unsigned set, unsigned w)
{
    const uint8_t *m = e->param + set * 32 + w * 4;
    return m[0] | m[1] << 8 | m[2] << 16 | (uint32_t)m[3] << 24;
}

static void param_word_put(Da810Edma *e, unsigned set, unsigned w, uint32_t v)
{
    uint8_t *m = e->param + set * 32 + w * 4;
    m[0] = v; m[1] = v >> 8; m[2] = v >> 16; m[3] = v >> 24;
}

static void irq_update(Da810Edma *s, unsigned region, int level)
{
    if (s->irq_level[region] != level) {
        s->irq_level[region] = level;
        s->irq(s->opaque, region, level);
    }
}

static void outputs(Da810Edma *s)
{
    Da810Edma *e = s;
    uint64_t pend = e->ipr & e->ier & UINT32_MAX;
    
    for (unsigned r = 0; r < 4; r++)
        irq_update(s, r, (pend & drae(e, r)) != 0);
    irq_update(s, 4, e->emr != 0 || e->qemr != 0);
}

static void copy_array(Da810Edma *s, uint32_t opt, uint32_t src, uint32_t dst, unsigned acnt)
{
    unsigned fifo = 1u << ((opt >> 8) & 7);
    unsigned chunk;
    /* The firmware also uses the controller as its block copier, megabytes
     * a second with master tempo on: those need not go a word at a time. */
    if (!(opt & (OPT_SAM | OPT_DAM)) && s->copy_mem && s->copy_mem(s->opaque, src, dst, acnt))
        return;
    if (opt & (OPT_SAM | OPT_DAM))
        chunk = fifo > 4 ? 4 : fifo;
    else
        chunk = acnt % 4 == 0 ? 4 : acnt % 2 == 0 ? 2 : 1;
    for (unsigned off = 0; off + chunk <= acnt; off += chunk) {
        uint32_t v = s->read_mem(s->opaque, (opt & OPT_SAM) ? src : src + off, chunk);
        s->write_mem(s->opaque, (opt & OPT_DAM) ? dst : dst + off, v, chunk);
    }
}

/* One transfer request from PaRAM set `set`: move the data, update or link the
 * set, and raise completion / chaining for its TCC. */
/* Returns 1 when the final transfer of the set loaded a linked set. */
static int run_set(Da810Edma *s, unsigned set)
{
    Da810Edma *e = s;
    if (set >= EDMA_PARAM_SETS)
        return 0;
    uint32_t opt = param_word(e, set, 0), src = param_word(e, set, 1);
    uint32_t abcnt = param_word(e, set, 2), dst = param_word(e, set, 3);
    uint32_t bidx = param_word(e, set, 4), lnk = param_word(e, set, 5);
    uint32_t cidx = param_word(e, set, 6), ccntw = param_word(e, set, 7);
    unsigned acnt = abcnt & 0xFFFF, bcnt = abcnt >> 16, ccnt = ccntw & 0xFFFF;
    int32_t sbidx = (int16_t)bidx, dbidx = (int16_t)(bidx >> 16);
    int32_t scidx = (int16_t)cidx, dcidx = (int16_t)(cidx >> 16);
    unsigned tcc = (opt >> 12) & 0x3F;
    int sam = opt & OPT_SAM, dam = opt & OPT_DAM;

    if (!acnt || !bcnt || !ccnt) {
        /* Normal after the last TR of an unlinked set (SPI TX raises XEVT once
         * more when its final word leaves); reported once. */
        e->null_transfers++;
        return 0;
    }

    /* A malformed AB request must not monopolize the emulator thread. */
    if ((opt & OPT_SYNCDIM) && (uint64_t)acnt * bcnt > 16 * 1024 * 1024) {
        e->emr |= 1ull << (set < 32 ? set : 0);
        return 0;
    }
    int final;
    if (!(opt & OPT_SYNCDIM)) {              /* A-synchronized: one array */
        copy_array(s, opt, src, dst, acnt);
        if (bcnt > 1) {
            bcnt--;
            if (!sam) src += sbidx;
            if (!dam) dst += dbidx;
            final = 0;
        } else {
            ccnt--;
            bcnt = lnk >> 16;
            if (!sam) src += scidx;
            if (!dam) dst += dcidx;
            final = ccnt == 0;
        }
    } else {                                 /* AB-synchronized: one frame */
        /* Arrays laid end to end on both sides are one block. */
        if (!(!sam && !dam && sbidx == (int32_t)acnt && dbidx == (int32_t)acnt && s->copy_mem
              && s->copy_mem(s->opaque, src, dst, acnt * bcnt)))
            for (unsigned k = 0; k < bcnt; k++)
                copy_array(s, opt, sam ? src : src + k * sbidx, dam ? dst : dst + k * dbidx, acnt);
        ccnt--;
        if (!sam) src += scidx;
        if (!dam) dst += dcidx;
        final = ccnt == 0;
    }

    int linked = 0;
    if (!(opt & OPT_STATIC)) {
        param_word_put(e, set, 1, src);
        param_word_put(e, set, 2, acnt | bcnt << 16);
        param_word_put(e, set, 3, dst);
        param_word_put(e, set, 7, (ccntw & ~0xFFFFu) | ccnt);
        unsigned link = lnk & 0xFFFF;
        if (final && link != 0xFFFF) {
            if (link >= EDMA_PARAM_BASE && link + 32 <= EDMA_PARAM_END && !((link - EDMA_PARAM_BASE) % 32)) {
                memmove(e->param + set * 32, e->param + (link - EDMA_PARAM_BASE), 32);
                linked = 1;
            }
            else e->emr |= 1ull << (set < 32 ? set : 0);
        }
    }

    if (tcc < 32 && (opt & (final ? OPT_TCINTEN : OPT_ITCINTEN)))
        e->ipr |= 1ull << tcc;
    if (tcc < 32 && (opt & (final ? OPT_TCCHEN : OPT_ITCCHEN)))
        e->cer |= 1ull << tcc;
    return linked;
}

/* Service everything pending. Transfers can raise events (a DMA write to a
 * peripheral) and chain; nested calls only latch, the outermost loop runs. */
static void service(Da810Edma *s)
{
    Da810Edma *e = s;
    if (e->depth)
        return;
    e->depth++;
    for (int guard = 0; guard < 4096; guard++) {
        uint64_t go = (e->er & e->eer) | e->cer;
        uint8_t qgo = e->qer & e->qeer;
        if (!go && !qgo)
            break;
        if (go) {
            unsigned ch = __builtin_ctzll(go);
            e->er &= ~(1ull << ch);
            e->cer &= ~(1ull << ch);
            run_set(s, ch);
        } else {
            unsigned q = __builtin_ctz(qgo);
            e->qer &= ~(1u << q);
            /* A QDMA set that links loads its trigger word with the link, which
             * requests the next transfer: the app's planar-to-interleaved copy
             * (PaRAM 80 -> static 96, TCC 8, 0x8007CFA8) waits on the second
             * set's completion and nothing else triggers it. */
            if (run_set(s, (cfg32(e, 0x200 + 4 * q) >> 5) & 0x1FF) && ((e->qeer >> q) & 1))
                e->qer |= 1u << q;
        }
    }
    e->depth--;
    outputs(s);
}

void da810_edma_event(Da810Edma *s, unsigned ch, int level)
{
    Da810Edma *e = s;
    if (ch >= 32)
        return;
    int was = e->level[ch];
    e->level[ch] = level;
    if (!level || was)
        return;
    uint64_t bit = 1ull << ch;
    if (e->er & bit)
        e->emr |= bit;                       /* missed: previous one still latched */
    e->er |= bit;
    service(s);
}

/* The event/interrupt register block at 0x1000 (global) or a shadow region at
 * 0x2000 + 0x200 n, whose view is masked by DRAEn / QRAEn. */
static uint32_t events_read(Da810Edma *e, uint32_t a, uint64_t mask, uint8_t qmask)
{
    uint64_t v64;
    switch (a & ~4u) {
    case 0x00: v64 = e->er; break;
    case 0x18: v64 = e->cer; break;
    case 0x20: v64 = e->eer; break;
    case 0x38: v64 = e->ser; break;
    case 0x50: v64 = e->ier; break;
    case 0x68: v64 = e->ipr; break;
    default:
        switch (a) {
        case 0x80: return e->qer & qmask;
        case 0x84: return e->qeer & qmask;
        case 0x90: return e->qser & qmask;
        default:   return 0;
        }
    }
    v64 &= mask;
    return (a & 4) ? (uint32_t)(v64 >> 32) : (uint32_t)v64;
}

static void events_write(Da810Edma *s, uint32_t a, uint32_t val, uint64_t mask, uint8_t qmask)
{
    Da810Edma *e = s;
    uint64_t bits = ((a & 4) ? (uint64_t)val << 32 : val) & mask;
    uint8_t qbits = val & qmask;
    if (a < 0x78)
        switch (a & ~4u) {
        case 0x08: e->er &= ~bits; break;
        case 0x10: e->cer |= bits; break;    /* manual trigger, regardless of EER */
        case 0x28: e->eer &= ~bits; break;
        case 0x30: e->eer |= bits; break;
        case 0x40: e->ser &= ~bits; break;
        case 0x58: e->ier &= ~bits; break;
        case 0x60: e->ier |= bits; break;
        case 0x70: e->ipr &= ~bits; break;
        default: break;                      /* read-only views */
        }
    else
        switch (a) {
        case 0x78:
            if (val & 1) {                   /* IEVAL: pulse whatever is pending */
                for (unsigned r = 0; r < 4; r++)
                    irq_update(s, r, 0);
                
            }
            break;
        case 0x88: e->qeer &= ~qbits; break;
        case 0x8C: e->qeer |= qbits; break;
        case 0x94: e->qser &= ~qbits; break;
        default: break;
        }
    service(s);
}

uint32_t da810_edma_read(Da810Edma *s, uint32_t off, unsigned size)
{
    Da810Edma *e = s;
    uint32_t a = off & ~3u, v;
    if (a < 0x1000) {
        switch (a) {
        case 0x000: v = 0x40015300; break;   /* PID; value unverified */
        case 0x004: v = 0x00213344; break;   /* CCCFG reset: SPRUH96B Table 18-11 */
        case 0x300: v = (uint32_t)e->emr; break;
        case 0x304: v = (uint32_t)(e->emr >> 32); break;
        case 0x310: v = e->qemr; break;
        case 0x308: case 0x30C: case 0x314: case 0x31C: case 0x320: case 0x318:
        case 0x640: v = 0; break;
        default:
            if (a >= 0x400 && a < 0x610) v = 0;   /* queue entries / status: idle */
            else v = cfg32(e, a);
        }
    } else if (a < 0x2000) {
        v = events_read(e, a - 0x1000, 0xffffffffull, 0xFF);
    } else if (a < 0x2800) {
        unsigned r = (a - 0x2000) / 0x200;
        v = events_read(e, (a - 0x2000) % 0x200, drae(e, r), e->cfg[0x380 + 4 * r]);
    } else if (a >= EDMA_PARAM_BASE && a < EDMA_PARAM_END) {
        uint32_t p = a - EDMA_PARAM_BASE;
        v = param_word(e, p / 32, (p % 32) / 4);
    } else {
        
        v = 0;
    }
    return reg_get(v, off, size);
}

void da810_edma_write(Da810Edma *s, uint32_t off, uint32_t val, unsigned size)
{
    Da810Edma *e = s;
    uint32_t a = off & ~3u;
    uint32_t v = reg_put(0, off, val, size);
    if (a < 0x1000) {
        switch (a) {
        case 0x000: case 0x004: case 0x300: case 0x304: case 0x310: case 0x318:
        case 0x640:
            return;                          /* read-only */
        case 0x308: e->emr &= ~(uint64_t)v; break;
        case 0x30C: e->emr &= ~((uint64_t)v << 32); break;
        case 0x314: e->qemr &= ~v; break;
        case 0x31C: case 0x320: break;
        default:
            cfg32_put(e, a, reg_put(cfg32(e, a), off, val, size));
        }
        outputs(s);
    } else if (a < 0x2000) {
        events_write(s, a - 0x1000, v, 0xffffffffull, 0xFF);
    } else if (a < 0x2800) {
        unsigned r = (a - 0x2000) / 0x200;
        events_write(s, (a - 0x2000) % 0x200, v, drae(e, r), e->cfg[0x380 + 4 * r]);
    } else if (a >= EDMA_PARAM_BASE && a < EDMA_PARAM_END) {
        uint32_t p = a - EDMA_PARAM_BASE;
        unsigned set = p / 32, w = (p % 32) / 4;
        param_word_put(e, set, w, reg_put(param_word(e, set, w), off, val, size));
        for (unsigned q = 0; q < 8; q++) {
            uint32_t map = cfg32(e, 0x200 + 4 * q);
            if (((e->qeer >> q) & 1) && ((map >> 5) & 0x1FF) == set && ((map >> 2) & 7) == w)
                e->qer |= 1u << q;
        }
        service(s);
    } else {
        
    }
}
