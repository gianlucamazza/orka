Name:           orka
Version:        %{pkg_version}
Release:        1%{?dist}
Summary:        AI agent orchestration server and CLI
License:        MIT AND Apache-2.0
URL:            https://github.com/gianlucamazza/orka
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  gcc
BuildRequires:  openssl-devel
BuildRequires:  pkgconfig(systemd)
BuildRequires:  systemd-rpm-macros

Requires:       redis
Recommends:     qdrant

%description
Orka is a Rust-based AI agent orchestration platform with queueing,
multi-channel adapters, workspace prompts, skills, and observability.

%prep
%autosetup -n %{name}-%{version}

%build
cargo build --release --locked --bin orka-server --bin orka

%install
install -Dm755 target/release/orka-server %{buildroot}%{_bindir}/orka-server
install -Dm755 target/release/orka %{buildroot}%{_bindir}/orka
install -Dm644 deploy/orka-server.service %{buildroot}%{_unitdir}/orka-server.service
sed -i 's|@BINDIR@|%{_bindir}|' %{buildroot}%{_unitdir}/orka-server.service
install -Dm644 deploy/orka-server.sysusers %{buildroot}%{_sysusersdir}/orka-server.conf
install -Dm644 deploy/orka-server.tmpfiles %{buildroot}%{_tmpfilesdir}/orka-server.conf
install -Dm644 orka.toml %{buildroot}%{_sysconfdir}/orka/orka.toml
install -Dm644 LICENSE-MIT %{buildroot}%{_licensedir}/%{name}/LICENSE-MIT
install -Dm644 LICENSE-APACHE %{buildroot}%{_licensedir}/%{name}/LICENSE-APACHE

%post
%systemd_post orka-server.service
systemd-tmpfiles --create %{_tmpfilesdir}/orka-server.conf >/dev/null 2>&1 || :

%preun
%systemd_preun orka-server.service

%postun
%systemd_postun_with_restart orka-server.service

%files
%license %{_licensedir}/%{name}/LICENSE-MIT
%license %{_licensedir}/%{name}/LICENSE-APACHE
%config(noreplace) %{_sysconfdir}/orka/orka.toml
%{_bindir}/orka
%{_bindir}/orka-server
%{_unitdir}/orka-server.service
%{_sysusersdir}/orka-server.conf
%{_tmpfilesdir}/orka-server.conf

%changelog
* Tue Mar 24 2026 Orka Maintainers <maintainers@example.invalid> - 1.0.0-1
- Initial RPM packaging scaffold
