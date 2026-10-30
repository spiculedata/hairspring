# Java probe (no SDK, no property): four platform threads, a lock, an unlocked counter, a queue and a map, one stdout line.
#
#   make jar      build/lang-at.jar (javac --release 21, reproducible jar)
#   make test     the self-test of the classes, then the native tests of the jar
#   make flags    each flag profile of profiles.tsv: valid JVM options, one clean line, nothing else on stdout
#   make clean
#
# The lost-update fixture (class langat.Main, the Java SDK in sdks/java shaded in; sources in at/) has its own targets:
#
#   make at-jar       build/at/lang-at.jar (the SDK, the probe and the fixture, one reproducible jar)
#   make at-test      the fixture's self-test, source check, native self-checks (default flags and each profile's), the
#                     PrintCompilation check and the two JVM flag profiles
#   make at-selftest  only the self-test of the fixture's classes
#   make at-k         only the native tests of the --k=N argument (iterations per worker)
#   make at-native    only the native self-checks (see ../README.md) and the negative control, with the default JVM flags
#   make at-profile-native  the same self-checks once under each profile's JVM flags
#   make at-extra-native    the same self-checks once under each profile of profiles-extra.tsv (a-prime: profile a with
#                     -Xshare:auto in place of -Xshare:off); not part of at-test
#
# The native self-check targets take an optional AT_K (make at-native AT_K=50): the wrapper then passes --k=50 and the self-check
# expects 4 x K (AT_K is 1 to MAX_K). Without AT_K every command, the wrapper and every expectation is the default (K = 1,000).
#   make at-compiled  -XX:+PrintCompilation: neither worker loop is ever compiled, with the default flags and each profile
#                     of profiles.tsv and of profiles-extra.tsv
#   make at-arms      each JVM flag profile of profiles.tsv and profiles-extra.tsv: valid flags, a clean run of fixed, how
#                     often buggy fires
#
# The service fixture (class langsvc.Main, the Java SDK shaded in; sources in svc/): a small in-process payments service whose
# idempotency check is racy in the buggy build, so a transfer delivered twice can be applied twice (final above 4 x K):
#
#   make svc-jar           build/svc/svc.jar (the SDK and the service, one reproducible jar)
#   make svc-test          the self-test, the source checks, then the native self-checks with the default flags and under
#                          profile a-prime (profiles-extra.tsv)
#   make svc-selftest      only the self-test of the service's classes
#   make svc-sources       no seed, /proc, CPU count, environment or clock read, except the two monotonic clock reads in
#                          Deadline.java (request deadlines)
#   make svc-window        the order of the check, the validation, the apply and the record in Store.handle
#   make svc-native        the native self-checks (LANG_AT_DIRECTION=up) and the negative control, with the default flags
#   make svc-profile-native / svc-extra-native   the same under each profile of profiles.tsv / profiles-extra.tsv
#
# The svc native targets take AT_K as the at- targets do (make svc-native AT_K=50).
#
# Needs a JDK 21: JAVA_HOME selects it (the acceptance run pins Temurin 21.0.12.1+1; `java -version` shows which one this is).
# SOURCE_DATE is the fixed timestamp of every jar entry. Nothing here runs Hermit.

ifneq ($(JAVA_HOME),)
JAVA_BIN := $(JAVA_HOME)/bin/
endif
JAVAC ?= $(JAVA_BIN)javac
JAR ?= $(JAVA_BIN)jar
JAVA ?= $(JAVA_BIN)java
SOURCE_DATE ?= 2026-01-01T00:00:00Z
BUILD ?= build

JAVAC_FLAGS = --release 21 -Xlint:all -Werror -encoding UTF-8 -proc:none
SOURCES := $(sort $(wildcard src/langat/*.java))
TEST_SOURCES := $(sort $(wildcard test/langat/*.java))

.PHONY: jar test selftest native flags clean FORCE at-jar at-test at-selftest at-sources at-k at-native at-profile-native at-max-k-check at-compiled at-arms at-extra-native svc-jar svc-test svc-selftest svc-sources svc-window svc-native svc-profile-native svc-extra-native

jar: $(BUILD)/lang-at.jar

# What the classes are built with: the compiler flags and the compiler's version. A change of either rebuilds everything.
BUILD_ID := $(JAVAC_FLAGS) $(shell $(JAVAC) -version 2>&1)

FORCE:

$(BUILD)/build-id: FORCE
	@mkdir -p $(BUILD)
	@echo '$(BUILD_ID)' | cmp -s - $@ || echo '$(BUILD_ID)' >$@

$(BUILD)/classes/.built: $(SOURCES) $(BUILD)/build-id
	rm -rf $(BUILD)/classes
	mkdir -p $(BUILD)/classes
	$(JAVAC) $(JAVAC_FLAGS) -d $(BUILD)/classes $(SOURCES)
	touch $@

# The entries are listed one by one in sorted order (no directory entries), so the jar does not depend on the order the
# file system lists the class files in; --date fixes every timestamp. manifest.mf names langat.Probe as the Main-Class
# because this jar holds the probe alone: a jar that also holds another program must not name langat.Probe as its Main-Class.
$(BUILD)/lang-at.jar: $(BUILD)/classes/.built manifest.mf
	(cd $(BUILD)/classes && find langat -type f -name '*.class' | LC_ALL=C sort | sed 's|^|-C classes |') >$(BUILD)/jar.args
	cd $(BUILD) && $(JAR) --create --file lang-at.jar --date=$(SOURCE_DATE) --manifest $(CURDIR)/manifest.mf @jar.args

$(BUILD)/test-classes/.built: $(BUILD)/classes/.built $(BUILD)/build-id $(TEST_SOURCES)
	rm -rf $(BUILD)/test-classes
	mkdir -p $(BUILD)/test-classes
	$(JAVAC) $(JAVAC_FLAGS) -cp $(BUILD)/classes -d $(BUILD)/test-classes $(TEST_SOURCES)
	touch $@

test: selftest native

selftest: $(BUILD)/test-classes/.built
	$(JAVA) -cp $(BUILD)/classes:$(BUILD)/test-classes langat.SelfTest

native: $(BUILD)/lang-at.jar $(BUILD)/test-classes/.built
	JAVA="$(JAVA)" sh test/test-probe.sh $(BUILD)/lang-at.jar $(BUILD)/test-classes

flags: $(BUILD)/lang-at.jar
	JAVA="$(JAVA)" sh ../scripts/check-java-profiles.sh profiles.tsv $(BUILD)/lang-at.jar

# ---- the lost-update fixture: lang-at.jar with langat.Main -------------------------------------------------------------
#
# The Java SDK is shaded into the jar: its classes, the probe's (a jar that holds langat.Main and langat.Probe) and the
# fixture's go through one javac run into one directory, so the jar holds no module descriptor and is run on the class
# path. The manifest names langat.Main as the Main-Class. The probe's own jar (above) is a different artifact and has no
# SDK in it.

AT_BUILD ?= $(BUILD)/at
# The largest --k=N the fixture accepts (Contract.MAX_ITERATIONS), which at-compiled also runs.
MAX_K ?= $(shell sed -n 's/.*MAX_ITERATIONS = \([0-9]*\);.*/\1/p' at/src/langat/Contract.java)
RUNS ?= 50
# AT_K, when set on the command line, is the iterations per worker the native self-checks run at: the wrapper passes
# --k=AT_K and the check expects 4 x AT_K. Unset or empty is the default run: no argument, K = 1,000, and the wrapper
# below is the one it always was. It is validated where AT_JAVA is defined, below.
ARM_RUNS ?= 5
SDK_DIR := ../../../sdks/java
AT_SOURCES := $(sort $(wildcard $(SDK_DIR)/src/hairspring/*.java)) $(SOURCES) $(sort $(wildcard at/src/langat/*.java))
AT_TEST_SOURCES := $(sort $(wildcard at/test/langat/*.java))

at-jar: $(AT_BUILD)/lang-at.jar

$(AT_BUILD)/build-id: FORCE
	@mkdir -p $(AT_BUILD)
	@echo '$(BUILD_ID)' | cmp -s - $@ || echo '$(BUILD_ID)' >$@

$(AT_BUILD)/classes/.built: $(AT_SOURCES) $(AT_BUILD)/build-id
	rm -rf $(AT_BUILD)/classes
	mkdir -p $(AT_BUILD)/classes
	$(JAVAC) $(JAVAC_FLAGS) -d $(AT_BUILD)/classes $(AT_SOURCES)
	touch $@

# The entries are listed one by one in sorted order, as for the probe's jar.
$(AT_BUILD)/lang-at.jar: $(AT_BUILD)/classes/.built at/manifest.mf
	(cd $(AT_BUILD)/classes && find hairspring langat -type f -name '*.class' | LC_ALL=C sort | sed 's|^|-C classes |') >$(AT_BUILD)/jar.args
	cd $(AT_BUILD) && $(JAR) --create --file lang-at.jar --date=$(SOURCE_DATE) --manifest $(CURDIR)/at/manifest.mf @jar.args

# The program as the self-checks run it: a command that takes the mode as its first argument. JAVA_FLAGS (read when it
# runs) are extra JVM flags, which is how the two flag profiles are tried.
# With AT_K the wrapper is a separate file, at-java-k<AT_K>, so a wrapper made for another K is never reused.
# Only the make command line sets AT_K (make at-native AT_K=50): an AT_K exported in the environment is ignored, so it cannot
# change the targets the build record builds. The value is checked in make itself, before it reaches a shell: digits only,
# no leading zero, no space, 1 to MAX_K.
ifeq ($(origin AT_K),environment)
AT_K :=
endif
empty :=
space := $(empty) $(empty)
digits_left = $(subst 9,,$(subst 8,,$(subst 7,,$(subst 6,,$(subst 5,,$(subst 4,,$(subst 3,,$(subst 2,,$(subst 1,,$(subst 0,,$(1)))))))))))
ifneq ($(subst $(space),S,$(AT_K)),)
ifneq ($(call digits_left,$(subst $(space),S,$(AT_K))),)
$(error AT_K='$(AT_K)' is not a number from 1 to $(MAX_K): digits only)
endif
ifneq ($(filter 0%,$(AT_K)),)
$(error AT_K='$(AT_K)' is not a number from 1 to $(MAX_K): no leading zero)
endif
ifneq ($(shell [ $(AT_K) -ge 1 ] && [ $(AT_K) -le '$(MAX_K)' ] 2>/dev/null && echo ok),ok)
$(error AT_K='$(AT_K)' is not a number from 1 to $(MAX_K) (Contract.MAX_ITERATIONS))
endif
endif
AT_JAVA := $(AT_BUILD)/at-java$(if $(AT_K),-k$(AT_K))
AT_K_ARG := $(if $(AT_K), --k=$(AT_K))
AT_K_ENV := $(if $(AT_K),LANG_AT_K=$(AT_K) )

$(AT_JAVA): $(AT_BUILD)/lang-at.jar
	printf '#!/bin/sh\nexec "%s" $$JAVA_FLAGS -cp "%s" langat.Main "$$@"$(AT_K_ARG)\n' "$(JAVA)" "$(abspath $(AT_BUILD)/lang-at.jar)" >$@
	chmod +x $@

$(AT_BUILD)/test-classes/.built: $(AT_BUILD)/classes/.built $(AT_BUILD)/build-id $(AT_TEST_SOURCES)
	rm -rf $(AT_BUILD)/test-classes
	mkdir -p $(AT_BUILD)/test-classes
	$(JAVAC) $(JAVAC_FLAGS) -cp $(AT_BUILD)/classes -d $(AT_BUILD)/test-classes $(AT_TEST_SOURCES)
	touch $@

at-test: at-selftest at-sources at-k at-native at-profile-native at-compiled at-arms

# Run with HAIRSPRING_EVENTS=off so the SDK writes no report for the evaluations the tests make.
at-selftest: $(AT_BUILD)/test-classes/.built
	HAIRSPRING_EVENTS=off $(JAVA) -cp $(AT_BUILD)/classes:$(AT_BUILD)/test-classes langat.AtSelfTest

# No read of the seed, /proc, the CPU count, the environment or the clock in the fixture's own sources.
at-sources:
	sh ../scripts/check-sources-java.sh at/src

# The --k=N argument, natively: the default run is unchanged, --k=50 shortens it, invalid values exit 64.
at-k: $(AT_BUILD)/lang-at.jar
	JAVA="$(JAVA)" sh at/test/test-k.sh $(AT_BUILD)/lang-at.jar

# The contract, natively, 50 runs of buggy and fixed (buggy must fire at least once, fixed never), wide as a contract
# check, and a negative control: the racy program standing in for fixed must be rejected.
at-native: $(AT_JAVA)
	$(AT_K_ENV)sh ../scripts/run-checks.sh java $(AT_JAVA) $(RUNS)

# The same self-checks under each profile's JVM flags (the at-java wrapper reads JAVA_FLAGS when it runs).
at-profile-native: $(AT_JAVA)
	$(AT_K_ENV)sh ../scripts/run-java-profile-checks.sh profiles.tsv $(AT_JAVA) $(RUNS)

# The same, under each profile of profiles-extra.tsv (a-prime, which uses the JVM's default class-data archive).
at-extra-native: $(AT_JAVA)
	$(AT_K_ENV)sh ../scripts/run-java-profile-checks.sh profiles-extra.tsv $(AT_JAVA) $(RUNS)

# A cap that could not be read (Contract.java reformatted) must fail the check, never skip the largest-K runs.
at-max-k-check:
	@case "$(MAX_K)" in '' | *[!0-9]* | 0) echo "FAIL MAX_K is '$(MAX_K)': could not read MAX_ITERATIONS from at/src/langat/Contract.java" >&2; exit 1 ;; esac

# The JIT never compiled the two worker loops, at the default K and at the largest --k (see Race.java and ../scripts/check-java-compiled.sh).
at-compiled: at-max-k-check $(AT_BUILD)/lang-at.jar
	JAVA="$(JAVA)" sh ../scripts/check-java-compiled.sh profiles.tsv $(AT_BUILD)/lang-at.jar $(MAX_K)
	JAVA="$(JAVA)" sh ../scripts/check-java-compiled.sh profiles-extra.tsv $(AT_BUILD)/lang-at.jar $(MAX_K)

at-arms: $(AT_BUILD)/lang-at.jar
	JAVA="$(JAVA)" ARM_RUNS=$(ARM_RUNS) sh ../scripts/check-java-arms.sh profiles.tsv $(AT_BUILD)/lang-at.jar
	JAVA="$(JAVA)" ARM_RUNS=$(ARM_RUNS) sh ../scripts/check-java-arms.sh profiles-extra.tsv $(AT_BUILD)/lang-at.jar

# ---- the service fixture: svc.jar with langsvc.Main ------------------------------------------------------------------
#
# The Java SDK is shaded in, as for lang-at.jar: one javac run, one directory, no module descriptor, run on the class path.

SVC_BUILD ?= $(BUILD)/svc
SVC_SOURCES := $(sort $(wildcard $(SDK_DIR)/src/hairspring/*.java)) $(sort $(wildcard svc/src/langsvc/*.java))
SVC_TEST_SOURCES := $(sort $(wildcard svc/test/langsvc/*.java))

svc-jar: $(SVC_BUILD)/svc.jar

$(SVC_BUILD)/build-id: FORCE
	@mkdir -p $(SVC_BUILD)
	@echo '$(BUILD_ID)' | cmp -s - $@ || echo '$(BUILD_ID)' >$@

$(SVC_BUILD)/classes/.built: $(SVC_SOURCES) $(SVC_BUILD)/build-id
	rm -rf $(SVC_BUILD)/classes
	mkdir -p $(SVC_BUILD)/classes
	$(JAVAC) $(JAVAC_FLAGS) -d $(SVC_BUILD)/classes $(SVC_SOURCES)
	touch $@

# The entries are listed one by one in sorted order, as for the other jars.
$(SVC_BUILD)/svc.jar: $(SVC_BUILD)/classes/.built svc/manifest.mf
	(cd $(SVC_BUILD)/classes && find hairspring langsvc -type f -name '*.class' | LC_ALL=C sort | sed 's|^|-C classes |') >$(SVC_BUILD)/jar.args
	cd $(SVC_BUILD) && $(JAR) --create --file svc.jar --date=$(SOURCE_DATE) --manifest $(CURDIR)/svc/manifest.mf @jar.args

# The program as the self-checks run it, as the at-java wrapper: JAVA_FLAGS are read when it runs; AT_K adds --k=AT_K.
SVC_JAVA := $(SVC_BUILD)/svc-java$(if $(AT_K),-k$(AT_K))

$(SVC_JAVA): $(SVC_BUILD)/svc.jar
	printf '#!/bin/sh\nexec "%s" $$JAVA_FLAGS -cp "%s" langsvc.Main "$$@"$(AT_K_ARG)\n' "$(JAVA)" "$(abspath $(SVC_BUILD)/svc.jar)" >$@
	chmod +x $@

$(SVC_BUILD)/test-classes/.built: $(SVC_BUILD)/classes/.built $(SVC_BUILD)/build-id $(SVC_TEST_SOURCES)
	rm -rf $(SVC_BUILD)/test-classes
	mkdir -p $(SVC_BUILD)/test-classes
	$(JAVAC) $(JAVAC_FLAGS) -cp $(SVC_BUILD)/classes -d $(SVC_BUILD)/test-classes $(SVC_TEST_SOURCES)
	touch $@

svc-test: svc-selftest svc-sources svc-window svc-native svc-extra-native

svc-selftest: $(SVC_BUILD)/test-classes/.built
	HAIRSPRING_EVENTS=off $(JAVA) -cp $(SVC_BUILD)/classes:$(SVC_BUILD)/test-classes langsvc.SvcSelfTest

# Every source but Deadline.java with no clock read at all; Deadline.java with exactly its two monotonic clock reads.
svc-sources:
	sh ../scripts/check-sources-java.sh $(filter-out %/Deadline.java,$(sort $(wildcard svc/src/langsvc/*.java)))
	sh ../scripts/check-sources-java.sh --allow-nano-time svc/src/langsvc/Deadline.java

svc-window:
	sh ../scripts/check-svc-window.sh svc/src/langsvc/Store.java

svc-native: $(SVC_JAVA)
	LANG_AT_DIRECTION=up $(AT_K_ENV)sh ../scripts/run-checks.sh java $(SVC_JAVA) $(RUNS)

svc-profile-native: $(SVC_JAVA)
	LANG_AT_DIRECTION=up $(AT_K_ENV)sh ../scripts/run-java-profile-checks.sh profiles.tsv $(SVC_JAVA) $(RUNS)

svc-extra-native: $(SVC_JAVA)
	LANG_AT_DIRECTION=up $(AT_K_ENV)sh ../scripts/run-java-profile-checks.sh profiles-extra.tsv $(SVC_JAVA) $(RUNS)

clean:
	rm -rf $(BUILD)
