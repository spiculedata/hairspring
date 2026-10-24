package hairspring;

/** The SDK's own tests: no framework, one process. Exits 0 when every case passed, 1 otherwise. */
public final class SelfTest {
    private SelfTest() {}

    static Suite suite() {
        Suite suite = new Suite();
        FmtTests.register(suite);
        CatalogTests.register(suite);
        LinesTests.register(suite);
        IdentityTests.register(suite);
        SlotsTests.register(suite);
        SitesTests.register(suite);
        OutputTests.register(suite);
        SecureTests.register(suite);
        RecorderTests.register(suite);
        return suite;
    }

    public static void main(String[] args) {
        int failed = suite().run(System.out);
        System.exit(failed == 0 ? 0 : 1);
    }
}
