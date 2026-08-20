# 🐭 Mouse Me

**Mouse Me** is a lightweight, blazingly fast cursor manager built in **Rust** with a modern **Slint GUI** and an instant **CLI**. Designed specifically for **Omarchy** and modern Linux desktops (Hyprland, Wayland, X11), it enables users to easily add custom cursor packs and applies them 100% consistently across all subsystems.

---

## ✨ Features

- 🚀 **Dual Interface in a Single Binary**:
  - **Modern Slint GUI**: Interactive visual browser, live cursor previews (default, pointer, wait, text), size switcher, one-click apply, and file picker importer.
  - **Blazingly Fast CLI (<20ms)**: Scriptable, pipeable, and perfect for dotfile automation and terminal keybindings.
- 🎯 **100% Universal System-Wide Application**:
  - **Wayland / Hyprland**: Active live session cursor switching via `hyprctl setcursor`.
  - **GTK 3 & GTK 4**: Live GSettings sync and `settings.ini` generation.
  - **GTK 2**: `~/.gtkrc-2.0` updates.
  - **Qt 5 / Qt 6 & KDE**: `~/.config/kdeglobals` and `~/.config/environment.d/10-cursor.conf`.
  - **XWayland & X11**: `~/.Xresources` + `xrdb -merge` synchronization.
  - **Default Fallback**: `~/.icons/default/index.theme` and `~/.local/share/icons/default/index.theme`.
  - **Flatpak Apps**: Configures user overrides to allow sandboxed applications to access custom cursor themes.
- 📦 **Smart Importer & Normalization**:
  - Directly import `.zip`, `.tar.gz`, `.tar.xz`, `.tar.bz2`, `.tar` archives, or folder directories.
  - Automatically un-nests messy archive hierarchies.
  - Auto-generates missing `index.theme` so GTK/X11 recognizes themes immediately.
  - Supports both **XCursor** and **Hyprcursor** themes.
- 🎨 **Built-in Pure-Rust XCursor Parser**:
  - Extracts live preview sprites directly from binary cursor files without external C libraries.

---

## 📦 Installation & Build

### Requirements
- Rust 1.80+ and Cargo
- Standard Wayland / X11 dev libraries (pre-installed on Omarchy/Arch)
- Optional runtime tools for live integration: `hyprctl`, `gsettings`, `xrdb`, `flatpak`, `xdg-open`, and `curl`

### Build Release Binary
```bash
git clone https://github.com/grenishrai/mouse-me.git
cd mouse-me
cargo build --release
```

The compiled binary will be in `target/release/mouse-me`.

### Install to System
```bash
sudo install -Dm755 target/release/mouse-me /usr/local/bin/mouse-me
install -Dm644 mouse-me.desktop ~/.local/share/applications/mouse-me.desktop
```

---

## 🖥️ Usage

### 1. Launching the GUI
Launch via your desktop launcher (**Walker**, **Rofi**, **App Grid**) or from the terminal:
```bash
mouse-me
# or
mouse-me gui
```

### 2. Command-Line Interface (CLI)

#### List installed cursor themes:
```bash
mouse-me list
```
*(Use `--json` for machine-readable output)*

#### Set a cursor theme and size system-wide:
```bash
# Sets cursor with default 24px size
mouse-me set Twilight-cursors

# Sets cursor with custom size (e.g. 32px)
mouse-me set Twilight-cursors 32
```

#### Get active cursor theme & size:
```bash
mouse-me get
```

#### Import a custom cursor pack archive:
```bash
mouse-me add ~/Downloads/MyCustomCursor.tar.gz
```

#### Remove a user-installed cursor theme:
```bash
mouse-me remove MyCustomCursor
```

---

## 🏗️ Architecture

```
mouse-me/
├── Cargo.toml
├── build.rs                 # Compiles Slint UI at build time
├── mouse-me.desktop         # Desktop application launcher
├── PKGBUILD                 # Arch Linux packaging recipe
├── ui/
│   └── main.slint           # Modern dark Slint UI declaration
└── src/
    ├── main.rs              # Entrypoint (dispatches CLI or GUI)
    ├── cli.rs               # Clap CLI definitions & handlers
    ├── gui.rs               # Slint window controller & state bindings
    └── core/
        ├── types.rs         # Data structures (Theme, Preview, Type)
        ├── xcursor.rs       # Pure Rust XCursor binary parser
        ├── scanner.rs       # Discovers installed cursors & active theme
        ├── importer.rs      # Archive unpacker, flattener & normalizer
        └── applier.rs       # Multi-subsystem synchronizer with per-target errors
```

---

## 📄 License
MIT / Apache-2.0
