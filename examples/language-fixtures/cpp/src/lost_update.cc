// C++ lost-update fixture. Build -DLANG_AT_LOCKED for the locked twin; without it, the racy program.
// Property: cpp.counter_exact (always): the final counter equals threads * iterations. Coverage: cpp.all_started.
// Output: one fixed-width line on stdout. Exit status: 0, whatever the property says.
//
// Every Hairspring call site is in an ordinary function (main). With g++ a call site inside a template, a lambda or
// another inline function is not catalogued (see docs/c-sdk.md), so none is used here.
#include <atomic>
#include <cstdint>
#include <mutex>
#include <thread>
#include <vector>

#include "hairspring.h"
#include "lang_at.h"

namespace {

std::atomic<uint64_t> counter{0};
std::atomic<unsigned> started{0};
std::atomic<unsigned> arrivals{0};
uint8_t order[LANG_AT_THREADS];

#ifdef LANG_AT_LOCKED
std::mutex gate;
// Holds the mutex for the whole read-spin-write section.
struct Section {
  std::lock_guard<std::mutex> hold{gate};
};
#else
// The racy build takes no lock; the destructor keeps the unused-variable warning quiet.
struct Section {
  ~Section() {}
};
#endif

void worker(unsigned id) {
  hs_thread("worker", id);
  started.fetch_add(1, std::memory_order_relaxed);
  for (unsigned i = 0; i < LANG_AT_ITERS; i++) {
    {
      Section section;
      uint64_t seen = counter.load(std::memory_order_relaxed);
      (void)lang_at_spin(seen, LANG_AT_SPIN);  // the window: user code only, no system call
      counter.store(seen + 1u, std::memory_order_relaxed);
    }
    HS_WORK(1);
  }
  order[arrivals.fetch_add(1, std::memory_order_relaxed)] = static_cast<uint8_t>(id);
}

}  // namespace

int main() {
  std::vector<std::thread> threads;
  for (unsigned i = 0; i < LANG_AT_THREADS; i++) threads.emplace_back(worker, i);
  for (std::thread &t : threads) t.join();

  uint64_t final_value = counter.load(std::memory_order_relaxed);
  unsigned started_count = started.load(std::memory_order_relaxed);
  HS_SOMETIMES(started_count == LANG_AT_THREADS, "cpp.all_started", "every worker started", started_count);
  HS_ALWAYS(final_value == LANG_AT_EXPECTED, "cpp.counter_exact", "no update was lost", final_value,
            LANG_AT_EXPECTED);
  lang_at_print("cpp", final_value, lang_at_order_hash(order, LANG_AT_THREADS), lang_at_random());
  return 0;
}
