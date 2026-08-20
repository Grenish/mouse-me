# Maintainer: Grenish Rai <grenishrai@gmail.com>
pkgname=mouse-me
pkgver=0.1.0
pkgrel=1
pkgdesc="Universal Linux & Omarchy cursor manager with modern Slint GUI & fast CLI"
arch=('x86_64')
url="https://github.com/grenishrai/mouse-me"
license=('GPL-3.0-or-later')
depends=('gcc-libs' 'glibc' 'fontconfig' 'freetype2' 'libxkbcommon' 'wayland')
makedepends=('cargo' 'pkgconf')
source=("$pkgname-$pkgver.tar.gz::$url/archive/v$pkgver.tar.gz")
sha256sums=('SKIP')

build() {
    cd "$pkgname-$pkgver"
    cargo build --release --locked
}

package() {
    cd "$pkgname-$pkgver"
    install -Dm755 "target/release/mouse-me" "$pkgdir/usr/bin/mouse-me"
    install -Dm644 "mouse-me.desktop" "$pkgdir/usr/share/applications/mouse-me.desktop"
}
