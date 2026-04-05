# Maintainer: Gianluca Homen <gianluca@homen.dev>
pkgname=orka-git
pkgver=1.6.0
pkgrel=1
pkgdesc="AI agent orchestration server and CLI"
arch=('x86_64')
url="https://github.com/gianlucamazza/orka"
license=('MIT' 'Apache-2.0')
depends=('gcc-libs')
makedepends=('cargo' 'git')
provides=('orka' 'orka-server')
conflicts=('orka' 'orka-server')
backup=('etc/orka/orka.toml')
source=("$pkgname::git+$url.git")
sha256sums=('SKIP')

pkgver() {
	cd "$pkgname"
	printf "0.1.0.r%s.%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short HEAD)"
}

prepare() {
	cd "$pkgname"
	export RUSTUP_TOOLCHAIN=stable
	cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
	cd "$pkgname"
	export RUSTUP_TOOLCHAIN=stable
	export CARGO_TARGET_DIR=target
	cargo build --release --locked --bin orka-server --bin orka
}

check() {
	cd "$pkgname"
	export RUSTUP_TOOLCHAIN=stable
	export CARGO_TARGET_DIR=target
	cargo test --release --locked
}

package() {
	cd "$pkgname"

	# Binaries
	install -Dm755 target/release/orka-server "$pkgdir/usr/bin/orka-server"
	install -Dm755 target/release/orka "$pkgdir/usr/bin/orka"

	# systemd unit — patch ExecStart for /usr/bin
	install -Dm644 deploy/orka-server.service "$pkgdir/usr/lib/systemd/system/orka-server.service"
	sed -i 's|@BINDIR@|/usr/bin|' "$pkgdir/usr/lib/systemd/system/orka-server.service"

	# sysusers / tmpfiles
	install -Dm644 deploy/orka-server.sysusers "$pkgdir/usr/lib/sysusers.d/orka-server.conf"
	install -Dm644 deploy/orka-server.tmpfiles "$pkgdir/usr/lib/tmpfiles.d/orka-server.conf"

	# Config
	install -Dm644 orka.toml "$pkgdir/etc/orka/orka.toml"
	sed -i 's|^workspace_dir = ".*"|workspace_dir = "/var/lib/orka/workspaces"|' "$pkgdir/etc/orka/orka.toml"
	# Arch always uses pacman — append pacman commands to allowed_commands
	sed -i 's|^\(allowed_commands = \[.*\)\]|\1, "pacman -S", "pacman -Syu"]|' "$pkgdir/etc/orka/orka.toml"

	# Licenses
	install -Dm644 LICENSE-MIT "$pkgdir/usr/share/licenses/$pkgname/LICENSE-MIT"
	install -Dm644 LICENSE-APACHE "$pkgdir/usr/share/licenses/$pkgname/LICENSE-APACHE"
}
