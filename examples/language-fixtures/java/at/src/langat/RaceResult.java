package langat;

/** What a run of the workers leaves: the final counter, how many workers started and the order they finished in. */
record RaceResult(long finalValue, int started, int[] arrival) {}
