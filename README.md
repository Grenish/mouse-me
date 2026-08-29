<div align="center">
<img src="assets/mouse-me.png" alt="Mouse Me" width="104">

<h1>Mouse Me</h1>
</div>

A cursor manager for Linux desktops (Hyprland, Omarchy, GTK, Qt, and X11). It includes a Slint GUI and a CLI in one binary.

> [!NOTE]
> Mouse Me is currently in active beta development. You may encounter bugs or incomplete features. If you find an issue, please open a GitHub issue with steps to reproduce it so it can be investigated and fixed.

![Mouse Me](assets/screens/main.png)

> [!IMPORTANT]
> This repository is under active development and is not accepting pull requests. If you run into a problem, [open an issue](https://github.com/Grenish/mouse-me/issues/new?template=problem.yml).

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/Grenish/mouse-me/main/install.sh | bash
```

## Build

```bash
git clone https://github.com/Grenish/mouse-me.git
cd mouse-me
cargo build --release
```

The binary is `target/release/mouse-me`.

```bash
sudo install -Dm755 target/release/mouse-me /usr/local/bin/mouse-me
install -Dm644 mouse-me.desktop ~/.local/share/applications/mouse-me.desktop
```

## Usage

GUI:

```bash
mouse-me
mouse-me gui
```

CLI:

```bash
mouse-me list
mouse-me list --json --user --type xcursor
mouse-me list --search modest
mouse-me get
mouse-me set modest-light
mouse-me set modest-light 32
mouse-me set modest-light 32 --all
mouse-me add modest-light
mouse-me add "Modest Light" --apply
mouse-me add ~/Downloads/cursors.zip
mouse-me add ~/Downloads/cursors.zip --apply
mouse-me remove MyCustomCursor
mouse-me auth login --email you@example.com
mouse-me whoami
mouse-me auth logout
mouse-me account
mouse-me account sign-up --name "Ada" --username ada --email ada@example.com
mouse-me update
mouse-me update --install
mouse-me update --stage
mouse-me doctor
mouse-me doctor --json
mouse-me settings
mouse-me settings set apply-hyprland false
mouse-me settings set auto-update true
mouse-me settings apply-hypr
```

`set` uses the apply targets saved in Settings. Pass `--all` to write every target. Passwords are prompted on a TTY, or read from `MOUSE_ME_PASSWORD`.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
