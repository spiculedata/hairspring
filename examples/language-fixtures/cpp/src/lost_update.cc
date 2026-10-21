// C++ lost-update fixture: at-cxx-clang or at-cxx-gxx buggy|fixed|wide (see ../../common/lang_at.h for the modes).
// Property: cxx.counter_exact (always): the final counter equals threads * iterations, witness values
// [final, 4000, 4, 1000]. Coverage: cxx.all_started (sometimes). Each is evaluated once, in main, after the join.
// Output: one contract line on stdout. Exit status: 0 whatever the property says, 3 for an internal invariant
// failure, 64 for a usage error.
//
// Every Hairspring call site is in an ordinary function (main). With g++ a call site inside a template, a lambda or
// another inline function is not catalogued (see docs/c-sdk.md), so none is used here.
#include <atomic>
#include <cstdint>
#include <mutex>
#include <optional>
#include <system_error>
#include <thread>
#include <vector>

#include "hairspring.h"
#include "lang_at.h"

namespace {

std::atomic<uint64_t> counter{0};
std::atomic<unsigned> started{0};
std::atomic<unsigned> arrivals{0};
std::atomic<bool> abandoned{false};  // set when a worker could not be created, so the others stop waiting
uint8_t order[LANG_AT_THREADS];
std::mutex gate;

// The start barrier: nobody iterates before everybody has started. Returns false if the run was abandoned.
bool wait_for_all_workers() {
  while (started.load(std::memory_order_acquire) < LANG_AT_THREADS) {
    if (abandoned.load(std::memory_order_acquire)) return false;
    std::this_thread::yield();
  }
  return true;
}

void worker(unsigned id, const lang_at_mode *mode) {
  hs_thread("worker", id);
  started.fetch_add(1, std::memory_order_acq_rel);
  if (!wait_for_all_workers()) return;
  for (unsigned i = 0; i < LANG_AT_ITERS; i++) {
    std::optional<std::lock_guard<std::mutex>> hold;  // engaged only in fixed mode
    if (mode->locked) hold.emplace(gate);
    uint64_t seen = counter.load(std::memory_order_relaxed);
    uint64_t pad = lang_at_spin(seen, mode->spin);  // the window: user code only, no system call
    counter.store(seen + 1u, std::memory_order_relaxed);
    hold.reset();
    lang_at_keep(id, pad);
    HS_WORK(1);
  }
  order[arrivals.fetch_add(1, std::memory_order_relaxed)] = static_cast<uint8_t>('0' + id);
}

// Starts the workers and joins the ones that started. Returns the number of workers that were created.
unsigned run_workers(const lang_at_mode *mode) {
  std::vector<std::thread> threads;
  try {
    for (unsigned i = 0; i < LANG_AT_THREADS; i++) threads.emplace_back(worker, i, mode);
  } catch (const std::system_error &) {
    abandoned.store(true, std::memory_order_release);
  }
  for (std::thread &t : threads) t.join();
  return static_cast<unsigned>(threads.size());
}

}  // namespace

int main(int argc, char **argv) {
  const lang_at_mode *mode = lang_at_parse_mode(argc, argv);
  if (mode == nullptr) return LANG_AT_EXIT_USAGE;

  unsigned created = run_workers(mode);
  unsigned finished = arrivals.load(std::memory_order_relaxed);
  if (created != LANG_AT_THREADS || finished != LANG_AT_THREADS) {
    fprintf(stderr, "at-cxx: %u workers created and %u finished, want %u\n", created, finished, LANG_AT_THREADS);
    return LANG_AT_EXIT_INVARIANT;
  }

  uint64_t final_value = counter.load(std::memory_order_relaxed);
  unsigned started_count = started.load(std::memory_order_relaxed);
  HS_ALWAYS(final_value == LANG_AT_EXPECTED, "cxx.counter_exact", "no update was lost", final_value, LANG_AT_EXPECTED,
            LANG_AT_THREADS, LANG_AT_ITERS);
  HS_SOMETIMES(started_count == LANG_AT_THREADS, "cxx.all_started", "every worker started", started_count);
  if (lang_at_print("cxx", mode->name, final_value, lang_at_order_hash(order, LANG_AT_THREADS), lang_at_random()) != 0) {
    fputs("at-cxx: cannot write the result line\n", stderr);
    return LANG_AT_EXIT_INVARIANT;
  }
  return 0;
}
