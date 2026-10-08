/* B B3 + five delay slots: measure actual compiled dispatch CPU cost. */
#include "c66x_priv.h"
#include <assert.h>
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static double cpu_now(void)
{
    struct timespec t;
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &t);
    return t.tv_sec + t.tv_nsec / 1e9;
}

int main(int argc, char **argv)
{
    assert(argc == 3);
    void *library = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    assert(library);
    c66x_core *(*create)(const c66x_bus *) = dlsym(library, "c66x_new");
    void (*map)(c66x_core *, uint32_t, uint32_t, uint8_t *) = dlsym(library, "c66x_map_ram");
    void (*reset)(c66x_core *, uint32_t) = dlsym(library, "c66x_reset");
    c66x_stop (*step)(c66x_core *, uint64_t, uint64_t *) = dlsym(library, "c66x_step");
    void (*destroy)(c66x_core *) = dlsym(library, "c66x_free");
    uint64_t (*hash)(const c66x_core *) = dlsym(library, "c66x_state_hash");
    assert(create && map && reset && step && destroy && hash);
    uint8_t ram[1024] = {0}, oracle_ram[1024] = {0};
    memcpy(ram, "\x62\x03\x0c\x00", 4);
    memcpy(ram + 32, "\x62\x03\x0c\x00", 4);
    memcpy(oracle_ram, ram, sizeof ram);
    unsetenv("C66X_JIT");
    c66x_core *oracle = create(NULL);
    map(oracle, 0x11800000, sizeof ram, oracle_ram);
    reset(oracle, 0x11800000); oracle->reg[35] = 0x11800020;
    setenv("C66X_JIT", argv[2], 1);
    c66x_core *c = create(NULL);
    map(c, 0x11800000, sizeof ram, ram);
    reset(c, 0x11800000); c->reg[35] = 0x11800020;
    uint64_t done;
    assert(step(c, 1024, &done) == C66X_STOP_BUDGET);
    assert(step(oracle, 1024, &done) == C66X_STOP_BUDGET);
    assert(hash(c) == hash(oracle));
    for (unsigned sample = 0; sample < 15; sample++) {
        uint64_t exits = c->jit_exit[1];
        double start = cpu_now();
        assert(step(c, 12000000, &done) == C66X_STOP_BUDGET && done == 12000000);
        double elapsed = cpu_now() - start;
        printf("{\"sample\":%u,\"cpu_ns_per_cycle\":%.4f,\"dynamic_exits\":%llu}\n",
               sample, elapsed * 1e9 / done, (unsigned long long)(c->jit_exit[1] - exits));
    }
    destroy(c); destroy(oracle); dlclose(library);
}
