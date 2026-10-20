/* cl_plant.h -- compile-time planted bugs for the detector self-tests. Counted binaries define none of them.
 *
 *   -DCONDLOCK_DROP_SIGNAL=k   from the k-th release whose blocked set is non-empty, pthread_cond_signal on the lock
 *                              condvar is skipped for that release and every later one (the condvar becomes deaf);
 *                              each skipped signal is still recorded as a release
 *   -DCONDLOCK_PLANT_LEAK=k    the k-th release skips clearing is_locked (a leaked lock; stall subtype L)
 *   -DCONDLOCK_PLANT_BARRIER=k the k-th last arriver at the barrier skips the broadcast (stall subtype B)
 *   -DCONDLOCK_PLANT_HOLDG=k   the k-th lock acquisition sleeps for CL_HOLDG_SLEEP_MS while holding the lock's own mutex G,
 *                              so the watchdog cannot take G for a whole tick (stall subtype M)
 *
 * `applied` (printed on stdout, planted builds only) says whether the plant took effect in this run.
 */
#ifndef CL_PLANT_H
#define CL_PLANT_H

#if (defined(CONDLOCK_DROP_SIGNAL) + defined(CONDLOCK_PLANT_LEAK) + defined(CONDLOCK_PLANT_BARRIER) + \
     defined(CONDLOCK_PLANT_HOLDG)) > 1
#error "at most one planted bug per binary"
#endif

/* Long enough that the watchdog's quorum (Q ticks) and a full failed trylock tick pass while G is held. */
#define CL_HOLDG_SLEEP_MS 30000u

#if defined(CONDLOCK_DROP_SIGNAL) || defined(CONDLOCK_PLANT_LEAK) || defined(CONDLOCK_PLANT_BARRIER) || \
    defined(CONDLOCK_PLANT_HOLDG)
#define CL_PLANTED 1
#else
#define CL_PLANTED 0
#endif

#endif
