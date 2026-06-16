%global _missing_build_ids_terminate_build 0
%global debug_package %{nil}

Name:           celestia-pdf-reader
Version:        0.6.2
Release:        1%{?dist}
Summary:        PDF viewer for terminals using the Kitty image protocol (Offline Build)
License:        AGPL-3.0
URL:            https://github.com/TurkeyC/Celestia-Toolkits/tree/pdf-reader

# 注意：这里的文件名必须是你即将生成的 tarball 的名字
# 假设我们生成的包叫 fancy-cat-0.6.0-offline.tar.gz
Source0:        %{name}-%{version}-offline.tar.gz

BuildArch:      x86_64
# 不需要 BuildRequires: zig，因为我们自带了
# 但可能需要一些基础开发库，具体看 mupdf 编译需求，通常 gcc 和 glibc-devel 是默认有的
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  glibc-devel
# 如果 mupdf 需要 clang 头文件进行静态链接，可能需要添加：
BuildRequires: clang-libs
BuildRequires: patchelf 

%description
fancy-cat is a PDF viewer for terminal emulators that support the Kitty graphics protocol.
This package includes the specific Zig compiler (0.15.2) and mupdf sources, 
requiring NO network access during the build process.

%prep
%setup -q -n %{name}-%{version}
# 不需要 git submodule update，代码已经在包里了
# 不需要下载 zig，编译器已经在包里了

%build
# 【核心】使用自带的 Zig 编译器
# 路径相对于 %setup 解压后的根目录
ZIG_BIN="./build-tools/zig-x86_64-linux-0.15.2/zig"

# 确保有执行权限
chmod +x $ZIG_BIN

# 执行构建
# 使用 -Dcpu="baseline" 提高兼容性，--release=small 优化体积
$ZIG_BIN build --release=small -Dcpu="baseline"

%install
mkdir -p %{buildroot}/%{_bindir}
cp zig-out/bin/fancy-cat %{buildroot}/%{_bindir}/

# 清除 RPATH
patchelf --remove-rpath %{buildroot}/%{_bindir}/fancy-cat

# 可选：验证
echo "Build-ID injected:"
readelf -n %{buildroot}/%{_bindir}/fancy-cat | grep "Build ID" || echo "Warning: Build ID not found"


%files
%{_bindir}/fancy-cat

%changelog
* Mon Jun 15 2026 CaoTurkey <cao-turkey@outlook.com> - 0.6.2-1
- Initial offline build package
- Bundled Zig 0.15.2 compiler to avoid version conflicts
- Bundled mupdf submodule sources to avoid network access
