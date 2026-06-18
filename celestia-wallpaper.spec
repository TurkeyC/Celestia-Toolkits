Name:           celestia-wallpaper
Version:        0.1.2
Release:        1%{?dist}
Summary:        Multi-type wallpaper player for wlroots-based Wayland compositors

License:        GPLv3+
URL:            https://github.com/TurkeyC/celestia-wallpapers
Source0:        celestia-wallpaper-%{version}.tar.gz

# ---- Rust 构建工具链 --------------------------------------------------
BuildRequires:  rust-packaging
BuildRequires:  cargo
BuildRequires:  rust

# ---- 系统库（pkgconfig）------------------------------------------------
BuildRequires:  pkgconfig(wayland-client)
BuildRequires:  pkgconfig(wayland-egl)
BuildRequires:  pkgconfig(mpv)
BuildRequires:  pkgconfig(luajit)

# ---- 运行时依赖（soname 自动处理）-------------------------------------
Requires:       mesa-libEGL
Requires:       mpv-libs
Requires:       luajit

%description
Celestia-WallPaper is a multi-type wallpaper player for wlroots-based Wayland
compositors (such as Sway, Hyprland, river). It supports:
- Video wallpaper via libmpv
- Static image wallpaper (PNG, JPEG, BMP, WebP)
- Spine 2D skeleton animation wallpaper (JSON/binary + TOML/Lua scripting)
- Web/HTML wallpaper (JavaScript, CSS animations)

This is a complete Rust rewrite of mpvpaper (formerly the C project of the same
name), offering:
- Type-safe Wayland/EGL/mpv bindings via Rust crates
- Memory safety through Rust's ownership system
- Thread safety checked at compile time
- Event-driven rendering via eventfd (no busy-waiting)
- Direct /proc scanning instead of subprocess spawning

%prep
%setup -q -n %{name}-%{version}

%build
# 注意：cargo_build 已经隐含 --profile rpm，产物在 target/rpm/ 下
%cargo_build

%install
# workspace 根是 virtual manifest，cargo_install 不适用，手动安装
install -D -m 0755 target/rpm/celestia-wallpaper %{buildroot}%{_bindir}/celestia-wallpaper
install -D -m 0755 target/rpm/celestia-wallpaper-holder %{buildroot}%{_bindir}/celestia-wallpaper-holder

%check
%cargo_test

%files
%{_bindir}/celestia-wallpaper
%{_bindir}/celestia-wallpaper-holder
%doc README.md
%license LICENSE

%changelog
* Wed Jun 17 2026 CaoTurkey <cao-turkey@outlook.com> - 0.1.2-1
- Initial Copr build for Fedora 43+
- Rust rewrite of mpvpaper, renamed to Celestia-WallPaper
- Supported types: video, picture, spine (2D skeleton), web (planned)
