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
#   make at-test      the fixture's self-test, source check, native self-checks and the two JVM flag profiles
#   make at-selftest  only the self-test of the fixture's classes
#   make at-native    only the native self-checks (see ../README.md) and the negative control
#   make at-arms      each JVM flag profile of profiles.tsv: valid flags, a clean run of fixed, how often buggy fires
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

.PHONY: jar test selftest native flags clean FORCE at-jar at-test at-selftest at-sources at-native at-arms

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
RUNS ?= 50
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
$(AT_BUILD)/at-java: $(AT_BUILD)/lang-at.jar
	printf '#!/bin/sh\nexec "%s" $$JAVA_FLAGS -cp "%s" langat.Main "$$@"\n' "$(JAVA)" "$(abspath $(AT_BUILD)/lang-at.jar)" >$@
	chmod +x $@

$(AT_BUILD)/test-classes/.built: $(AT_BUILD)/classes/.built $(AT_BUILD)/build-id $(AT_TEST_SOURCES)
	rm -rf $(AT_BUILD)/test-classes
	mkdir -p $(AT_BUILD)/test-classes
	$(JAVAC) $(JAVAC_FLAGS) -cp $(AT_BUILD)/classes -d $(AT_BUILD)/test-classes $(AT_TEST_SOURCES)
	touch $@

at-test: at-selftest at-sources at-native at-arms

# Run with HAIRSPRING_EVENTS=off so the SDK writes no report for the evaluations the tests make.
at-selftest: $(AT_BUILD)/test-classes/.built
	HAIRSPRING_EVENTS=off $(JAVA) -cp $(AT_BUILD)/classes:$(AT_BUILD)/test-classes langat.AtSelfTest

# No read of the seed, /proc, the CPU count, the environment or the clock in the fixture's own sources.
at-sources:
	sh ../scripts/check-sources-java.sh at/src

# The contract, natively, 50 runs of buggy and fixed (buggy must fire at least once, fixed never), wide as a contract
# check, and a negative control: the racy program standing in for fixed must be rejected.
at-native: $(AT_BUILD)/at-java
	sh ../scripts/run-checks.sh java $(AT_BUILD)/at-java $(RUNS)

at-arms: $(AT_BUILD)/lang-at.jar
	JAVA="$(JAVA)" ARM_RUNS=$(ARM_RUNS) sh ../scripts/check-java-arms.sh profiles.tsv $(AT_BUILD)/lang-at.jar

clean:
	rm -rf $(BUILD)
