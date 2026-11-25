# Read after the upstream Makefile so the game and platform source lists match
# the pinned native port. Each Rust module replaces one complete C unit.
# The cartridge clock is implemented directly by src/rtc.rs.
PORT_UNITS := $(shell cat ../../tools/rust-units.txt)
HOST_UNITS := siirtc
# Upstream's compiler stamp already invalidates every native object. Include
# the deployment target and flags so differently configured builds cannot mix.
NATIVE_CC_ID := $(NATIVE_CC_ID) $(NATIVE_CFLAGS) $(MACOSX_DEPLOYMENT_TARGET)
PATCH_UNITS := m4a sound main malloc
RUSTIC_DIR := $(NATIVE_BUILD_DIR)/rustic
# Select the implementation by translation unit. The reference uses original C
# gameplay throughout. Both binaries use the same device, save and audio hooks.
GAME_UNITS := $(patsubst src/%.c,%,$(C_SRCS))
UNKNOWN_PORT_UNITS := $(filter-out $(GAME_UNITS),$(PORT_UNITS))
ifneq ($(UNKNOWN_PORT_UNITS),)
$(error Unexpected Rust translation units: $(UNKNOWN_PORT_UNITS))
endif
UNPORTED_UNITS := $(filter-out $(PORT_UNITS) $(HOST_UNITS),$(GAME_UNITS))
C_REFERENCE_UNITS := $(filter-out $(HOST_UNITS),$(GAME_UNITS))
objects_for = $(addprefix $(NATIVE_OBJ_DIR)/,$(addsuffix .o,$(filter-out $(PATCH_UNITS),$1))) $(addprefix $(RUSTIC_DIR)/,$(addsuffix .o,$(filter $(PATCH_UNITS),$1)))
# Native platform glue and generated maps/scripts are shared by both archives.
PLATFORM_OBJECTS := $(NATIVE_OBJ_DIR)/native_game_save.o $(NATIVE_OBJ_DIR)/native_platform.o $(RUSTIC_DIR)/native_engine.o $(NATIVE_DATA_OBJS)
CORE_OBJECTS := $(PLATFORM_OBJECTS) $(RUSTIC_DIR)/bios_rustic.o
REFERENCE_OBJECTS := $(call objects_for,$(C_REFERENCE_UNITS)) $(PLATFORM_OBJECTS) $(RUSTIC_DIR)/bios.o

.PHONY: native-rustic native-reference native-rustic-audit
native-rustic: native-rustic-audit $(NATIVE_BUILD_DIR)/librustic_core.a $(NATIVE_BUILD_DIR)/librustic_test_support.a
native-reference: $(NATIVE_BUILD_DIR)/librustic_reference.a $(NATIVE_BUILD_DIR)/librustic_test_support.a

# Enforce the full Rust inventory if the pinned upstream source list changes.
native-rustic-audit:
	$(if $(UNPORTED_UNITS),$(error Game units missing Rust implementations: $(UNPORTED_UNITS)),@echo All game translation units use Rust)

define core_archive
$(NATIVE_BUILD_DIR)/lib$1.a: $2 ../../tools/build-core.mk ../../tools/rust-units.txt
	$$(RM) $$@.tmp
	$$(AR) crs $$@.tmp $2
	mv $$@.tmp $$@
endef
$(eval $(call core_archive,rustic_core,$(CORE_OBJECTS)))
$(eval $(call core_archive,rustic_reference,$(REFERENCE_OBJECTS)))

# Referenced only by the Rust integration test. Its new-game save/load path
# exercises the actual Flash sector serializer, checksums and layout.
$(NATIVE_BUILD_DIR)/librustic_test_support.a: $(NATIVE_OBJ_DIR)/native_test_game.o
	$(AR) crs $@ $<

# These two upstream generators use only the Python standard library.
wasm-assets: $(GFX)
	python3 tools/generate_wasm_assets.py
$(WASM_SOUND_HEADER): tools/generate_wasm_sound.py sound/song_table.inc sound/songs/midi/midi.cfg $(MID_SRCS)
	@mkdir -p $(dir $@)
	python3 tools/generate_wasm_sound.py $@

# Patch only private compiler inputs, then run the same source-preparation
# and compile stages as tools/native/build.mk.
define patched_game_unit
$(RUSTIC_DIR)/$1.c: src/$1.c ../../native/$2.patch
	@mkdir -p $$(dir $$@)
	patch -s -o $$@.tmp $$< ../../native/$2.patch
	mv $$@.tmp $$@

$(RUSTIC_DIR)/$1.o: $(RUSTIC_DIR)/$1.c tools/native/prepare_source.py $(NATIVE_CC_STAMP) ../../tools/build-core.mk | generated wasm-assets
	@mkdir -p $$(dir $$@)
	$(NATIVE_CC) $(NATIVE_CFLAGS) $(NATIVE_GAME_FLAGS) $(NATIVE_INCLUDES) -MMD -MP -MF $$@.d -MT $$@ -E $(RUSTIC_DIR)/$1.c | $(PREPROC) -i -g $(ASSETS_DIR_NAME) src/$1.c charmap.txt > $$@.i
	python3 tools/native/prepare_source.py $$@.i $$@.c $(NATIVE_FORMAT) $(NATIVE_AST_CC) --target=$(NATIVE_AST_TARGET)
	$(NATIVE_CC) $(NATIVE_CFLAGS) $(NATIVE_GAME_FLAGS) -x c -c $$@.c -o $$@
endef
$(eval $(call patched_game_unit,m4a,m4a))
$(eval $(call patched_game_unit,sound,cry))
$(eval $(call patched_game_unit,main,reset-keys))
$(eval $(call patched_game_unit,malloc,heap))
$(RUSTIC_DIR)/m4a.o: $(WASM_SOUND_HEADER)
-include $(addprefix $(RUSTIC_DIR)/,$(addsuffix .o.d,$(PATCH_UNITS)))

$(RUSTIC_DIR)/native_engine.c: tools/native/native_engine.c ../../native/reset.patch
	@mkdir -p $(dir $@)
	patch -s -o $@.tmp $< ../../native/reset.patch
	mv $@.tmp $@

$(RUSTIC_DIR)/native_engine.o: $(RUSTIC_DIR)/native_engine.c $(NATIVE_ENGINE_H) $(NATIVE_CC_STAMP)
	$(NATIVE_CC) $(NATIVE_CFLAGS) -I tools/native -c $< -o $@

# SoftReset is an upstream no-op. The Rust host owns resetting this session.
$(RUSTIC_DIR)/bios.o: tools/native/bios.c ../../tools/build-core.mk $(NATIVE_CC_STAMP)
	@mkdir -p $(dir $@)
	$(NATIVE_CC) $(NATIVE_CFLAGS) -DSoftReset=RusticUnusedSoftReset -c $< -o $@

# Rust supplies both LZ77 entry points in the port.
$(RUSTIC_DIR)/bios_rustic.c: tools/native/bios.c ../../native/bios.patch
	@mkdir -p $(dir $@)
	patch -s -o $@.tmp $< ../../native/bios.patch
	mv $@.tmp $@

$(RUSTIC_DIR)/bios_rustic.o: $(RUSTIC_DIR)/bios_rustic.c ../../tools/build-core.mk $(NATIVE_CC_STAMP)
	$(NATIVE_CC) $(NATIVE_CFLAGS) -DSoftReset=RusticUnusedSoftReset -c $< -o $@

# Materialize preprocessed translation units (including C objects replaced by
# Rust) for regenerating Rust or inspecting an ABI difference.
.PHONY: native-rustic-prepare
native-rustic-prepare: $(NATIVE_C_OBJS) $(NATIVE_DATA_OBJS)
