/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef C66X_FENV_H
#define C66X_FENV_H
#include <fenv.h>
#include <stdint.h>

/* ARM FPCR RMode [23:22], Arm 100446 section 2.3. Darwin's FE_* encodes
 * exactly these bits. Preserve exception enables, NaN/denormal modes and
 * FPSR flags. Other hosts retain libc fenv semantics. Compile with
 * -frounding-math and -ffp-contract=off, including generated DSP regions. */
static inline int c66x_fesetround(int mode)
{
#if defined(__APPLE__) && defined(__aarch64__)
    const uint64_t mask = UINT64_C(3) << 22;
    uint64_t control;
    if ((uint64_t)mode & ~mask) { return -1; }
    __asm__ volatile("mrs %0, fpcr" : "=r"(control) : : "memory");
    control = (control & ~mask) | (uint64_t)mode;
    __asm__ volatile("msr fpcr, %0" : : "r"(control) : "memory");
    return 0;
#else
    return fesetround(mode);
#endif
}

/* A compiled DSP batch can keep its rounding mode between operations. Avoid
 * serializing the ARM FP pipeline when that mode is already selected. */
static inline int c66x_fesetround_checked(int mode)
{
#if defined(__APPLE__) && defined(__aarch64__)
    const uint64_t mask = UINT64_C(3) << 22;
    uint64_t control;
    if ((uint64_t)mode & ~mask) { return -1; }
    __asm__ volatile("mrs %0, fpcr" : "=r"(control) : : "memory");
    if ((control & mask) != (uint64_t)mode) {
        control = (control & ~mask) | (uint64_t)mode;
        __asm__ volatile("msr fpcr, %0" : : "r"(control) : "memory");
    }
    return 0;
#else
    return fegetround() == mode ? 0 : fesetround(mode);
#endif
}

static inline int c66x_fegetround(void)
{
#if defined(__APPLE__) && defined(__aarch64__)
    uint64_t control;
    __asm__ volatile("mrs %0, fpcr" : "=r"(control) : : "memory");
    return (int)(control & (UINT64_C(3) << 22));
#else
    return fegetround();
#endif
}
#endif
