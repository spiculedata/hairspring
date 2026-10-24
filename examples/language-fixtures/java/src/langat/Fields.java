package langat;

/** The five values of the line that are not fixed text, in the order they are printed. */
record Fields(long lockOrder, long arrival, long rand, long vtimeNs, long extra) {}
