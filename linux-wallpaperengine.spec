# =============================================================================
# Name/Version/Release
# =============================================================================
Name:           linux-wallpaperengine
Version:        1.0.0
Release:        1%{?dist}
Summary:        Wallpaper Engine backgrounds for Linux

# =============================================================================
# License & URLs
# =============================================================================
License:        GPL-3.0-only
URL:            https://github.com/mtkennerly/linux-wallpaperengine
Source0:        linux-wallpaperengine-%{version}.tar.gz

# =============================================================================
# Build Requirements
# =============================================================================
# Core build tools
BuildRequires:  cmake >= 3.10
BuildRequires:  gcc-c++
BuildRequires:  make
BuildRequires:  chrpath

# Graphics dependencies
BuildRequires:  glew-devel
BuildRequires:  glfw-devel
BuildRequires:  freetype-devel
BuildRequires:  freeglut-devel
BuildRequires:  glm-devel
BuildRequires:  SDL2-devel
BuildRequires:  mesa-libGL-devel
BuildRequires:  libXrandr-devel
BuildRequires:  libXinerama-devel
BuildRequires:  libXcursor-devel
BuildRequires:  libXi-devel

# Multimedia dependencies
BuildRequires:  mpv-devel
BuildRequires:  ffmpeg-free-devel
BuildRequires:  pulseaudio-libs-devel
BuildRequires:  fftw-devel

# Data/Network dependencies
BuildRequires:  lz4-devel

# Security dependencies (required by CEF)
BuildRequires:  nss-devel
BuildRequires:  nspr-devel
BuildRequires:  zlib-devel

# X11/Wayland support
BuildRequires:  wayland-devel
BuildRequires:  wayland-protocols-devel
BuildRequires:  libXxf86vm-devel
BuildRequires:  gmp-devel

# =============================================================================
# Runtime Requirements
# =============================================================================
Requires:       glew
Requires:       glfw
Requires:       freetype
Requires:       mpv-libs
Requires:       pulseaudio-libs
Requires:       freeglut
Requires:       SDL2
Requires:       zlib
Requires:       lz4
Requires:       fftw-libs
Requires:       gmp
Requires:       glslang

# Security runtime dependencies (pulled in by CEF)
Requires:       nss
Requires:       nspr

# =============================================================================
# Package Description
# =============================================================================
%description
Linux Wallpaper Engine is an open-source implementation that allows running
Wallpaper Engine-style live wallpapers on Linux systems. It supports various
wallpaper formats including video, web, and scene-based wallpapers.

Features:
- OpenGL 3.3+ rendering
- Video wallpaper support via FFmpeg/MPV
- Web wallpaper support via CEF
- X11 and Wayland compatibility
- Multi-monitor support

%package        assets
Summary:        Asset files for linux-wallpaperengine
Requires:       %{name} = %{version}-%{release}
BuildArch:      noarch

%description assets
Asset directory containing default wallpapers and resources for linux-wallpaperengine.
This package can be installed separately to save space if you only use custom wallpapers.


# =============================================================================
# Preparation & Build
# =============================================================================
%prep
%setup -q -n linux-wallpaperengine-%{version}

# Apply patches if needed
# %patch0 -p1 -b .fix-something

%build
%cmake -DCMAKE_BUILD_TYPE=Release \
       -DCMAKE_INSTALL_PREFIX=%{_prefix} \
       -DCMAKE_SKIP_RPATH:BOOL=OFF \
       -DBUILD_TESTING=OFF \
       ..
%cmake_build

%install
%cmake_install

# Remove unwanted build tool/test binaries that CMake's install(DIRECTORY) puts in prefix
rm -f %{buildroot}%{_prefix}/spirv-cross \
      %{buildroot}%{_prefix}/glslang \
      %{buildroot}%{_prefix}/glslangValidator \
      %{buildroot}%{_prefix}/spirv-remap \
      %{buildroot}%{_prefix}/qjs \
      %{buildroot}%{_prefix}/qjsc \
      %{buildroot}%{_prefix}/run-test262 \
      %{buildroot}%{_prefix}/api-test \
      %{buildroot}%{_prefix}/function_source \
      %{buildroot}%{_prefix}/bm_kiss-float \
      %{buildroot}%{_prefix}/bm_fftw-float \
      %{buildroot}%{_prefix}/st-float \
      %{buildroot}%{_prefix}/tkfc-float \
      %{buildroot}%{_prefix}/ffr-float \
      %{buildroot}%{_prefix}/tr-float \
      %{buildroot}%{_prefix}/testcpp-float \
      %{buildroot}%{_prefix}/fastconv-float \
      %{buildroot}%{_prefix}/fastconvr-float \
      %{buildroot}%{_prefix}/fastfilt-float \
      %{buildroot}%{_prefix}/fft-float \
      %{buildroot}%{_prefix}/psdpng-float

# Remove duplicate installs from /usr/bin/ and /usr/lib64/ (cmake subproject installs)
rm -f %{buildroot}%{_bindir}/spirv-cross \
      %{buildroot}%{_bindir}/qjs \
      %{buildroot}%{_bindir}/qjsc \
      %{buildroot}%{_bindir}/fastconv-float \
      %{buildroot}%{_bindir}/fastconvr-float \
      %{buildroot}%{_bindir}/fft-float \
      %{buildroot}%{_bindir}/psdpng-float

rm -f %{buildroot}%{_libdir}/libspirv-cross-*.a

# Remove bundled library headers and CMake configs (build artifacts, not for distribution)
rm -rf %{buildroot}%{_includedir}/kissfft \
       %{buildroot}%{_includedir}/spirv_cross \
       %{buildroot}%{_includedir}/quickjs.h \
       %{buildroot}%{_libdir}/cmake/kissfft \
       %{buildroot}%{_libdir}/cmake/quickjs \
       %{buildroot}%{_libdir}/pkgconfig/kissfft-float.pc \
       %{buildroot}%{_libdir}/pkgconfig/spirv-cross-c.pc \
       %{buildroot}%{_docdir}/quickjs

# Remove bundled SPIRV-Cross cmake configs
rm -rf %{buildroot}%{_datadir}/spirv_cross_*

# Copy bundled shared libraries that CMake's install(DIRECTORY) does not cover
cp -a redhat-linux-build/lib/libglslang.so* %{buildroot}%{_prefix}/
cp -a redhat-linux-build/lib/libcef_dll_wrapper.so* %{buildroot}%{_prefix}/

# Strip build-time RPATH from manually copied shared libraries
chrpath -d %{buildroot}%{_prefix}/libglslang.so* 2>/dev/null || :
chrpath -d %{buildroot}%{_prefix}/libcef_dll_wrapper.so 2>/dev/null || :

# Create symlink in PATH (binary installed to /usr/, not /usr/bin/)
mkdir -p %{buildroot}%{_bindir}
ln -s %{_prefix}/linux-wallpaperengine %{buildroot}%{_bindir}/linux-wallpaperengine

# 创建额外目录（仅当 CMake 未安装时）
mkdir -p %{buildroot}%{_datadir}/%{name}/assets
mkdir -p %{buildroot}%{_datadir}/applications
mkdir -p %{buildroot}%{_docdir}/%{name}
mkdir -p %{buildroot}%{_mandir}/man1

# 安装文档
install -m 0644 README.md %{buildroot}%{_docdir}/%{name}/ 2>/dev/null || true


# Install man page (create if doesn't exist)
cat > %{buildroot}%{_mandir}/man1/%{name}.1 << 'EOF'
.TH LINUX-WALLPAPERENGINE 1 "2026" "Linux Wallpaper Engine" "User Commands"
.SH NAME
linux-wallpaperengine \- Wallpaper Engine backgrounds for Linux
.SH SYNOPSIS
.B linux-wallpaperengine
[\fB\-\-assets-dir\fR \fIDIR\fR]
[\fB\-\-screen-root\fR \fISCREEN\fR]
[\fB\-\-bg\fR \fIWALLPAPER\fR]
[\fB\-\-scaling\fR \fIMODE\fR]
.SH DESCRIPTION
Linux Wallpaper Engine allows running Wallpaper Engine-style live wallpapers on Linux.
.SH OPTIONS
.TP
.B \-\-assets-dir DIR
Specify the assets directory path
.TP
.B \-\-screen-root SCREEN
Specify the target screen/output name (e.g., eDP-1)
.TP
.B \-\-bg WALLPAPER
Specify the wallpaper to load
.TP
.B \-\-scaling MODE
Scaling mode: fill, fit, stretch, center
.SH SEE ALSO
.BR gtk3 (3),
.BR mpv (1),
.BR ffmpeg (1)
.SH AUTHOR
Community contributors. See https://github.com/Almamu/linux-wallpaperengine
EOF
gzip %{buildroot}%{_mandir}/man1/%{name}.1

# Create desktop entry
cat > %{name}.desktop << 'EOF'
[Desktop Entry]
Name=Linux Wallpaper Engine
Comment=Wallpaper Engine backgrounds for Linux
Exec=%{_prefix}/linux-wallpaperengine
Icon=preferences-desktop-wallpaper
Terminal=false
Type=Application
Categories=Utility;DesktopSettings;
Keywords=wallpaper;live;animated;background;
StartupNotify=false
EOF
install -m 0644 %{name}.desktop %{buildroot}%{_datadir}/applications/

# =============================================================================
# Post-install Scripts
# =============================================================================
%post
# Update desktop database
update-desktop-database &>/dev/null || :

%postun
if [ $1 -eq 0 ]; then
    update-desktop-database &>/dev/null || :
fi

# =============================================================================
# File Lists
# =============================================================================
%files
%doc README.md
%license LICENSE
%{_prefix}/linux-wallpaperengine
%{_bindir}/linux-wallpaperengine
%{_prefix}/liblinux-wallpaperengine-lib.so
%{_prefix}/libcef.so
%{_prefix}/libcef_dll_wrapper.so
%{_prefix}/libglslang.so*
%{_prefix}/libEGL.so
%{_prefix}/libGLESv2.so
%{_prefix}/libvk_swiftshader.so
%{_prefix}/vk_swiftshader_icd.json
%{_libdir}/libkissfft-float.so*
%{_libdir}/libqjs.so*
%attr(4755, root, root) %{_prefix}/chrome-sandbox
%{_prefix}/*.pak
%{_prefix}/*.bin
%{_prefix}/*.dat
%{_prefix}/locales/
%{_datadir}/applications/%{name}.desktop
%{_mandir}/man1/%{name}.1.*
%dir %{_datadir}/%{name}

%files assets
%dir %{_datadir}/%{name}/assets

# =============================================================================
# Changelog
# =============================================================================
%changelog
* Tue Jun 16 2026 CaoTurkey <cao-turkey@outlook.com> - 1.0.0-1
- Initial package for Fedora 43+
- Build with CMake following Fedora guidelines [[44]]
- Include runtime dependencies for glew, glfw, gtk3, mpv, ffmpeg [[27]]
- Add optional Wayland support packages
- Split assets into subpackage for flexibility
- Add debuginfo subpackage per Fedora standards

* Tue Jun 09 2026 CaoTurkey <cao-turkey@outlook.com> - 0.9.0-1
- Pre-release packaging
