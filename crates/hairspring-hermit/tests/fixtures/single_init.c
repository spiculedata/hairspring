// The fixed twin: the check and the act are one critical section.
#include <pthread.h>
#include <sched.h>
#include <stdio.h>

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static int initialized = 0;
static int init_count = 0;

static void *worker(void *arg) {
    (void)arg;
    pthread_mutex_lock(&lock);
    if (!initialized) {
        sched_yield();
        init_count++;
        initialized = 1;
    }
    pthread_mutex_unlock(&lock);
    return 0;
}

int main(void) {
    pthread_t a, b;
    pthread_create(&a, 0, worker, 0);
    pthread_create(&b, 0, worker, 0);
    pthread_join(a, 0);
    pthread_join(b, 0);
    if (init_count != 1) {
        printf("HAIRSPRING_VIOLATION: initialised %d times\n", init_count);
        return 1;
    }
    puts("ok");
    return 0;
}
