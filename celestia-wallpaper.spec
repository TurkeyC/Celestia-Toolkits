Name:           celestia-wallpaper
Version:        1.8.0
Release:        1%{?dist}
Summary:        Video wallpaper player using mpv for wlroots-based Wayland compositors

License:        GPLv3+
URL:            https://github.com/GhostNaN/mpvpaper
Source0:        celestia-wallpaper-%{version}.tar.gz

# Rust 包必需的构建依赖（提供 cargo_build / cargo_install 等 RPM 宏）
BuildRequires:  rust-packaging
BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  pkgconfig(wayland-client)
BuildRequires:  pkgconfig(wayland-egl)
BuildRequires:  pkgconfig(mpv)

Requires:       mesa-libEGL
Requires:       mpv-libs

%description
Celestia-WallPaper is a video wallpaper program for wlroots-based Wayland
compositors (such as Sway, Hyprland, river). It plays videos via mpv and
renders them as your desktop wallpaper through the wlr-layer-shell protocol.

This is a complete Rust rewrite of the original C mpvpaper (hence the
"mpvpaper" references in the codebase), offering:
- Type-safe Wayland/EGL/mpv bindings via Rust crates
- Memory safety through Rust's ownership system
- Thread safety checked at compile time
- Event-driven rendering via eventfd (no busy-waiting)
- Direct /proc scanning instead of subprocess spawning

%prep
%setup -q -n %{name}-%{version}

%build
%cargo_build

%install
# cargo_build --profile rpm 产物在 target/rpm/ 下
install -D -m 0755 target/rpm/celestia-wallpaper %{buildroot}%{_bindir}/celestia-wallpaper
install -D -m 0755 target/rpm/celestia-wallpaper-holder %{buildroot}%{_bindir}/celestia-wallpaper-holder

%files
%{_bindir}/celestia-wallpaper
%{_bindir}/celestia-wallpaper-holder
%doc README.md
%license LICENSE

%changelog
* Wed Jun 17 2026 CaoTurkey <cao-turkey@outlook.com> - 1.8.0-1
- Initial Copr build for Fedora 43+
- Rust rewrite of mpvpaper, renamed to Celestia-WallPaper
