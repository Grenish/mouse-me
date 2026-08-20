#!/usr/bin/env bash
# Install Mouse Me for the current user.
# curl -fsSL https://raw.githubusercontent.com/Grenish/mouse-me/main/install.sh | bash
set -euo pipefail

REPO="${MOUSE_ME_REPO:-Grenish/mouse-me}"
VERSION="${MOUSE_ME_VERSION:-latest}"
PREFIX="${PREFIX:-$HOME/.local}"
USER_AGENT="mouse-me-install"

usage() {
    cat <<'EOF'
Install Mouse Me from a GitHub release.

Usage:
  curl -fsSL https://raw.githubusercontent.com/Grenish/mouse-me/main/install.sh | bash
  curl -fsSL https://raw.githubusercontent.com/Grenish/mouse-me/main/install.sh | bash -s -- --prefix ~/.local

Options:
  --prefix DIR    Install root (default: ~/.local)
  --version TAG   Release tag (default: latest)
  -h, --help      Show this help

Environment:
  PREFIX, MOUSE_ME_VERSION, MOUSE_ME_REPO
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "Need $1 on PATH"
}

latest_tag() {
    local body status
    body="$(mktemp)"
    status="$(curl -sS -A "$USER_AGENT" -H "Accept: application/vnd.github+json" \
        -o "$body" -w "%{http_code}" \
        "https://api.github.com/repos/${REPO}/releases/latest" || true)"
    if [[ "$status" == "404" ]]; then
        rm -f "$body"
        die "No GitHub release found for ${REPO}. Publish a v*.*.* tag, or build from source:
  git clone https://github.com/${REPO}.git
  cd mouse-me && cargo build --release"
    fi
    if [[ "$status" != "200" ]]; then
        local details
        details="$(tr '\n' ' ' <"$body" | head -c 200)"
        rm -f "$body"
        die "Could not read latest release (HTTP ${status})${details:+: $details}"
    fi
    local tag
    tag="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$body" | head -n 1)"
    rm -f "$body"
    [[ -n "$tag" ]] || die "Could not parse latest release tag"
    printf '%s\n' "$tag"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix)
            [[ $# -ge 2 ]] || die "--prefix needs a directory"
            PREFIX="$2"
            shift 2
            ;;
        --version)
            [[ $# -ge 2 ]] || die "--version needs a tag"
            VERSION="$2"
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            die "Unknown option: $1"
            ;;
    esac
done

need curl
need tar
need install
need sha256sum

os="$(uname -s)"
arch="$(uname -m)"
[[ "$os" == "Linux" ]] || die "Mouse Me ships Linux binaries only (found ${os})"
[[ "$arch" == "x86_64" || "$arch" == "amd64" ]] || die "No prebuilt binary for ${arch}; x86_64 Linux is required"

if [[ "$VERSION" == "latest" ]]; then
    VERSION="$(latest_tag)"
elif [[ "$VERSION" != v* ]]; then
    VERSION="v${VERSION}"
fi

bin_dir="${PREFIX}/bin"
app_dir="${PREFIX}/share/applications"
icon_dir="${PREFIX}/share/icons/hicolor/256x256/apps"
asset="mouse-me-${VERSION}-linux-x86_64.tar.gz"
base="https://github.com/${REPO}/releases/download/${VERSION}"

mkdir -p "$bin_dir" "$app_dir" "$icon_dir" || die "Cannot write to ${PREFIX}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

printf 'Downloading Mouse Me %s\n' "$VERSION"
curl -fsSL -A "$USER_AGENT" -o "${tmp}/${asset}" "${base}/${asset}"
curl -fsSL -A "$USER_AGENT" -o "${tmp}/${asset}.sha256" "${base}/${asset}.sha256"

(
    cd "$tmp"
    sha256sum -c "${asset}.sha256" >/dev/null
)

tar -xzf "${tmp}/${asset}" -C "$tmp"
[[ -f "${tmp}/mouse-me" ]] || die "Archive did not contain mouse-me"

install -Dm755 "${tmp}/mouse-me" "${bin_dir}/mouse-me"

icon_path="${icon_dir}/mouse-me.png"
if curl -fsSL -A "$USER_AGENT" -o "${tmp}/mouse-me.png" \
    "https://raw.githubusercontent.com/${REPO}/${VERSION}/assets/mouse-me.png"; then
    install -Dm644 "${tmp}/mouse-me.png" "$icon_path"
    icon_name="mouse-me"
else
    icon_name="preferences-desktop-theme"
fi

cat >"${tmp}/mouse-me.desktop" <<EOF
[Desktop Entry]
Name=Mouse Me
GenericName=Cursor Manager
Comment=Universal cursor theme manager for Omarchy and Linux
Exec=${bin_dir}/mouse-me gui
Icon=${icon_name}
Terminal=false
Type=Application
Categories=Settings;DesktopSettings;Utility;
Keywords=cursor;mouse;theme;hyprland;omarchy;xcursor;hyprcursor;
StartupNotify=true
EOF
install -Dm644 "${tmp}/mouse-me.desktop" "${app_dir}/mouse-me.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$app_dir" >/dev/null 2>&1 || true
fi

printf 'Installed %s to %s\n' "$VERSION" "${bin_dir}/mouse-me"

case ":${PATH}:" in
    *":${bin_dir}:"*) ;;
    *)
        printf 'Add %s to PATH, then reopen the terminal:\n' "$bin_dir"
        printf '  export PATH="%s:$PATH"\n' "$bin_dir"
        ;;
esac

printf 'Launch with: mouse-me\n'
