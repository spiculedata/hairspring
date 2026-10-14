// Deliberately racy: 4 threads append their id to a shared log with no lock ordering.
// Prints an FNV hash of the interleaving. Deterministic runtime => same hash every run.
#include <pthread.h>
#include <stdio.h>
#include <stdint.h>
#define N 4
#define ITERS 20000
static volatile unsigned char logbuf[N * ITERS];
static volatile int idx = 0;
static void *work(void *a) {
    unsigned char id = (unsigned char)(long)a;
    for (int i = 0; i < ITERS; i++) { int j = idx; idx = j + 1; logbuf[j % (N * ITERS)] = id; }
    return 0;
}
int main(void) {
    pthread_t t[N];
    for (long i = 0; i < N; i++) pthread_create(&t[i], 0, work, (void *)i);
    for (int i = 0; i < N; i++) pthread_join(t[i], 0);
    uint64_t h = 1469598103934665603ULL;
    for (int i = 0; i < N * ITERS; i++) { h ^= logbuf[i]; h *= 1099511628211ULL; }
    printf("%016llx idx=%d\n", (unsigned long long)h, idx);
    return 0;
}
