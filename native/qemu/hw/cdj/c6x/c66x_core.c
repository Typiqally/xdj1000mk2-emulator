/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * C66x core: a cycle-level interpreter.
 *
 * Timing model (see c66x_core.md). The C6000 exposes its pipeline to software
 * and the DSP image depends on it (load results are read exactly 5 cycles
 * later, 16x16 multiplies 2), so every register write lands at the cycle
 * binutils' operand table gives it
 * (E1 = cycle 1: single-cycle ops at 1, loads at 5, mpysp at 4, adddp 6/7) and
 * becomes readable the cycle after. A branch takes effect after five delay
 * slots, where a slot is one E1 advance: an execute packet or a NOP cycle
 * (NOP n, BNOP n, CALLP, ADDKPC n, PROT loads), never a stall.
 */#include "c66x_priv.h"

#ifdef C66X_HCOUNT
uint64_t hc_handler[H_COUNT], hc_fop[32], hc_path[8], hc_gather[8];
/* Cycles per (kind, pc): 0 fast packet, 1 fast loop kernel (SPLOOP address),
 * 2 general cycle inside a loop, 3 general cycle outside one. */
#define HC_PCS 65536
typedef struct hc_pc_ent { uint32_t pc; uint8_t kind; uint8_t ii, dynlen; uint64_t n; } hc_pc_ent;
static hc_pc_ent hc_pcs[HC_PCS];
void hc_pc(uint32_t pc, uint8_t kind, int ii, int dynlen)
{
    uint32_t key = pc ^ ((uint32_t)kind << 30);
    unsigned s = (key * 2654435761u) >> 16;
    for (unsigned i = 0; i < HC_PCS; i++, s = (s + 1) & (HC_PCS - 1)) {
        hc_pc_ent *e = &hc_pcs[s];
        if (e->n && (e->pc != pc || e->kind != kind))
            continue;
        e->pc = pc; e->kind = kind; e->ii = ii; e->dynlen = dynlen; e->n++;
        return;
    }
}
/* Finished loops per SPLOOP address: invocations, iterations, cycles. */
typedef struct hc_loop_ent { uint32_t addr; uint64_t n, iters, cycles, hist[8]; } hc_loop_ent;
static hc_loop_ent hc_loops[4096];
void hc_loop(uint32_t addr, uint64_t iters, uint64_t cycles)
{
    unsigned s = (addr * 2654435761u) >> 20;
    for (unsigned i = 0; i < 4096; i++, s = (s + 1) & 4095) {
        hc_loop_ent *e = &hc_loops[s];
        if (e->n && e->addr != addr)
            continue;
        e->addr = addr; e->n++; e->iters += iters; e->cycles += cycles;
        e->hist[iters < 2 ? 0 : iters < 4 ? 1 : iters < 8 ? 2 : iters < 16 ? 3 : iters < 32 ? 4 : iters < 64 ? 5 : iters < 256 ? 6 : 7]++;
        return;
    }
}
#endif

static const struct { const char *name; uint16_t h; } hmap[] = {
    { "nop", H_NOP }, { "idle", H_IDLE }, { "dint", H_DINT }, { "rint", H_RINT },
    { "swe", H_SWE }, { "swenr", H_SWE },
    { "abs", H_ABS }, { "abs2", H_ABS2 }, { "add", H_ADD }, { "addu", H_ADDU },
    { "sub", H_SUB }, { "subu", H_SUBU }, { "add2", H_ADD2 }, { "sub2", H_SUB2 },
    { "add4", H_ADD4 }, { "sub4", H_SUB4 },
    { "addab", H_ADDA }, { "addah", H_ADDA }, { "addaw", H_ADDA }, { "addad", H_ADDA },
    { "subab", H_SUBA }, { "subah", H_SUBA }, { "subaw", H_SUBA },
    { "addk", H_ADDK }, { "addkpc", H_ADDKPC },
    { "addsub", H_ADDSUB }, { "addsub2", H_ADDSUB2 }, { "saddsub", H_SADDSUB },
    { "saddsub2", H_SADDSUB2 },
    { "and", H_AND }, { "andn", H_ANDN }, { "or", H_OR }, { "xor", H_XOR },
    { "not", H_NOT }, { "neg", H_NEG }, { "mv", H_MV }, { "mvk", H_MVK },
    { "mvkl", H_MVK }, { "mvkh", H_MVKH }, { "mvklh", H_MVKH }, { "zero", H_ZERO },
    { "dmv", H_DMV }, { "mvc", H_MVC }, { "mvd", H_MVD },
    { "cmpeq", H_CMPEQ }, { "cmpgt", H_CMPGT }, { "cmpgtu", H_CMPGTU },
    { "cmplt", H_CMPLT }, { "cmpltu", H_CMPLTU },
    { "cmpeq2", H_CMPEQ2 }, { "cmpgt2", H_CMPGT2 }, { "cmplt2", H_CMPLT2 },
    { "cmpeq4", H_CMPEQ4 }, { "cmpgtu4", H_CMPGTU4 }, { "cmpltu4", H_CMPLTU4 },
    { "shl", H_SHL }, { "shr", H_SHR }, { "shru", H_SHRU }, { "sshl", H_SSHL },
    { "shr2", H_SHR2 }, { "shru2", H_SHRU2 }, { "sshvl", H_SSHVL }, { "sshvr", H_SSHVR },
    { "shlmb", H_SHLMB }, { "shrmb", H_SHRMB }, { "rotl", H_ROTL },
    { "clr", H_CLR }, { "set", H_SET }, { "ext", H_EXT }, { "extu", H_EXTU },
    { "sadd", H_SADD }, { "ssub", H_SSUB }, { "sat", H_SAT }, { "sadd2", H_SADD2 },
    { "ssub2", H_SSUB2 }, { "saddus2", H_SADDUS2 }, { "saddsu2", H_SADDSU2 },
    { "saddu4", H_SADDU4 },
    { "subc", H_SUBC }, { "subabs4", H_SUBABS4 }, { "norm", H_NORM }, { "lmbd", H_LMBD },
    { "bitc4", H_BITC4 }, { "bitr", H_BITR }, { "deal", H_DEAL }, { "shfl", H_SHFL },
    { "shfl3", H_SHFL3 }, { "xpnd2", H_XPND2 }, { "xpnd4", H_XPND4 },
    { "swap2", H_SWAP2 }, { "swap4", H_SWAP4 },
    { "avg2", H_AVG2 }, { "avgu4", H_AVGU4 }, { "max2", H_MAX2 }, { "maxu4", H_MAXU4 },
    { "min2", H_MIN2 }, { "minu4", H_MINU4 },
    { "pack2", H_PACK2 }, { "packh2", H_PACKH2 }, { "packhl2", H_PACKHL2 },
    { "packlh2", H_PACKLH2 }, { "packh4", H_PACKH4 }, { "packl4", H_PACKL4 },
    { "spack2", H_SPACK2 }, { "spacku4", H_SPACKU4 }, { "rpack2", H_RPACK2 },
    { "dpack2", H_DPACK2 }, { "dpackx2", H_DPACKX2 },
    { "unpkhu4", H_UNPKHU4 }, { "unpklu4", H_UNPKLU4 },
    { "mpy", H_MPY16 }, { "mpyu", H_MPY16 }, { "mpyus", H_MPY16 }, { "mpysu", H_MPY16 },
    { "mpyh", H_MPY16 }, { "mpyhu", H_MPY16 }, { "mpyhus", H_MPY16 }, { "mpyhsu", H_MPY16 },
    { "mpyhl", H_MPY16 }, { "mpyhlu", H_MPY16 }, { "mpyhslu", H_MPY16 },
    { "mpyhuls", H_MPY16 }, { "mpylh", H_MPY16 }, { "mpylhu", H_MPY16 },
    { "mpylshu", H_MPY16 }, { "mpyluhs", H_MPY16 },
    { "smpy", H_SMPY16 }, { "smpyh", H_SMPY16 }, { "smpyhl", H_SMPY16 }, { "smpylh", H_SMPY16 },
    { "mpy32", H_MPY32 }, { "mpy32u", H_MPY32 }, { "mpy32su", H_MPY32 }, { "mpy32us", H_MPY32 },
    { "mpyi", H_MPYI }, { "mpyid", H_MPYID },
    { "mpyhi", H_MPYHI }, { "mpyih", H_MPYHI }, { "mpyhir", H_MPYHIR }, { "mpyihr", H_MPYHIR },
    { "mpyli", H_MPYLI }, { "mpyil", H_MPYLI }, { "mpylir", H_MPYLIR }, { "mpyilr", H_MPYLIR },
    { "mpy2", H_MPY2 }, { "smpy2", H_SMPY2 }, { "mpysu4", H_MPYSU4 }, { "mpyu4", H_MPYU4 },
    { "mpyus4", H_MPYUS4 }, { "smpy32", H_SMPY32 }, { "mpy2ir", H_MPY2IR },
    { "dotp2", H_DOTP2 }, { "dotpn2", H_DOTPN2 }, { "dotprsu2", H_DOTPRSU2 },
    { "dotprus2", H_DOTPRSU2 }, { "dotpnrsu2", H_DOTPNRSU2 }, { "dotpnrus2", H_DOTPNRSU2 },
    { "dotpsu4", H_DOTPSU4 }, { "dotpus4", H_DOTPSU4 }, { "dotpu4", H_DOTPU4 },
    { "ddotp4", H_DDOTP4 }, { "ddotph2", H_DDOTPH2 }, { "ddotpl2", H_DDOTPL2 },
    { "ddotph2r", H_DDOTPH2R }, { "ddotpl2r", H_DDOTPL2R },
    { "cmpy", H_CMPY }, { "cmpyr", H_CMPYR }, { "cmpyr1", H_CMPYR1 },
    { "gmpy", H_GMPY }, { "gmpy4", H_GMPY4 }, { "xormpy", H_XORMPY },
    { "abssp", H_ABSSP }, { "absdp", H_ABSDP }, { "addsp", H_ADDSP }, { "adddp", H_ADDDP },
    { "subsp", H_SUBSP }, { "subdp", H_SUBDP }, { "mpysp", H_MPYSP }, { "mpydp", H_MPYDP },
    { "mpyspdp", H_MPYSPDP }, { "mpysp2dp", H_MPYSP2DP },
    { "cmpeqsp", H_CMPEQSP }, { "cmpgtsp", H_CMPGTSP }, { "cmpltsp", H_CMPLTSP },
    { "cmpeqdp", H_CMPEQDP }, { "cmpgtdp", H_CMPGTDP }, { "cmpltdp", H_CMPLTDP },
    { "intsp", H_INTSP }, { "intspu", H_INTSPU }, { "intdp", H_INTDP }, { "intdpu", H_INTDPU },
    { "spint", H_SPINT }, { "dpint", H_DPINT }, { "sptrunc", H_SPTRUNC }, { "dptrunc", H_DPTRUNC },
    { "spdp", H_SPDP }, { "dpsp", H_DPSP }, { "rcpsp", H_RCPSP }, { "rcpdp", H_RCPDP },
    { "rsqrsp", H_RSQRSP }, { "rsqrdp", H_RSQRDP },
    { "ldb", H_LOAD }, { "ldbu", H_LOAD }, { "ldh", H_LOAD }, { "ldhu", H_LOAD },
    { "ldw", H_LOAD }, { "ldnw", H_LOAD }, { "lddw", H_LOAD }, { "ldndw", H_LOAD },
    { "ll", H_LOAD }, { "cmtl", H_LOAD },
    { "stb", H_STORE }, { "sth", H_STORE }, { "stw", H_STORE }, { "stnw", H_STORE },
    { "stdw", H_STORE }, { "stndw", H_STORE }, { "sl", H_STORE },
    { "b", H_B }, { "bnop", H_BNOP }, { "callp", H_CALLP }, { "bdec", H_BDEC },
    { "bpos", H_BPOS },
    { "sploop", H_SPLOOP }, { "sploopd", H_SPLOOPD }, { "sploopw", H_SPLOOPW },
    { "spkernel", H_SPKERNEL }, { "spkernelr", H_SPKERNELR },
    { "spmask", H_SPMASK }, { "spmaskr", H_SPMASKR },
    { "faddsp", H_ADDSP }, { "fsubsp", H_SUBSP }, { "fadddp", H_ADDDP }, { "fsubdp", H_SUBDP },
    { "fmpydp", H_MPYDP }, { "dmvd", H_DMV },
    { "dadd", H_DADD }, { "dsub", H_DSUB }, { "dsadd", H_DSADD }, { "dssub", H_DSSUB },
    { "daddsp", H_DADDSP }, { "dsubsp", H_DSUBSP }, { "dmpysp", H_DMPYSP }, { "qmpysp", H_QMPYSP }, { "cmpysp", H_CMPYSP },
    { "mpyu2", H_MPYU2 }, { "land", H_LAND }, { "landn", H_LANDN }, { "lor", H_LOR },
    { "dinthsp", H_DINTHSP }, { "dinthspu", H_DINTHSPU }, { "dspint", H_DSPINT },
    { "dsadd2", H_DSADD2 }, { "davgnr2", H_DAVGNR2 }, { "unpkh2", H_UNPKH2 },
    { "dshl2", H_DSHL2 }, { "dcmpgt2", H_DCMPGT2 }, { "dmpy2", H_DMPY2 },
    { "qsmpy32r1", H_QSMPY32R1 },
    { "shl2", H_SHL2 }, { "dpackl4", H_DPACKL4 }, { "dccmpyr1", H_DCCMPYR1 },
    { "dintsp", H_DINTSP }, { "dintspu", H_DINTSPU },
};

/* ------------------------------------------------------------------------ */



/* C66X_HCOUNT builds only: where execution goes, printed by c66x_free. */

/* ------------------------------------------------------------------------ */
/* record (c66x_record_open; read back by c6xreplay.c)                       */


/* ------------------------------------------------------------------------ */
/* API                                                                       */


static c66x_core *core_new(const c66x_bus *bus, int c674x_only)
{
    c66x_core *c = calloc(1, sizeof(*c));
    if (!c)
        return NULL;
    if (bus)
        c->bus = *bus;
    unsigned n = c66x_opcode_count();
    for (unsigned i = 0; i < 2256; i++) {
        if (i >= n && i < 2000)
            continue;
        const char *name = c66x_opcode_name(i);
        c->htab[i] = H_UNIMP;
        for (unsigned k = 0; k < sizeof hmap / sizeof hmap[0]; k++)
            if (!strcmp(hmap[k].name, name)) {
                c->htab[i] = hmap[k].h;
                break;
            }
    }
    /* Sparse: only the pages of mapped RAM are ever written, and pt_none never. */
    c->pt_ld = calloc(C66X_PT_PAGES, sizeof *c->pt_ld);
    c->pt_st = calloc(C66X_PT_PAGES, sizeof *c->pt_st);
    c->pt_none = calloc(C66X_PT_PAGES, sizeof *c->pt_none);
    if (!c->pt_ld || !c->pt_st || !c->pt_none) {
        free(c->pt_ld);
        free(c->pt_st);
        free(c->pt_none);
        free(c);
        return NULL;
    }
    c->c674x_only = c674x_only;
    if (c674x_only) {
        for (unsigned i = 2000; i < 2256; i++) { c->htab[i] = H_UNIMP; }
    }
    c66x_reset(c, 0);
    jit_init(c);
    /* C66X_IDLE_ISR_FAST=1: handlers keep the fast store paths while the
     * busy-wait skip is armed (idle_page_stored). */
    const char *isrf = getenv("C66X_IDLE_ISR_FAST");
    c->idle_isr_fast = isrf && strcmp(isrf, "0");
    return c;
}

c66x_core *c66x_new(const c66x_bus *bus) { return core_new(bus, 0); }
c66x_core *c66x_new_c674x(const c66x_bus *bus) { return core_new(bus, 1); }

void c66x_free(c66x_core *c)
{
    if (!c)
        return;
    c66x_record_close(c);
    jit_free(c);
    free(c->idle_auto_tab);
    kstats_dump_fwd();
#ifdef C66X_HCOUNT
    fprintf(stderr, "paths: fast-packet %llu nop-run %llu spl-fast %llu general %llu | general with "
            "spl active %llu, entry/idle/drain %llu, ifr set but nothing deliverable %llu\n",
            (unsigned long long)hc_path[0], (unsigned long long)hc_path[1], (unsigned long long)hc_path[2],
            (unsigned long long)hc_path[3], (unsigned long long)hc_path[5], (unsigned long long)hc_path[6],
            (unsigned long long)hc_path[7]);
    for (unsigned i = 0; i < 32; i++)
        if (hc_fop[i])
            fprintf(stderr, "fop %2u %llu\n", i, (unsigned long long)hc_fop[i]);
    for (unsigned i = 0; i < H_COUNT; i++)
        if (hc_handler[i])
            fprintf(stderr, "handler %3u %llu\n", i, (unsigned long long)hc_handler[i]);
    fprintf(stderr, "gather: pkc hit %llu, block fallback %llu, uncached cross-block %llu, uncached long %llu\n",
            (unsigned long long)hc_gather[0], (unsigned long long)hc_gather[1],
            (unsigned long long)hc_gather[2], (unsigned long long)hc_gather[3]);
    for (unsigned i = 0; i < 4096; i++)
        if (hc_loops[i].n)
            fprintf(stderr, "loop 0x%08x runs %llu iters %llu cycles %llu hist <2:%llu <4:%llu <8:%llu <16:%llu "
                    "<32:%llu <64:%llu <256:%llu more:%llu\n", hc_loops[i].addr, (unsigned long long)hc_loops[i].n,
                    (unsigned long long)hc_loops[i].iters, (unsigned long long)hc_loops[i].cycles,
                    (unsigned long long)hc_loops[i].hist[0], (unsigned long long)hc_loops[i].hist[1],
                    (unsigned long long)hc_loops[i].hist[2], (unsigned long long)hc_loops[i].hist[3],
                    (unsigned long long)hc_loops[i].hist[4], (unsigned long long)hc_loops[i].hist[5],
                    (unsigned long long)hc_loops[i].hist[6], (unsigned long long)hc_loops[i].hist[7]);
    for (unsigned i = 0; i < HC_PCS; i++)
        if (hc_pcs[i].n)
            fprintf(stderr, "pc %u 0x%08x ii %u dynlen %u %llu\n", hc_pcs[i].kind, hc_pcs[i].pc,
                    hc_pcs[i].ii, hc_pcs[i].dynlen, (unsigned long long)hc_pcs[i].n);
#endif
    for (unsigned i = 0; i < c->nram; i++) {
        ramreg *r = &c->ram[i];
        if (r->fpp) {
            for (uint32_t p = 0; p <= r->size >> FP_PAGE_SHIFT; p++) {
                if (!r->fpp[p])
                    continue;
                for (unsigned f = 0; f < 1u << (FP_PAGE_SHIFT - 5); f++)
                    free(r->fpp[p][f]);
                free(r->fpp[p]);
            }
            free(r->fpp);
        }
        int shared = 0;
        for (unsigned j = 0; j < i; j++)
            shared |= c->ram[j].codepage == r->codepage;
        if (!shared)
            free(r->codepage);
    }
    free(c->pt_ld);
    free(c->pt_st);
    free(c->pt_none);
    free(c);
}

/* Host buffers are identified by the index of their first mapping, which is
 * stable within a core: L2 and its alias share one. */
uint32_t rec_host_id(const c66x_core *c, const ramreg *r)
{
    for (unsigned i = 0; i < c->nram; i++)
        if (c->ram[i].host == r->host && c->ram[i].size == r->size)
            return i;
    return 0;
}

uint64_t c66x_state_hash(const c66x_core *c)
{
    uint64_t h = 1469598103934665603ull;
    const uint8_t *p = (const uint8_t *)c->reg;
    for (size_t i = 0; i < sizeof c->reg; i++)
        h = (h ^ p[i]) * 1099511628211ull;
    p = (const uint8_t *)c->cr;
    for (size_t i = 0; i < sizeof c->cr; i++)
        h = (h ^ p[i]) * 1099511628211ull;
    h = (h ^ c->pc) * 1099511628211ull;
    h = (h ^ c->ifr) * 1099511628211ull;
    return h;
}

int c66x_record_open(c66x_core *c, const char *path)
{
    if (!path || !*path)
        return 0;
    c->rec = fopen(path, "ab");
    if (!c->rec)
        return -1;
    setvbuf(c->rec, NULL, _IOFBF, 1 << 20);
    /* K identifies the C674x instruction set. Old N recordings retain the
     * compatibility decoder; replay must never silently broaden a new CPU. */
    rec_head(c, c->c674x_only ? 'K' : REC_NEW);
    c->rec_hash_at = 0;
    return 0;
}

void c66x_record_close(c66x_core *c)
{
    if (c->rec)
        fclose(c->rec);
    c->rec = NULL;
}

void c66x_map_ram(c66x_core *c, uint32_t base, uint32_t size, uint8_t *host)
{
    if (c->nram >= 8)
        return;
    if (c->rec) {
        uint32_t id = c->nram;
        for (unsigned i = 0; i < c->nram; i++)
            if (c->ram[i].host == host && c->ram[i].size == size)
                id = i;
        rec_head(c, REC_MAP);
        fwrite(&base, 4, 1, c->rec);
        fwrite(&size, 4, 1, c->rec);
        fwrite(&id, 4, 1, c->rec);
    }
    uint8_t *cp = NULL;
    for (unsigned i = 0; i < c->nram; i++)
        if (c->ram[i].host == host && c->ram[i].size == size)
            cp = c->ram[i].codepage;
    if (!cp)
        cp = calloc((size >> FP_PAGE_SHIFT) + 1, 1);
    c->ram[c->nram++] = (ramreg){ base, size, host, NULL, cp };
    pt_map(c, &c->ram[c->nram - 1]);
}

void c66x_reset(c66x_core *c, uint32_t pc)
{
    if (c->rec) {
        /* The program is already in RAM when the chip is reset: keep it. */
        static const uint8_t zero[4096];
        for (unsigned i = 0; i < c->nram; i++) {
            ramreg *r = &c->ram[i];
            uint32_t id = rec_host_id(c, r);
            if (id != i)
                continue;
            for (uint32_t off = 0; off + 4096 <= r->size; off += 4096) {
                if (!memcmp(r->host + off, zero, 4096))
                    continue;
                rec_head(c, REC_PAGE);
                fwrite(&id, 4, 1, c->rec);
                fwrite(&off, 4, 1, c->rec);
                fwrite(r->host + off, 1, 4096, c->rec);
            }
        }
        rec_head(c, REC_RESET);
        fwrite(&pc, 4, 1, c->rec);
    }
    memset(c->reg, 0, sizeof c->reg);
    memset(c->cr, 0, sizeof c->cr);
    memset(c->wqn, 0, sizeof c->wqn);
    c->imm_n = 0;
    c->wmask = 0;
    memset(&c->spl, 0, sizeof c->spl);
    c->cr[CR_CSR] = 0x14000100;         /* C674x ID 0x14 (SPRUFE8B 2-9), little endian */
    c->cr[CR_TSR] = 0;                  /* supervisor, GIE off */
    c->cr[CR_IER] = 1;
    c->ifr = 0;
    c->efr = 0;
    c->pc = pc;
    c->mcnop = 0;
    c->pm_resume_at = 0;
    c->spl_irq_pending = 0;
    c->int_entry = 0;
    c->idle = 0;
    c->nbr = 0;
    c->branch_block = 0;
    c->npst = 0;
    c->isr_depth = 0;
    if (c->idle_isr_disarmed) {
        c->idle_isr_disarmed = 0;
        c->idle_armed = 1;
    }
    c->idle_fp = 0;
    c->idle_ret_pending = 0;
    c66x_mem_mode_sync(c);
    c66x_round_sync(c);
}

void c66x_set_irq(c66x_core *c, int line, int level)
{
    uint32_t bit = 1u << line;
    if (c->rec) {
        uint8_t l = (uint8_t)line, v = level != 0;
        rec_head(c, REC_IRQ);
        fwrite(&l, 1, 1, c->rec);
        fwrite(&v, 1, 1, c->rec);
    }
    if (level && !(c->irq_level & bit)) {
        c->ifr |= bit;
        c->irq_level |= bit;
        if (line >= 0 && line < 16)
            c->st.irq_edges[line]++;
    } else if (!level) {
        c->irq_level &= ~bit;
    }
}

void c66x_hook_pc(c66x_core *c, uint32_t pc, c66x_hook_fn fn, void *opaque)
{
    for (unsigned i = 0; i < NHOOK; i++) {
        unsigned k = (pc / 2 + i) & (NHOOK - 1);
        if (!c->hooks[k].fn || c->hooks[k].pc == pc) {
            if (!c->hooks[k].fn)
                c->nhooks++;
            c->hooks[k] = (hook){ pc, fn, opaque };
            return;
        }
    }
}


uint32_t c66x_get_pc(const c66x_core *c) { return c->pc; }
void c66x_set_pc(c66x_core *c, uint32_t pc) { c->pc = pc; c->nbr = 0; c->mcnop = 0; }
uint32_t c66x_get_reg(const c66x_core *c, unsigned r) { return r < C66X_NREGS ? c->reg[r] : 0; }
void c66x_set_reg(c66x_core *c, unsigned r, uint32_t v) { if (r < C66X_NREGS) c->reg[r] = v; }
uint32_t c66x_trap_pc(const c66x_core *c) { return c->trap_pc; }
void c66x_get_stats(const c66x_core *c, c66x_stats *out) { *out = c->st; }
void c66x_set_trace(c66x_core *c, c66x_trace_fn fn, void *opaque) { c->trace = fn; c->trace_opaque = opaque; }
uint32_t c66x_get_exec_pc(const c66x_core *c) { return c->exec_pc; }
uint64_t c66x_get_cycle(const c66x_core *c) { return c->cycle; }

#define IDLE_PASS_SLACK UINT64_C(100000)    /* cycles of handlers a pass may be interrupted by */
#define IDLE_PASS_FIRST UINT64_C(2000000)   /* the limit before any fixed point is known */

void c66x_set_idle_loop(c66x_core *c, uint32_t head_pc, uint32_t stack_lo,
                        uint32_t stack_hi, int allow_bus_reads)
{
    uint32_t previous = c->idle_head;
    if (c->rec) {
        uint32_t reads = allow_bus_reads != 0;
        rec_head(c, REC_IDLE);
        fwrite(&head_pc, 4, 1, c->rec);
        fwrite(&stack_lo, 4, 1, c->rec);
        fwrite(&stack_hi, 4, 1, c->rec);
        fwrite(&reads, 4, 1, c->rec);
    }
    c->idle_stack_pending = head_pc && stack_lo == stack_hi;
    c->idle_pass_limit = IDLE_PASS_FIRST;
    c->idle_frame_lo = c->idle_frame_hi = 0;
    c->idle_frame_dirty = 0;
    c->idle_head = head_pc;
    c->idle_slo = stack_lo;
    c->idle_shi = stack_hi;
    c->idle_allow_reads = allow_bus_reads;
    c->idle_armed = 0;
    c->idle_isr_disarmed = 0;
    c->idle_fx = 0;
    c->idle_fp = 0;
    c->idle_isr_hit = 0;
    c->idle_ret_pending = 0;
    idle_rs_clear(c);
    c66x_mem_mode_sync(c);
    /* Regions stop at the head they were built for: those of another head
     * would run through this one, so every cached packet looks again. */
    if (previous != head_pc)
        jit_relink(c);
}

void c66x_idle_isr_fast(c66x_core *c, int on)
{
    c->idle_isr_fast = on != 0;
}

void c66x_idle_wake(c66x_core *c)
{
    if (!c->idle_head)
        return;
    /* Declaring the same loop again starts its tracking over, and is what a
     * replay of this run repeats at the same cycle. */
    if (c->idle_stack_pending)
        c66x_set_idle_loop(c, c->idle_head, 0, 0, c->idle_allow_reads);
    else
        c66x_set_idle_loop(c, c->idle_head, c->idle_slo, c->idle_shi, c->idle_allow_reads);
}

/* ------------------------------------------------------------------------ */
/* head discovery (c66x_idle_discover)                                       */

enum { IDLE_AUTO_OFF, IDLE_AUTO_WAIT, IDLE_AUTO_SAMPLE, IDLE_AUTO_PROBE, IDLE_AUTO_WATCH };

#define IDLE_AUTO_SLOTS       8192                      /* power of two */
#define IDLE_AUTO_CLASS_MIN   8                         /* packets that must recur together */
#define IDLE_AUTO_FIRST       UINT64_C(20000000)        /* cycles from reset to the first window */
#define IDLE_AUTO_WINDOW      UINT64_C(400000)          /* cycles one window samples */
#define IDLE_AUTO_PROBE_FOR   UINT64_C(600000)          /* cycles a proposal has to prove itself */
#define IDLE_AUTO_PROBE_PASSES 12                       /* ... or passes without one fixed point */
#define IDLE_AUTO_BACKOFF     UINT64_C(100000000)
#define IDLE_AUTO_BACKOFF_MAX UINT64_C(1600000000)
#define IDLE_AUTO_WATCH_EVERY UINT64_C(50000000)
#define IDLE_AUTO_LOST        UINT64_C(2700000000)      /* cycles without a pass: the program moved on */
#define IDLE_SP               (32 + 15)                 /* B15 */

struct idle_auto_slot { uint32_t pc, n; };

/* C66X_IDLE_DEBUG=1: the search's proposals and verdicts, on stderr. */
static int idle_auto_debug(void)
{
    static int on = -1;
    if (on < 0) {
        const char *e = getenv("C66X_IDLE_DEBUG");
        on = e && *e && strcmp(e, "0");
    }
    return on;
}

/* The idle check runs where this is called: before a packet, in the general
 * loop. A packet is a possible head only where the check can pass, with no
 * handler running, no branch in flight and no multi-cycle NOP pending. */
static void idle_auto_sample(c66x_core *c, void *opaque, uint32_t pc)
{
    if (c->idle_auto_trace)
        c->idle_auto_trace(c, c->idle_auto_trace_opaque, pc);
    if (c->isr_depth || c->nbr || c->mcnop || c->spl.active)
        return;
    struct idle_auto_slot *t = c->idle_auto_tab;
    unsigned s = (pc * 2654435761u) >> 19 & (IDLE_AUTO_SLOTS - 1);
    for (unsigned probes = 0; probes < 64; probes++, s = (s + 1) & (IDLE_AUTO_SLOTS - 1)) {
        if (t[s].n && t[s].pc != pc)
            continue;
        t[s].pc = pc;
        t[s].n++;
        return;
    }
}

static int idle_auto_slot_order(const void *a, const void *b)
{
    const struct idle_auto_slot *x = a, *y = b;
    return x->n != y->n ? (x->n < y->n ? -1 : 1) : x->pc < y->pc ? -1 : x->pc > y->pc;
}

static int idle_auto_is_bad(const c66x_core *c, uint32_t pc)
{
    for (unsigned i = 0; i < IDLE_AUTO_BAD; i++)
        if (c->idle_auto_bad[i] == pc)
            return 1;
    return pc == c->idle_auto_base;
}

/* Straight-line code a loop runs once per pass shows up as many packets with
 * one common count; the outermost polling loop is the largest such group.
 * Propose the lowest packet of each of the largest groups. */
static void idle_auto_propose(c66x_core *c)
{
    struct idle_auto_slot *t = c->idle_auto_tab;
    unsigned size[IDLE_AUTO_CANDS] = { 0 };
    unsigned used = 0;

    for (unsigned i = 0; i < IDLE_AUTO_SLOTS; i++)
        if (t[i].n)
            t[used++] = t[i];
    qsort(t, used, sizeof t[0], idle_auto_slot_order);
    c->idle_auto_ncand = 0;
    for (unsigned i = 0, j; i < used; i = j) {
        uint32_t head = 0;
        for (j = i; j < used && t[j].n == t[i].n; j++)
            if (!head && !idle_auto_is_bad(c, t[j].pc))
                head = t[j].pc;
        unsigned members = j - i;
        if (t[i].n < 3 || members < IDLE_AUTO_CLASS_MIN || !head)
            continue;
        /* insertion into the few largest, largest first */
        unsigned at = c->idle_auto_ncand;
        while (at && size[at - 1] < members)
            at--;
        if (at >= IDLE_AUTO_CANDS)
            continue;
        unsigned last = c->idle_auto_ncand < IDLE_AUTO_CANDS ? c->idle_auto_ncand : IDLE_AUTO_CANDS - 1;
        for (unsigned k = last; k > at; k--) {
            size[k] = size[k - 1];
            c->idle_auto_cand[k] = c->idle_auto_cand[k - 1];
        }
        size[at] = members;
        c->idle_auto_cand[at] = head;
        if (c->idle_auto_ncand < IDLE_AUTO_CANDS)
            c->idle_auto_ncand++;
    }
    if (idle_auto_debug()) {
        fprintf(stderr, "c66x idle: cycle %llu: %u background packets sampled;", (unsigned long long)c->cycle, used);
        for (unsigned i = 0; i < c->idle_auto_ncand; i++)
            fprintf(stderr, " proposal 0x%08x (%u packets)", c->idle_auto_cand[i], size[i]);
        fprintf(stderr, "\n");
    }
}

static void idle_auto_watch(c66x_core *c)
{
    c->idle_auto = c->idle_auto_base ? IDLE_AUTO_WATCH : IDLE_AUTO_WAIT;
    c->idle_auto_since = c->cycle;
    c->idle_auto_iters = c->idle_info.iterations;
    c->idle_auto_at = c->cycle + (c->idle_auto_base ? IDLE_AUTO_WATCH_EVERY : c->idle_auto_backoff);
}

static void idle_auto_probe(c66x_core *c)
{
    if (c->idle_auto_next >= c->idle_auto_ncand) {
        /* Nothing held: back to the head there was, and wait longer. */
        if (c->idle_head != c->idle_auto_base)
            c66x_set_idle_loop(c, c->idle_auto_base, 0, 0, 0);
        c->idle_auto_hits = c->idle_info.idle_hits;
        if (c->idle_auto_backoff < IDLE_AUTO_BACKOFF_MAX)
            c->idle_auto_backoff *= 2;
        idle_auto_watch(c);
        if (c->idle_auto == IDLE_AUTO_WATCH)
            c->idle_auto_at = c->cycle + c->idle_auto_backoff;
        return;
    }
    c66x_set_idle_loop(c, c->idle_auto_cand[c->idle_auto_next], 0, 0, 0);
    c->idle_auto = IDLE_AUTO_PROBE;
    c->idle_auto_since = c->cycle;
    c->idle_auto_iters = c->idle_info.iterations;
    c->idle_auto_hits = c->idle_info.idle_hits;
    c->idle_auto_at = c->cycle + IDLE_AUTO_PROBE_FOR / 8;
}

void idle_auto_poll(c66x_core *c)
{
    const c66x_idle_info *ii = &c->idle_info;

    switch (c->idle_auto) {
    case IDLE_AUTO_WAIT:
    sample:
        /* A caller's trace keeps running underneath; a core that cannot
         * rebuild its regions for a new head does not look for one. */
        if (!jit_can_retarget(c) || !c->idle_auto_tab) {
            c->idle_auto_at = c->cycle + IDLE_AUTO_BACKOFF_MAX;
            return;
        }
        memset(c->idle_auto_tab, 0, IDLE_AUTO_SLOTS * sizeof c->idle_auto_tab[0]);
        c->idle_auto_trace = c->trace;
        c->idle_auto_trace_opaque = c->trace_opaque;
        c->trace = idle_auto_sample;
        c->idle_auto = IDLE_AUTO_SAMPLE;
        c->idle_auto_at = c->cycle + IDLE_AUTO_WINDOW;
        return;
    case IDLE_AUTO_SAMPLE:
        c->trace = c->idle_auto_trace;
        c->trace_opaque = c->idle_auto_trace_opaque;
        c->idle_auto_trace = NULL;
        idle_auto_propose(c);
        c->idle_auto_next = 0;
        idle_auto_probe(c);
        return;
    case IDLE_AUTO_PROBE:
        if (idle_auto_debug())
            fprintf(stderr, "c66x idle: cycle %llu: head 0x%08x after %llu cycles: %llu passes, %llu fixed points, "
                    "last reason %u at 0x%08x pc 0x%08x\n", (unsigned long long)c->cycle, c->idle_head,
                    (unsigned long long)(c->cycle - c->idle_auto_since),
                    (unsigned long long)(ii->iterations - c->idle_auto_iters),
                    (unsigned long long)(ii->idle_hits - c->idle_auto_hits), ii->last_reason, ii->last_addr,
                    ii->last_pc);
        if (ii->idle_hits - c->idle_auto_hits >= 2) {
            c->idle_auto_base = c->idle_head;
            c->idle_auto_backoff = IDLE_AUTO_BACKOFF;
            idle_auto_watch(c);
        } else if (c->cycle - c->idle_auto_since >= IDLE_AUTO_PROBE_FOR
                   || ii->iterations - c->idle_auto_iters >= IDLE_AUTO_PROBE_PASSES) {
            c->idle_auto_bad[c->idle_auto_nbad++ % IDLE_AUTO_BAD] = c->idle_head;
            c->idle_auto_next++;
            idle_auto_probe(c);
        } else {
            c->idle_auto_at = c->cycle + IDLE_AUTO_PROBE_FOR / 8;
        }
        return;
    case IDLE_AUTO_WATCH:
        /* The head is right while passes keep arriving, idle or busy. A
         * program that left its loop for good (a loader handing over to the
         * application) gets a new search; the old head stays armed until a
         * proposal replaces it. */
        if (ii->iterations != c->idle_auto_iters) {
            c->idle_auto_iters = ii->iterations;
            c->idle_auto_since = c->cycle;
        } else if (c->cycle - c->idle_auto_since >= IDLE_AUTO_LOST) {
            c->idle_auto_since = c->cycle;
            goto sample;
        }
        c->idle_auto_at = c->cycle + IDLE_AUTO_WATCH_EVERY;
        return;
    }
}

void c66x_idle_discover(c66x_core *c, uint32_t hint)
{
    if (!c->idle_auto_tab)
        c->idle_auto_tab = calloc(IDLE_AUTO_SLOTS, sizeof c->idle_auto_tab[0]);
    memset(c->idle_auto_bad, 0, sizeof c->idle_auto_bad);
    c->idle_auto_nbad = 0;
    c->idle_auto_base = hint;
    c->idle_auto_backoff = IDLE_AUTO_BACKOFF;
    if (hint != c->idle_head)
        c66x_set_idle_loop(c, hint, 0, 0, 0);
    c->idle_auto_hits = c->idle_info.idle_hits;
    idle_auto_watch(c);
    if (!hint)
        c->idle_auto_at = c->cycle + IDLE_AUTO_FIRST;
}

uint32_t c66x_idle_head(const c66x_core *c, int *confirmed)
{
    if (confirmed)
        *confirmed = c->idle_head && (c->idle_auto == IDLE_AUTO_OFF ? c->idle_info.idle_hits != 0
            : c->idle_auto == IDLE_AUTO_WATCH && c->idle_info.idle_hits > c->idle_auto_hits);
    return c->idle_head;
}

void c66x_idle_idempotent_write(c66x_core *c, uint32_t addr)
{
    if (c->idle_nidem < IDLE_IDEM)
        c->idle_idem[c->idle_nidem++] = addr;
}

void c66x_skip_cycles(c66x_core *c, uint64_t n)
{
    if (c->rec) {
        rec_head(c, REC_SKIP);
        fwrite(&n, 8, 1, c->rec);
    }
    /* Writes in flight land first, in order, as if the cycles had run. */
    for (unsigned d = 0; d < WQ_SLOTS && n; d++) {
        commit_writes(c);
        c->cycle++;
        n--;
    }
    c->cycle += n;
    c->st.cycles = c->cycle;
    /* The loop spun at its head all the while: its pass is no longer for it. */
    c->idle_head_cycle = c->cycle;
}

void c66x_get_idle_info(const c66x_core *c, c66x_idle_info *out) { *out = c->idle_info; }

/* At the loop head: close the iteration that just ended, open the next one.
 * Returns 1 when the ended iteration was a fixed point. */
int idle_check(c66x_core *c)
{
    int idle = 0;
    if (c->idle_stack_pending) {
        /* B15 points at a free word; from there down nothing belongs to the
         * loop's caller. The first arrival only arms, so no pass is judged
         * against a window that was not there yet. */
        c->idle_shi = c->reg[IDLE_SP] + 8;
        c->idle_slo = c->idle_shi - 64 * 1024;
        c->idle_stack_pending = 0;
        c->idle_frame_lo = c->idle_shi;
        c->idle_frame_hi = ram_ptr(c, c->idle_shi, IDLE_FRAME) ? c->idle_shi + IDLE_FRAME : c->idle_shi;
    }
    uint8_t *frame = c->idle_frame_hi > c->idle_frame_lo ? ram_ptr(c, c->idle_frame_lo, IDLE_FRAME) : NULL;
    if (c->idle_armed) {
        c66x_idle_info *ii = &c->idle_info;
        uint8_t why = c->idle_fx;
        if (!why && (c->idle_isr_hit || c->isr_depth))
            why = 4;
        if (c->idle_isr_hit)
            ii->isr_store_hits++;
        if (!why && memcmp(c->reg, c->idle_reg, sizeof c->reg))
            why = 5;
        if (!why && c->idle_frame_dirty && frame && memcmp(frame, c->idle_frame, IDLE_FRAME)) {
            /* a frame word ended the pass different: a store that counts */
            why = 1;
            c->idle_fx_pc = 0;
            for (c->idle_fx_addr = c->idle_frame_lo; frame[c->idle_fx_addr - c->idle_frame_lo]
                 == c->idle_frame[c->idle_fx_addr - c->idle_frame_lo]; c->idle_fx_addr++)
                ;
        }
        if (!why)
            for (unsigned s = 0; s < WQ_SLOTS; s++)
                if (c->wqn[s])
                    why = 7;
        /* Decided here as well as in c66x_step, so the verdict does not
         * depend on where the caller's step boundaries fell. */
        if (!why && c->cycle - c->idle_head_cycle > c->idle_pass_limit)
            why = 8;
        ii->iterations++;
        ii->last_iter_cycles = (uint32_t)(c->cycle - c->idle_head_cycle);
        ii->last_reason = why;
        ii->last_addr = c->idle_fx_addr;
        ii->last_pc = c->idle_fx_pc;
        if (!why) {
            ii->idle_hits++;
            idle = 1;
            c->idle_pass_limit = 4 * (c->cycle - c->idle_head_cycle) + IDLE_PASS_SLACK;
        }
    }
    /* The words a fixed-point pass reads stay in the set; a pass that did
     * something starts it over. */
    if (!idle)
        idle_rs_clear(c);
    c->idle_fp = idle;
    c->idle_armed = 1;
    c->idle_isr_disarmed = 0;
    c->idle_fx = 0;
    c->idle_isr_hit = 0;
    c66x_mem_mode_sync(c);
    c->idle_head_cycle = c->cycle;
    c->idle_head_irqs = c->st.interrupts;
    memcpy(c->idle_reg, c->reg, sizeof c->reg);
    if (frame && (c->idle_frame_dirty || !idle))
        memcpy(c->idle_frame, frame, IDLE_FRAME);
    c->idle_frame_dirty = 0;
    return idle;
}

void c66x_watch_writes(c66x_core *c, uint32_t lo, uint32_t hi, c66x_watch_fn fn, void *opaque)
{
    c->watch_lo = lo;
    c->watch_hi = hi;
    c->watch_fn = fn;
    c->watch_opaque = opaque;
    c66x_mem_mode_sync(c);
}



uint32_t c66x_get_creg(const c66x_core *c, unsigned crlo)
{
    return ctrl_read((c66x_core *)c, crlo, c->pc & ~31u);
}

void c66x_set_creg(c66x_core *c, unsigned crlo, uint32_t v) { ctrl_write_now(c, crlo, v); }
