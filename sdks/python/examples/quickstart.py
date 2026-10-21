"""A small instrumented program: `PYTHONPATH=sdks/python/src python3 sdks/python/examples/quickstart.py`.

It prints the report to stderr when it exits. `HAIRSPRING_EVENTS=off` silences it.
"""

import threading

import hairspring as hs

# IMPORTANT: a site that never runs is invisible (not "unmet") unless it is declared. Declare every site whose absence
# matters, once, at import time, like this one (it is never evaluated below, and the report still lists it).
hs.declare(hs.Kind.REACHABLE, "quickstart.recovery.ran", "recovery ran")

balance = 100
lock = threading.Lock()


def spend(amount: int) -> None:
    global balance
    hs.thread("spender", amount)
    with lock:
        balance -= amount
        hs.always(balance >= 0, "quickstart.balance_non_negative", "no overdraft", balance, amount)
    hs.work(1)


workers = [threading.Thread(target=spend, args=(amount,)) for amount in (30, 40, 50)]
for worker in workers:
    worker.start()
for worker in workers:
    worker.join()

hs.sometimes(balance < 0, "quickstart.overdrawn", "the account went negative", balance)
