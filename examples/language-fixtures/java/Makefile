# Java probe (no SDK, no property): four platform threads, a lock, an unlocked counter, a queue and a map, one stdout line.
#
#   make jar      build/lang-at.jar (javac --release 21, reproducible jar)
#   make test     the self-test of the classes, then the native tests of the jar
#   make flags    each flag profile of profiles.tsv: valid JVM options, one clean line, nothing else on stdout
#   make clean
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

.PHONY: jar test selftest native flags clean

jar: $(BUILD)/lang-at.jar

$(BUILD)/classes/.built: $(SOURCES)
	rm -rf $(BUILD)/classes
	mkdir -p $(BUILD)/classes
	$(JAVAC) $(JAVAC_FLAGS) -d $(BUILD)/classes $(SOURCES)
	touch $@

# The entries are listed one by one in sorted order (no directory entries), so the jar does not depend on the order the
# file system lists the class files in; --date fixes every timestamp.
$(BUILD)/lang-at.jar: $(BUILD)/classes/.built manifest.mf
	(cd $(BUILD)/classes && find langat -type f -name '*.class' | LC_ALL=C sort | sed 's|^|-C classes |') >$(BUILD)/jar.args
	cd $(BUILD) && $(JAR) --create --file lang-at.jar --date=$(SOURCE_DATE) --manifest $(CURDIR)/manifest.mf @jar.args

$(BUILD)/test-classes/.built: $(BUILD)/classes/.built $(TEST_SOURCES)
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

clean:
	rm -rf $(BUILD)
