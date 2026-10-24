# Hairspring guest SDK for Java: pure Java 21, JDK classes only.
#
#   make jar        build/hairspring-sdk.jar (javac --release 21, reproducible jar, module hairspring)
#   make test       the self-test of the classes, then the shell tests (the sample programs as real processes)
#   make selftest   only the self-test
#   make samples    compile the sample programs into build/samples
#   make clean
#
# Needs a JDK 21: JAVA_HOME selects it (`java -version` shows which one this is). SOURCE_DATE is the fixed timestamp of
# every jar entry. Nothing here runs Hermit.

ifneq ($(JAVA_HOME),)
JAVA_BIN := $(JAVA_HOME)/bin/
endif
JAVAC ?= $(JAVA_BIN)javac
JAR ?= $(JAVA_BIN)jar
JAVA ?= $(JAVA_BIN)java
SOURCE_DATE ?= 2026-01-01T00:00:00Z
BUILD ?= build

JAVAC_FLAGS = --release 21 -Xlint:all -Werror -encoding UTF-8 -proc:none
SOURCES := $(sort $(wildcard src/hairspring/*.java)) src/module-info.java
TEST_SOURCES := $(sort $(wildcard test/hairspring/*.java))
SAMPLE_SOURCES := $(sort $(wildcard samples/hssample/*.java))

.PHONY: jar test selftest process samples clean FORCE

jar: $(BUILD)/hairspring-sdk.jar

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
# file system lists the class files in; --date fixes every timestamp.
$(BUILD)/hairspring-sdk.jar: $(BUILD)/classes/.built
	(cd $(BUILD)/classes && find . -type f -name '*.class' | sed 's|^\./||' | LC_ALL=C sort | sed 's|^|-C classes |') >$(BUILD)/jar.args
	cd $(BUILD) && $(JAR) --create --file hairspring-sdk.jar --date=$(SOURCE_DATE) @jar.args

$(BUILD)/test-classes/.built: $(BUILD)/classes/.built $(BUILD)/build-id $(TEST_SOURCES)
	rm -rf $(BUILD)/test-classes
	mkdir -p $(BUILD)/test-classes
	$(JAVAC) $(JAVAC_FLAGS) -cp $(BUILD)/classes -d $(BUILD)/test-classes $(TEST_SOURCES)
	touch $@

$(BUILD)/samples/.built: $(BUILD)/classes/.built $(BUILD)/build-id $(SAMPLE_SOURCES)
	rm -rf $(BUILD)/samples
	mkdir -p $(BUILD)/samples
	$(JAVAC) $(JAVAC_FLAGS) -cp $(BUILD)/classes -d $(BUILD)/samples $(SAMPLE_SOURCES)
	touch $@

samples: $(BUILD)/samples/.built

test: selftest process

selftest: $(BUILD)/test-classes/.built
	$(JAVA) -cp $(BUILD)/classes:$(BUILD)/test-classes hairspring.SelfTest

process: $(BUILD)/samples/.built $(BUILD)/test-classes/.built
	JAVA="$(JAVA)" sh test/test-process.sh $(BUILD)/classes $(BUILD)/samples $(BUILD)/test-classes

clean:
	rm -rf $(BUILD)
