/* Synthetic DMA ranges through the real DSP core; no firmware/audio. */
#include "c66x_core_int.h"
#include <assert.h>
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static double now(void)
{
    struct timespec t;
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}

int main(int argc, char **argv)
{
    assert(argc == 2);
    void *lib = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    assert(lib);
    c66x_core *(*new_core)(const c66x_bus *) = dlsym(lib, "c66x_new");
    void (*free_core)(c66x_core *) = dlsym(lib, "c66x_free");
    void (*map)(c66x_core *, uint32_t, uint32_t, void *) = dlsym(lib, "c66x_map_ram");
    c66x_insn *(*fetch)(c66x_core *, uint32_t) = dlsym(lib, "fetch_insn");
    void (*invalidate)(c66x_core *, uint32_t, uint32_t) = dlsym(lib, "c66x_invalidate");
    assert(new_core && free_core && map && fetch && invalidate);
    const unsigned ranges[] = {1, 16, 128};
    for (unsigned range = 0; range < 3; range++) {
        unsigned blocks = ranges[range];
        uint8_t ram[8192] = {0};
        c66x_core *c = new_core(NULL);
        map(c, 0x10000, sizeof ram, ram);
        map(c, 0x20000, sizeof ram, ram);
        /* Both addresses decode the same underlying bytes. Warm-up and
         * re-decoding stay outside the timed notification. */
        for (unsigned sample = 0; sample < 15; sample++) {
            for (unsigned i = 0; i < blocks; i++) {
                assert(fetch(c, 0x10000 + 32 * i));
                assert(fetch(c, 0x20000 + 32 * i));
            }
            memset(c->pkc, 1, sizeof c->pkc);
            uint64_t generation = c->code_gen;
            double start = now();
            invalidate(c, 0x20000, blocks * 32);
            double elapsed = now() - start;
            for (unsigned alias = 0; alias < 2; alias++)
                for (unsigned i = 0; i < blocks; i++)
                    assert(!c->ram[alias].fpp[0][i]->valid);
            for (unsigned i = 0; i < PKC_SIZE; i++)
                assert(!c->pkc[i].first);
            printf("{\"blocks\":%u,\"aliases\":2,\"sample\":%u,\"ns\":%.0f,\"generations\":%llu,\"cache_bytes\":%zu}\n",
                   blocks, sample, elapsed * 1e9,
                   (unsigned long long)(c->code_gen - generation), sizeof c->pkc);
        }
        free_core(c);
    }
    uint8_t data[8192] = {0};
    c66x_core *c = new_core(NULL);
    map(c, 0x10000, sizeof data, data);
    for (unsigned sample = 0; sample < 15; sample++) {
        double start = now();
        for (unsigned i = 0; i < 1000000; i++) invalidate(c, 0x10000, 4);
        printf("{\"blocks\":0,\"sample\":%u,\"ns\":%.2f}\n", sample,
               (now() - start) * 1e9 / 1000000);
    }
    assert(!c->code_gen);
    free_core(c);
    dlclose(lib);
}
