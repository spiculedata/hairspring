package langat;

import java.util.List;

/** What one run of the workload observed. */
record Outcome(
        List<Integer> lockOrder,
        List<Integer> arrival,
        long racyFinal,
        long lockedFinal,
        long elapsedNs,
        List<String> mapOrder,
        long rand) {}
