#!/usr/bin/env python3
"""Generate build.ninja for PixelTerm-C."""

import subprocess
import sys
import os

def sh(cmd, default=""):
    try:
        return subprocess.check_output(cmd, shell=True, text=True).strip()
    except subprocess.CalledProcessError:
        return default

def get_pkg_config(args):
    """Run pkg-config and return output."""
    cmd = f"pkg-config {args}"
    result = sh(cmd)
    if not result and "Error" in args:
        print(f"Warning: pkg-config {args} returned empty", file=sys.stderr)
    return result

def main():
    # --- Gather flags ---
    version = sh("git describe --tags --exact-match 2>/dev/null || "
                  "git describe --tags --always --dirty 2>/dev/null | cut -d'-' -f1 | cut -c2- || "
                  "echo 'unknown'")

    build_dir = os.path.dirname(os.path.abspath(__file__))
    src_dir = os.path.join(build_dir, "src")
    inc_dir = os.path.join(build_dir, "include")
    obj_dir = os.path.join(build_dir, "obj")
    bin_dir = os.path.join(build_dir, "bin")

    os.makedirs(obj_dir, exist_ok=True)
    os.makedirs(bin_dir, exist_ok=True)

    # Core dependency modules
    core_deps = "chafa gdk-pixbuf-2.0 gio-2.0 libavformat libavcodec libswscale libavutil"

    pkg_cflags = get_pkg_config(f"--cflags glib-2.0 {core_deps}")
    pkg_libs = get_pkg_config(f"--libs {core_deps}")

    # Check optional libs
    extra_libs = ""
    if sh("pkg-config --exists zlib 2>/dev/null && echo yes") == "yes":
        extra_libs += " " + get_pkg_config("--libs zlib")
    else:
        extra_libs += " -lz"

    for pkg in ["openjp2", "libopenjp2"]:
        if sh(f"pkg-config --exists {pkg} 2>/dev/null && echo yes") == "yes":
            extra_libs += " " + get_pkg_config(f"--libs {pkg}")
            break

    mupdf_available = sh("pkg-config --exists mupdf 2>/dev/null && echo yes") == "yes"
    mupdf_libs = ""
    mupdf_cflags = ""
    if mupdf_available:
        mupdf_libs = " " + get_pkg_config("--libs mupdf")
        if sh("pkg-config --exists harfbuzz 2>/dev/null && echo yes") == "yes":
            mupdf_libs += " " + get_pkg_config("--libs harfbuzz")
        mupdf_cflags = " " + get_pkg_config("--cflags mupdf")

    # --- Source file list ---
    cflags_common = ("-Wall -Wextra -std=c11 -O2 "
                     "-Wno-sign-compare -Wno-unused-variable "
                     "-Wno-unused-but-set-variable -Wno-switch "
                     "-ffunction-sections -fdata-sections")
    includes = f"-I{inc_dir} {pkg_cflags}{mupdf_cflags}"
    ldflags = ("-Wl,-rpath -Wl,/usr/local/lib "
               "-Wl,--gc-sections -Wl,--no-as-needed")
    libs = f"{pkg_libs} -lpthread -lm{extra_libs}{mupdf_libs}"

    src_files = sorted(os.listdir(src_dir))
    c_files = sorted([f for f in src_files if f.endswith(".c")])

    # Ensure video_player.c is linked last (referencing video_player_*.c symbols)
    if "video_player.c" in c_files:
        c_files.remove("video_player.c")
        c_files.append("video_player.c")

    defines = f'-DAPP_VERSION=\\"{version}\\"'
    if mupdf_available:
        defines += " -DHAVE_MUPDF"

    # --- Generate build.ninja ---
    ninja = '''
ninja_required_version = 1.3

cc = gcc
cflags = {cflags_common}
defines = {defines}
includes = {includes}
ldflags = {ldflags}
libs = {libs}

rule cc
  command = $cc $defines $cflags $includes -MMD -MF $out.d -c $in -o $out
  depfile = $out.d
  description = CC $in

rule link
  command = $cc $cflags $ldflags -o $out $in $libs
  description = LINK $out

rule clean
  command = rm -rf obj bin
  description = CLEAN

build obj/build.stamp: cc | clean
  cflags = $cflags
  defines = $defines
  includes = $includes
  ldflags = $ldflags
  libs = $libs
  command = echo "PixelTerm-C build configured" > $out
  description = CONFIGURE

'''.format(
        cflags_common=cflags_common,
        defines=defines,
        includes=includes,
        ldflags=ldflags,
        libs=libs,
    )

    # Per-object build rules
    for cf in c_files:
        src_path = f"src/{cf}"
        obj_path = f"obj/{cf.replace('.c', '.o')}"
        ninja += f"build {obj_path}: cc {src_path}\n"

    # Link rule — Ninja deps must be on one line
    obj_line = " ".join([f"obj/{f.replace('.c', '.o')}" for f in c_files])
    ninja += f"build bin/pixelterm: link {obj_line}\n"
    ninja += "\nbuild all: phony bin/pixelterm\n"
    ninja += "default all\n"

    # Test targets (if any exist)
    test_dir = os.path.join(build_dir, "tests")
    if os.path.isdir(test_dir):
        test_sources = sorted([f for f in os.listdir(test_dir) if f.endswith(".c")])

        # Link objects reused across tests
        common_test_objs = [
            "obj/common.o", "obj/browser.o", "obj/renderer.o",
            "obj/gif_player.o", "obj/input.o", "obj/text_utils.o",
            "obj/process_env.o", "obj/pixbuf_utils.o", "obj/media_buffer.o",
            "obj/preloader.o", "obj/app_mode.o",
            "obj/input_dispatch_pending_clicks.o", "obj/input_dispatch_delete.o",
            "obj/input_dispatch_core.o", "obj/input_dispatch_key_single.o",
            "obj/input_dispatch_key_book.o", "obj/input_dispatch_key_file_manager.o",
            "obj/input_dispatch_mouse_modes.o", "obj/app_preview_shared.o",
            "obj/app_media_session.o", "obj/app_single_render.o",
            "obj/app_config_runtime.o", "obj/media_utils.o",
            "obj/ui_render_utils.o", "obj/video_player_clock.o",
            "obj/video_player_debug.o", "obj/video_player_decode.o",
            "obj/video_player_layout.o", "obj/video_player_playback.o",
            "obj/video_player_seek.o", "obj/video_player.o",
            "obj/terminal_probe.o", "obj/terminal_protocols.o",
            "obj/terminal_protocol_resolver.o", "obj/app_cli.o",
            "obj/book.o", "obj/app_startup.o",
        ]

        excluded_main = {"test_app_file_manager", "test_app_preview_grid", "test_app_preview_book"}
        has_common = "test_common.c" in test_sources
        main_test_link_objs = " ".join(common_test_objs)

        # Compile rules for all tests
        for ts in test_sources:
            base = ts.replace(".c", "")
            to = f"obj/{base}.o"
            ninja += f"build {to}: cc tests/{ts}\n"

        # Main test binary (test_common.c provides main())
        if has_common:
            main_objs = ["obj/test_common.o"]
            for ts in test_sources:
                base = ts.replace(".c", "")
                if base not in excluded_main and base != "test_common":
                    main_objs.append(f"obj/{base}.o")
            all_main = " ".join(main_objs) + " " + main_test_link_objs
            ninja += f"build bin/pixelterm-tests: link {all_main}\n"
            ninja += "build test: phony bin/pixelterm-tests\n"

        # Special test binaries (each provides its own main())
        special = {
            "test_app_file_manager": "obj/common.o obj/process_env.o obj/app_core.o obj/app_mode.o obj/app_file_manager.o obj/app_file_manager_render.o obj/text_utils.o",
            "test_app_preview_grid": "obj/app_preview_grid.o obj/ui_render_utils.o obj/text_utils.o",
            "test_app_preview_book": "obj/app_preview_book.o",
        }
        for base, extra in special.items():
            if f"{base}.c" not in test_sources:
                continue
            to = f"obj/{base}.o"
            name = base.replace("test_", "").replace("_", "-")
            all_o = f"{to} {extra}"
            ninja += f"build bin/pixelterm-{name}-tests: link {all_o}\n"

    write_path = os.path.join(build_dir, "build.ninja")
    with open(write_path, "w") as f:
        f.write(ninja.strip() + "\n")

    print(f"Generated: {write_path}")
    print(f"Version:   {version}")
    print(f"MuPDF:     {'yes' if mupdf_available else 'no'}")
    print(f"Sources:   {len(c_files)} C files")
    print(f"Target:    bin/pixelterm")
    print()
    print("Run: ninja")

if __name__ == "__main__":
    main()
