.DEFAULT_GOAL := help

.PHONY: help clean check check-release-metadata test \
       patinae patinae-dev patinae-fast patinae-fast-plugins \
       plugins plugins-install \
       python-release python-dev \
       icon icon-windows app app-full \
       sign sign-full notarize dmg dmg-full \
       bundle-windows \
       web-build web-dev web-clean \
       widget-assets widget-build \
       version

# ── Variables ─────────────────────────────────────────────────────

APP_NAME       := Patinae
BUNDLE_ID      := me.yakovlev.patinae
VERSION        := 0.4.6
BINARY_NAME    := patinae

CARGO          ?= cargo
PYTHON         ?= python3
UV             ?= uv
UVX            ?= uvx

TARGET_DIR     := target
RELEASE_DIR    := $(TARGET_DIR)/release
FAST_PROFILE   ?= dev-fast
FAST_DIR       := $(TARGET_DIR)/$(FAST_PROFILE)

BINARY         := $(RELEASE_DIR)/$(BINARY_NAME)
WINDOWS_BINARY := $(RELEASE_DIR)/$(BINARY_NAME).exe

APP_ROOT       := $(TARGET_DIR)/app
APP_DIR        := $(APP_ROOT)/$(APP_NAME).app
APP_CONTENTS   := $(APP_DIR)/Contents
APP_MACOS      := $(APP_CONTENTS)/MacOS
APP_RESOURCES  := $(APP_CONTENTS)/Resources
APP_PLUGINS    := $(APP_CONTENTS)/PlugIns
APP_ZIP        := $(APP_ROOT)/$(APP_NAME).zip

DMG_STAGE      := $(TARGET_DIR)/dmg-stage
DMG_PATH       := $(TARGET_DIR)/$(APP_NAME).dmg
BUNDLE_DIR     := $(TARGET_DIR)/bundle/$(APP_NAME)

ICON_SRC       := images/patinae.png
ICONSET        := $(APP_ROOT)/AppIcon.iconset
ICNS           := $(APP_ROOT)/AppIcon.icns
ICO            := images/patinae.ico
PLIST_TPL      := macos/Info.plist.in
WINDOWS_LAUNCHER := windows/$(APP_NAME).vbs

PYTHON_VERSION := 3.13
PYTHON_VENV    := $(APP_ROOT)/python-venv
WHEEL_GLOB     := python/target/wheels/patinae-$(VERSION)-*.whl
ENV_FILE       := .env

PLUGIN_INSTALL_DIR ?= $(HOME)/.patinae/plugins
PLUGIN_STAGE_DIR   := $(RELEASE_DIR)/plugins

WEB_STATIC_DIR := python/patinae/widget/static

ifeq ($(OS),Windows_NT)
MKDIRP = powershell -NoProfile -Command '$$null = New-Item -ItemType Directory -Force -Path'
else
MKDIRP = mkdir -p
endif

# Python installation root (uv-managed only — no system/Homebrew Python)
ifeq ($(OS),Windows_NT)
PYTHON_DIST = $(shell powershell -NoProfile -Command "Split-Path (uv python find --python-preference only-managed $(PYTHON_VERSION))")
else
UV_PYTHON_DIR := $(shell $(UV) python dir 2>/dev/null)
PYTHON_DIST   := $(shell ls -d "$(UV_PYTHON_DIR)"/cpython-$(PYTHON_VERSION)*-*-none 2>/dev/null | sort -V | tail -1)
endif
ifeq ($(OS),Windows_NT)
PYO3_PYTHON := $(PYTHON_DIST)/python.exe
else
PYO3_PYTHON := $(PYTHON_DIST)/bin/python3
endif
export PYO3_PYTHON

# maturin passes Darwin install_name as a clang-style -Wl,... argument.
# Python extension modules need the platform linker driver so those flags are
# expanded correctly on macOS.
ifeq ($(OS),Windows_NT)
PYTHON_MATURIN_ENV :=
else
UNAME_S := $(shell uname -s)
ifeq ($(UNAME_S),Darwin)
PYTHON_MATURIN_ENV := CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=clang CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=clang
else
PYTHON_MATURIN_ENV :=
endif
endif

# ── Build ─────────────────────────────────────────────────────────

check:
	$(CARGO) check --locked --workspace

check-release-metadata:
	bash scripts/check-release-metadata.sh

test:
	$(CARGO) test --locked

clean:
	$(CARGO) clean
	rm -rf python/target $(TARGET_DIR)/wheels $(APP_ROOT) $(DMG_STAGE) $(DMG_PATH) $(BUNDLE_DIR)
	rm -rf web/pkg web/dist $(WEB_STATIC_DIR)

# ── Patinae (Slint GUI) ──────────────────────────────────────────

patinae:
	$(CARGO) build --locked -p patinae --release

patinae-dev:
	$(CARGO) run -p patinae

patinae-fast:
	$(CARGO) build -p patinae --profile $(FAST_PROFILE)

# ── Python ────────────────────────────────────────────────────────

python-release: widget-assets
	cd python && $(PYTHON_MATURIN_ENV) $(UVX) maturin build --release --locked

python-dev: widget-assets
	cd python && $(PYTHON_MATURIN_ENV) $(UVX) maturin develop

# ── Plugins ───────────────────────────────────────────────────────

PLUGIN_CRATES := -p raytracer-plugin -p hello-plugin -p python-plugin
ifneq ($(OS),Windows_NT)
PLUGIN_CRATES += -p ipc-plugin
endif

plugins:
	$(CARGO) build --locked --release $(PLUGIN_CRATES)
ifeq ($(OS),Windows_NT)
	powershell -NoProfile -Command "if (Test-Path '$(PLUGIN_STAGE_DIR)') { Remove-Item -Recurse -Force '$(PLUGIN_STAGE_DIR)' }"
	$(MKDIRP) $(PLUGIN_STAGE_DIR)
	powershell -NoProfile -Command "Copy-Item '$(RELEASE_DIR)/*_plugin.dll' '$(PLUGIN_STAGE_DIR)/' -ErrorAction SilentlyContinue"
	powershell -NoProfile -Command "Copy-Item 'plugins/*/*.deps' '$(PLUGIN_STAGE_DIR)/' -ErrorAction SilentlyContinue"
else
	rm -rf $(PLUGIN_STAGE_DIR)
	$(MKDIRP) $(PLUGIN_STAGE_DIR)
	cp $(RELEASE_DIR)/lib*_plugin.dylib $(PLUGIN_STAGE_DIR)/ 2>/dev/null || \
	cp $(RELEASE_DIR)/lib*_plugin.so    $(PLUGIN_STAGE_DIR)/ 2>/dev/null || true
endif

patinae-fast-plugins:
	$(CARGO) build --profile $(FAST_PROFILE) -p patinae $(PLUGIN_CRATES)
ifeq ($(OS),Windows_NT)
	powershell -NoProfile -Command "if (Test-Path '$(FAST_DIR)/plugins') { Remove-Item -Recurse -Force '$(FAST_DIR)/plugins' }"
	$(MKDIRP) $(FAST_DIR)/plugins
	powershell -NoProfile -Command "Copy-Item '$(FAST_DIR)/*_plugin.dll' '$(FAST_DIR)/plugins/' -ErrorAction SilentlyContinue"
	powershell -NoProfile -Command "Copy-Item 'plugins/*/*.deps' '$(FAST_DIR)/plugins/' -ErrorAction SilentlyContinue"
else
	rm -rf $(FAST_DIR)/plugins
	$(MKDIRP) $(FAST_DIR)/plugins
	cp $(FAST_DIR)/lib*_plugin.dylib $(FAST_DIR)/plugins/ 2>/dev/null || \
	cp $(FAST_DIR)/lib*_plugin.so    $(FAST_DIR)/plugins/ 2>/dev/null || true
endif
	@echo "✓ $(FAST_DIR)/patinae + $(FAST_DIR)/plugins"

plugins-install: plugins
	$(MKDIRP) $(PLUGIN_INSTALL_DIR)
	cp $(PLUGIN_STAGE_DIR)/* $(PLUGIN_INSTALL_DIR)/

# ── macOS App Bundle ──────────────────────────────────────────────

icon: $(ICON_SRC)
	@echo "── Generating .icns ──"
	@mkdir -p $(ICONSET)
	@for size in 16 32 64 128 256 512 1024; do \
	    sips -z $$size $$size $(ICON_SRC) --out $(ICONSET)/icon_$${size}x$${size}.png >/dev/null 2>&1; \
	done
	@sips -z   32   32 $(ICON_SRC) --out $(ICONSET)/icon_16x16@2x.png    >/dev/null
	@sips -z   64   64 $(ICON_SRC) --out $(ICONSET)/icon_32x32@2x.png    >/dev/null
	@sips -z  256  256 $(ICON_SRC) --out $(ICONSET)/icon_128x128@2x.png  >/dev/null
	@sips -z  512  512 $(ICON_SRC) --out $(ICONSET)/icon_256x256@2x.png  >/dev/null
	@sips -z 1024 1024 $(ICON_SRC) --out $(ICONSET)/icon_512x512@2x.png  >/dev/null
	@rm -f $(ICONSET)/icon_64x64.png $(ICONSET)/icon_1024x1024.png
	$(PYTHON) scripts/iconset_to_icns.py $(ICONSET) $(ICNS)
	@rm -rf $(ICONSET)
	@echo "✓ $(ICNS)"

icon-windows: $(ICON_SRC)
	@echo "── Generating .ico ──"
	magick $(ICON_SRC) \
	  \( -clone 0 -resize 16x16 \) \
	  \( -clone 0 -resize 32x32 \) \
	  \( -clone 0 -resize 48x48 \) \
	  \( -clone 0 -resize 256x256 \) \
	  -delete 0 $(ICO)
	@echo "✓ $(ICO)"

# Shared: assemble base .app structure (binary + icon + Info.plist)
define assemble-app
	@rm -rf $(APP_DIR)
	@mkdir -p $(APP_MACOS) $(APP_RESOURCES)
	cp $(BINARY) $(APP_MACOS)/$(BINARY_NAME)
	cp $(ICNS)   $(APP_RESOURCES)/AppIcon.icns
	sed -e 's/@APP_NAME@/$(APP_NAME)/g' \
	    -e 's/@BUNDLE_ID@/$(BUNDLE_ID)/g' \
	    -e 's/@VERSION@/$(VERSION)/g' \
	    -e 's/@BINARY_NAME@/$(BINARY_NAME)/g' \
	    $(PLIST_TPL) > $(APP_CONTENTS)/Info.plist
	@rm -f $(ICNS)
endef

# Minimal .app (binary only)
app: patinae icon
	@echo "── Assembling $(APP_NAME).app ──"
	$(assemble-app)
	@echo "✓ $(APP_DIR)"

# Full .app (binary + plugins + Python + venv)
app-full: patinae plugins icon python-release
	@echo "── Creating bundled venv (uv + Python $(PYTHON_VERSION)) ──"
	@rm -rf $(PYTHON_VENV)
	$(UV) venv --python-preference only-managed --python $(PYTHON_VERSION) $(PYTHON_VENV)
	$(UV) pip install --python $(PYTHON_VENV)/bin/python3 \
	    $$(ls $(WHEEL_GLOB) | head -1)
	@echo "── Assembling full $(APP_NAME).app ──"
	$(assemble-app)
	@mkdir -p $(APP_PLUGINS)
	cp $(PLUGIN_STAGE_DIR)/lib*_plugin.dylib $(APP_PLUGINS)/ 2>/dev/null || true
	cp $(PLUGIN_STAGE_DIR)/lib*_plugin.so    $(APP_PLUGINS)/ 2>/dev/null || true
	cp -R $(PYTHON_DIST) $(APP_RESOURCES)/python
	cp -R $(PYTHON_VENV) $(APP_RESOURCES)/python-venv
	@# Fix venv python symlinks: point to bundled Python, not system Python
	@cd $(APP_RESOURCES)/python-venv/bin && \
	  rm -f python python3 python$(PYTHON_VERSION) && \
	  ln -s ../../python/bin/python3 python && \
	  ln -s python python3 && \
	  ln -s python python$(PYTHON_VERSION)
	bash macos/fix-dylib-paths.sh $(APP_DIR)
	@echo "✓ $(APP_DIR) (full bundle)"

# ── Code Signing ──────────────────────────────────────────────────

sign: app
	@echo "── Signing $(APP_NAME).app ──"
	@if [ ! -f $(ENV_FILE) ]; then echo "⚠ Signing skipped: $(ENV_FILE) not found"; exit 0; fi; \
	. ./$(ENV_FILE); \
	if [ -z "$$PATINAE_APPLE_TEAMID" ]; then echo "⚠ Signing skipped: PATINAE_APPLE_TEAMID not set"; exit 0; fi; \
	codesign --force --options runtime \
	    --sign "Developer ID Application: $$PATINAE_APPLE_TEAMID" $(APP_DIR) && \
	codesign --verify --verbose $(APP_DIR) && \
	echo "✓ Signed"

sign-full: app-full
	@echo "── Deep-signing $(APP_NAME).app ──"
	@if [ ! -f $(ENV_FILE) ]; then echo "⚠ Signing skipped: $(ENV_FILE) not found"; exit 0; fi; \
	. ./$(ENV_FILE); \
	if [ -z "$$PATINAE_APPLE_TEAMID" ]; then echo "⚠ Signing skipped: PATINAE_APPLE_TEAMID not set"; exit 0; fi; \
	IDENTITY="Developer ID Application: $$PATINAE_APPLE_TEAMID"; \
	find $(APP_RESOURCES)/python \
	     $(APP_RESOURCES)/python-venv \
	     $(APP_PLUGINS) \
	  -type f \( -name '*.dylib' -o -name '*.so' \) 2>/dev/null | \
	  xargs -I{} codesign --force --options runtime --sign "$$IDENTITY" {} && \
	codesign --force --options runtime --sign "$$IDENTITY" \
	  $(APP_RESOURCES)/python/bin/python3 && \
	codesign --force --options runtime --sign "$$IDENTITY" $(APP_DIR) && \
	codesign --verify --verbose --deep $(APP_DIR) && \
	echo "✓ Signed (deep)"

# ── Notarize & DMG ───────────────────────────────────────────────

notarize:
	@echo "── Notarizing $(APP_NAME).app ──"
	@if [ ! -f $(ENV_FILE) ]; then echo "⚠ Notarization skipped: $(ENV_FILE) not found"; exit 0; fi; \
	. ./$(ENV_FILE); \
	if [ -z "$$PATINAE_APPLE_EMAIL" ] || [ -z "$$PATINAE_APPLE_TEAMID" ] || [ -z "$$PATINAE_APPLE_APP_PASS" ]; then \
	    echo "⚠ Notarization skipped: missing credentials in $(ENV_FILE)"; exit 0; \
	fi; \
	ditto -c -k --keepParent $(APP_DIR) $(APP_ZIP) && \
	xcrun notarytool submit $(APP_ZIP) \
	    --apple-id "$$PATINAE_APPLE_EMAIL" \
	    --team-id "$$PATINAE_APPLE_TEAMID" \
	    --password "$$PATINAE_APPLE_APP_PASS" \
	    --wait && \
	xcrun stapler staple $(APP_DIR) && \
	echo "✓ Notarized & stapled" || \
	echo "⚠ Notarization failed — continuing without it"; \
	rm -f $(APP_ZIP)

# Shared: create DMG from a clean staging directory (only .app + Applications link)
define create-dmg
	$(MAKE) notarize
	@echo "── Creating DMG ──"
	@rm -rf $(DMG_STAGE)
	@mkdir -p $(DMG_STAGE)
	@cp -R $(APP_DIR) $(DMG_STAGE)/
	@ln -sf /Applications $(DMG_STAGE)/Applications
	hdiutil create -volname "$(APP_NAME)" \
	    -srcfolder $(DMG_STAGE) -ov -format UDZO \
	    $(DMG_PATH)
	@rm -rf $(DMG_STAGE)
	@echo "✓ $(DMG_PATH)"
endef

dmg: sign
	$(create-dmg)

dmg-full: sign-full
	$(create-dmg)

# ── Windows Bundle ────────────────────────────────────────────────

bundle-windows: patinae plugins python-release
	@echo ── Assembling Windows bundle ──
	powershell -NoProfile -Command "if (Test-Path '$(BUNDLE_DIR)') { Remove-Item -Recurse -Force '$(BUNDLE_DIR)' }"
	$(MKDIRP) "$(BUNDLE_DIR)\plugins"
	powershell -NoProfile -Command "Copy-Item '$(WINDOWS_BINARY)' '$(BUNDLE_DIR)/'"
	powershell -NoProfile -Command "Copy-Item '$(PLUGIN_STAGE_DIR)/*_plugin.dll' '$(BUNDLE_DIR)/plugins/' -ErrorAction SilentlyContinue"
	powershell -NoProfile -Command "Copy-Item 'plugins/*/*.deps' '$(BUNDLE_DIR)/plugins/' -ErrorAction SilentlyContinue"
	powershell -NoProfile -Command "Copy-Item -Recurse '$(PYTHON_DIST)' '$(BUNDLE_DIR)/python'"
	$(UV) venv --python-preference only-managed --python $(PYTHON_VERSION) "$(BUNDLE_DIR)\python-venv"
	$(UV) pip install --python "$(BUNDLE_DIR)/python-venv/Scripts/python.exe" $(WHEEL_GLOB)
	powershell -NoProfile -Command "Copy-Item '$(WINDOWS_LAUNCHER)' '$(BUNDLE_DIR)/'"
	@echo ✓ $(BUNDLE_DIR)

# ── Web ──────────────────────────────────────────────────────────

web-build:
	cd web && npm install && npm run build

web-dev:
	cd web && npm install && npm run dev

web-clean:
	rm -rf web/pkg web/dist web/node_modules

# ── Widget ───────────────────────────────────────────────────────

# Copy pre-built web dist into the Python package (requires web-build to have run once)
widget-assets:
ifeq ($(OS),Windows_NT)
	@powershell -NoProfile -Command "if (-not (Test-Path 'web/dist/patinae_web_bg.wasm')) { Write-Host '── Web dist not found, building... ──'; & '$(MAKE)' web-build }"
	$(MKDIRP) $(WEB_STATIC_DIR)
	powershell -NoProfile -Command "Copy-Item 'web/dist/patinae-viewer.js' '$(WEB_STATIC_DIR)/'"
	powershell -NoProfile -Command "Copy-Item 'web/dist/patinae_web_bg.wasm' '$(WEB_STATIC_DIR)/'"
	powershell -NoProfile -Command "Get-ChildItem 'web/dist/patinae_web-*.js' | Select-Object -First 1 | Copy-Item -Destination '$(WEB_STATIC_DIR)/patinae_web_glue.js'"
else
	@if [ ! -f web/dist/patinae_web_bg.wasm ]; then \
		echo "── Web dist not found, building... ──"; \
		$(MAKE) web-build; \
	fi
	$(MKDIRP) $(WEB_STATIC_DIR)
	cp web/dist/patinae-viewer.js $(WEB_STATIC_DIR)/
	cp web/dist/patinae_web_bg.wasm $(WEB_STATIC_DIR)/
	cp web/dist/patinae_web-*.js $(WEB_STATIC_DIR)/patinae_web_glue.js
endif

# Full rebuild: web + copy assets
widget-build: web-build widget-assets

# ── Version ──────────────────────────────────────────────────────

version:
	@if [ -z "$(V)" ]; then echo "Usage: make version V=X.Y.Z"; exit 1; fi
	bash scripts/version.sh $(V)

# ── Help ──────────────────────────────────────────────────────────

help:
	@echo "Build:"
	@echo "  check            Check the Rust workspace"
	@echo "  check-release-metadata  Check release versions and Cargo lockfiles"
	@echo "  test             Run tests"
	@echo "  clean            Clean all build artifacts"
	@echo ""
	@echo "Patinae (Slint GUI):"
	@echo "  patinae          Build Patinae release binary"
	@echo "  patinae-dev      Run Patinae in debug mode"
	@echo "  patinae-fast     Build Patinae with fast profile (FAST_PROFILE=dev-fast)"
	@echo "  patinae-fast-plugins  Build Patinae + plugins fast, staged beside binary"
	@echo ""
	@echo "Python:"
	@echo "  python-release   Build Python wheel (release)"
	@echo "  python-dev       Install Python package in dev mode"
	@echo ""
	@echo "Plugins:"
	@echo "  plugins-install  Build + install plugins to ~/.patinae/plugins"
	@echo "  plugins          Build all plugins (release)"
	@echo ""
	@echo "Icons:"
	@echo "  icon             Generate .icns from PNG (macOS, requires sips + python3)"
	@echo "  icon-windows     Generate .ico from PNG (requires ImageMagick)"
	@echo ""
	@echo "macOS app bundle:"
	@echo "  app              Minimal .app (binary + icon)"
	@echo "  app-full         Full .app (+ plugins + Python + venv)"
	@echo "  sign / sign-full Code-sign the .app"
	@echo "  dmg / dmg-full   Distributable DMG (signed + notarized)"
	@echo ""
	@echo "Windows bundle:"
	@echo "  bundle-windows   Bundle (exe + plugins + Python + venv + launcher)"
	@echo ""
	@echo "Version:"
	@echo "  version V=X.Y.Z Update version across crates, packages, and README badge"
	@echo ""
	@echo "Web:"
	@echo "  web-build        Build WASM + TypeScript bundle"
	@echo "  web-dev          Dev server with hot reload"
	@echo "  web-clean        Clean web build artifacts"
	@echo ""
	@echo "Widget:"
	@echo "  widget-build     Build web + copy WASM assets for Jupyter widget"
	@echo ""
	@echo "Signing credentials (.env):"
	@echo "  PATINAE_APPLE_TEAMID    Apple Developer Team ID"
	@echo "  PATINAE_APPLE_EMAIL     Apple ID email"
	@echo "  PATINAE_APPLE_APP_PASS  App-specific password"
