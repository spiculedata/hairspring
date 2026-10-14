// Check-then-act race: both threads can pass the `initialized` check before either sets it.
// Whether that happens depends on the thread schedule, which Hermit controls by seed.
#include <pthread.h>
#include <sched.h>
#include <stdio.h>

static volatile int initialized = 0;
static volatile int init_count = 0;

static void *worker(void *arg) {
    (void)arg;
    if (!initialized) {
        sched_yield(); // a scheduling point between the check and the act
        init_count++;
        initialized = 1;
    }
    return 0;
}

int main(void) {
    pthread_t a, b;
    pthread_create(&a, 0, worker, 0);
    pthread_create(&b, 0, worker, 0);
    pthread_join(a, 0);
    pthread_join(b, 0);
    if (init_count != 1) {
        printf("ANTHILL_VIOLATION: initialised %d times\n", init_count);
        return 1;
    }
    puts("ok");
    return 0;
}
