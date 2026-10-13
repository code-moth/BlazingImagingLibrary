/* SPDX-License-Identifier: LGPL-2.1-or-later */
/*
 * Experiment 3: a description of the process-wide state a C library could change behind BIL's
 * back, so that the checks can compare it before and after thousands of corrupt files.
 *
 * It covers the locale, every signal's handler and flags, the floating-point environment
 * (rounding and, on x86, MXCSR's flush-to-zero and denormals-are-zero bits, which would change
 * every float32 BIL computes), the file mode mask, the working directory, the environment, and
 * the open file descriptors. Heap in use is reported apart, since it may vary a little.
 */

#define _GNU_SOURCE
#include <dirent.h>
#include <fenv.h>
#include <locale.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#if defined(__GLIBC__)
#include <malloc.h>
#endif

extern char **environ;

typedef struct {
    char *buf;
    size_t cap;
    size_t at;
} Out;

static void put(Out *o, const char *fmt, ...) {
    if (o->at >= o->cap) return;
    va_list args;
    va_start(args, fmt);
    int n = vsnprintf(o->buf + o->at, o->cap - o->at, fmt, args);
    va_end(args);
    if (n > 0) o->at += (size_t)n;
    if (o->at > o->cap) o->at = o->cap;
}

/* Writes the description into buf, and returns its length. */
size_t bil_state_describe(char *buf, size_t cap) {
    Out o = {buf, cap, 0};
    /* The exception flags are set by ordinary arithmetic, so they are cleared first: what is
     * compared is the modes, which only a deliberate call changes. FE_ALL_EXCEPT leaves out the
     * flag for a denormal operand (DE in x86's MXCSR and x87 status word, IDC in ARM's FPSR),
     * which libraries' float arithmetic sets on corrupt data, so it is cleared here too. */
    feclearexcept(FE_ALL_EXCEPT);
#if defined(__x86_64__)
    unsigned int csr;
    __asm__ volatile("stmxcsr %0" : "=m"(csr));
    csr &= ~0x3Fu;
    __asm__ volatile("ldmxcsr %0" : : "m"(csr));
    __asm__ volatile("fnclex");
#elif defined(__aarch64__)
    __asm__ volatile("msr fpsr, xzr");
#endif
    fenv_t env;
    fegetenv(&env);
    put(&o, "fenv=");
    const unsigned char *e = (const unsigned char *)&env;
    for (size_t i = 0; i < sizeof env; i++) put(&o, "%02x", e[i]);
    put(&o, "\nrounding=%d\n", fegetround());
    const char *loc = setlocale(LC_ALL, NULL);
    put(&o, "locale=%s\n", loc ? loc : "(none)");
    for (int sig = 1; sig < 32; sig++) {
        struct sigaction sa;
        /* glibc adds SA_RESTORER to every action it installs, even one that restores the
         * default, so that bit says nothing about what the handler does and is left out. */
        if (sigaction(sig, NULL, &sa) == 0)
            put(&o, "signal %d handler=%p flags=%lx\n", sig, (void *)(uintptr_t)sa.sa_handler,
                (unsigned long)sa.sa_flags & ~0x04000000ul);
    }
    mode_t mask = umask(0);
    umask(mask);
    put(&o, "umask=%o\n", (unsigned)mask);
    char cwd[4096];
    put(&o, "cwd=%s\n", getcwd(cwd, sizeof cwd) ? cwd : "?");
    unsigned long long h = 1469598103934665603ULL, count = 0;
    for (char **v = environ; v && *v; v++, count++)
        for (const char *c = *v; *c; c++) h = (h ^ (unsigned char)*c) * 1099511628211ULL;
    put(&o, "environment=%llu variables, hash %llx\n", count, h);
    unsigned long long fds = 0;
    DIR *d = opendir("/proc/self/fd");
    if (!d) d = opendir("/dev/fd");
    if (d) {
        for (struct dirent *ent; (ent = readdir(d));)
            if (ent->d_name[0] != '.') fds++;
        closedir(d);
    }
    /* The directory itself was one of them while it was open. */
    put(&o, "open descriptors=%llu\n", fds ? fds - 1 : 0);
    return o.at;
}

/* Bytes of heap in use, where the C library reports it; 0 where it does not. */
unsigned long long bil_heap_in_use(void) {
#if defined(__GLIBC__)
    struct mallinfo2 m = mallinfo2();
    return (unsigned long long)m.uordblks + (unsigned long long)m.hblkhd;
#else
    return 0;
#endif
}

/*
 * For the check of the check: makes one change of the kind the description must notice, or
 * undoes it. 1 sets flush-to-zero, 2 installs a signal handler, 3 changes the file mode mask.
 * Returns 0 where the change cannot be made on this system.
 */
static void bil_ignore(int sig) { (void)sig; }

int bil_state_poke(int what, int undo) {
    static mode_t old_mask;
    static struct sigaction old_action;
    switch (what) {
    case 1: {
#if defined(__x86_64__)
        unsigned int csr;
        __asm__ volatile("stmxcsr %0" : "=m"(csr));
        csr = undo ? (csr & ~0x8000u) : (csr | 0x8000u);
        __asm__ volatile("ldmxcsr %0" : : "m"(csr));
        return 1;
#elif defined(__aarch64__)
        unsigned long fpcr;
        __asm__ volatile("mrs %0, fpcr" : "=r"(fpcr));
        fpcr = undo ? (fpcr & ~(1ul << 24)) : (fpcr | (1ul << 24));
        __asm__ volatile("msr fpcr, %0" : : "r"(fpcr));
        return 1;
#else
        return 0;
#endif
    }
    case 2: {
        if (undo) return sigaction(SIGUSR2, &old_action, NULL) == 0;
        struct sigaction sa;
        memset(&sa, 0, sizeof sa);
        sa.sa_handler = bil_ignore;
        return sigaction(SIGUSR2, &sa, &old_action) == 0;
    }
    case 3:
        if (undo) {
            umask(old_mask);
        } else {
            old_mask = umask(0077);
            if (old_mask == 0077) umask(0022);
        }
        return 1;
    default:
        return 0;
    }
}
